use super::*;

impl PhotoSortApp {
    pub(super) fn header(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("header")
            .exact_height(68.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .inner_margin(egui::Margin::symmetric(24, 14)),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.label(RichText::new("PS").size(27.0).color(ACCENT));
                    ui.label(RichText::new("PhotoSort").size(21.0).strong());
                    ui.add_space(15.0);
                    if ctx.content_rect().width() > 850.0
                        && let Some(root) = &self.root
                    {
                        ui.label(
                            RichText::new(root.file_name().unwrap_or_default().to_string_lossy())
                                .color(MUTED),
                        )
                        .on_hover_text(root.display().to_string());
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        self.app_menu(ui);
                        if ui
                            .add_enabled(!self.busy, egui::Button::new("Open folder"))
                            .clicked()
                        {
                            self.select_folder();
                        }
                        if (self.catalog.is_some() || self.retry_plan.is_some())
                            && ui
                                .add_enabled(
                                    !self.busy,
                                    egui::Button::new(RichText::new("Review changes").color(BG))
                                        .fill(ACCENT)
                                        .stroke(Stroke::NONE),
                                )
                                .clicked()
                        {
                            self.review_changes();
                        }
                    });
                });
            });
    }

    pub(super) fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("steps")
            .exact_width(214.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(PANEL).inner_margin(20))
            .show(ctx, |ui| {
                ui.add_space(18.0);
                ui.label(RichText::new("YOUR WORKFLOW").small().color(MUTED));
                ui.add_space(22.0);
                for step in self.navigation_steps() {
                    let selected = self.step == step;
                    let (rect, response) = ui.allocate_exact_size(
                        egui::vec2(ui.available_width(), 52.0),
                        Sense::click(),
                    );
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Button,
                            true,
                            selected,
                            format!("{} {}", step.index() + 1, step.title()),
                        )
                    });
                    if selected || response.hovered() {
                        ui.painter().rect_filled(
                            rect,
                            8.0,
                            if selected {
                                Color32::from_rgb(40, 61, 53)
                            } else {
                                CARD
                            },
                        );
                    }
                    let color = if selected { ACCENT } else { MUTED };
                    ui.painter().text(
                        rect.left_center() + egui::vec2(15.0, 0.0),
                        Align2::LEFT_CENTER,
                        format!("{:02}", step.index() + 1),
                        FontId::monospace(13.0),
                        color,
                    );
                    ui.painter().text(
                        rect.left_center() + egui::vec2(49.0, 0.0),
                        Align2::LEFT_CENTER,
                        step.title(),
                        FontId::proportional(14.0),
                        if selected { INK } else { MUTED },
                    );
                    if response.clicked() {
                        self.switch_step(step);
                    }
                    ui.add_space(5.0);
                }
                ui.add_space(24.0);
                ui.separator();
                ui.add_space(12.0);
                ui.label(
                    RichText::new("A little less.\nA lot more meaning.")
                        .size(18.0)
                        .color(MUTED),
                );
                ui.with_layout(egui::Layout::bottom_up(Align::LEFT), |ui| {
                    ui.label(
                        RichText::new("Local files. Your decisions.")
                            .small()
                            .color(MUTED),
                    );
                    ui.label(
                        RichText::new("NATIVE RUST  /  PREVIEW")
                            .small()
                            .color(ACCENT),
                    );
                });
            });
    }

    pub(super) fn switch_step(&mut self, step: Step) {
        if self.step != step && self.preferences.visible(step) {
            self.request_departure(Destination::Step(step));
        }
    }
    pub(super) fn set_step(&mut self, step: Step) {
        self.browse_filters_open = false;
        if self.step != step {
            self.organize_preview_requested = false;
        }
        self.checkpoint_bookmarks();
        self.runtime.browsing.cancel();
        self.browse_run = None;
        self.cancel_metadata_index();
        self.step = step;
        self.discard_stale_model_retry();
        self.navigation_notice = None;
        self.selection.clear();
        self.library_reveal = true;
        self.comparison.clear();
        self.filter = Filter::All;
        self.filter_dirty = true;
        self.review = step != Step::Organize;
        self.fit_view();
        self.view_started = Instant::now();
        if step == Step::PickBest {
            self.sync_pick_view();
        }
        self.start_ready_analysis();
        self.prepare_bookmark();
        if step == Step::Cull || self.metadata_organize_mode() {
            self.start_metadata_index();
        }
    }

    pub(super) fn set_group_mode(&mut self, mode: GroupMode) {
        if self.group_mode != mode {
            // An explicit choice supersedes deferred view restoration, whether
            // it came from the dropdown or the workflow's mode shortcuts.
            self.pending_bookmark = None;
            let was_metadata = self.metadata_organize_mode();
            self.group_mode = mode;
            self.organize_preview_requested = false;
            self.discard_stale_model_retry();
            if self.metadata_organize_mode() {
                self.start_metadata_index();
            } else if was_metadata {
                self.cancel_metadata_index();
            }
        }
    }

    pub(super) fn compact_steps(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("compact_steps").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                for step in self.navigation_steps() {
                    if ui
                        .selectable_label(
                            self.step == step,
                            format!("{}  {}", step.index() + 1, step.title()),
                        )
                        .clicked()
                    {
                        self.switch_step(step);
                    }
                }
            });
        });
    }

    pub(super) fn footer(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(36.0)
            .show(ctx, |ui| {
                // Reserve controls at the right edge before fitting status text.
                // Stop must remain reachable even with long paths in small windows.
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if let Some(stop) = &self.plan_stop {
                        let stopping = stop.load(std::sync::atomic::Ordering::Acquire);
                        if ui.add_enabled(!stopping, egui::Button::new(if stopping { "Cancelling…" } else { "Cancel planning" })).clicked() {
                            stop.store(true, std::sync::atomic::Ordering::Release);
                        }
                    }
                    if let Some(stop) = &self.apply_stop {
                        let stopping = stop.load(std::sync::atomic::Ordering::Acquire);
                        if ui.add_enabled(!stopping, egui::Button::new(if stopping { "Stopping…" } else { "Stop" })
                            .min_size(egui::vec2(96.0, 24.0)))
                            .on_hover_text("Stop safely. Completed changes stay applied; unfinished work stays available for Retry.")
                            .clicked()
                        {
                            self.stop_file_operations();
                        }
                    }
                    if let Some((done, total)) = self.progress {
                        ui.add(egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                            .desired_width(150.0).text(format!("{done} / {total}")));
                    } else if self.apply_stop.is_none() {
                        ui.label(RichText::new(format!("{} visible  ·  {:.0} MB cache",
                            self.filtered.len(), self.stats.resident_bytes as f64 / 1048576.0))
                            .small().color(MUTED));
                    }
                    ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                        if self.scanning || self.busy || self.analysis_run.is_some() {
                            ui.spinner();
                        } else {
                            ui.colored_label(ACCENT, "Ready");
                        }
                        let recovery_status = self.model_recovery_status(&self.runtime.model_assets.state());
                        let scoped_status = self.workflow_status();
                        let status = self.navigation_notice.as_deref().or(recovery_status).unwrap_or(&scoped_status);
                        ui.add(egui::Label::new(RichText::new(status).small().color(MUTED)).truncate())
                            .on_hover_text(status);
                    });
                });
            });
    }

    pub(super) fn toolbar(&mut self, ui: &mut egui::Ui) {
        let before = (
            self.query.clone(),
            self.filter,
            self.browse.clone(),
            self.library_layout,
            self.review,
            self.inspector,
            self.group_mode,
            self.delete_categories.clone(),
            self.rotation_queue_only,
            self.pick_focus,
        );
        let compact = ui.available_width() < 850.0;
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                if !compact {
                    ui.label(
                        RichText::new(format!(
                            "{:02} / {}",
                            self.step.index() + 1,
                            self.step.title().to_uppercase()
                        ))
                        .small()
                        .color(ACCENT),
                    );
                }
                if compact {
                    ui.label(RichText::new(self.step.title()).size(24.0));
                } else {
                    ui.heading(self.step.subtitle());
                }
            });
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.selectable_label(self.inspector, "Details").clicked() {
                    self.inspector ^= true;
                }
                if ui.selectable_label(self.review, "Review").clicked() {
                    self.review = true;
                }
                if self.step == Step::Cull {
                    egui::ComboBox::from_id_salt("library_layout")
                        .width(76.0)
                        .selected_text(self.library_layout.title())
                        .show_ui(ui, |ui| {
                            for layout in library::Layout::ALL {
                                if ui
                                    .selectable_label(self.library_layout == layout, layout.title())
                                    .clicked()
                                {
                                    self.set_library_layout(layout);
                                    ui.close();
                                }
                            }
                        });
                } else if ui.selectable_label(!self.review, "Grid").clicked() {
                    self.review = false;
                }
            });
        });
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if self.step != Step::PickBest {
                let mut query = self.query.clone();
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut query)
                            .id(egui::Id::new("library_search"))
                            .hint_text("Search your library…")
                            .desired_width(190.0),
                    )
                    .changed()
                    && self.allow_review_departure()
                {
                    self.query = query;
                    self.filter_dirty = true;
                }
                egui::ComboBox::from_id_salt("filter")
                    .selected_text(self.filter.title())
                    .show_ui(ui, |ui| {
                        for f in Filter::ALL {
                            if ui.selectable_label(self.filter == f, f.title()).clicked()
                                && self.allow_review_departure()
                            {
                                self.filter = f;
                                self.filter_dirty = true;
                                ui.close();
                                if f == Filter::Rated {
                                    self.start_metadata_index();
                                }
                            }
                        }
                    });
            }
            if self.analysis_run.is_some() {
                if ui.button("Cancel analysis").clicked() {
                    self.cancel_analysis();
                }
            } else if self.step != Step::Rotation
                && ui
                    .add_enabled(
                        self.can_start_analysis(),
                        egui::Button::new("Analyze library"),
                    )
                    .on_hover_text("Analyze this workflow locally · Ctrl/Cmd+S")
                    .clicked()
            {
                self.analyze_current_workflow();
            }
        });
        ui.add_space(4.0);
        match self.step {
            Step::Organize => {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("Group by").color(MUTED));
                    egui::ComboBox::from_id_salt("grouping")
                        .selected_text(self.group_mode.title())
                        .show_ui(ui, |ui| {
                            for mode in GroupMode::ALL {
                                if ui
                                    .selectable_label(self.group_mode == mode, mode.title())
                                    .clicked()
                                {
                                    self.set_group_mode(mode);
                                    ui.close();
                                }
                            }
                        });
                    if self.group_mode == GroupMode::Location {
                        self.location_layout_controls(ui);
                    }
                    if self.scene_editing() {
                        self.group_controls(ui);
                        if ui
                            .add_enabled(
                                (2..=9).contains(&self.selection.len()) && !self.busy,
                                egui::Button::new("Compare selected"),
                            )
                            .clicked()
                        {
                            self.compare_selection();
                        }
                    }
                    ui.checkbox(&mut self.companions, "Keep RAW + JPEG + XMP together");
                    if ui
                        .add_enabled(self.can_edit_folders(), egui::Button::new("Edit folders…"))
                        .clicked()
                    {
                        self.open_organize_editor();
                    }
                    if ui
                        .add_enabled(
                            !self.scanning && !self.busy && self.organize_metadata_ready(),
                            egui::Button::new("Preview moves >"),
                        )
                        .clicked()
                    {
                        self.organize();
                    }
                });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Destination").small().color(MUTED));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output)
                            .desired_width((ui.available_width() - 55.0).max(100.0)),
                    );
                    if ui.button("…").clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        self.output = path.display().to_string();
                    }
                });
                if self.group_mode == GroupMode::Mixed {
                    ui.label(RichText::new(if self.mixed_labels.is_some() {
                        "Dates contain similar-photo groups; single photos stay in the date folder."
                    } else { "Preview moves analyzes dates and similarity first. Your files stay unchanged until Apply." }).small().color(MUTED));
                } else if self.group_mode == GroupMode::Faces {
                    ui.label(RichText::new("Groups photos by the main people. Group portraits get one shared folder, such as People 001 + 003. Unclear matches stay unassigned. Preview moves analyzes first; Apply moves the files.").small().color(MUTED));
                } else if self.group_mode == GroupMode::Visual {
                    ui.label(RichText::new("Select photos with Cmd/Ctrl-click or Shift-click, then use Scenes to correct groups. Preview moves analyzes first. Ungrouped photos and videos stay in place.").small().color(MUTED));
                } else if self.metadata_organize_mode() {
                    ui.label(RichText::new("Folder proposals use photo metadata. Location names are resolved offline; AI analysis is not needed.").small().color(MUTED));
                    self.metadata_status(ui);
                }
            }
            Step::EasyDelete => self.delete_controls(ui),
            Step::Rotation => self.rotation_controls(ui),
            Step::PickBest => self.pick_controls(ui),
            Step::Cull => {
                self.browse_controls(ui);
                if !self.review {
                    self.selection_controls(ui);
                }
            }
        }
        ui.add_space(6.0);
        ui.separator();
        ui.add_space(8.0);
        let after = (
            self.query.clone(),
            self.filter,
            self.browse.clone(),
            self.library_layout,
            self.review,
            self.inspector,
            self.group_mode,
            self.delete_categories.clone(),
            self.rotation_queue_only,
            self.pick_focus,
        );
        if before != after {
            self.pending_bookmark = None;
        }
    }

    pub(super) fn welcome(&mut self, ui: &mut egui::Ui) {
        let width = ui.available_width();
        ui.add_space((ui.available_height() * 0.15).max(25.0));
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("PS").size(62.0).color(ACCENT));
            ui.add_space(20.0);
            ui.label(
                RichText::new("Make room for your best work.")
                    .size(if width > 750.0 { 38.0 } else { 27.0 })
                    .strong(),
            );
            ui.add_space(12.0);
            ui.label(
                RichText::new(
                    "A calmer way from a full memory card to a collection worth keeping.",
                )
                .size(15.0)
                .color(MUTED),
            );
            ui.add_space(28.0);
            if ui
                .add(
                    egui::Button::new(RichText::new("Open a photo folder  >").color(BG).strong())
                        .fill(ACCENT)
                        .min_size(egui::vec2(225.0, 46.0)),
                )
                .clicked()
            {
                self.select_folder();
            }
            ui.add_space(12.0);
            ui.label(
                RichText::new("Or drop a folder anywhere in this window")
                    .small()
                    .color(MUTED),
            );
            ui.add_space(40.0);
            ui.horizontal_wrapped(|ui| {
                for step in self.navigation_steps() {
                    egui::Frame::new()
                        .fill(CARD)
                        .corner_radius(10)
                        .inner_margin(18)
                        .show(ui, |ui| {
                            ui.set_min_width(110.0);
                            ui.label(
                                RichText::new(format!("{:02}", step.index() + 1))
                                    .color(ACCENT)
                                    .small(),
                            );
                            ui.label(RichText::new(step.title()).strong());
                        });
                }
            });
            ui.add_space(22.0);
            ui.label(
                RichText::new(
                    "Start anywhere. Return to any step. Keep every decision in your hands.",
                )
                .small()
                .color(MUTED),
            );
        });
    }
}

#[cfg(test)]
mod cull_navigation_tests {
    use super::*;
    fn fixture() -> PhotoSortApp {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.step = Step::Cull;
        app.media = (0..5)
            .map(|i| Media {
                id: MediaId(i),
                path: format!("/missing/{i}.jpg").into(),
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
        app.filtered = (0..5).collect();
        app.active = Some(MediaId(0));
        for i in [1, 2] {
            app.decisions.insert(
                MediaId(i),
                Decision {
                    rejected: true,
                    ..Default::default()
                },
            );
        }
        app
    }
    #[test]
    fn cull_skips_marks_and_shift_comparison_uses_the_same_target() {
        let mut app = fixture();
        app.review = true;
        app.navigate_selection(1, false, false);
        assert_eq!(app.active, Some(MediaId(3)));
        app.navigate_selection(-1, false, true);
        assert_eq!(app.active, Some(MediaId(2)));
        app.navigate_selection(-1, false, false);
        assert_eq!(app.active, Some(MediaId(0)));
        app.navigate_selection(1, true, false);
        assert_eq!(app.comparison, vec![MediaId(0), MediaId(3)]);
        assert_eq!(app.active, Some(MediaId(3)));
        app.navigate_selection(-1, true, true);
        assert_eq!(app.comparison, vec![MediaId(0), MediaId(3), MediaId(2)]);
        assert_eq!(app.stats.decodes, 0);
    }
    #[test]
    fn all_marked_edges_and_other_workflows_preserve_expected_navigation() {
        let mut app = fixture();
        for m in &app.media {
            app.decisions.entry(m.id).or_default().rejected = true;
        }
        app.inspection_view.zoom = 3.0;
        app.navigate_selection(1, false, false);
        assert_eq!(app.active, Some(MediaId(0)));
        assert_eq!(
            app.inspection_view.zoom, 3.0,
            "no candidate must not reset zoom"
        );
        app.filter = Filter::Rejected;
        app.navigate_selection(1, false, false);
        assert_eq!(
            app.active,
            Some(MediaId(1)),
            "explicit Trash-only review includes marks"
        );
        app.filter = Filter::All;
        for step in [Step::Organize, Step::Rotation] {
            app.step = step;
            app.active = Some(MediaId(0));
            app.navigate_selection(1, false, false);
            assert_eq!(app.active, Some(MediaId(1)));
        }
    }
    #[test]
    fn plain_and_ctrl_or_command_arrow_events_route_skip_override() {
        for modifiers in [
            egui::Modifiers::NONE,
            egui::Modifiers::CTRL,
            egui::Modifiers::COMMAND,
        ] {
            let mut app = fixture();
            let ctx = egui::Context::default();
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key: egui::Key::ArrowDown,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ctx| app.keyboard(ctx),
            );
            assert_eq!(
                app.active,
                Some(MediaId(if modifiers == egui::Modifiers::NONE {
                    3
                } else {
                    1
                }))
            );
        }
    }
    #[test]
    fn shared_navigation_preferences_route_buttons_keys_and_preserve_other_workflows() {
        let mut app = fixture();
        app.decisions.clear();
        app.filter_dirty = false;
        app.navigation_index = Some(Arc::new(
            crate::core::navigation::Index::build(
                (0..5).map(|i| (MediaId(i), Some(i as usize / 2), Some(i as u8), false)),
                || false,
            )
            .unwrap(),
        ));
        app.status = "Analysis results retained".into();
        app.preferences.navigation.rating = Some(3);
        app.navigate_selection(1, false, false);
        assert_eq!(app.active, Some(MediaId(3)));
        app.navigate_selection(1, false, false);
        assert_eq!(app.active, Some(MediaId(3)));
        assert!(
            app.navigation_notice
                .as_deref()
                .unwrap()
                .contains("No more photos")
        );
        assert_eq!(app.status, "Analysis results retained");
        app.navigate_selection(1, false, true);
        assert_eq!(app.active, Some(MediaId(4)));
        assert!(app.navigation_notice.is_none());
        assert_eq!(app.status, "Analysis results retained");
        app.navigate_selection(1, false, false);
        assert!(app.navigation_notice.is_some());
        app.set_active(MediaId(3));
        assert!(
            app.navigation_notice.is_none(),
            "pointer selection also clears movement feedback"
        );
        app.preferences.navigation.cycle_similarity = true;
        app.active = Some(MediaId(3));
        app.navigate_axis(1, false, false, true);
        assert_eq!(app.active, Some(MediaId(2)));
        app.browse_run = Some(99);
        app.navigate_axis(1, false, false, true);
        assert_eq!(app.active, Some(MediaId(2)));
        app.browse_run = None;
        app.filter_dirty = true;
        app.navigate_axis(1, false, false, true);
        assert_eq!(app.active, Some(MediaId(2)));
        app.filter_dirty = false;
        app.active = Some(MediaId(4));
        app.inspection_view.zoom = 3.0;
        app.navigate_axis(1, false, false, true);
        assert_eq!(
            app.inspection_view.zoom, 3.0,
            "single-photo cycle preserves inspection"
        );
        for step in [Step::Organize, Step::Rotation] {
            app.step = step;
            app.active = Some(MediaId(0));
            app.navigate_selection(1, false, false);
            assert_eq!(app.active, Some(MediaId(1)));
        }
        app.step = Step::Cull;
        app.active = Some(MediaId(0));
        app.navigation_index = None;
        app.scanning = true;
        app.navigate_axis(1, false, false, true);
        assert_eq!(app.active, Some(MediaId(0)));
        app.scanning = false;
        app.browse_run = Some(123);
        app.navigate_selection(1, false, false);
        assert_eq!(app.active, Some(MediaId(0)));
        app.browse_run = None;
        app.busy = true;
        app.navigate_selection(1, false, true);
        assert_eq!(app.active, Some(MediaId(0)));
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        assert!(app.requested.is_empty() && app.analysis_run.is_none());
    }
}
