use super::review_card::{Badge, CardEvent, CardPhoto, CardState, Fact, GridClick, ReviewCard};
use super::*;
use crate::core::pick_review::{Cluster, Reason, Review};

/// Photos a card shows, best first; the panel beside the grid shows them all.
const CARD_PHOTOS: usize = 3;

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

impl PhotoSortApp {
    /// Easy Delete, Fix Rotation and Pick Best start their analysis on entry
    /// when they have nothing to show yet, so the step is never empty.
    pub(super) fn start_ready_analysis(&mut self) {
        let unanalysed = match self.step {
            Step::PickBest => {
                !self.view_data_ready.contains(&Step::PickBest)
                    && self.pick_review.clusters().is_empty()
            }
            Step::EasyDelete => {
                !self.view_data_ready.contains(&Step::EasyDelete)
                    && self.delete_review.entries().is_empty()
            }
            Step::Rotation => {
                !self.view_data_ready.contains(&Step::Rotation)
                    && self.rotation_review.ids().is_empty()
            }
            Step::Organize => self.organize_grouping_wanted(),
            _ => false,
        };
        if unanalysed
            && self.model_issue.is_none()
            && !self.model_dialog
            && self.catalog.is_some()
            && self.retry_plan.is_none()
            && !self.scanning
            && !self.busy
            && !self.close_authorized
            && self.departure.is_none()
            && self.after_apply.is_none()
            && !self.plan_open
            && self.analysis_run.is_none()
        {
            self.analyze_current_workflow();
        }
    }

    pub(super) fn finish_pick_analysis(&mut self) {
        self.view_data_ready.insert(Step::PickBest);
        let clusters = crate::core::pick_review::from_analysis(
            &self.media,
            &self.analysis,
            &self.cull_groups,
            &self.metadata,
        );
        self.pick_review.replace(clusters);
        self.pick_run = None;
        self.stage_pick_suggestions();
        let groups = self.pick_review.clusters().len();
        let trash = self
            .pick_review
            .clusters()
            .iter()
            .flat_map(|c| c.ordered.iter())
            .filter(|id| self.decision(**id).rejected)
            .count();
        self.set_workflow_status(
            Step::PickBest,
            match groups {
                0 => "No similar photos found · every photo is one of a kind".to_owned(),
                n => format!(
                    "{} · {} marked for Trash · nothing changes on disk until you apply Review changes",
                    count(n, "group of similar photos", "groups of similar photos"),
                    count(trash, "photo", "photos")
                ),
            } + &self.failure_note(),
        );
        self.filter_dirty = true;
        self.review_changed();
    }
    /// With the setting on, suggestions are the default decisions: each
    /// photo's is staged once, when it is first found or when Pick Best
    /// opens after staged choices were discarded.
    pub(super) fn stage_pick_suggestions(&mut self) {
        if !self.preferences.pick_best_auto || !self.view_data_ready.contains(&Step::PickBest) {
            return;
        }
        let marked = self.pick_review.stage_new_suggestions(&mut self.decisions);
        if !marked.is_empty() {
            self.changed_items(marked);
        }
    }
    fn pick_enabled(&self) -> bool {
        !self.busy && self.pick_run.is_none()
    }
    /// The selected group.
    pub(super) fn pick_group(&self) -> Option<usize> {
        self.pick_current
            .and_then(|id| self.pick_review.group_of(id))
    }
    fn pick_cluster(&self) -> Option<&Cluster> {
        self.pick_group().map(|g| &self.pick_review.clusters()[g])
    }
    /// The selected group's photos, best first, as the panel shows them.
    pub(super) fn pick_panel_ids(&self) -> Vec<MediaId> {
        self.pick_cluster()
            .map(|c| c.ordered.clone())
            .unwrap_or_default()
    }
    /// Switches one photo between Keep and Trash.
    pub(super) fn toggle_pick(&mut self, id: MediaId) {
        if !self.pick_enabled() {
            return;
        }
        self.pick_review.toggle(id, &mut self.decisions);
        self.set_active(id);
        self.changed_items([id]);
    }
    /// The number keys: the nth photo of the group shown beside the grid.
    pub(super) fn toggle_pick_slot(&mut self, slot: usize) {
        if let Some(id) = self.pick_panel_ids().get(slot).copied() {
            self.toggle_pick(id);
        }
    }
    /// Puts groups back to PhotoSort's suggestion.
    fn accept_pick_suggestions(&mut self, groups: &[usize]) {
        if !self.pick_enabled() {
            return;
        }
        self.pick_review.accept(groups, &mut self.decisions);
        let members: Vec<MediaId> = groups
            .iter()
            .filter_map(|g| self.pick_review.clusters().get(*g))
            .flat_map(|c| c.ordered.iter().copied())
            .collect();
        self.changed_items(members);
    }
    /// Undoes the user's changes to the selected group.
    pub(super) fn undo_pick(&mut self) {
        if let Some(group) = self.pick_group() {
            self.accept_pick_suggestions(&[group]);
        }
    }
    /// Every group back to its suggestion.
    pub(super) fn accept_pick_defaults(&mut self) {
        let groups: Vec<usize> = (0..self.pick_review.clusters().len()).collect();
        self.accept_pick_suggestions(&groups);
    }
    /// A click on the grid's empty space: no group is selected, and the
    /// panel beside the grid closes.
    pub(super) fn deselect_pick_group(&mut self) {
        self.pick_panel = false;
        self.pick_current = None;
        self.active = None;
    }
    /// The previous or next group, shown beside the grid.
    pub(super) fn navigate_pick(&mut self, delta: isize) {
        let groups = self.pick_review.clusters().len();
        if groups == 0 {
            return;
        }
        let next = match self.pick_group() {
            Some(current) => (current as isize + delta).clamp(0, groups as isize - 1) as usize,
            None if delta < 0 => groups - 1,
            None => 0,
        };
        self.pending_bookmark = None;
        let best = self.pick_review.clusters()[next].best();
        self.set_active(best);
        self.pick_panel = true;
    }
    /// Enter: the suggestion is already the decision, so show the selected
    /// group, or move on to the next one.
    pub(super) fn next_pick_group(&mut self) {
        if self.pick_current.is_some() && !self.pick_panel {
            self.pick_panel = true;
        } else {
            self.navigate_pick(1);
        }
    }
    /// The panel's Undo, beside its Keep/Trash switches.
    pub(super) fn pick_undo_button(&mut self, ui: &mut egui::Ui) {
        if let Some(group) = self.pick_group() {
            let changed = self.pick_review.changed(group, &self.decisions);
            if ui
                .add_enabled(
                    changed && self.pick_enabled(),
                    egui::Button::new(self.pick_undo_label()),
                )
                .on_hover_text("Back to PhotoSort's suggestion · R")
                .clicked()
            {
                self.undo_pick();
            }
        }
    }
    /// With suggestions applied automatically, going back to one undoes the
    /// user's change; otherwise it applies the suggestion for the first time.
    fn pick_undo_label(&self) -> &'static str {
        if self.preferences.pick_best_auto {
            "Undo"
        } else {
            "Use suggestion"
        }
    }
}

impl PhotoSortApp {
    /// Pick Best's only view: a summary with Accept all suggestions, then
    /// one card per group with its best photos. Clicking a card opens all of
    /// its photos large in a panel beside the grid.
    pub(super) fn pick_grid(&mut self, ui: &mut egui::Ui) {
        self.pick_summary(ui);
        ui.add_space(S2);
        let shown = self.pick_panel && self.pick_current.is_some();
        let layout = super::review_card::panel_layout(
            ui,
            "pick_panel_slide",
            shown,
            &mut self.pick_panel_width,
        );
        let groups = self.pick_review.clusters().len();
        let current = self.pick_group();
        let clicked = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(layout.grid)
                    .layout(egui::Layout::top_down(Align::Min)),
                |ui| {
                    self.flat_card_grid(
                        ui,
                        "pick",
                        groups,
                        |app, i| app.pick_card_aspects(i),
                        |_, _| true,
                        |app, i| Some(app.pick_card(i)),
                        current,
                    )
                },
            )
            .inner;
        if layout.slide > 0.0
            && let Some(width) =
                super::review_card::side_panel(ui, &layout, "pick_side_panel", |ui| {
                    self.pick_compare_panel(ui)
                })
        {
            self.pick_panel_width = Some(width);
        }
        ui.advance_cursor_after_rect(layout.area);
        match clicked {
            Some(GridClick::Card(i, event)) => self.pick_card_event(i, event),
            Some(GridClick::Background) => self.deselect_pick_group(),
            None => {}
        }
    }

    /// One line over the grid: what is staged, the photo size, and Accept
    /// all suggestions.
    fn pick_summary(&mut self, ui: &mut egui::Ui) {
        let members: Vec<MediaId> = self
            .pick_review
            .clusters()
            .iter()
            .flat_map(|c| c.ordered.iter().copied())
            .collect();
        let trash: Vec<MediaId> = members
            .iter()
            .copied()
            .filter(|id| self.decision(*id).rejected)
            .collect();
        let freed: u64 = trash
            .iter()
            .filter_map(|id| self.index.get(id))
            .map(|i| self.media[*i].fingerprint.bytes)
            .sum();
        let groups: Vec<usize> = (0..self.pick_review.clusters().len()).collect();
        let changed = self.pick_review.changed_count(&groups, &self.decisions);
        let note = match changed {
            0 => " · nothing changes on disk until you apply Review changes".to_owned(),
            n if self.preferences.pick_best_auto => format!(
                " · {} changed by you · nothing changes on disk until you apply Review changes",
                count(n, "group", "groups")
            ),
            n => format!(
                " · {} not following the suggestion · nothing changes on disk until you apply Review changes",
                count(n, "group", "groups")
            ),
        };
        ui.horizontal(|ui| {
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .add_enabled(
                        changed > 0 && self.pick_enabled(),
                        egui::Button::new("Accept all suggestions"),
                    )
                    .on_hover_text(if changed > 0 {
                        "Puts every group back to PhotoSort's suggestion: keep the best photo, move the rest to Trash · A"
                    } else {
                        "Every group already follows PhotoSort's suggestion"
                    })
                    .clicked()
                {
                    self.accept_pick_defaults();
                }
                ui.add_space(S3);
                self.size_slider(ui);
                ui.add_space(S3);
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    let mut job = egui::text::LayoutJob::default();
                    let body = egui::TextStyle::Body.resolve(ui.style());
                    job.append(
                        &format!(
                            "{} · {} going to Trash · frees {} · {} kept",
                            count(groups.len(), "group", "groups"),
                            count(trash.len(), "photo", "photos"),
                            super::review_card::size_text(freed),
                            members.len() - trash.len()
                        ),
                        0.0,
                        egui::TextFormat::simple(body.clone(), INK),
                    );
                    job.append(&note, 0.0, egui::TextFormat::simple(body, MUTED));
                    let full = job.text.clone();
                    ui.add(egui::Label::new(job).truncate()).on_hover_text(full);
                });
            });
        });
    }

    /// The selected group's photos, large, beside the grid, with zoom,
    /// Keep/Trash and Undo. The grid is its strip.
    fn pick_compare_panel(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let Some(group) = self.pick_group() else {
            self.pick_panel = false;
            return;
        };
        let title = self.pick_title(group);
        let n = self.pick_review.clusters()[group].ordered.len();
        ui.horizontal(|ui| {
            ui.label(RichText::new(title).font(semibold(13.0)).color(INK));
            ui.label(RichText::new(count(n, "photo", "photos")).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if icon_button(ui, icon::X, "Close · Esc").clicked() {
                    self.pick_panel = false;
                }
            });
        });
        ui.add_space(S1);
        self.review_canvas(ui);
    }

    /// A group's heading: when it was taken, or its number.
    fn pick_title(&self, group: usize) -> String {
        let cluster = &self.pick_review.clusters()[group];
        cluster
            .ordered
            .iter()
            .filter_map(|id| self.metadata.get(id)?.captured_ms)
            .min()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map_or_else(
                || format!("Group {}", group + 1),
                |time| time.format("%-d %b %Y, %H:%M").to_string(),
            )
    }

    /// Each photo's width over its height on a group's card, in order.
    fn pick_card_aspects(&self, group: usize) -> Vec<f32> {
        self.pick_review.clusters()[group]
            .ordered
            .iter()
            .take(CARD_PHOTOS)
            .map(|id| self.photo_aspect(*id, 0))
            .collect()
    }

    /// Why PhotoSort suggests keeping the group's best photo, for its
    /// Suggested mark: what put it ahead of the next best one.
    pub(super) fn pick_keep_why(&self, cluster: &Cluster) -> String {
        let best = cluster.best();
        let next = cluster.ordered[1];
        let why = match cluster.reason {
            Reason::Sharper => {
                let soft = cluster.soft.len();
                format!(
                    "It is sharp, and {} noticeably softer.",
                    if soft == 1 {
                        "1 other photo is".to_owned()
                    } else {
                        format!("{soft} others are")
                    }
                )
            }
            Reason::EyesOpen => match cluster.blinks(next) {
                1 => "Eyes are open here. In the next best photo, someone's eyes are closed."
                    .to_owned(),
                n => format!(
                    "Eyes are open here. In the next best photo, {n} people's eyes are closed."
                ),
            },
            Reason::Score => format!(
                "It has the best overall score ({:.0} against {:.0} for the next best), from how the photo looks to the aesthetic model, its sharpness and closed eyes.",
                cluster.scores[&best], cluster.scores[&next]
            ),
        };
        format!("Suggested to keep. {why}")
    }

    /// The facts that tell a group's photos apart, for the photos given:
    /// only those that differ, the winner's value highlighted where it is
    /// the best of the group.
    fn pick_facts(&self, cluster: &Cluster, ids: &[MediaId]) -> Vec<Vec<Fact>> {
        let best = cluster.best();
        let all = &cluster.ordered;
        let analysis = |id: &MediaId| self.analysis.get(id);
        let sharpness = |id: &MediaId| {
            analysis(id)
                .map(|a| a.quality_sharpness.unwrap_or(a.sharpness))
                .filter(|v| v.is_finite())
        };
        let aesthetic = |id: &MediaId| analysis(id).and_then(|a| a.aesthetic).map(f64::from);
        let faces = |id: &MediaId| analysis(id).and_then(|a| a.faces.as_ref()).map(|f| f.count);
        let pixels = |id: &MediaId| {
            self.metadata
                .get(id)
                .filter(|m| m.width > 0 && m.height > 0)
                .map(|m| f64::from(m.width) * f64::from(m.height))
        };
        let bytes = |id: &MediaId| self.index.get(id).map(|i| self.media[*i].fingerprint.bytes);
        // A numeric fact differs when its spread passes `margin`; the winner
        // is highlighted when it holds the group's highest value.
        let spread = |value: &dyn Fn(&MediaId) -> Option<f64>, margin: f64| {
            let values: Vec<f64> = all.iter().filter_map(value).collect();
            let (low, high) = values
                .iter()
                .fold((f64::INFINITY, f64::NEG_INFINITY), |(l, h), v| {
                    (l.min(*v), h.max(*v))
                });
            (values.len() >= 2 && high > low + margin.max(low.abs() * 0.05)).then_some(high)
        };
        let sharp_high = spread(&sharpness, 0.0);
        let look_high = spread(&aesthetic, 0.02);
        let pixel_high = spread(&pixels, 0.0);
        let faces_differ = all.iter().map(faces).collect::<HashSet<_>>().len() > 1;
        let bytes_differ = all.iter().map(bytes).collect::<HashSet<_>>().len() > 1;
        let tops =
            |value: Option<f64>, high: Option<f64>| value.zip(high).is_some_and(|(v, h)| v >= h);
        ids.iter()
            .map(|id| {
                let winner = *id == best;
                let mut facts = Vec::new();
                if let Some(eyes) = cluster.eyes.get(id) {
                    if eyes.closed > 0 {
                        facts.push(Fact {
                            text: match eyes.closed {
                                1 => "Eyes closed".to_owned(),
                                n => format!("{n} with eyes closed"),
                            },
                            better: false,
                        });
                    } else if eyes.open > 0 {
                        facts.push(Fact {
                            text: "Eyes open".to_owned(),
                            better: winner,
                        });
                    }
                }
                if let Some(value) = sharpness(id).filter(|_| sharp_high.is_some()) {
                    facts.push(Fact {
                        text: if cluster.soft.contains(id) {
                            format!("Soft {value:.0}")
                        } else {
                            format!("Sharpness {value:.0}")
                        },
                        better: winner && tops(Some(value), sharp_high),
                    });
                }
                if let Some(value) = aesthetic(id).filter(|_| look_high.is_some()) {
                    facts.push(Fact {
                        text: format!("Aesthetic {:.0}", value * 100.0),
                        better: winner && tops(Some(value), look_high),
                    });
                }
                if let Some(n) = faces(id).filter(|_| faces_differ) {
                    facts.push(Fact {
                        text: count(n, "face", "faces"),
                        better: false,
                    });
                }
                if pixel_high.is_some()
                    && let Some(m) = self.metadata.get(id).filter(|m| m.width > 0)
                {
                    let (w, h) = if (5..=8).contains(&m.orientation) {
                        (m.height, m.width)
                    } else {
                        (m.width, m.height)
                    };
                    facts.push(Fact {
                        text: format!("{w}×{h}"),
                        better: winner && tops(pixels(id), pixel_high),
                    });
                }
                if let Some(b) = bytes(id).filter(|_| bytes_differ) {
                    facts.push(Fact {
                        text: super::review_card::size_text(b),
                        better: false,
                    });
                }
                facts
            })
            .collect()
    }

    /// One photo's facts in one line, for the panel beside the grid.
    pub(super) fn pick_photo_facts(&self, id: MediaId) -> Option<String> {
        let cluster = self.pick_review.cluster_of(id)?;
        let facts = self.pick_facts(cluster, &[id]).swap_remove(0);
        (!facts.is_empty()).then(|| {
            facts
                .iter()
                .map(|f| f.text.as_str())
                .collect::<Vec<_>>()
                .join(" · ")
        })
    }

    /// The card for one group: its best photos with a Keep/Trash pill each
    /// and their deciding facts, the suggested one marked, and Undo once it
    /// differs from the suggestion.
    fn pick_card(&self, group: usize) -> ReviewCard {
        let cluster = &self.pick_review.clusters()[group];
        let enabled = self.pick_enabled();
        let shown: Vec<MediaId> = cluster.ordered.iter().take(CARD_PHOTOS).copied().collect();
        let facts = self.pick_facts(cluster, &shown);
        let photos = shown
            .iter()
            .zip(facts)
            .enumerate()
            .map(|(side, (id, facts))| {
                let keep = Review::keeps(&self.decisions, *id);
                let media = self.index.get(id).map(|i| &self.media[*i]);
                let suggested = *id == cluster.best();
                CardPhoto {
                    id: *id,
                    turns: self.preview_turns(*id),
                    badge: Badge::keep(keep),
                    suggested,
                    suggested_hover: suggested.then(|| self.pick_keep_why(cluster)),
                    caption: media.map(Media::name).unwrap_or_default(),
                    name: media
                        .map(|m| m.path.display().to_string())
                        .unwrap_or_default(),
                    facts,
                    trash: !keep,
                    inset: None,
                    toggle: enabled.then(|| {
                        format!(
                            "{} · {}",
                            if keep {
                                "Move to Trash instead"
                            } else {
                                "Keep instead"
                            },
                            side + 1
                        )
                    }),
                }
            })
            .collect();
        let n = cluster.ordered.len();
        let hidden = &cluster.ordered[shown.len()..];
        let state_text = if hidden.is_empty() {
            count(n, "photo", "photos")
        } else {
            let kept = hidden
                .iter()
                .filter(|id| Review::keeps(&self.decisions, **id))
                .count();
            format!(
                "{} · {} more{}",
                count(n, "photo", "photos"),
                hidden.len(),
                if kept > 0 {
                    format!(", {kept} kept")
                } else {
                    String::new()
                }
            )
        };
        ReviewCard {
            title: self.pick_title(group),
            state: CardState::Suggested,
            state_text: Some(state_text),
            photos,
            note: String::new(),
            warning: false,
            buttons: Vec::new(),
            // The selected group shows in the panel beside the grid, which
            // is enough: no outline that could be read as Keep or Trash.
            current: false,
            shown: self.pick_panel && self.pick_group() == Some(group),
            title_hover: (!hidden.is_empty())
                .then(|| "Click the card to see every photo of the group".to_owned()),
            undo: (enabled && self.pick_review.changed(group, &self.decisions)).then(|| {
                (
                    self.pick_undo_label().to_owned(),
                    "Back to PhotoSort's suggestion · R on the selected group".to_owned(),
                )
            }),
        }
    }

    /// Applies a card's event to its group, after making it the selected
    /// group so the keyboard follows the mouse.
    pub(super) fn pick_card_event(&mut self, group: usize, event: CardEvent) {
        self.pending_bookmark = None;
        let Some(ordered) = self
            .pick_review
            .clusters()
            .get(group)
            .map(|c| c.ordered.clone())
        else {
            return;
        };
        match event {
            CardEvent::Select(side) | CardEvent::Open(side) => {
                if let Some(id) = ordered.get(side) {
                    self.set_active(*id);
                }
                // A click shows the group's photos large beside the grid.
                self.pick_panel = true;
            }
            CardEvent::Toggle(side) => {
                if let Some(id) = ordered.get(side) {
                    self.toggle_pick(*id);
                }
            }
            CardEvent::Undo => self.accept_pick_suggestions(&[group]),
            CardEvent::Button(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::faces::{EyeState, FaceEyes, FaceSummary};
    use crate::core::pick_review::Cluster;
    fn photo(n: u64) -> Media {
        Media {
            id: MediaId(n),
            path: format!("/{n}.jpg").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        }
    }
    /// Two groups: photos 1-3 (3 is best) and 4-5 (5 is best).
    fn app() -> PhotoSortApp {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.media = (1..=5).map(photo).collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.pick_review.replace(vec![
            Cluster::new(
                1,
                vec![MediaId(1), MediaId(2), MediaId(3)],
                HashMap::from([(MediaId(1), 10.), (MediaId(2), 20.), (MediaId(3), 30.)]),
            ),
            Cluster::new(
                2,
                vec![MediaId(4), MediaId(5)],
                HashMap::from([(MediaId(4), 10.), (MediaId(5), 20.)]),
            ),
        ]);
        app.view_data_ready.insert(Step::PickBest);
        app.set_step(Step::PickBest);
        app.refilter();
        app
    }
    fn trashed(app: &PhotoSortApp) -> Vec<u64> {
        let mut ids: Vec<u64> = (1..=5)
            .filter(|n| app.decision(MediaId(*n)).rejected)
            .collect();
        ids.sort();
        ids
    }
    fn key(app: &mut PhotoSortApp, key: egui::Key, modifiers: egui::Modifiers) {
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let input = egui::RawInput {
            events: vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            }],
            ..Default::default()
        };
        let _ = ctx.run_test(input, |ui| {
            let ctx = &ui.ctx().clone();
            app.keyboard(ctx)
        });
    }

    #[test]
    fn entering_pick_best_stages_its_suggestions_once() {
        let mut app = app();
        assert_eq!(trashed(&app), vec![1, 2, 4]);
        // The user keeps photo 1; leaving and coming back never re-trashes it.
        app.toggle_pick(MediaId(1));
        app.set_step(Step::Cull);
        app.set_step(Step::PickBest);
        assert_eq!(trashed(&app), vec![2, 4]);
        // Review changes lists them: no "complete every comparison" gate.
        app.review_changes();
        assert!(!app.status.contains("Complete every comparison"));
    }

    #[test]
    fn with_automatic_suggestions_off_nothing_is_staged_until_accepted() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.preferences.pick_best_auto = false;
        app.media = (1..=5).map(photo).collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.pick_review.replace(vec![Cluster::new(
            1,
            vec![MediaId(1), MediaId(2)],
            HashMap::from([(MediaId(1), 10.), (MediaId(2), 20.)]),
        )]);
        app.view_data_ready.insert(Step::PickBest);
        app.set_step(Step::PickBest);
        assert!(trashed(&app).is_empty());
        app.accept_pick_defaults();
        assert_eq!(trashed(&app), vec![1]);
    }

    #[test]
    fn a_click_toggles_and_undo_restores_the_suggestion() {
        let mut app = app();
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert_eq!(trashed(&app), vec![1, 4], "Photo 2 is kept too");
        assert_eq!(app.pick_group(), Some(0));
        assert!(app.pick_review.changed(0, &app.decisions));
        // Toggling back by hand is no change.
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert!(!app.pick_review.changed(0, &app.decisions));
        // Pick another photo as the keeper, then Undo.
        app.pick_card_event(0, CardEvent::Toggle(0));
        app.pick_card_event(0, CardEvent::Toggle(1));
        assert_eq!(trashed(&app), vec![1, 3, 4]);
        app.pick_card_event(0, CardEvent::Undo);
        assert_eq!(trashed(&app), vec![1, 2, 4]);
        // Accept all puts every group back.
        app.pick_card_event(1, CardEvent::Toggle(1));
        app.pick_card_event(0, CardEvent::Toggle(2));
        app.accept_pick_defaults();
        assert_eq!(trashed(&app), vec![1, 2, 4]);
    }

    #[test]
    fn pick_best_opens_with_nothing_selected_and_the_keyboard_selects_and_shows() {
        let mut app = app();
        assert_eq!(app.pick_current, None);
        assert!(!app.pick_panel);
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(!shown.contains("Round"), "{shown}");
        // Enter shows the first group beside the grid.
        key(&mut app, egui::Key::Enter, egui::Modifiers::NONE);
        assert_eq!(app.pick_group(), Some(0));
        assert!(app.pick_panel);
        // Number keys toggle the nth photo of the group shown, best first.
        key(&mut app, egui::Key::Num2, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 4], "Photo 2 is the second best");
        key(&mut app, egui::Key::R, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 2, 4], "R undoes the group");
        // Arrows move to the next group; Esc closes the panel.
        key(&mut app, egui::Key::ArrowDown, egui::Modifiers::NONE);
        assert_eq!(app.pick_group(), Some(1));
        key(&mut app, egui::Key::Escape, egui::Modifiers::NONE);
        assert!(!app.pick_panel);
        // With the panel closed, number keys change nothing.
        key(&mut app, egui::Key::Num1, egui::Modifiers::NONE);
        assert_eq!(trashed(&app), vec![1, 2, 4]);
        // A click on the grid background deselects.
        app.deselect_pick_group();
        assert_eq!(app.pick_current, None);
    }

    #[test]
    fn the_grid_shows_one_card_per_group_with_its_suggested_photo_and_no_confirm() {
        let mut app = app();
        let shown = super::super::review_card::render_grid(&mut app);
        for text in [
            "Group 1",
            "Group 2",
            "3 photos",
            "2 photos",
            "Suggested",
            "Accept all suggestions",
        ] {
            assert!(shown.contains(text), "{text} missing from:\n{shown}");
        }
        for text in ["Confirm", "Provisional", "Keep all", "Round"] {
            assert!(!shown.contains(text), "{text} in:\n{shown}");
        }
        assert!(shown.contains("3 photos going to Trash"), "{shown}");
    }

    #[test]
    fn cards_say_who_has_eyes_closed_and_the_winner_has_eyes_open() {
        let mut app = app();
        let face = |state, ratio| FaceEyes {
            centre: [0.5, 0.4],
            size: [0.1, 0.12],
            state,
            ratio,
        };
        let analysis = |score: f64, eyes: FaceEyes| crate::core::analysis::Analysis {
            sharpness: 300.0,
            quality_sharpness: Some(300.0),
            aesthetic: Some(0.6),
            faces: Some(FaceSummary {
                count: 1,
                eyes: vec![eyes],
                ..Default::default()
            }),
            score,
            ..Default::default()
        };
        app.analysis = HashMap::from([
            (
                MediaId(4),
                analysis(80.0, face(EyeState::Closed, Some(0.05))),
            ),
            (MediaId(5), analysis(60.0, face(EyeState::Open, Some(0.3)))),
        ]);
        app.cull_groups = Arc::new(HashMap::from([(MediaId(4), 7), (MediaId(5), 7)]));
        app.pick_review = Default::default();
        app.decisions.clear();
        app.finish_pick_analysis();
        let cluster = &app.pick_review.clusters()[0];
        assert_eq!(
            cluster.best(),
            MediaId(5),
            "Open eyes win over a higher score"
        );
        assert_eq!(trashed(&app), vec![4]);
        let facts = app.pick_facts(cluster, &[MediaId(5), MediaId(4)]);
        assert_eq!(
            facts[0][0],
            Fact {
                text: "Eyes open".into(),
                better: true
            }
        );
        assert_eq!(
            facts[1][0],
            Fact {
                text: "Eyes closed".into(),
                better: false
            }
        );
        assert!(app.pick_keep_why(cluster).contains("eyes are closed"));
        let shown = super::super::review_card::render_grid(&mut app);
        assert!(
            shown.contains("Eyes closed") && shown.contains("Eyes open"),
            "{shown}"
        );
    }

    #[test]
    fn background_pick_results_do_not_change_another_workflows_active_photo() {
        let mut app = app();
        app.set_step(Step::Cull);
        app.active = Some(MediaId(3));
        app.finish_pick_analysis();
        assert_eq!(app.active, Some(MediaId(3)));
        assert_eq!(app.step, Step::Cull);
    }
    fn loaded_folder(app: &mut PhotoSortApp) {
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);
        app.runtime.events = receiver;
        sender
            .send(crate::core::jobs::Envelope {
                epoch: app.runtime.epoch(),
                event: Event::Opened(
                    Arc::new(Catalog {
                        media: app.media.clone(),
                        ..Default::default()
                    }),
                    Session::default(),
                    Default::default(),
                ),
            })
            .unwrap();
        app.poll(&egui::Context::default());
    }
    #[test]
    fn folder_completion_starts_pick_analysis_without_reentering_the_step() {
        let mut app = app();
        app.pick_review = Default::default();
        app.view_data_ready.clear();
        app.catalog = None;
        app.scanning = true;
        app.set_step(Step::PickBest);
        assert!(app.analysis_run.is_none(), "Do not analyze a partial scan");
        loaded_folder(&mut app);
        assert!(!app.scanning);
        assert!(
            app.pick_run.is_some(),
            "The completed scan must start quality analysis"
        );
        assert_eq!(app.pick_run, app.analysis_run);
        let run = app.pick_run;
        app.set_step(Step::PickBest);
        assert_eq!(
            run, app.pick_run,
            "Step reentry must reuse the existing run"
        );
    }
    #[test]
    fn post_apply_departure_does_not_start_unwanted_pick_analysis() {
        for destination in [Destination::Step(Step::Cull), Destination::Close] {
            let mut app = app();
            app.pick_review = Default::default();
            app.view_data_ready.clear();
            app.after_apply = Some(destination);
            loaded_folder(&mut app);
            assert!(app.pick_run.is_none());
            assert!(app.analysis_run.is_none());
            assert!(app.step == Step::Cull || app.close_authorized);
        }
    }
}
