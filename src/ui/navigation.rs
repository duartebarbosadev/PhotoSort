use super::*;

/// Height of the bottom page bar (status, step tabs, progress); photos and
/// controls must end above it.
pub(super) const STATUS_HEIGHT: f32 = 44.0;
/// Height of the title bar.
pub(super) const HEADER_HEIGHT: f32 = 38.0;

impl Step {
    /// The page icon in the bottom step bar.
    fn icon(self) -> &'static str {
        use egui_phosphor::regular as icon;
        match self {
            Step::Organize => icon::FOLDERS,
            Step::EasyDelete => icon::BROOM,
            Step::Rotation => icon::ARROW_CLOCKWISE,
            Step::PickBest => icon::CROWN_SIMPLE,
            Step::Cull => icon::STAR,
        }
    }
}

/// Widths of the bottom bar's page tabs: icon and name, or icon and number.
fn step_tab_widths(ui: &egui::Ui, steps: &[Step], compact: bool) -> Vec<f32> {
    steps
        .iter()
        .map(|step| {
            if compact {
                36.0
            } else {
                let text = ui
                    .painter()
                    .layout_no_wrap(step.title().into(), medium(10.5), INK)
                    .size()
                    .x;
                (text + 2.0 * S4).max(64.0)
            }
        })
        .collect()
}

impl PhotoSortApp {
    /// The title bar: brand and folder on the left, the staged-change review
    /// and app actions on the right. The steps live in the bottom page bar.
    pub(super) fn header(&mut self, root: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        egui::Panel::top("header")
            .exact_size(HEADER_HEIGHT)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(S3 as i8, 0)),
            )
            .show(root, |ui| {
                ui.horizontal_centered(|ui| {
                    ui.spacing_mut().button_padding = egui::vec2(S2, 3.0);
                    ui.label(RichText::new(icon::APERTURE).size(16.0).color(ACCENT));
                    ui.label(RichText::new("PhotoSort").font(semibold(12.5)));
                    let left_end = ui.min_rect().right();
                    let right_start = ui
                        .with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            self.header_actions(ui);
                            ui.min_rect().left()
                        })
                        .inner;
                    // The library name sits centred, like a project title.
                    if let Some(root) = &self.root {
                        let mid = ui.ctx().content_rect().center().x;
                        let half = (mid - left_end).min(right_start - mid) - S4;
                        let name = root.file_name().unwrap_or_default().to_string_lossy();
                        let galley =
                            ui.painter()
                                .layout_no_wrap(name.to_string(), medium(12.0), INK);
                        let width = galley.size().x.min(2.0 * half);
                        if width > 40.0 {
                            let rect = egui::Rect::from_center_size(
                                egui::pos2(mid, ui.max_rect().center().y),
                                egui::vec2(width, HEADER_HEIGHT - 8.0),
                            );
                            ui.put(
                                rect,
                                egui::Label::new(RichText::new(name).font(medium(12.0))).truncate(),
                            )
                            .on_hover_text(root.display().to_string());
                        }
                    }
                });
            });
    }
    fn header_actions(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        self.app_menu(ui);
        if icon_button(ui, icon::FOLDER_OPEN, "Open folder… (Ctrl/Cmd+O)").clicked() && !self.busy
        {
            self.select_folder();
        }
        if self.catalog.is_some() || self.retry_plan.is_some() {
            // Accent only when there is something to review, so the title
            // bar doubles as the staged-change indicator.
            let staged = self.staged_change_count();
            let button = if staged > 0 || self.retry_plan.is_some() {
                primary(if staged > 0 {
                    format!(
                        "Review {staged} {}",
                        if staged == 1 { "change" } else { "changes" }
                    )
                } else {
                    "Review changes".into()
                })
            } else {
                egui::Button::new(RichText::new("Review changes").color(MUTED))
            };
            if ui
                .add_enabled(!self.busy, button)
                .on_hover_text("See every staged change before applying · Shift+Enter")
                .clicked()
            {
                self.review_changes();
            }
        }
    }

    /// The workflow pages as tabs in the bottom bar, as in an editing suite.
    fn step_tabs(&mut self, ui: &mut egui::Ui, area: egui::Rect, compact: bool) {
        let steps = self.navigation_steps();
        let widths = step_tab_widths(ui, &steps, compact);
        let total: f32 = widths.iter().sum();
        let mut x = area.center().x - total / 2.0;
        let height = area.height() - 8.0;
        for (step, width) in steps.iter().zip(widths) {
            let rect = egui::Rect::from_min_size(
                egui::pos2(x, area.center().y - height / 2.0),
                egui::vec2(width, height),
            );
            x += width;
            let active = *step == self.step;
            let response = ui
                .interact(rect, egui::Id::new(("step-tab", *step)), Sense::click())
                .on_hover_text(format!(
                    "{} {} · {} · Ctrl/Cmd+Alt+{}",
                    step.index() + 1,
                    step.title(),
                    step.subtitle(),
                    step.index() + 1
                ));
            let name = format!("{} {}", step.index() + 1, step.title());
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, &name)
            });
            let hover = ui
                .ctx()
                .animate_bool_responsive(response.id.with("hover"), response.hovered());
            let painter = ui.painter();
            let selected = ui
                .ctx()
                .animate_bool_responsive(response.id.with("active"), active);
            let pill = rect.shrink2(egui::vec2(3.0, 1.0));
            let lift = (selected + hover * 0.6).min(1.0);
            if lift > 0.0 {
                painter.rect_filled(pill, R_CONTROL + 3, CARD.gamma_multiply(lift));
            }
            let icon_color = if active {
                ACCENT
            } else {
                lerp_color(MUTED, INK, hover)
            };
            let text_color = if active {
                INK
            } else {
                lerp_color(MUTED, INK, hover)
            };
            if compact {
                painter.text(
                    rect.center() - egui::vec2(0.0, 8.0),
                    Align2::CENTER_CENTER,
                    step.icon(),
                    FontId::proportional(15.0),
                    icon_color,
                );
                painter.text(
                    rect.center() + egui::vec2(0.0, 5.0),
                    Align2::CENTER_CENTER,
                    (step.index() + 1).to_string(),
                    medium(9.5),
                    text_color,
                );
            } else {
                painter.text(
                    rect.center() - egui::vec2(0.0, 9.0),
                    Align2::CENTER_CENTER,
                    step.icon(),
                    FontId::proportional(15.0),
                    icon_color,
                );
                painter.text(
                    rect.center() + egui::vec2(0.0, 5.0),
                    Align2::CENTER_CENTER,
                    step.title(),
                    if active {
                        medium(10.5)
                    } else {
                        FontId::proportional(10.5)
                    },
                    text_color,
                );
            }
            if selected > 0.0 {
                // A short rounded indicator under the active page.
                let width = 18.0 * selected;
                let bar = egui::Rect::from_center_size(
                    egui::pos2(rect.center().x, pill.bottom() - 3.0),
                    egui::vec2(width, 3.0),
                );
                painter.rect_filled(bar, 2.0, ACCENT.gamma_multiply(selected));
            }
            if response.clicked() && !self.busy {
                self.switch_step(*step);
            }
        }
    }

    /// Photos with a staged Trash mark, rating or rotation, plus confirmed
    /// rotations waiting in Fix Rotation.
    pub(super) fn staged_change_count(&self) -> usize {
        let decided = self.decisions.values().filter(|d| !d.is_empty()).count();
        if self.step == Step::Rotation {
            decided.max(self.rotation_review.pending_count())
        } else {
            decided
        }
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
        // Choosing an analysis-based grouping, or choosing it again after it
        // stopped, starts it; without this nothing happened until Preview moves.
        self.start_organize_grouping();
    }

    /// A single status line: what is happening on the left, progress and safe
    /// stop controls on the right, keyboard hints when there is room.
    pub(super) fn footer(&mut self, root: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .exact_size(STATUS_HEIGHT)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(S3 as i8, 0)),
            )
            .show(root, |ui| {
                ui.spacing_mut().button_padding = egui::vec2(S2, 2.0);
                let narrow = ui.ctx().content_rect().width() < 900.0;
                let full = ui.max_rect();
                // Reserve controls at the right edge first: Stop must remain
                // reachable even with long paths in small windows.
                let right = ui
                    .scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(full)
                            .layout(egui::Layout::right_to_left(Align::Center)),
                        |ui| {
                            self.footer_controls(ui, narrow);
                            ui.min_rect()
                        },
                    )
                    .inner;
                let right_edge = if right.width() > 0.5 {
                    right.left() - S3
                } else {
                    full.right()
                };
                // The pages sit centred in the window, like an editing suite's
                // page bar; they shift left or drop their labels before they
                // would collide with the controls on the right.
                let mut left_edge = right_edge;
                if self.root.is_some() {
                    let steps = self.navigation_steps();
                    let mid = ui.ctx().content_rect().center().x;
                    let labelled: f32 = step_tab_widths(ui, &steps, false).iter().sum();
                    let compact = mid + labelled / 2.0 > right_edge
                        || mid - labelled / 2.0 < full.left() + 140.0;
                    let width: f32 = if compact {
                        step_tab_widths(ui, &steps, true).iter().sum()
                    } else {
                        labelled
                    };
                    let center = mid
                        .min(right_edge - width / 2.0)
                        .max(full.left() + width / 2.0);
                    let area = egui::Rect::from_center_size(
                        egui::pos2(center, full.center().y),
                        egui::vec2(width, full.height()),
                    );
                    self.step_tabs(ui, area, compact);
                    left_edge = area.left() - S3;
                }
                let left = full.with_max_x(left_edge.max(full.left()));
                if left.width() > 16.0 {
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(left)
                            .layout(egui::Layout::left_to_right(Align::Center)),
                        |ui| self.footer_status(ui),
                    );
                }
            });
    }
    /// The status line: what is happening, then keyboard hints when room is left.
    fn footer_status(&mut self, ui: &mut egui::Ui) {
        let recovery_status = self.model_recovery_status(&self.runtime.model_assets.state());
        let scoped_status = self.workflow_status();
        let status = self
            .navigation_notice
            .clone()
            .or(recovery_status.map(str::to_owned))
            .unwrap_or_else(|| scoped_status.into_owned());
        if self.scanning
            || self.busy
            || self.analysis_run.is_some()
            || !self.requested.is_empty()
            || self.browse_run.is_some()
        {
            ui.add(egui::Spinner::new().size(12.0));
        } else {
            ui.label(
                RichText::new(egui_phosphor::regular::CHECK_CIRCLE)
                    .color(GREEN)
                    .small(),
            );
        }
        let font = egui::TextStyle::Small.resolve(ui.style());
        let status_width = ui
            .painter()
            .layout_no_wrap(status.clone(), font, MUTED)
            .size()
            .x;
        let hint_room = ui.available_width() - status_width - S6;
        ui.add(egui::Label::new(RichText::new(&status).small().color(MUTED)).truncate())
            .on_hover_text(&status);
        // The status message has priority; shortcut hints use only what is
        // left and disappear before the message truncates.
        if self.preferences.show_shortcuts && self.root.is_some() && hint_room > 200.0 {
            let hint = preferences::workflow_hint(self.step);
            ui.add_space(S3);
            ui.add(egui::Label::new(RichText::new(hint).small().color(FAINT)).truncate())
                .on_hover_text(hint);
        }
    }
    /// Stop and cancel controls, progress and the photo count, added right
    /// to left so the changing text never moves the buttons.
    fn footer_controls(&mut self, ui: &mut egui::Ui, narrow: bool) {
        if let Some(stop) = &self.plan_stop {
            let stopping = stop.load(std::sync::atomic::Ordering::Acquire);
            if ui
                .add_enabled(
                    !stopping,
                    egui::Button::new(if stopping {
                        "Cancelling…"
                    } else {
                        "Cancel planning"
                    }),
                )
                .clicked()
            {
                stop.store(true, std::sync::atomic::Ordering::Release);
            }
        }
        if let Some(stop) = &self.apply_stop {
            let stopping = stop.load(std::sync::atomic::Ordering::Acquire);
            if ui
                .add_enabled(
                    !stopping,
                    egui::Button::new(if stopping { "Stopping…" } else { "Stop" })
                        .min_size(egui::vec2(96.0, 24.0)),
                )
                .on_hover_text(
                    "Stop safely. Completed changes stay applied; unfinished work stays available for Retry.",
                )
                .clicked()
            {
                self.stop_file_operations();
            }
        }
        self.footer_metadata(ui, narrow);
        if let Some((done, total)) = self.progress {
            let bar = ui.add(
                egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                    .desired_width(if narrow { 90.0 } else { 160.0 })
                    .desired_height(12.0)
                    .text(RichText::new(format!("{done} / {total}")).small()),
            );
            if let Some(timing) = self.progress_timing() {
                bar.on_hover_text(timing);
            }
        } else if self.apply_stop.is_none() && !narrow {
            // Thumbnails and previews still decoding for what is on screen.
            let loading = self.requested.len();
            // A refreshing view is reported here, in the fixed-height bar: a
            // row above the photos would push the grid and navigator under
            // the pointer whenever a query starts.
            let count = if self.browse_run.is_some() && !self.scanning {
                format!("Updating library view · {} photos", self.filtered.len())
            } else if loading > 0 && !self.scanning {
                format!(
                    "Loading {loading} {} · {} photos",
                    if loading == 1 { "preview" } else { "previews" },
                    self.filtered.len()
                )
            } else {
                format!("{} photos", self.filtered.len())
            };
            ui.label(RichText::new(count).small().color(MUTED))
                .on_hover_text(format!(
                    "{:.0} MB of previews in memory",
                    self.stats.resident_bytes as f64 / 1048576.0
                ));
        }
    }

    /// Organize's own controls, inline in the context bar: how photos are
    /// grouped, where they go, then edit and preview.
    fn organize_controls(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        ui.label(RichText::new("Group by").color(MUTED));
        wrap_before(ui, ui.spacing().combo_width + 2.0 * S3);
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
        if let Some(hint) = self.organize_hint() {
            named(
                ui.label(RichText::new(icon::INFO).color(MUTED)),
                egui::WidgetType::Label,
                hint,
            )
            .on_hover_text(hint);
        }
        if self.automatic_organize_mode()
            && self.model_issue.is_some()
            && ui
                .button("AI models…")
                .on_hover_text("Download or import the models this grouping needs")
                .clicked()
        {
            self.open_model_setup();
        }
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
        // Destination and companion handling live in one popover chip.
        let destination = std::path::Path::new(&self.output)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Choose a folder".into());
        let chip = ui
            .add(egui::Button::new(format!(
                "{}  Move to {destination}  {}",
                icon::FOLDER_SIMPLE,
                icon::CARET_DOWN
            )))
            .on_hover_text(format!("Destination: {}", self.output));
        let label = format!("Move to {destination}");
        chip.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        egui::Popup::menu(&chip)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(420.0)
            .show(|ui| {
                ui.set_width(420.0);
                ui.label(RichText::new("Destination").font(medium(12.5)));
                ui.horizontal(|ui| {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.output)
                            .background_color(BG)
                            .desired_width(ui.available_width() - 36.0),
                    )
                    .on_hover_text("Folders are created inside this folder");
                    if icon_button(ui, icon::FOLDER_OPEN, "Choose destination folder").clicked()
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        self.output = path.display().to_string();
                    }
                });
                ui.add_space(S1);
                if ui
                    .add(Switch::new(
                        &mut self.companions,
                        "Keep RAW + JPEG + XMP together",
                    ))
                    .on_hover_text("Move sidecars and paired RAW files with their photo")
                    .changed()
                {
                    self.preferences.keep_companions = self.companions;
                    self.save_preferences();
                }
            });
        if ui
            .add_enabled(self.can_edit_folders(), egui::Button::new("Edit folders…"))
            .clicked()
        {
            self.open_organize_editor();
        }
        if ui
            .add_enabled(
                !self.scanning && !self.busy && self.organize_metadata_ready(),
                primary("Preview moves"),
            )
            .on_hover_text("See every move before anything changes")
            .clicked()
        {
            self.organize();
        }
    }
    /// One-line explanation of the current Organize mode, shown on hover.
    fn organize_hint(&self) -> Option<&'static str> {
        match self.group_mode {
            GroupMode::Mixed => Some(if self.mixed_labels.is_some() {
                "Dates contain similar-photo groups; single photos stay in the date folder."
            } else {
                "Dates and similarity are analyzed when you choose this grouping. Your files stay unchanged until Apply."
            }),
            GroupMode::Faces => Some(
                "Groups photos by the main people; finding faces starts when you choose Faces. Group portraits get one shared folder, such as People 001 + 003. Photos without a recognized person and videos stay in place. Apply moves the files.",
            ),
            GroupMode::Visual => Some(
                "Select photos with Cmd/Ctrl-click or Shift-click, then use Scenes to correct groups. Analysis starts when you choose this grouping. Ungrouped photos and videos stay in place.",
            ),
            _ if self.metadata_organize_mode() => Some(
                "Folder proposals use photo metadata. Location names are resolved offline; AI analysis is not needed.",
            ),
            _ => None,
        }
    }
    /// The library search field; Ctrl/Cmd+F focuses it.
    pub(super) fn search_field(&mut self, ui: &mut egui::Ui) {
        wrap_before(ui, 150.0 + 2.0 * S3);
        let mut query = self.query.clone();
        if ui
            .add(
                egui::TextEdit::singleline(&mut query)
                    .id(egui::Id::new("library_search"))
                    .hint_text(format!(
                        "{}  Search",
                        egui_phosphor::regular::MAGNIFYING_GLASS
                    ))
                    .background_color(BG)
                    .desired_width(150.0),
            )
            .changed()
            && self.allow_review_departure()
        {
            self.query = query;
            self.filter_dirty = true;
        }
    }
    /// Which photos the step shows: all, unmarked, marked, rated and so on.
    pub(super) fn filter_combo(&mut self, ui: &mut egui::Ui) {
        wrap_before(ui, ui.spacing().combo_width + 2.0 * S3);
        egui::ComboBox::from_id_salt("filter")
            .selected_text(self.filter.title())
            .show_ui(ui, |ui| self.filter_choices(ui));
    }
    fn filter_choices(&mut self, ui: &mut egui::Ui) {
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
    }
    /// Analysis runs on its own when a guided step opens; running it again
    /// is a secondary action, kept in the step's More menu.
    fn analysis_menu_item(&mut self, ui: &mut egui::Ui) {
        let label = if self.step == Step::Rotation {
            "Detect rotations again · Ctrl/Cmd+S"
        } else {
            "Analyze library · Ctrl/Cmd+S"
        };
        if ui
            .add_enabled(self.can_start_analysis(), egui::Button::new(label))
            .on_hover_text("Runs on this computer")
            .clicked()
        {
            self.analyze_current_workflow();
            ui.close();
        }
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
        use egui_phosphor::regular as icon;
        // Right group first (view switch, Details); the step's controls then
        // get exactly the width left over and wrap within it.
        let mut row = ui.available_rect_before_wrap();
        row.max.y = row.min.y + ui.spacing().interact_size.y;
        let right = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(row)
                    .layout(egui::Layout::right_to_left(Align::Center)),
                |ui| {
                    if icon_text_button(ui, true, self.inspector, icon::INFO, "Details")
                        .on_hover_text("Photo details · I")
                        .clicked()
                    {
                        self.inspector ^= true;
                    }
                    // Pick Best is always side by side; Organize previews
                    // the clicked photo beside its folders.
                    if !matches!(self.step, Step::PickBest | Step::Organize) {
                        ui.add_space(S1);
                        self.view_switch(ui);
                    }
                    if self.step == Step::Cull {
                        self.cull_view_options(ui);
                    }
                },
            )
            .response
            .rect;
        let left_room = row.with_max_x((right.left() - S3).max(row.left()));
        let left = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(left_room)
                    .layout(egui::Layout::left_to_right(Align::Center).with_main_wrap(true)),
                |ui| {
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    match self.step {
                        Step::Organize => self.organize_controls(ui),
                        Step::EasyDelete => self.delete_controls(ui),
                        Step::Rotation => self.rotation_controls(ui),
                        Step::PickBest => self.pick_controls(ui),
                        Step::Cull => self.browse_controls(ui),
                    }
                    if self.analysis_run.is_some()
                        && icon_text_button(ui, true, false, icon::STOP, "Cancel analysis")
                            .clicked()
                    {
                        self.cancel_analysis();
                    }
                    if self.step != Step::PickBest && self.step != Step::Cull {
                        self.search_field(ui);
                    }
                    if !self.step_menu_empty() {
                        more_menu(ui, "More actions", |ui| self.step_menu(ui));
                    }
                },
            )
            .response
            .rect;
        ui.advance_cursor_after_rect(left.union(right));
        ui.add_space(2.0);
        self.step_context(ui);
        ui.add_space(S1);
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
    /// The step's secondary actions, behind the toolbar's More button.
    fn step_menu(&mut self, ui: &mut egui::Ui) {
        match self.step {
            Step::EasyDelete => self.delete_menu(ui),
            Step::Rotation => self.rotation_menu(ui),
            Step::PickBest => self.pick_menu(ui),
            _ => {}
        }
        if self.step != Step::Cull && self.step != Step::PickBest {
            ui.menu_button(format!("Show: {}", self.filter.title()), |ui| {
                self.filter_choices(ui)
            });
        }
        if self.analysis_run.is_none() {
            // Cull's menu starts with this item; elsewhere it follows the
            // step's own actions.
            if self.step != Step::Cull {
                ui.separator();
            }
            self.analysis_menu_item(ui);
        }
    }
    /// Cull's More menu holds only Analyze, which is unavailable while an
    /// analysis runs; every other step always has items.
    fn step_menu_empty(&self) -> bool {
        self.step == Step::Cull && self.analysis_run.is_some()
    }
    /// One quiet line under the toolbar with what the current photo needs.
    fn step_context(&mut self, ui: &mut egui::Ui) {
        match self.step {
            Step::EasyDelete => self.delete_context(ui),
            Step::Rotation => self.rotation_context(ui),
            Step::PickBest => self.pick_context(ui),
            Step::Cull => self.cull_filter_notice(ui),
            _ => {}
        }
    }
    /// One segmented view switch: library layouts, then Review. Narrow
    /// windows show icons only; names stay for tooltips and AT.
    fn view_switch(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let narrow = ui.available_width() < 720.0;
        let layouts: &[library::Layout] = if self.step == Step::Cull {
            &library::Layout::ALL
        } else {
            &[library::Layout::Grid]
        };
        let mut segments: Vec<Segment> = layouts
            .iter()
            .map(|layout| {
                let (glyph, title) = match layout {
                    library::Layout::Grid => (icon::SQUARES_FOUR, layout.title()),
                    library::Layout::List => (icon::LIST, layout.title()),
                    library::Layout::Icons => (icon::DOTS_NINE, layout.title()),
                };
                Segment {
                    shown: if narrow {
                        glyph.to_owned()
                    } else {
                        format!("{glyph}  {title}")
                    },
                    name: title.to_owned(),
                    hover: narrow.then(|| title.to_owned()),
                }
            })
            .collect();
        segments.push(Segment {
            shown: if narrow {
                icon::IMAGE.to_owned()
            } else {
                format!("{}  Review", icon::IMAGE)
            },
            name: "Review".into(),
            hover: Some("One photo at a time, or a selection side by side".into()),
        });
        let current = if self.review {
            Some(layouts.len())
        } else if self.step == Step::Cull {
            layouts.iter().position(|l| *l == self.library_layout)
        } else {
            Some(0)
        };
        if let Some(index) = segmented(
            ui,
            egui::Id::new(("view-switch", self.step == Step::Cull)),
            &segments,
            current,
            button_row_height(ui),
        ) {
            if index == layouts.len() {
                self.open_review();
            } else if self.step == Step::Cull {
                self.set_library_layout(layouts[index]);
            } else {
                self.review = false;
            }
        }
    }
    pub(super) fn welcome(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let width = ui.available_width();
        let column = (width - 2.0 * S6).clamp(280.0, 520.0);
        egui::ScrollArea::vertical().id_salt("welcome").show(ui, |ui| {
            ui.add_space((ui.available_height() * 0.12).max(S6));
            ui.vertical_centered(|ui| {
                ui.label(RichText::new(icon::APERTURE).size(52.0).color(ACCENT));
                ui.add_space(S4);
                ui.label(
                    RichText::new("Make room for your best work.")
                        .font(semibold(if width > 750.0 { 34.0 } else { 26.0 })),
                );
                ui.add_space(S2);
                ui.label(
                    RichText::new("Sort, clean up and pick the keepers from a folder of photos. Nothing changes on disk until you review it.")
                        .size(14.5)
                        .color(MUTED),
                );
                ui.add_space(S6);
                let open = ui.add(
                    primary(format!("{}  Open a photo folder", icon::FOLDER_OPEN))
                        .min_size(egui::vec2(220.0, 42.0)),
                );
                let enabled = open.enabled();
                open.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, "Open a photo folder")
                });
                if open.clicked() {
                    self.select_folder();
                }
                ui.add_space(S2);
                ui.label(
                    RichText::new("or drop a folder anywhere in this window")
                        .small()
                        .color(FAINT),
                );
                let recent: Vec<PathBuf> = self
                    .preferences
                    .recent_folders
                    .iter()
                    .filter(|p| !self.missing_recent.contains(*p))
                    .take(5)
                    .cloned()
                    .collect();
                if !recent.is_empty() {
                    ui.add_space(S6);
                    welcome_section(ui, column, "Recent folders", |ui| {
                        for path in recent {
                            let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                            let row = ui
                                .add_enabled(
                                    !self.busy,
                                    egui::Button::new(format!("{}  {name}", icon::FOLDER))
                                        .frame(false)
                                        .min_size(egui::vec2(column - 2.0 * S3, 30.0)),
                                )
                                .on_hover_text(path.display().to_string());
                            let enabled = row.enabled();
                            row.widget_info(|| {
                                egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name)
                            });
                            if row.clicked() {
                                self.open(path);
                            }
                        }
                    });
                }
                ui.add_space(S6);
                let steps = self.navigation_steps();
                welcome_section(ui, column, "Steps, in any order", |ui| {
                    for step in steps {
                        ui.horizontal_top(|ui| {
                            ui.label(
                                RichText::new(format!("{}", step.index() + 1))
                                    .font(semibold(13.0))
                                    .color(ACCENT),
                            );
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 2.0;
                                ui.label(RichText::new(step.title()).font(medium(13.5)));
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(step_summary(step)).small().color(MUTED),
                                    )
                                    .wrap(),
                                );
                            });
                        });
                        ui.add_space(S1);
                    }
                });
                ui.add_space(S6);
            });
        });
    }
}

/// A plain-language line for each step on the welcome screen.
fn step_summary(step: Step) -> &'static str {
    match step {
        Step::Organize => "Sort photos into folders by date, place, camera or people.",
        Step::EasyDelete => "Find duplicates, blurry and badly exposed shots to clear out.",
        Step::Rotation => "Spot sideways photos and turn them upright.",
        Step::PickBest => "Keep the best frame from each run of similar shots.",
        Step::Cull => "Rate, mark and compare every photo yourself.",
    }
}

/// A left-aligned card centred in the welcome column.
fn welcome_section(ui: &mut egui::Ui, width: f32, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0, EDGE))
        .corner_radius(R_SURFACE)
        .inner_margin(S3)
        .show(ui, |ui| {
            ui.set_width(width - 2.0 * S3);
            ui.with_layout(egui::Layout::top_down(Align::Min), |ui| {
                ui.label(RichText::new(title).font(medium(12.5)).color(MUTED));
                ui.add_space(S1);
                body(ui);
            });
        });
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
            theme_install(&ctx);
            let _ = ctx.run_test(
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
                |ui| {
                    let ctx = &ui.ctx().clone();
                    app.keyboard(ctx)
                },
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
