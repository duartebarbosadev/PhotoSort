use super::*;

/// Height of the bottom page bar (status, step tabs, progress); photos and
/// controls must end above it.
pub(super) const STATUS_HEIGHT: f32 = 44.0;
/// Height of the title bar.
pub(super) const HEADER_HEIGHT: f32 = 38.0;
/// Below this toolbar width, Organize's Show original and Reset changes
/// show only their icons.
const ORGANIZE_COMPACT: f32 = 1100.0;

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

    /// Photos with a staged Trash mark, rating or rotation, plus planned
    /// changes in Organize.
    pub(super) fn staged_change_count(&self) -> usize {
        let decided = self.decisions.values().filter(|d| !d.is_empty()).count();
        match self.step {
            // Moves, renames and new folders are reviewed with them.
            Step::Organize => decided + self.board.changes,
            _ => decided,
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
        self.review = !matches!(step, Step::Organize | Step::Rotation);
        self.fit_view();
        self.view_started = Instant::now();
        if step == Step::PickBest {
            self.sync_pick_view();
        }
        self.start_ready_analysis();
        self.prepare_bookmark();
        if step == Step::EasyDelete {
            self.stage_delete_suggestions();
        }
        if step == Step::Rotation {
            // As in Easy Delete, no card is selected until the user picks one.
            self.active = None;
            self.rotation_panel = false;
            self.stage_rotation_suggestions();
        }
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
        // stopped, starts it; without this nothing happened until a review.
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
    /// grouped and where they go. Folders are edited on the board itself and
    /// every change is reviewed with Review changes.
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
        if self.group_mode == GroupMode::Faces {
            self.faceless_controls(ui);
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
        self.destination_chip(ui);
    }
    /// Where the organized folders are made, and whether a photo's RAW, JPEG
    /// and edit files move together, in one popover chip.
    fn destination_chip(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let output = PathBuf::from(&self.output);
        let here = self.root.as_deref().is_some_and(|root| root == output);
        let name = output
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Choose a folder".into());
        let label = format!("Organize into {name}");
        let chip = ui
            .add(egui::Button::new(format!(
                "{}  {label}  {}",
                icon::FOLDER_SIMPLE,
                icon::CARET_DOWN
            )))
            .on_hover_text(format!(
                "New folders are made inside {}",
                super::inspector::readable_path(&output)
            ));
        chip.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        egui::Popup::menu(&chip)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(400.0)
            .show(|ui| {
                ui.set_width(400.0);
                ui.label(RichText::new("Organize into").font(medium(13.0)));
                ui.label(
                    RichText::new(
                        "The folder where the new folders are made. Nothing moves until you review the changes and apply them.",
                    )
                    .small()
                    .color(MUTED),
                );
                ui.add_space(S2);
                if let Some(root) = self.root.clone() {
                    let root_name = root
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string());
                    if ui
                        .radio(here, format!("This folder · {root_name}"))
                        .on_hover_text(super::inspector::readable_path(&root))
                        .clicked()
                    {
                        self.output = root.display().to_string();
                    }
                }
                ui.horizontal(|ui| {
                    let other = ui.radio(!here, "Another folder…");
                    let change = !here
                        && ui
                            .small_button("Change…")
                            .on_hover_text("Choose another folder")
                            .clicked();
                    if (other.clicked() || change)
                        && let Some(path) = rfd::FileDialog::new().pick_folder()
                    {
                        self.output = path.display().to_string();
                    }
                });
                if !here {
                    // The folder's name stands out; where it is, quietly.
                    let parent = output
                        .parent()
                        .map(|p| format!("{}/", super::inspector::readable_path(p)))
                        .unwrap_or_default();
                    ui.horizontal_wrapped(|ui| {
                        ui.add_space(S6);
                        ui.spacing_mut().item_spacing.x = 0.0;
                        ui.label(RichText::new(parent).small().color(MUTED));
                        ui.label(RichText::new(&name).small().color(INK));
                    });
                }
                ui.add_space(S2);
                ui.separator();
                ui.add_space(S1);
                if ui
                    .add(Switch::new(
                        &mut self.companions,
                        "Move RAW, JPEG and edit files of the same photo together",
                    ))
                    .changed()
                {
                    self.preferences.keep_companions = self.companions;
                    self.save_preferences();
                }
                ui.label(
                    RichText::new(
                        "IMG_0042.ARW, IMG_0042.JPG and IMG_0042.xmp always land in the same folder, under the same name.",
                    )
                    .small()
                    .color(MUTED),
                );
            });
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
                "Groups photos by the main people; finding faces starts when you choose Faces. Group portraits get one shared folder, such as People 001 + 003. Photos without a recognized person stay in their folders, or go to a “No people” folder if you choose. Videos stay in their folders. Apply moves the files.",
            ),
            GroupMode::Visual => Some(
                "Select photos with Cmd/Ctrl-click or Shift-click, then use Scenes to correct groups. Analysis starts when you choose this grouping. Photos without similar photos, and videos, stay in their current folders.",
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
        self.search_box(ui, 150.0, BG);
    }
    /// The search text field itself, `width` wide on a `fill` background.
    pub(super) fn search_box(
        &mut self,
        ui: &mut egui::Ui,
        width: f32,
        fill: Color32,
    ) -> egui::Response {
        let mut query = self.query.clone();
        let response = ui.add(
            egui::TextEdit::singleline(&mut query)
                .id(egui::Id::new("library_search"))
                .hint_text(format!(
                    "{}  Search",
                    egui_phosphor::regular::MAGNIFYING_GLASS
                ))
                .background_color(fill)
                .desired_width(width),
        );
        if response.changed() && self.allow_review_departure() {
            self.query = query;
            self.filter_dirty = true;
            // A saved view waiting to be restored must not undo the typing.
            self.pending_bookmark = None;
        }
        response
    }
    /// Which photos the step shows: all, unmarked, marked, rated and so on.
    pub(super) fn filter_combo(&mut self, ui: &mut egui::Ui) {
        wrap_before(ui, ui.spacing().combo_width + 2.0 * S3);
        egui::ComboBox::from_id_salt("filter")
            .selected_text(self.filter.title())
            .show_ui(ui, |ui| self.filter_choices(ui));
    }
    pub(super) fn filter_choices(&mut self, ui: &mut egui::Ui) {
        for f in Filter::ALL {
            if ui.selectable_label(self.filter == f, f.title()).clicked()
                && self.allow_review_departure()
            {
                self.filter = f;
                self.filter_dirty = true;
                self.pending_bookmark = None;
                ui.close();
                if f == Filter::Rated {
                    self.start_metadata_index();
                }
            }
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
        // The bar's text buttons show only their icons in a narrow bar.
        let compact = row.width() < ORGANIZE_COMPACT;
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
                    // No step has a More menu: Analyze and the step's own
                    // actions sit in the bar. Organize previews the clicked
                    // photo beside its folders, Easy Delete and Fix Rotation
                    // the clicked card beside their grid, so none of them
                    // has a view switch.
                    if self.step == Step::Organize {
                        self.organize_bar_actions(ui, compact);
                    } else {
                        ui.add_space(S1);
                        self.analyze_icon_button(ui);
                        if !matches!(self.step, Step::EasyDelete | Step::Rotation) {
                            ui.add_space(S1);
                            self.view_switch(ui);
                        }
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
                    // Organize searches from its board, over the photos.
                    if !matches!(self.step, Step::Organize | Step::PickBest | Step::Cull) {
                        self.search_field(ui);
                    }
                },
            )
            .response
            .rect;
        ui.advance_cursor_after_rect(left.union(right));
        // Organize, Easy Delete and Fix Rotation have no context line, so
        // the bar ends right under its row.
        if !matches!(
            self.step,
            Step::Organize | Step::EasyDelete | Step::Rotation
        ) {
            ui.add_space(2.0);
            self.step_context(ui);
            ui.add_space(S1);
        }
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
    /// Organize's buttons at the right of the bar, before Details: Show
    /// original, Reset changes and Analyze.
    fn organize_bar_actions(&mut self, ui: &mut egui::Ui, compact: bool) {
        ui.add_space(S1);
        self.analyze_icon_button(ui);
        ui.add_space(S2);
        self.reset_button(ui, compact);
        self.original_button(ui, compact);
    }
    /// Analyze as a row-high icon in the bar, so its buttons stay on one
    /// line. While analysis runs, Cancel analysis takes its place on the left.
    fn analyze_icon_button(&mut self, ui: &mut egui::Ui) {
        if self.analysis_run.is_some() {
            return;
        }
        let button = ui.add_enabled(
            self.can_start_analysis(),
            egui::Button::new(RichText::new(egui_phosphor::regular::SPARKLE).size(15.0))
                .frame(false)
                .min_size(egui::vec2(28.0, 0.0)),
        );
        // Fix Rotation's analysis detects the photos' orientation.
        let name = if self.step == Step::Rotation {
            "Detect rotations again · Ctrl/Cmd+S"
        } else {
            "Analyze library · Ctrl/Cmd+S"
        };
        if named(button, egui::WidgetType::Button, name)
            .on_hover_text(name)
            .clicked()
        {
            self.analyze_current_workflow();
        }
    }
    /// One quiet line under the toolbar with what the current photo needs.
    fn step_context(&mut self, ui: &mut egui::Ui) {
        match self.step {
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

#[cfg(test)]
mod step_bar_tests {
    use super::super::compact_cull_tests::{fixture, rendered};
    use super::*;
    use crate::core::models::RotationSuggestion;
    use crate::core::pick_review::Cluster;

    fn found<'a>(nodes: &'a [(String, egui::Rect)], label: &str) -> Vec<&'a egui::Rect> {
        nodes
            .iter()
            .filter(|(s, _)| s == label)
            .map(|(_, rect)| rect)
            .collect()
    }
    /// Real photos with two rotation suggestions, the second uncertain,
    /// and one Pick Best group, so no step starts an analysis on entry.
    fn app() -> (tempfile::TempDir, PhotoSortApp, Vec<MediaId>) {
        let (temp, mut app) = fixture();
        let ids: Vec<MediaId> = app.media.iter().map(|m| m.id).collect();
        for (id, turns, uncertain) in [(ids[0], 1, false), (ids[1], 3, true)] {
            app.rotations.insert(
                id,
                RotationSuggestion {
                    turns,
                    confidence: 0.9,
                    uncertain,
                    views: 3,
                },
            );
        }
        app.finish_rotation_detection();
        app.pick_review.replace(
            vec![Cluster {
                key: 1,
                ordered: vec![ids[2], ids[3]],
                scores: HashMap::from([(ids[2], 10.), (ids[3], 20.)]),
            }],
            &mut app.decisions,
        );
        (temp, app, ids)
    }
    #[test]
    fn rotation_pick_and_cull_bars_hold_their_actions_without_a_more_menu() {
        let (_temp, mut app, _) = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        for step in [Step::Rotation, Step::PickBest, Step::Cull] {
            app.set_step(step);
            assert!(app.analysis_run.is_none(), "{step:?}");
            let analyze = if step == Step::Rotation {
                "Detect rotations again · Ctrl/Cmd+S"
            } else {
                "Analyze library · Ctrl/Cmd+S"
            };
            let bar = [analyze, "Details"];
            for width in [760.0, 1000.0, 1280.0, 1800.0] {
                let nodes = rendered(&mut app, &ctx, egui::vec2(width, 800.0));
                assert!(
                    found(&nodes, "More actions").is_empty(),
                    "{step:?} at {width}"
                );
                let rects: Vec<egui::Rect> = bar
                    .iter()
                    .map(|name| {
                        let rects = found(&nodes, name);
                        assert_eq!(rects.len(), 1, "{name} in {step:?} at {width}");
                        *rects[0]
                    })
                    .collect();
                for (name, rect) in bar.iter().zip(&rects) {
                    assert!(
                        rect.left() >= 0.0 && rect.right() <= width,
                        "{name} {rect:?} outside the window in {step:?} at {width}"
                    );
                }
                for pair in rects.windows(2) {
                    assert!(
                        pair[0].right() <= pair[1].left() + 0.5,
                        "{pair:?} overlap in {step:?} at {width}"
                    );
                    assert!(
                        (pair[0].center().y - pair[1].center().y).abs() < 1.0,
                        "{pair:?} out of line in {step:?} at {width}"
                    );
                }
            }
            // While an analysis runs, Cancel analysis takes Analyze's place.
            app.analysis_run = Some(1);
            let nodes = rendered(&mut app, &ctx, egui::vec2(1280.0, 800.0));
            assert!(found(&nodes, analyze).is_empty(), "{step:?}");
            assert_eq!(found(&nodes, "Cancel analysis").len(), 1, "{step:?}");
            app.analysis_run = None;
        }
        app.runtime.shutdown();
    }
    #[test]
    fn fix_rotation_bar_has_no_confirm_or_view_switch_and_stages_certain_turns() {
        let (_temp, mut app, ids) = app();
        app.set_step(Step::Rotation);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let nodes = rendered(&mut app, &ctx, egui::vec2(1440.0, 800.0));
        for gone in ["Confirm all suggestions", "Grid", "Review"] {
            assert!(found(&nodes, gone).is_empty(), "{gone}");
        }
        assert_eq!(found(&nodes, "Suggestions only").len(), 1);
        assert_eq!(app.decision(ids[0]).turns, 1);
        assert_eq!(app.decision(ids[1]).turns, 0, "the uncertain photo waits");
        app.runtime.shutdown();
    }
}
