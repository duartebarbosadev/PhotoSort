use super::*;
use crate::core::delete_review::{self, Category};
impl PhotoSortApp {
    pub(super) fn finish_delete_analysis(&mut self) {
        let entries = delete_review::from_analysis(
            &self.media,
            &self.analysis,
            &self.metadata,
            &self.delete_pairs,
        );
        let mut changed: std::collections::HashSet<_> = self
            .duplicates
            .keys()
            .chain(self.near_duplicates.keys())
            .copied()
            .collect();
        self.duplicates.clear();
        self.near_duplicates.clear();
        for pair in &self.delete_pairs {
            let map = if pair.assessment.decision
                == crate::core::near_duplicates::Decision::ExactDuplicate
            {
                &mut self.duplicates
            } else {
                &mut self.near_duplicates
            };
            map.insert(pair.discard, pair.keep);
            changed.insert(pair.discard);
        }
        for id in changed {
            self.update_browse_item(id);
        }
        self.delete_review.replace(entries);
        self.view_data_ready.insert(Step::EasyDelete);
        self.delete_categories = self
            .pending_bookmark
            .as_ref()
            .map(|v| v.delete_categories.iter().copied().collect())
            .unwrap_or_else(|| Category::ALL.into_iter().collect());
        self.delete_run = None;
        self.filter_dirty = true;
        self.set_workflow_status(
            Step::EasyDelete,
            format!(
                "{} reviews · choices stay provisional until confirmed",
                self.delete_review.entries().len()
            ),
        );
    }
    pub(super) fn delete_review_ids(&self) -> Vec<MediaId> {
        self.filtered.iter().map(|i| self.media[*i].id).collect()
    }
    pub(super) fn toggle_delete_candidate(&mut self, side: usize) {
        if self.busy || self.delete_run.is_some() {
            return;
        }
        if let Some(review) = self.delete_current
            && let Some(candidate) = self
                .delete_review
                .entry(review)
                .and_then(|e| e.candidates().get(side).copied())
        {
            self.delete_review
                .toggle(review, candidate, &mut self.decisions);
            self.active = Some(candidate);
            self.focused_slot = side as u64;
            self.review_changed();
        }
    }
    pub(super) fn confirm_delete(&mut self) {
        if self.busy || self.delete_run.is_some() {
            return;
        }
        if let Some(id) = self.delete_current {
            if self.delete_review.confirm(id, &mut self.decisions) {
                let rows = self.delete_review_ids();
                if let Some(index) = rows.iter().position(|other| *other == id)
                    && let Some(next) = rows[index + 1..]
                        .iter()
                        .find(|id| !self.delete_review.confirmed(**id))
                {
                    self.set_active(*next);
                }
            }
            self.review_changed();
        }
    }
    pub(super) fn reset_delete(&mut self, all: bool) {
        if all {
            self.delete_review.reset_all(&mut self.decisions);
        } else if let Some(id) = self.delete_current {
            self.delete_review.reset(id, &mut self.decisions);
        }
        self.review_notice = None;
        self.review_changed();
    }
    pub(super) fn accept_delete_defaults(&mut self) {
        if !self.allow_review_departure() {
            return;
        }
        let ids = self.delete_review_ids();
        self.delete_review
            .accept_defaults(&ids, &mut self.decisions);
        self.review_changed();
    }
    pub(super) fn delete_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            for category in Category::ALL {
                let count = self
                    .delete_review
                    .entries()
                    .iter()
                    .filter(|e| e.category == category)
                    .count();
                if count == 0 {
                    continue;
                }
                let mut enabled = self.delete_categories.contains(&category);
                if ui
                    .checkbox(&mut enabled, format!("{} ({count})", category.title()))
                    .changed()
                    && self.allow_review_departure()
                {
                    if enabled {
                        self.delete_categories.insert(category);
                    } else {
                        self.delete_categories.remove(&category);
                    }
                    self.filter_dirty = true;
                }
            }
        });
        ui.horizontal_wrapped(|ui| {
            if let Some(id) = self.delete_current {
                if ui
                    .button(if self.delete_review.confirmed(id) {
                        "Cancel confirmation"
                    } else {
                        "Confirm →"
                    })
                    .clicked()
                {
                    self.confirm_delete();
                }
                if ui.button("Reset default").clicked() {
                    self.reset_delete(false);
                }
                if ui.button("Reset all").clicked() {
                    self.reset_delete(true);
                }
            }
            if ui
                .add_enabled(
                    !self.filtered.is_empty(),
                    egui::Button::new("Accept filtered defaults"),
                )
                .clicked()
            {
                self.accept_delete_defaults();
            }
            if ui
                .add_enabled(
                    self.decisions.values().any(|d| d.rejected),
                    egui::Button::new("Review marked files…"),
                )
                .clicked()
            {
                self.review_changes();
            }
        });
        if let Some(id) = self.delete_current
            && let Some(entry) = self.delete_review.entry(id)
        {
            ui.label(RichText::new(&entry.reason).small().color(MUTED));
            ui.label(
                RichText::new(if self.delete_review.confirmed(id) {
                    "Confirmed · shared Trash marks updated"
                } else {
                    "Choose Keep or Trash for each photo, then confirm. Files stay unchanged."
                })
                .small()
                .color(ACCENT),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> PhotoSortApp {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.media = (1..=3)
            .map(|n| Media {
                id: MediaId(n),
                path: format!("/{n}.jpg").into(),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint::default(),
            })
            .collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.delete_review.replace(vec![
            delete_review::Entry {
                subject: MediaId(1),
                companion: Some(MediaId(2)),
                category: Category::Exact,
                reason: "copy".into(),
            },
            delete_review::Entry {
                subject: MediaId(3),
                companion: None,
                category: Category::Dark,
                reason: "dark".into(),
            },
        ]);
        app.set_step(Step::EasyDelete);
        app.refilter();
        app
    }
    #[test]
    fn confirm_and_quit_in_one_frame_cannot_bypass_the_shared_departure_guard() {
        for smoke in [false, true] {
            let mut app = app();
            let ctx = egui::Context::default();
            if smoke {
                app = app.with_smoke_test(Duration::from_millis(200));
                let _ = ctx.run(
                    egui::RawInput {
                        time: Some(0.0),
                        ..Default::default()
                    },
                    |ctx| {
                        assert!(!app.smoke_close_requested(ctx));
                    },
                );
            }
            let mut input = egui::RawInput {
                time: Some(0.3),
                ..Default::default()
            };
            input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            let output = ctx.run(input, |ctx| app.update_workspace(ctx, !smoke));
            assert!(app.decisions[&MediaId(1)].rejected);
            assert!(!app.close_authorized);
            assert!(matches!(app.departure, Some(Destination::Close)));
            assert!(
                !output.viewport_output[&egui::ViewportId::ROOT]
                    .commands
                    .iter()
                    .any(|c| matches!(c, egui::ViewportCommand::Close))
            );
        }
    }
    #[test]
    fn duplicate_review_and_filters_publish_together_only_after_completed_run() {
        let mut app = app();
        let original = app.delete_review.entries().to_vec();
        app.analysis_run = Some(41);
        app.delete_run = Some(41);
        for media in &app.media {
            app.analysis.insert(media.id, Default::default());
        }
        let pairs = vec![crate::core::near_duplicates::batch::Pair {
            discard: MediaId(2),
            keep: MediaId(1),
            reason: "Exact copy".into(),
            assessment: crate::core::near_duplicates::Comparator::default().assess(
                None,
                None,
                true,
                |_| None,
                || false,
            ),
        }];
        let deliver = |app: &mut PhotoSortApp, event| {
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            app.runtime.events = rx;
            tx.send(crate::core::jobs::Envelope {
                epoch: app.runtime.epoch(),
                event,
            })
            .unwrap();
            app.poll(&egui::Context::default());
        };
        deliver(
            &mut app,
            Event::DuplicatePairs {
                run: 40,
                pairs: pairs.clone(),
            },
        );
        assert!(app.delete_pairs.is_empty());
        deliver(&mut app, Event::DuplicatePairs { run: 41, pairs });
        assert_eq!(app.delete_review.entries(), original);
        assert!(app.duplicates.is_empty());
        deliver(
            &mut app,
            Event::AnalysisDone {
                run: 41,
                stats: Default::default(),
            },
        );
        assert_eq!(app.delete_review.entries().len(), 1);
        assert_eq!(app.delete_review.entries()[0].subject, MediaId(2));
        assert_eq!(app.duplicates.get(&MediaId(2)), Some(&MediaId(1)));
        assert!(app.decisions.is_empty());
    }
    #[test]
    fn cancelled_duplicate_pass_preserves_review_and_rejects_late_pairs() {
        let mut app = app();
        let original = app.delete_review.entries().to_vec();
        app.analysis_run = Some(41);
        app.delete_run = Some(41);
        app.cancel_analysis();
        assert_eq!(app.delete_review.entries(), original);
        let (tx, rx) = std::sync::mpsc::sync_channel(2);
        app.runtime.events = rx;
        for event in [
            Event::DuplicatePairs {
                run: 41,
                pairs: Vec::new(),
            },
            Event::AnalysisDone {
                run: 41,
                stats: Default::default(),
            },
        ] {
            tx.send(crate::core::jobs::Envelope {
                epoch: app.runtime.epoch(),
                event,
            })
            .unwrap();
        }
        app.poll(&egui::Context::default());
        assert_eq!(app.delete_review.entries(), original);
        assert!(app.decisions.is_empty());
    }
    #[test]
    fn pending_choice_blocks_navigation_and_departure_without_losing_focused_pair() {
        let mut app = app();
        app.toggle_delete_candidate(1);
        assert_eq!(app.active, Some(MediaId(2)));
        assert_eq!(app.delete_current, Some(MediaId(1)));
        app.navigate_selection(1, false, false);
        assert_eq!(app.delete_current, Some(MediaId(1)));
        assert_eq!(app.review_notice, Some(Step::EasyDelete));
        app.request_departure(Destination::Step(Step::Cull));
        assert_eq!(app.step, Step::EasyDelete);
        assert!(app.decisions.is_empty());
        app.reset_delete(false);
        app.navigate_selection(1, false, false);
        assert_eq!(app.delete_current, Some(MediaId(3)));
    }
    #[test]
    fn confirmation_advances_and_departure_requires_resolving_shared_marks() {
        let mut app = app();
        app.confirm_delete();
        assert_eq!(app.delete_current, Some(MediaId(3)));
        assert!(app.decisions[&MediaId(1)].rejected);
        app.request_departure(Destination::Step(Step::Cull));
        assert!(app.departure.is_some());
        assert_eq!(app.step, Step::EasyDelete);
        app.departure = None;
        app.reset_delete(true);
        app.request_departure(Destination::Step(Step::Cull));
        assert_eq!(app.step, Step::Cull);
    }
    #[test]
    fn generic_card_mark_is_provisional_in_easy_delete_and_filtered_accept_is_scoped() {
        let mut app = app();
        app.reject(MediaId(1));
        assert!(app.decisions.is_empty());
        assert!(app.delete_review.unfinished(app.delete_current));
        app.reset_delete(false);
        app.delete_categories.remove(&Category::Dark);
        app.filter_dirty = true;
        app.refilter();
        app.accept_delete_defaults();
        assert!(app.decisions[&MediaId(1)].rejected);
        assert!(!app.decisions.contains_key(&MediaId(3)));
    }
}
