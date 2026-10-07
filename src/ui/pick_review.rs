use super::*;
impl PhotoSortApp {
    pub(super) fn start_ready_pick_analysis(&mut self) {
        if self.step == Step::PickBest
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
            && self.pick_review.tournaments().is_empty()
        {
            self.analyze();
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
            format!(
                "{} clusters ready · choose Keep/Trash, then confirm",
                self.pick_review.tournaments().len()
            ),
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
    pub(super) fn pick_controls(&mut self, ui: &mut egui::Ui) {
        if self.pick_run.is_some() {
            return;
        }
        let current = self.pick_review.current_index();
        let Some(t) = self.pick_review.current() else {
            return;
        };
        let mut selected_cluster = current;
        let mut selected_round = t.current;
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("pick_cluster")
                .selected_text(format!(
                    "Cluster {} / {} · {} photos",
                    current + 1,
                    self.pick_review.tournaments().len(),
                    t.cluster.ordered.len()
                ))
                .show_ui(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("pick_clusters_scroll")
                        .max_height(240.0)
                        .show_rows(
                            ui,
                            ui.spacing().interact_size.y,
                            self.pick_review.tournaments().len(),
                            |ui, rows| {
                                for i in rows {
                                    let t = &self.pick_review.tournaments()[i];
                                    if ui
                                        .selectable_value(
                                            &mut selected_cluster,
                                            i,
                                            format!(
                                                "Cluster {} · {} photos · {}",
                                                i + 1,
                                                t.cluster.ordered.len(),
                                                if t.finalized {
                                                    "Complete"
                                                } else if t.comparisons.iter().any(|c| c.confirmed)
                                                {
                                                    "In progress"
                                                } else {
                                                    "Not started"
                                                }
                                            ),
                                        )
                                        .clicked()
                                    {
                                        ui.close();
                                    }
                                }
                            },
                        );
                });
            egui::ComboBox::from_id_salt("pick_comparison")
                .selected_text(format!(
                    "Comparison {}{}",
                    t.current + 1,
                    if t.comparison().confirmed {
                        " · Confirmed"
                    } else {
                        ""
                    }
                ))
                .show_ui(ui, |ui| {
                    ui.set_width(ui.ctx().content_rect().width().min(520.0) - 48.0);
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                    egui::ScrollArea::vertical()
                        .id_salt("pick_comparisons_scroll")
                        .max_height(240.0)
                        .show_rows(
                            ui,
                            ui.spacing().interact_size.y,
                            t.comparisons.len() + t.upcoming().len(),
                            |ui, rows| {
                                for i in rows {
                                    if let Some(c) = t.comparisons.get(i) {
                                        let names = c
                                            .paths
                                            .iter()
                                            .map(|id| {
                                                self.index
                                                    .get(id)
                                                    .map(|i| self.media[*i].name())
                                                    .unwrap_or_default()
                                            })
                                            .collect::<Vec<_>>()
                                            .join(" ↔ ");
                                        if ui
                                            .selectable_value(
                                                &mut selected_round,
                                                i,
                                                format!(
                                                    "{} · {} · {}",
                                                    i + 1,
                                                    names,
                                                    if c.confirmed {
                                                        "Confirmed"
                                                    } else {
                                                        "Needs review"
                                                    }
                                                ),
                                            )
                                            .on_hover_text(names)
                                            .clicked()
                                        {
                                            ui.close();
                                        }
                                    } else {
                                        let id = t.upcoming()[i - t.comparisons.len()];
                                        let name = self
                                            .index
                                            .get(&id)
                                            .map(|i| self.media[*i].name())
                                            .unwrap_or_default();
                                        ui.add_enabled(
                                            false,
                                            egui::Button::new(format!("Up next · {name}"))
                                                .frame(false),
                                        )
                                        .on_disabled_hover_text(name);
                                    }
                                }
                            },
                        );
                });
            ui.label(format!(
                "{} clusters complete",
                self.pick_review
                    .tournaments()
                    .iter()
                    .filter(|t| t.finalized)
                    .count()
            ));
        });
        if selected_cluster != current && self.allow_review_departure() {
            self.pick_review.select_cluster(selected_cluster);
            self.sync_pick_view();
        } else if self
            .pick_review
            .current()
            .is_some_and(|t| t.current != selected_round)
            && self.allow_review_departure()
        {
            self.pick_review.select_comparison(selected_round);
            self.sync_pick_view();
        }
        let confirmed = self.pick_review.comparison().is_some_and(|c| c.confirmed);
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    !confirmed,
                    egui::Button::new(if confirmed { "Confirmed" } else { "Confirm >" }),
                )
                .clicked()
            {
                self.confirm_pick(false);
            }
            if ui.button("Reset default").clicked() {
                self.reset_pick(false);
            }
            if ui.button("Reset all").clicked() {
                self.reset_pick(true);
            }
            if ui.button("Keep all").clicked() {
                self.confirm_pick(true);
            }
            if ui
                .selectable_label(self.pick_focus, "Focus photo (C)")
                .clicked()
            {
                self.toggle_pick_focus();
            }
            if ui
                .add_enabled(
                    self.pick_review.any_completed(),
                    egui::Button::new("Review changes…"),
                )
                .clicked()
            {
                self.review_changes();
            }
        });
        if let Some(c) = self.pick_review.comparison() {
            let name = self
                .index
                .get(&c.ai_pick)
                .map(|i| self.media[*i].name())
                .unwrap_or_default();
            let score = self.pick_review.current().unwrap().cluster.scores[&c.ai_pick];
            ui.label(
                RichText::new(format!(
                    "Suggested: {name} · score {score:.1} · {}",
                    if c.confirmed {
                        "Confirmed choices"
                    } else {
                        "Choose Keep/Trash, then confirm"
                    }
                ))
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
            let _ = ctx.run(input, |ctx| app.keyboard(ctx));
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
                    Arc::new(Catalog::default()),
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
            let _ = ctx.run(input, |ctx| {
                egui::Popup::open_id(ctx, egui::Id::new("comparison-menu"));
                app.keyboard(ctx);
                assert!(
                    ctx.input(|i| i.key_pressed(key)),
                    "Leave keys available to the popup"
                );
            });
        }
        assert!(!app.pick_review.unfinished());
        assert_eq!(app.pick_review.current().unwrap().current, 0);
        assert!(app.decisions.is_empty());
        assert!(app.review_notice.is_none());
    }
}
