use super::*;

/// Height of the bottom page bar (status, step tabs, progress); photos and
/// controls must end above it.
pub(super) const STATUS_HEIGHT: f32 = 44.0;

impl Step {
    /// The step's icon, as shown in Settings.
    pub(super) fn icon(self) -> &'static str {
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

/// Height of the bottom bar's step control.
const STEP_HEIGHT: f32 = 28.0;
/// Inset between the step control's container and its segments.
const STEP_INSET: f32 = 3.0;
/// A step's label in the bottom bar: "1. Organize", or just "1" when narrow.
fn step_label(step: Step, compact: bool) -> String {
    if compact {
        (step.index() + 1).to_string()
    } else {
        format!("{}. {}", step.index() + 1, step.title())
    }
}
/// Widths of the step control's segments.
fn step_tab_widths(ui: &egui::Ui, steps: &[Step], compact: bool) -> Vec<f32> {
    steps
        .iter()
        .map(|step| {
            let text = ui
                .painter()
                .layout_no_wrap(step_label(*step, compact), semibold(11.5), INK)
                .size()
                .x;
            text + 2.0 * if compact { S2 + 2.0 } else { S3 + 2.0 }
        })
        .collect()
}
/// The whole step control's width, container included.
fn step_bar_width(ui: &egui::Ui, steps: &[Step], compact: bool) -> f32 {
    step_tab_widths(ui, steps, compact).iter().sum::<f32>() + 2.0 * STEP_INSET
}

impl PhotoSortApp {
    /// The workflow steps as a segmented control in the bottom bar: one
    /// rounded container, numbered labels, and an accent pill on the current
    /// step that slides to the next one when the step changes.
    fn step_tabs(&mut self, ui: &mut egui::Ui, area: egui::Rect, compact: bool) {
        let steps = self.navigation_steps();
        let widths = step_tab_widths(ui, &steps, compact);
        let total = step_bar_width(ui, &steps, compact);
        let time = ui.style().animation_time;
        let container = egui::Rect::from_center_size(
            egui::pos2(area.center().x, area.center().y),
            egui::vec2(total, STEP_HEIGHT),
        );
        let inner = container.shrink(STEP_INSET);
        let mut x = inner.left();
        // Interact first, so the pill can be painted under every label.
        let mut tabs = Vec::with_capacity(steps.len());
        for (step, width) in steps.iter().zip(widths) {
            let rect = egui::Rect::from_min_size(
                egui::pos2(x, inner.top()),
                egui::vec2(width, inner.height()),
            );
            x += width;
            let active = *step == self.step;
            let enabled = active || !self.busy;
            let tip = if enabled {
                format!(
                    "{} · {}\nStep {} · Ctrl/Cmd+Alt+{}",
                    step.title(),
                    step.subtitle(),
                    step.index() + 1,
                    step.index() + 1
                )
            } else {
                format!(
                    "{} · available when the current task finishes",
                    step.title()
                )
            };
            let response = ui
                .interact(rect, egui::Id::new(("step-tab", *step)), Sense::click())
                .on_hover_text(tip);
            let name = step_label(*step, false);
            response.widget_info(|| {
                egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, active, &name)
            });
            let ctx = ui.ctx();
            let hover = ctx.animate_bool_with_time(
                response.id.with("hover"),
                enabled && !active && response.hovered(),
                time,
            );
            let press = enabled && !active && response.is_pointer_button_down_on();
            tabs.push((*step, rect, active, enabled, hover, press));
            if response.clicked() && enabled && !active {
                self.switch_step(*step);
            }
        }
        let painter = ui.painter();
        let outer = egui::CornerRadius::same(R_CONTROL + 3);
        let radius = egui::CornerRadius::same(R_CONTROL);
        painter.rect(
            container,
            outer,
            PANEL,
            Stroke::new(1.0, LINE),
            StrokeKind::Inside,
        );
        // Inactive steps have no frame of their own; hovering one fades in a
        // faint backdrop so it still reads as clickable.
        for &(_, rect, _, _, hover, press) in &tabs {
            if hover > 0.0 {
                let fill = if press { SELECTED } else { CARD_HOVER };
                painter.rect_filled(rect, radius, fill.gamma_multiply(hover));
            }
        }
        let mut pill = None;
        if let Some(&(_, rect, ..)) = tabs.iter().find(|t| t.2) {
            let ctx = ui.ctx();
            let id = egui::Id::new("step-indicator");
            let left = ctx.animate_value_with_time(id.with("left"), rect.left(), time);
            let right = ctx.animate_value_with_time(id.with("right"), rect.right(), time);
            let slid = egui::Rect::from_x_y_ranges(left..=right, rect.y_range());
            pill = Some(slid);
            painter.rect(
                slid,
                radius,
                ACCENT,
                Stroke::new(1.0, lerp_color(ACCENT, Color32::WHITE, 0.3)),
                StrokeKind::Inside,
            );
            #[cfg(test)]
            ctx.data_mut(|data| data.insert_temp(id, slid));
        }
        for (step, rect, active, enabled, hover, _) in tabs {
            let rest = if enabled {
                lerp_color(MUTED, INK, hover)
            } else {
                FAINT
            };
            // While the pill slides, a label takes the pill's text colour as
            // far as the pill covers it.
            let covered = pill.map_or(0.0, |pill| {
                (pill.right().min(rect.right()) - pill.left().max(rect.left())).max(0.0)
                    / rect.width()
            });
            let color = lerp_color(rest, ON_ACCENT, covered);
            let font = if active && covered > 0.5 {
                semibold(11.5)
            } else {
                medium(11.5)
            };
            let text = painter.layout_no_wrap(step_label(step, compact), font, color);
            painter.galley(rect.center() - text.size() / 2.0, text, color);
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
        self.comparison_focus = None;
        self.filter = Filter::All;
        self.filter_dirty = true;
        self.review = !matches!(step, Step::Organize | Step::Rotation);
        self.fit_view();
        self.view_started = Instant::now();
        self.start_ready_analysis();
        self.prepare_bookmark();
        // The guided steps select nothing until the user picks a card or a
        // group (a saved view restores its own); another step's photo is
        // not on screen for Copy, rating or rotation to act on.
        if matches!(step, Step::EasyDelete | Step::Rotation | Step::PickBest) {
            self.active = None;
        }
        if step == Step::EasyDelete {
            self.stage_delete_suggestions();
        }
        if step == Step::Rotation {
            self.rotation_panel = false;
            self.stage_rotation_suggestions();
        }
        if step == Step::PickBest {
            self.pick_current = None;
            self.pick_panel = false;
            self.stage_pick_suggestions();
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
            // A folder being named belongs to the old grouping's plan.
            self.board.editing = None;
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
                    let labelled = step_bar_width(ui, &steps, false);
                    let compact = mid + labelled / 2.0 > right_edge
                        || mid - labelled / 2.0 < full.left() + 140.0;
                    let width = if compact {
                        step_bar_width(ui, &steps, true)
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
        // The status message has priority, then the Shortcuts button; hints
        // use only what is left, dropping the least useful whole hints first
        // instead of cutting one off.
        let mut room = ui.available_width() - status_width - S3;
        let sheet = self.root.is_some()
            && room >= hints::shortcuts_button_width(ui, false) + ui.spacing().item_spacing.x;
        let labelled = sheet && room > 260.0;
        if sheet {
            room -= hints::shortcuts_button_width(ui, labelled) + ui.spacing().item_spacing.x;
        }
        let mut shown = Vec::new();
        if sheet && self.preferences.show_shortcuts {
            room -= S3;
            for hint in hints::step_hints(self.step) {
                let width = hints::hint_width(ui, hint) + hints::HINT_GAP;
                if width > room {
                    break;
                }
                room -= width;
                shown.push(hint);
            }
        }
        ui.add(egui::Label::new(RichText::new(&status).small().color(MUTED)).truncate())
            .on_hover_text(&status);
        if !shown.is_empty() {
            ui.add_space(S1);
            divider(ui);
            ui.add_space(S1);
            let spacing = std::mem::replace(&mut ui.spacing_mut().item_spacing.x, hints::HINT_GAP);
            for hint in shown {
                hints::hint_row(ui, hint, MUTED);
            }
            ui.spacing_mut().item_spacing.x = spacing;
        }
        if sheet {
            self.shortcuts_button(ui, labelled);
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

    /// The library search field; Ctrl/Cmd+F focuses it.
    pub(super) fn search_field(&mut self, ui: &mut egui::Ui) {
        let width = if self.bar_fit >= top_bar::Fit::Tight {
            110.0
        } else {
            150.0
        };
        wrap_before(ui, width + 2.0 * S3);
        self.search_box(ui, width, BG);
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
                .desired_width(width)
                // Its full width even at the end of a full bar, so the bar
                // sees it does not fit instead of squeezing it.
                .min_size(egui::vec2(width, 0.0)),
        );
        if response.changed() {
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
            if ui.selectable_label(self.filter == f, f.title()).clicked() {
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
    pub(super) fn welcome(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let width = ui.available_width();
        let column = (width - 2.0 * S6).clamp(280.0, 520.0);
        egui::ScrollArea::vertical().id_salt("welcome").show(ui, |ui| {
            ui.add_space((ui.available_height() * 0.12).max(S6));
            ui.vertical_centered(|ui| {
                crate::ui::brand::app_logo(ui, 64.0);
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
                        let labels = crate::core::preferences::folder_labels(&recent);
                        for (path, name) in recent.into_iter().zip(labels) {
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
    use super::super::compact_cull_tests::{click_label, fixture, rendered};
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
        app.pick_review.replace(vec![Cluster::new(
            1,
            vec![ids[2], ids[3]],
            HashMap::from([(ids[2], 10.), (ids[3], 20.)]),
        )]);
        (temp, app, ids)
    }
    /// One frame at `time` seconds; returns the accessible nodes.
    fn frame_at(
        app: &mut PhotoSortApp,
        ctx: &egui::Context,
        time: f64,
        events: Vec<egui::Event>,
    ) -> Vec<(String, egui::Rect)> {
        let out = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1800.0, 900.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| app.update_workspace(ui, false),
        );
        out.platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, n)| {
                let b = n.bounds()?;
                Some((
                    n.label()?.to_owned(),
                    egui::Rect::from_min_max(
                        egui::pos2(b.x0 as f32, b.y0 as f32),
                        egui::pos2(b.x1 as f32, b.y1 as f32),
                    ),
                ))
            })
            .collect()
    }
    fn indicator(ctx: &egui::Context) -> egui::Rect {
        ctx.data(|d| d.get_temp(egui::Id::new("step-indicator")))
            .expect("the step bar paints its indicator")
    }
    #[test]
    fn step_buttons_switch_steps_but_wait_while_busy() {
        let (temp, mut app, ids) = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1800.0, 900.0);
        // Staged rotations would ask to resolve them before leaving.
        app.decisions.clear();
        app.busy = true;
        click_label(&mut app, &ctx, size, "4. Pick Best");
        assert_eq!(app.step, Step::Cull, "a busy app keeps its step");
        app.busy = false;
        // A photo clicked in Cull's comparison is not what Copy or Trash act
        // on in the next step, or in the next folder.
        app.comparison_focus = Some(ids[0]);
        assert!(app.active.is_some());
        click_label(&mut app, &ctx, size, "4. Pick Best");
        assert_eq!(app.step, Step::PickBest);
        assert_eq!(app.comparison_focus, None);
        // Cull's photo is not shown here, so Copy or Alt+R must not act on it.
        assert_eq!(app.active, None);
        app.comparison_focus = Some(ids[0]);
        app.scan_folder(temp.path().to_path_buf());
        assert_eq!(app.comparison_focus, None);
        app.runtime.shutdown();
    }
    #[test]
    fn shortcut_hints_follow_the_preference_and_never_cut_a_hint() {
        let (_temp, mut app, _) = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let hints: Vec<String> = hints::step_hints(Step::Cull)
            .iter()
            .map(hints::Hint::text)
            .collect();
        let shown = |nodes: &[(String, egui::Rect)]| {
            hints
                .iter()
                .filter(|h| !found(nodes, h).is_empty())
                .cloned()
                .collect::<Vec<_>>()
        };
        let nodes = rendered(&mut app, &ctx, egui::vec2(1800.0, 900.0));
        let wide = shown(&nodes);
        assert!(wide.len() >= 2, "{wide:?}");
        // The most useful hints come first; a narrower bar drops from the end.
        assert_eq!(wide, hints[..wide.len()]);
        let steps = found(&nodes, "1. Organize")[0].left();
        for hint in &wide {
            assert!(
                found(&nodes, hint)[0].right() < steps,
                "{hint} overlaps the steps"
            );
        }
        let narrow = shown(&rendered(&mut app, &ctx, egui::vec2(1200.0, 900.0)));
        assert!(
            narrow.len() < wide.len() && wide.starts_with(&narrow),
            "{narrow:?}"
        );
        app.preferences.show_shortcuts = false;
        let nodes = rendered(&mut app, &ctx, egui::vec2(1800.0, 900.0));
        assert!(shown(&nodes).is_empty());
        assert_eq!(
            found(&nodes, "Keyboard shortcuts").len(),
            1,
            "the sheet stays reachable"
        );
        app.runtime.shutdown();
    }
    #[test]
    fn the_step_indicator_slides_unless_motion_is_reduced() {
        let (_temp, mut app, _) = app();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        for reduce in [true, false] {
            set_reduce_motion(&ctx, reduce);
            app.set_step(Step::Cull);
            let mut time = if reduce { 0.0 } else { 100.0 };
            for _ in 0..3 {
                time += 1.0;
                frame_at(&mut app, &ctx, time, Vec::new());
            }
            app.set_step(Step::Rotation);
            time += 1.0 / 60.0;
            let nodes = frame_at(&mut app, &ctx, time, Vec::new());
            let target = *found(&nodes, "3. Fix Rotation")[0];
            let moving = indicator(&ctx);
            if reduce {
                assert!(
                    (moving.center().x - target.center().x).abs() < 3.0,
                    "{moving:?}"
                );
            } else {
                assert!(moving.center().x - target.center().x > 20.0, "{moving:?}");
                time += 1.0;
                frame_at(&mut app, &ctx, time, Vec::new());
                let rest = indicator(&ctx);
                assert!(
                    (rest.center().x - target.center().x).abs() < 3.0,
                    "{rest:?}"
                );
            }
        }
        app.runtime.shutdown();
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
