use super::review_card::{Badge, CardButton, CardEvent, CardPhoto, CardState, ReviewCard};
use super::*;
impl PhotoSortApp {
    /// Easy Delete, Fix Rotation and Pick Best start their analysis on entry
    /// when they have nothing to show yet, so the step is never empty.
    pub(super) fn start_ready_analysis(&mut self) {
        let unanalysed = match self.step {
            Step::PickBest => self.pick_review.tournaments().is_empty(),
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
            && self.review_notice.is_none()
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
            &self.embeddings,
        );
        self.pick_review.replace(clusters, &mut self.decisions);
        self.pick_run = None;
        self.set_workflow_status(
            Step::PickBest,
            match self.pick_review.tournaments().len() {
                0 => "No similar photos found · every photo is one of a kind".to_owned(),
                1 => "1 group of similar photos · choose Keep/Trash, then confirm".to_owned(),
                n => format!("{n} groups of similar photos · choose Keep/Trash, then confirm"),
            } + &self.failure_note(),
        );
        if self.step == Step::PickBest {
            self.sync_pick_view();
        }
        self.pick_changed();
    }
    fn pick_changed(&mut self) {
        self.delete_review.reconcile_external(&self.decisions);
        self.review_changed();
    }
    pub(super) fn sync_pick_view(&mut self) {
        self.pick_focus = false;
        self.active = self
            .pick_review
            .comparison()
            .and_then(|c| c.paths.first().copied());
        self.focused_slot = 0;
        self.inspection_view = Default::default();
        self.independent_views.clear();
        self.view_started = Instant::now();
        self.filter_dirty = true;
    }
    pub(super) fn toggle_pick(&mut self, id: MediaId) {
        if self.busy || self.pick_run.is_some() {
            return;
        }
        self.pick_review.toggle(id, &mut self.decisions);
        if let Some(slot) = self
            .pick_review
            .comparison()
            .and_then(|c| c.paths.iter().position(|p| *p == id))
        {
            self.active = Some(id);
            self.focused_slot = if self.pick_focus { 0 } else { slot as u64 };
        }
        self.pick_changed();
    }
    pub(super) fn confirm_pick(&mut self, keep_all: bool) {
        if self.busy || self.pick_run.is_some() {
            return;
        }
        if self.pick_review.confirm(keep_all, &mut self.decisions) {
            self.sync_pick_view();
            self.pick_changed();
        }
    }
    pub(super) fn reset_pick(&mut self, all: bool) {
        if all {
            self.pick_review.reset_all(&mut self.decisions);
        } else {
            self.pick_review.reset_current(&mut self.decisions);
        }
        self.review_notice = None;
        self.sync_pick_view();
        self.pick_changed();
    }
    pub(super) fn navigate_pick(&mut self, delta: isize, cluster: bool) {
        if !self.allow_review_departure() {
            return;
        }
        if !cluster && let Some(t) = self.pick_review.current() {
            let next = t.current as isize + delta;
            if next >= 0 && (next as usize) < t.comparisons.len() {
                self.pick_review.select_comparison(next as usize);
                self.sync_pick_view();
                return;
            }
        }
        let next = self.pick_review.current_index() as isize + delta;
        if next >= 0 && self.pick_review.select_cluster(next as usize) {
            self.sync_pick_view();
        }
    }
    pub(super) fn toggle_pick_focus(&mut self) {
        self.pick_focus ^= true;
        self.focused_slot = if self.pick_focus {
            0
        } else {
            self.pick_review
                .comparison()
                .and_then(|c| c.paths.iter().position(|id| Some(*id) == self.active))
                .unwrap_or(0) as u64
        };
        self.inspection_view = Default::default();
        self.independent_views.clear();
        self.view_started = Instant::now();
    }
    /// One group's status for the group list.
    fn pick_group_status(&self, index: usize) -> String {
        let t = &self.pick_review.tournaments()[index];
        if t.finalized {
            let kept = t
                .cluster
                .ordered
                .iter()
                .filter(|id| !self.decisions.get(id).is_some_and(|d| d.rejected))
                .count();
            format!("Complete · keeping {kept}")
        } else if t.comparisons.iter().any(|c| c.confirmed) {
            "In progress".to_owned()
        } else {
            "Not started".to_owned()
        }
    }
    /// Where the review stands: which group, how far through it, and how
    /// many groups are done. Choices and Confirm live under the photos.
    pub(super) fn pick_controls(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        if self.pick_run.is_some() {
            return;
        }
        let current = self.pick_review.current_index();
        let groups = self.pick_review.tournaments().len();
        let Some(t) = self.pick_review.current() else {
            return;
        };
        let (photos, round, rounds) = (
            t.cluster.ordered.len(),
            t.current,
            t.comparisons.len() + t.upcoming().len(),
        );
        let confirmed: Vec<bool> = t.comparisons.iter().map(|c| c.confirmed).collect();
        let mut selected_cluster = current;
        if named(
            ui.add_enabled(current > 0, egui::Button::new(icon::CARET_LEFT)),
            egui::WidgetType::Button,
            "Previous group",
        )
        .on_hover_text("Previous group · Left")
        .clicked()
        {
            selected_cluster = current - 1;
        }
        let group = ui
            .add(egui::Button::new(
                RichText::new(format!(
                    "Group {} of {groups}  ·  {photos} photos  {}",
                    current + 1,
                    icon::CARET_DOWN
                ))
                .font(medium(12.0)),
            ))
            .on_hover_text("Jump to another group");
        if named(
            ui.add_enabled(current + 1 < groups, egui::Button::new(icon::CARET_RIGHT)),
            egui::WidgetType::Button,
            "Next group",
        )
        .on_hover_text("Next group · Right")
        .clicked()
        {
            selected_cluster = current + 1;
        }
        egui::Popup::menu(&group)
            .id(egui::Id::new("pick_clusters_menu"))
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(260.0);
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                egui::ScrollArea::vertical()
                    .id_salt("pick_clusters_scroll")
                    .max_height(280.0)
                    .show_rows(ui, button_row_height(ui), groups, |ui, rows| {
                        for i in rows {
                            let text = format!(
                                "Group {} · {} photos · {}",
                                i + 1,
                                self.pick_review.tournaments()[i].cluster.ordered.len(),
                                self.pick_group_status(i)
                            );
                            if ui
                                .selectable_value(&mut selected_cluster, i, text)
                                .clicked()
                            {
                                ui.close();
                            }
                        }
                    });
            });
        // Rounds: each one brings the next photo against the current keeper.
        ui.add_space(S2);
        ui.label(RichText::new(format!("Round {} of {rounds}", round + 1)).color(MUTED));
        let mut selected_round = round;
        ui.add_space(S1);
        let dot = 8.0;
        // Long groups show a window of twelve rounds around the current one.
        let shown = rounds.min(12);
        let first = round.saturating_sub(6).min(rounds - shown);
        let (strip, _) = ui.allocate_exact_size(
            egui::vec2(shown as f32 * (dot + 5.0), dot + 6.0),
            Sense::hover(),
        );
        for (slot, i) in (first..first + shown).enumerate() {
            let center = egui::pos2(
                strip.left() + dot / 2.0 + slot as f32 * (dot + 5.0),
                strip.center().y,
            );
            let rect = egui::Rect::from_center_size(center, egui::vec2(dot + 4.0, dot + 4.0));
            let done = confirmed.get(i).copied().unwrap_or(false);
            let reachable = i < confirmed.len();
            let response = ui.interact(rect, egui::Id::new(("pick-round", i)), Sense::click());
            let name = format!(
                "Round {}: {}",
                i + 1,
                if done {
                    "confirmed"
                } else if reachable {
                    "needs review"
                } else {
                    "up next"
                }
            );
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Button, reachable, i == round, &name)
            });
            let response = response.on_hover_text(&name);
            let painter = ui.painter();
            if done {
                painter.circle_filled(center, dot / 2.0, ACCENT.gamma_multiply(0.75));
            } else if reachable {
                painter.circle_filled(center, dot / 2.0, CARD_HOVER);
            } else {
                painter.circle_stroke(center, dot / 2.0 - 0.5, Stroke::new(1.0, FAINT));
            }
            if i == round {
                painter.circle_stroke(center, dot / 2.0 + 2.0, Stroke::new(1.5, INK));
            }
            if reachable && response.clicked() {
                selected_round = i;
            }
        }
        let complete = self
            .pick_review
            .tournaments()
            .iter()
            .filter(|t| t.finalized)
            .count();
        ui.add_space(S2);
        ui.add(
            egui::ProgressBar::new(complete as f32 / groups.max(1) as f32)
                .desired_width(90.0)
                .desired_height(6.0),
        )
        .on_hover_text(format!("{complete} of {groups} groups complete"));
        ui.label(
            RichText::new(format!("{complete} of {groups} groups complete"))
                .small()
                .color(MUTED),
        );
        if icon_text_button(ui, true, self.pick_focus, icon::CORNERS_OUT, "Focus photo")
            .on_hover_text("Show one photo at a time · C")
            .clicked()
        {
            self.toggle_pick_focus();
        }
        if selected_cluster != current && self.allow_review_departure() {
            self.pick_review.select_cluster(selected_cluster);
            self.sync_pick_view();
        } else if selected_round != round && self.allow_review_departure() {
            self.pick_review.select_comparison(selected_round);
            self.sync_pick_view();
        }
    }
    pub(super) fn pick_menu(&mut self, ui: &mut egui::Ui) {
        let ready = self.pick_review.current().is_some() && self.pick_run.is_none();
        ui.add_enabled_ui(ready, |ui| {
            if ui
                .button("Keep all in this round · K")
                .on_hover_text("Keep every photo shown and move on")
                .clicked()
            {
                self.confirm_pick(true);
                ui.close();
            }
            if ui
                .button("Reset this round · R")
                .on_hover_text("Back to the suggestion")
                .clicked()
            {
                self.reset_pick(false);
                ui.close();
            }
            if ui.button("Reset all groups").clicked() {
                self.reset_pick(true);
                ui.close();
            }
        });
    }
    /// The suggestion and how alike the photos are, in one quiet line.
    pub(super) fn pick_context(&mut self, ui: &mut egui::Ui) {
        if self.pick_run.is_some() {
            return;
        }
        let Some(c) = self.pick_review.comparison() else {
            return;
        };
        let name = self
            .index
            .get(&c.ai_pick)
            .map(|i| self.media[*i].name())
            .unwrap_or_default();
        let score = self.pick_review.current().unwrap().cluster.scores[&c.ai_pick];
        // Why these were grouped: the least similar photo to the suggested
        // keeper, from the same normalized embeddings the grouping used.
        let alike = self.embeddings.get(&c.ai_pick).and_then(|keeper| {
            c.paths
                .iter()
                .filter(|id| **id != c.ai_pick)
                .map(|id| {
                    self.embeddings
                        .get(id)
                        .map(|other| crate::core::analysis::cosine(keeper, other))
                })
                .collect::<Option<Vec<_>>>()
                .and_then(|values| values.into_iter().reduce(f32::min))
        });
        let alike = alike
            .map(|value| format!(" · {:.0}% alike", value.clamp(0.0, 1.0) * 100.0))
            .unwrap_or_default();
        let next = if c.confirmed {
            "Confirmed."
        } else {
            "Click a photo (or press its number) to switch Keep/Trash, then Confirm."
        };
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!(
                    "{}  Suggested: {name}",
                    egui_phosphor::regular::STAR
                ))
                .small()
                .color(ACCENT),
            );
            ui.add(
                egui::Label::new(
                    RichText::new(format!("score {score:.1}{alike} · {next}"))
                        .small()
                        .color(MUTED),
                )
                .truncate(),
            );
        });
    }
    /// Confirm (and keep all) under the compared photos.
    pub(super) fn pick_confirm_buttons(&mut self, ui: &mut egui::Ui) {
        let confirmed = self.pick_review.comparison().is_some_and(|c| c.confirmed);
        let confirm = if confirmed {
            egui::Button::new(format!("{}  Confirmed", egui_phosphor::regular::CHECK))
        } else {
            primary(format!("Confirm  {}", egui_phosphor::regular::ARROW_RIGHT))
        };
        if ui
            .add_enabled(!confirmed, confirm)
            .on_hover_text("Apply these Keep/Trash choices and move to the next round · Enter")
            .clicked()
        {
            self.confirm_pick(false);
        }
        if ui
            .button("Keep all")
            .on_hover_text("Keep every photo shown and move on · K")
            .clicked()
        {
            self.confirm_pick(true);
        }
    }
}

impl PhotoSortApp {
    /// Pick Best's grid: one card per group, showing its current round with
    /// the suggested keeper and the same Keep/Trash, Confirm, Keep all and
    /// Reset as Review. The focused card is Review's current group.
    pub(super) fn pick_grid(&mut self, ui: &mut egui::Ui) {
        let count = self.pick_review.tournaments().len();
        let slots = self
            .pick_review
            .tournaments()
            .iter()
            .map(|t| t.comparison().paths.len())
            .max()
            .unwrap_or(2)
            .max(2);
        let current = (count > 0).then(|| self.pick_review.current_index());
        let clicked = self.card_grid(
            ui,
            "pick",
            count,
            slots,
            |app, i| Some(app.pick_card(i)),
            current,
        );
        if let Some((i, event)) = clicked {
            self.pick_card_event(i, event);
        }
    }

    /// The card for one group: its current round's photos with their
    /// Keep/Trash choice, the suggested keeper and the round actions.
    fn pick_card(&self, index: usize) -> ReviewCard {
        let t = &self.pick_review.tournaments()[index];
        let c = t.comparison();
        let current = index == self.pick_review.current_index();
        let changed = current && self.pick_review.unfinished();
        let enabled = !self.busy && self.pick_run.is_none();
        let name = |id: &MediaId| {
            self.index
                .get(id)
                .map(|i| self.media[*i].name())
                .unwrap_or_default()
        };
        let rounds = t.comparisons.len() + t.upcoming().len();
        let (state, state_text) = if t.finalized {
            (CardState::Confirmed, Some(self.pick_group_status(index)))
        } else if changed {
            (
                CardState::Changed,
                Some(format!("Round {} of {rounds} · changed", t.current + 1)),
            )
        } else if c.confirmed {
            (
                CardState::Confirmed,
                Some(format!("Round {} of {rounds} confirmed", t.current + 1)),
            )
        } else {
            (
                CardState::Provisional,
                Some(format!("Round {} of {rounds}", t.current + 1)),
            )
        };
        let mut buttons: Vec<CardButton> = c
            .paths
            .iter()
            .enumerate()
            .map(|(side, id)| {
                CardButton::new(
                    super::delete_review::decision_label(side, c.keeps(*id)),
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
                if c.confirmed { "Confirmed" } else { "Confirm" },
                "Apply these Keep/Trash choices and move to the next round · Enter on the selected card",
            )
            .primary(!c.confirmed)
            .enabled(enabled && !c.confirmed)
            .trailing(),
        );
        buttons.push(
            CardButton::new(
                "Keep all",
                "Keep every photo shown and move on · K on the selected card",
            )
            .enabled(enabled)
            .trailing(),
        );
        buttons.push(
            CardButton::new("Reset", "Back to the suggestion · R on the selected card")
                .enabled(enabled && (changed || c.confirmed || t.finalized))
                .trailing(),
        );
        let score = t
            .cluster
            .scores
            .get(&c.ai_pick)
            .copied()
            .unwrap_or_default();
        let upcoming = match t.upcoming().len() {
            0 => String::new(),
            1 => " · 1 more photo to compare".to_owned(),
            n => format!(" · {n} more photos to compare"),
        };
        ReviewCard {
            title: format!("Group {} · {} photos", index + 1, t.cluster.ordered.len()),
            state,
            state_text,
            photos: c
                .paths
                .iter()
                .map(|id| CardPhoto {
                    id: *id,
                    turns: self.preview_turns(*id),
                    badge: Badge::keep(c.keeps(*id)),
                    suggested: *id == c.ai_pick,
                    caption: name(id),
                    name: self
                        .index
                        .get(id)
                        .map(|i| self.media[*i].path.display().to_string())
                        .unwrap_or_default(),
                })
                .collect(),
            note: format!(
                "Suggested keeper: {} · score {score:.1}{upcoming}",
                name(&c.ai_pick)
            ),
            warning: false,
            buttons,
            current,
        }
    }

    /// Applies a card's event to its group, which first becomes the current
    /// group through Review's guard: an unconfirmed change in another group
    /// keeps the focus, and the action, where it is.
    pub(super) fn pick_card_event(&mut self, index: usize, event: CardEvent) {
        self.pending_bookmark = None;
        if self.pick_review.current_index() != index && self.allow_review_departure() {
            self.pick_review.select_cluster(index);
            self.sync_pick_view();
        }
        if self.pick_review.current_index() != index {
            return;
        }
        let paths = self
            .pick_review
            .comparison()
            .map(|c| c.paths.clone())
            .unwrap_or_default();
        match event {
            CardEvent::Select(side) | CardEvent::Open(side) => {
                if let Some(id) = paths.get(side) {
                    self.set_active(*id);
                }
                if matches!(event, CardEvent::Open(_)) {
                    self.open_review();
                }
            }
            CardEvent::Button(side) if side < paths.len() => self.toggle_pick(paths[side]),
            CardEvent::Button(button) if button == paths.len() => self.confirm_pick(false),
            CardEvent::Button(button) if button == paths.len() + 1 => self.confirm_pick(true),
            CardEvent::Button(_) => self.reset_pick(false),
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
        app.pick_review.replace(
            vec![crate::core::pick_review::Cluster {
                key: 1,
                ordered: vec![MediaId(1), MediaId(2), MediaId(3)],
                scores: HashMap::from([(MediaId(1), 10.), (MediaId(2), 20.), (MediaId(3), 30.)]),
            }],
            &mut app.decisions,
        );
        app.set_step(Step::PickBest);
        app.refilter();
        app
    }
    #[test]
    fn focus_exits_after_confirmation_and_shows_both_next_candidates() {
        let mut app = app();
        app.toggle_pick_focus();
        assert_eq!(app.comparison_ids().len(), 1);
        app.confirm_pick(false);
        assert!(!app.pick_focus);
        assert_eq!(app.comparison_ids(), vec![MediaId(2), MediaId(3)]);
        app.refilter();
        assert_eq!(app.filtered, vec![1, 2]);
    }
    #[test]
    fn provisional_pick_blocks_departure_and_generic_mark_does_not_publish() {
        let mut app = app();
        app.reject(MediaId(1));
        assert!(app.decisions.is_empty());
        app.set_active(MediaId(2));
        assert_eq!(app.active, Some(MediaId(2)));
        assert!(app.review_notice.is_none());
        app.switch_step(Step::Cull);
        assert_eq!(app.review_notice, Some(Step::PickBest));
        assert_eq!(app.step, Step::PickBest);
        app.reset_pick(false);
        app.switch_step(Step::Cull);
        assert_eq!(app.step, Step::Cull);
    }
    #[test]
    fn applying_requires_a_completed_cluster_and_keeps_pending_choices_local() {
        let mut app = app();
        app.confirm_pick(false);
        app.review_changes();
        assert!(app.status.contains("Complete every comparison"));
        assert!(!app.plan_open);
        app.toggle_pick(MediaId(2));
        app.review_changes();
        assert_eq!(app.review_notice, Some(Step::PickBest));
    }
    #[test]
    fn workflow_keys_toggle_keep_and_keep_all_without_staging_ratings() {
        let mut app = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        for key in [egui::Key::Num1, egui::Key::C, egui::Key::K] {
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            let _ = ctx.run_test(input, |ui| {
                let ctx = &ui.ctx().clone();
                app.keyboard(ctx)
            });
        }
        assert!(!app.pick_focus);
        assert_eq!(app.pick_review.current().unwrap().current, 1);
        assert!(
            app.decisions
                .values()
                .all(|d| !d.rejected && d.rating.is_none())
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
            app.after_apply = Some(destination);
            loaded_folder(&mut app);
            assert!(app.pick_run.is_none());
            assert!(app.analysis_run.is_none());
            assert!(app.step == Step::Cull || app.close_authorized);
        }
    }
    #[test]
    fn popup_keys_do_not_confirm_or_change_the_underlying_review() {
        let mut app = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        for key in [
            egui::Key::Num1,
            egui::Key::Enter,
            egui::Key::ArrowDown,
            egui::Key::Escape,
        ] {
            let input = egui::RawInput {
                events: vec![egui::Event::Key {
                    key,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                ..Default::default()
            };
            let _ = ctx.run_test(input, |ui| {
                let ctx = &ui.ctx().clone();
                {
                    egui::Popup::open_id(ctx, egui::Id::new("comparison-menu"));
                    app.keyboard(ctx);
                    assert!(
                        ctx.input(|i| i.key_pressed(key)),
                        "Leave keys available to the popup"
                    );
                }
            });
        }
        assert!(!app.pick_review.unfinished());
        assert_eq!(app.pick_review.current().unwrap().current, 0);
        assert!(app.decisions.is_empty());
        assert!(app.review_notice.is_none());
    }
    fn two_groups() -> PhotoSortApp {
        let mut app = app();
        app.media.push(Media {
            id: MediaId(4),
            path: "/4.jpg".into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        });
        app.index.insert(MediaId(4), 3);
        app.pick_review.replace(
            vec![
                crate::core::pick_review::Cluster {
                    key: 1,
                    ordered: vec![MediaId(1), MediaId(2)],
                    scores: HashMap::from([(MediaId(1), 10.), (MediaId(2), 20.)]),
                },
                crate::core::pick_review::Cluster {
                    key: 2,
                    ordered: vec![MediaId(3), MediaId(4)],
                    scores: HashMap::from([(MediaId(3), 10.), (MediaId(4), 20.)]),
                },
            ],
            &mut app.decisions,
        );
        app.sync_pick_view();
        app.refilter();
        app
    }
    #[test]
    fn grid_shows_each_groups_round_with_the_suggested_keeper() {
        let mut app = two_groups();
        let shown = super::super::review_card::render_grid(&mut app);
        for text in [
            "Group 1 · 2 photos",
            "Group 2 · 2 photos",
            "Round 1 of 1",
            "Suggested",
            "Suggested keeper: 2.jpg",
            "Keep all",
        ] {
            assert!(shown.contains(text), "{text} missing from:\n{shown}");
        }
        assert!(app.decisions.is_empty());
    }
    #[test]
    fn pick_card_actions_select_their_group_and_respect_pending_choices() {
        let mut app = two_groups();
        // Keep the second group's lower-scored photo too.
        app.pick_card_event(1, CardEvent::Button(0));
        assert_eq!(app.pick_review.current_index(), 1);
        assert!(app.pick_review.unfinished());
        // The first group cannot be confirmed while that is pending.
        app.pick_card_event(0, CardEvent::Button(2));
        assert_eq!(app.pick_review.current_index(), 1);
        assert_eq!(app.review_notice, Some(Step::PickBest));
        assert!(app.decisions.is_empty());
        app.review_notice = None;
        // Reset that group from its card, then confirm the first group.
        app.pick_card_event(1, CardEvent::Button(4));
        assert!(!app.pick_review.unfinished());
        app.pick_card_event(0, CardEvent::Button(2));
        assert!(app.decisions[&MediaId(1)].rejected);
        assert!(!app.decisions.get(&MediaId(2)).is_some_and(|d| d.rejected));
        // Keep all on the second group keeps both.
        app.pick_card_event(1, CardEvent::Button(3));
        assert!(app.pick_review.tournaments()[1].finalized);
        assert!(!app.decisions.get(&MediaId(3)).is_some_and(|d| d.rejected));
        app.pick_card_event(0, CardEvent::Open(0));
        assert!(app.review);
        assert_eq!(app.pick_review.current_index(), 0);
    }
}
