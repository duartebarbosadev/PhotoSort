use super::review_card::{Badge, CardButton, CardEvent, CardPhoto, CardState, ReviewCard};
use super::*;
use crate::core::delete_review::{self, Category, Entry};

/// A card's title: what kind of problem the suggestion is about.
fn card_title(category: Category) -> &'static str {
    match category {
        Category::Exact => "Duplicate",
        Category::Near => "Near-duplicate",
        Category::Blur => "Blurry",
        Category::Dark => "Too dark",
        Category::Bright => "Too bright",
    }
}

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
            match self.delete_review.entries().len() {
                0 => "Nothing to clean up · no duplicates or problem shots found".to_owned(),
                1 => "1 suggestion · choices stay provisional until confirmed".to_owned(),
                n => format!("{n} suggestions · choices stay provisional until confirmed"),
            } + &self.failure_note(),
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
    /// The categories to review, as filter chips with their counts. The
    /// decision and Confirm live under the photos in Review and on each card
    /// in the grid; bulk actions in More.
    pub(super) fn delete_controls(&mut self, ui: &mut egui::Ui) {
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
            let enabled = self.delete_categories.contains(&category);
            if chip(ui, enabled, &format!("{} ({count})", category.title()))
                .on_hover_text(if enabled {
                    "Hide this kind of suggestion"
                } else {
                    "Show this kind of suggestion"
                })
                .clicked()
                && self.allow_review_departure()
            {
                if enabled {
                    self.delete_categories.remove(&category);
                } else {
                    self.delete_categories.insert(category);
                }
                self.filter_dirty = true;
            }
        }
    }
    pub(super) fn delete_menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                !self.filtered.is_empty(),
                egui::Button::new(format!(
                    "Accept suggestions ({} shown) · A",
                    self.filtered.len()
                )),
            )
            .on_hover_text(
                "Confirm the suggested Keep/Trash choice for every photo in the categories shown",
            )
            .clicked()
        {
            self.accept_delete_defaults();
            ui.close();
        }
        ui.add_enabled_ui(self.delete_current.is_some(), |ui| {
            if ui.button("Reset this photo · R").clicked() {
                self.reset_delete(false);
                ui.close();
            }
            if ui.button("Reset all · Shift+R").clicked() {
                self.reset_delete(true);
                ui.close();
            }
        });
    }
    /// Why the current photo was suggested, in one line.
    pub(super) fn delete_context(&mut self, ui: &mut egui::Ui) {
        if let Some(id) = self.delete_current
            && let Some(entry) = self.delete_review.entry(id)
        {
            let state = if self.delete_review.confirmed(id) {
                "Confirmed · the Trash marks are shared with every step."
            } else {
                "Choose Keep or Trash for each photo, then confirm. Files stay unchanged until you review changes."
            };
            ui.horizontal(|ui| {
                ui.add(egui::Label::new(RichText::new(&entry.reason).color(INK)).truncate())
                    .on_hover_text(state);
            });
        }
    }
    /// The confirmation under the compared photos.
    pub(super) fn delete_confirm_button(&mut self, ui: &mut egui::Ui) {
        if let Some(id) = self.delete_current {
            let confirmed = self.delete_review.confirmed(id);
            let confirm = if confirmed {
                ui.button("Cancel confirmation")
            } else {
                ui.add(primary("Confirm"))
            };
            if confirm.on_hover_text("Enter").clicked() {
                self.confirm_delete();
            }
        }
    }
}

impl PhotoSortApp {
    /// Easy Delete's grid: one card per suggestion, a pair side by side or a
    /// single photo, with what PhotoSort suggests and the same Keep/Trash,
    /// Confirm and Reset as Review. The focused card is Review's current
    /// suggestion, so the keyboard shortcuts act on it in either view.
    pub(super) fn delete_grid(&mut self, ui: &mut egui::Ui) {
        let subjects = self.delete_review_ids();
        let current = self
            .delete_current
            .and_then(|id| subjects.iter().position(|other| *other == id));
        let clicked = self.card_grid(
            ui,
            "delete",
            subjects.len(),
            2,
            |app, i| {
                let entry = app.delete_review.entry(subjects[i])?.clone();
                Some(app.delete_card(&entry))
            },
            current,
        );
        if let Some((i, event)) = clicked {
            self.delete_card_event(subjects[i], event);
        }
    }

    /// The card for one suggestion: its photos with their Keep/Trash
    /// choice, the reason, and Keep/Trash per photo, Reset and Confirm.
    fn delete_card(&self, entry: &Entry) -> ReviewCard {
        let subject = entry.subject;
        let confirmed = self.delete_review.confirmed(subject);
        let changed = self.delete_review.unfinished(Some(subject));
        let enabled = !self.busy && self.delete_run.is_none();
        let candidates = entry.candidates();
        let mut buttons: Vec<CardButton> = candidates
            .iter()
            .enumerate()
            .map(|(side, id)| {
                let keep = self.delete_review.keeps(subject, *id);
                CardButton::new(
                    decision_label(side, keep),
                    format!(
                        "Toggle photo {} · {} on the selected card",
                        side + 1,
                        side + 1
                    ),
                )
                .enabled(enabled)
            })
            .collect();
        buttons.push(
            CardButton::new(
                if confirmed {
                    "Cancel confirmation"
                } else {
                    "Confirm"
                },
                "Enter on the selected card",
            )
            .primary(!confirmed)
            .enabled(enabled)
            .trailing(),
        );
        buttons.push(
            CardButton::new("Reset", "Back to the suggestion · R on the selected card")
                .enabled(enabled && (confirmed || changed))
                .trailing(),
        );
        ReviewCard {
            title: card_title(entry.category).into(),
            state: if confirmed {
                CardState::Confirmed
            } else if changed {
                CardState::Changed
            } else {
                CardState::Provisional
            },
            state_text: None,
            photos: candidates
                .iter()
                .map(|id| {
                    let media = self.index.get(id).map(|i| &self.media[*i]);
                    CardPhoto {
                        id: *id,
                        turns: self.preview_turns(*id),
                        badge: Badge::keep(self.delete_review.keeps(subject, *id)),
                        suggested: false,
                        caption: media.map(Media::name).unwrap_or_default(),
                        name: media
                            .map(|m| m.path.display().to_string())
                            .unwrap_or_default(),
                    }
                })
                .collect(),
            note: entry.reason.clone(),
            warning: false,
            buttons,
            current: self.delete_current == Some(subject),
        }
    }

    /// Applies a card's event to its suggestion. The card becomes the
    /// current suggestion first, through the same guard as Review: while
    /// another card has an unconfirmed change, nothing happens to this one.
    pub(super) fn delete_card_event(&mut self, subject: MediaId, event: CardEvent) {
        self.pending_bookmark = None;
        if self.delete_current != Some(subject) && self.delete_review.entry(subject).is_some() {
            self.set_active(subject);
        }
        if self.delete_current != Some(subject) {
            return;
        }
        let sides = self
            .delete_review
            .entry(subject)
            .map_or(0, |e| e.candidates().len());
        match event {
            CardEvent::Select(side) | CardEvent::Open(side) => {
                if let Some(id) = self
                    .delete_review
                    .entry(subject)
                    .and_then(|e| e.candidates().get(side).copied())
                {
                    self.active = Some(id);
                    self.focused_slot = side as u64;
                }
                if matches!(event, CardEvent::Open(_)) {
                    self.open_review();
                }
            }
            CardEvent::Button(side) if side < sides => self.toggle_delete_candidate(side),
            CardEvent::Button(index) if index == sides => self.confirm_delete(),
            CardEvent::Button(_) => self.reset_delete(false),
        }
    }
}

/// The Keep/Trash toggle's label for one side, as on Review's bar.
pub(super) fn decision_label(side: usize, keep: bool) -> String {
    use egui_phosphor::regular as icon;
    format!(
        "{}  {}  {}",
        side + 1,
        if keep { icon::CHECK } else { icon::TRASH },
        if keep { "Keep" } else { "Trash" }
    )
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
            theme_install(&ctx);
            if smoke {
                app = app.with_smoke_test(Duration::from_millis(200));
                // The smoke run started 300 ms ago, so this frame requests quit.
                let started = Instant::now() - Duration::from_millis(300);
                let _ = ctx.run_test(egui::RawInput::default(), |ui| {
                    let ctx = &ui.ctx().clone();
                    {
                        assert!(!app.smoke_close_requested_at(ctx, started));
                    }
                });
            }
            let mut input = egui::RawInput::default();
            input.events.push(egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            });
            let output = ctx.run_test(input, |ui| app.update_workspace(ui, !smoke));
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
    #[test]
    fn grid_shows_one_card_per_suggestion_with_its_suggestion_and_state() {
        let mut app = app();
        let shown = super::super::review_card::render_grid(&mut app);
        for text in [
            "Duplicate",
            "Too dark",
            "copy",
            "dark",
            "Provisional",
            "Confirm",
        ] {
            assert!(shown.contains(text), "{text} missing from:\n{shown}");
        }
        // The pair is one card: both photos, one Trash and one Keep pill.
        assert!(shown.contains("1.jpg") && shown.contains("2.jpg"));
        assert_eq!(shown.matches("Duplicate").count(), 1);
        assert!(
            app.decisions.is_empty(),
            "Drawing the grid changed a decision"
        );
        // Category chips filter the grid like Review.
        app.delete_categories.remove(&Category::Dark);
        app.filter_dirty = true;
        app.refilter();
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(!shown.contains("Too dark"));
        app.confirm_delete();
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(shown.contains("Confirmed") && shown.contains("Cancel confirmation"));
    }
    #[test]
    fn card_actions_focus_their_suggestion_and_never_bypass_a_pending_choice() {
        let mut app = app();
        assert_eq!(app.delete_current, Some(MediaId(1)));
        // Keep the dark photo: its card becomes Review's current suggestion.
        app.delete_card_event(MediaId(3), CardEvent::Button(0));
        assert_eq!(app.delete_current, Some(MediaId(3)));
        assert!(app.delete_review.keeps(MediaId(3), MediaId(3)));
        assert!(
            app.decisions.is_empty(),
            "A provisional choice changed a Trash mark"
        );
        // The pair's card cannot be confirmed while that choice is pending.
        app.delete_card_event(MediaId(1), CardEvent::Button(2));
        assert_eq!(app.delete_current, Some(MediaId(3)));
        assert_eq!(app.review_notice, Some(Step::EasyDelete));
        assert!(app.decisions.is_empty());
        app.review_notice = None;
        // Reset on the dark card, then confirm the pair from its card.
        app.delete_card_event(MediaId(3), CardEvent::Button(2));
        assert!(!app.delete_review.keeps(MediaId(3), MediaId(3)));
        app.delete_card_event(MediaId(1), CardEvent::Button(2));
        assert!(app.decisions[&MediaId(1)].rejected);
        assert!(!app.decisions.get(&MediaId(2)).is_some_and(|d| d.rejected));
        assert_eq!(app.delete_current, Some(MediaId(3)));
        // Opening a card's second photo shows that suggestion in Review.
        app.delete_card_event(MediaId(1), CardEvent::Open(1));
        assert!(app.review);
        assert_eq!(app.delete_current, Some(MediaId(1)));
        assert_eq!(app.active, Some(MediaId(2)));
        assert_eq!(app.comparison_ids(), vec![MediaId(1), MediaId(2)]);
    }
    #[test]
    fn keyboard_shortcuts_act_on_the_card_selected_in_the_grid() {
        let mut app = app();
        app.delete_card_event(MediaId(3), CardEvent::Select(0));
        app.review = false;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        };
        let _ = ctx.run_test(input, |ui| app.update_workspace(ui, true));
        assert!(app.delete_review.confirmed(MediaId(3)));
        assert!(!app.delete_review.confirmed(MediaId(1)));
        assert!(app.decisions[&MediaId(3)].rejected);
    }
}
