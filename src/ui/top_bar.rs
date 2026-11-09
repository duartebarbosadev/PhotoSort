use super::*;

/// Height of the one bar above the photos: the folder, the step's controls
/// and actions, then the app's own actions.
pub(super) const HEADER_HEIGHT: f32 = 44.0;
/// macOS: where the window's close button sits from the left edge, and the
/// room its three buttons take before the bar's own contents.
const BUTTONS_INSET: f32 = 14.0;
const BUTTONS_ROOM: f32 = 90.0;
/// Extra room a roomier fit needs before the bar goes back to it, so a
/// count that changes width does not make the bar flicker between fits.
const SLACK: f32 = 12.0;

/// How much the top bar gives up to stay on one line, least first.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Fit {
    /// Every label.
    #[default]
    Full,
    /// Secondary buttons show only their icons.
    Icons,
    /// The folder name and captions go; Open folder moves into Menu.
    Tight,
    /// The step's secondary controls move into a popover.
    Overflow,
}

impl Fit {
    const ALL: [Fit; 4] = [Fit::Full, Fit::Icons, Fit::Tight, Fit::Overflow];
}

/// The width each fit needed when last drawn, per step. Measuring the drawn
/// bar, rather than guessing breakpoints, keeps it right as controls come
/// and go (Faces adds a menu, a selection adds its count).
pub(super) type BarNeeds = [[Option<f32>; 4]; 5];

/// The roomiest fit that is known, or hoped, to fit in `room`.
fn choose(needs: &[Option<f32>; 4], current: Fit, room: f32) -> Fit {
    Fit::ALL
        .into_iter()
        .find(|fit| match needs[*fit as usize] {
            Some(need) => need <= room - if *fit < current { SLACK } else { 0.0 },
            None => true,
        })
        .unwrap_or(Fit::Overflow)
}

/// Records what `fit` needs now. The other fits move by the same amount:
/// what changed (a count, a menu) is in every fit.
fn record(needs: &mut [Option<f32>; 4], fit: Fit, need: f32) {
    if let Some(old) = needs[fit as usize] {
        let shift = need - old;
        for other in needs.iter_mut().flatten() {
            *other += shift;
        }
    }
    needs[fit as usize] = Some(need);
}

/// A quiet vertical rule between groups of the bar.
fn divider(ui: &mut egui::Ui) {
    ui.add_space(S1);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(1.0, 18.0), Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0, LINE));
    ui.add_space(S1);
}

/// A row-high button showing only `glyph`; `name` is its accessible name.
pub(super) fn bar_icon(
    ui: &mut egui::Ui,
    enabled: bool,
    selected: bool,
    glyph: &str,
    name: &str,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(RichText::new(glyph).size(15.0))
            .selected(selected)
            .min_size(egui::vec2(28.0, 0.0)),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, name)
    });
    response
}

/// A "⋯" button whose popover holds the controls the bar has no room for.
pub(super) fn overflow_popover(
    ui: &mut egui::Ui,
    id: egui::Id,
    name: &str,
    body: impl FnOnce(&mut egui::Ui),
) {
    let response =
        bar_icon(ui, true, false, egui_phosphor::regular::DOTS_THREE, name).on_hover_text(name);
    let mut open = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if response.clicked() {
        open = !open;
    }
    // Owns its open state, so combo boxes inside keep their own popups.
    egui::Popup::from_response(&response)
        .id(id.with("popup"))
        .open_bool(&mut open)
        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
        .show(|ui| {
            ui.set_min_width(240.0);
            body(ui)
        });
    ui.ctx().data_mut(|d| d.insert_temp(id, open));
}

/// Makes `rect` work as the window's title bar: dragging it moves the
/// window, and a double-click maximises or restores it (on macOS, what
/// System Settings sets for title bars). Widgets added after it stay usable.
fn title_bar(ui: &mut egui::Ui, rect: egui::Rect, id: &str) {
    let title = ui.interact(rect, egui::Id::new(id), Sense::click_and_drag());
    if title.drag_started() {
        ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag);
    }
    if title.double_clicked() {
        crate::ui::platform::title_bar_double_click(ui.ctx());
    }
}

impl PhotoSortApp {
    /// Whether the bar is the window's title bar: on macOS, with the
    /// window's buttons in it, and on Windows, which has no native frame.
    fn is_title_bar(&self) -> bool {
        self.window_buttons || self.window_frame
    }

    /// The top of a sidebar that runs up beside the bar, as tall as the bar
    /// and part of the window's title bar like it. Left: room for the
    /// window's buttons on macOS, the app icon elsewhere. Right: Open folder
    /// and Menu. A sidebar below the bar starts with its own contents.
    pub(super) fn sidebar_title_band(&mut self, ui: &mut egui::Ui, top_margin: f32) {
        let inner = ui.max_rect();
        let top = inner.top() - top_margin;
        if top > ui.ctx().content_rect().top() + 0.5 {
            return;
        }
        let band = egui::Rect::from_x_y_ranges(inner.x_range(), top..=top + HEADER_HEIGHT);
        if self.is_title_bar() {
            title_bar(ui, band, "sidebar-title-bar");
        }
        let mut row = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(band)
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        row.spacing_mut().button_padding = egui::vec2(S2, 3.0);
        if !self.window_buttons {
            row.add_space(S1);
            self.folder_logo(&mut row);
        }
        row.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            self.app_menu(ui, true, false);
            self.open_folder_button(ui);
        });
        ui.add_space(HEADER_HEIGHT - top_margin);
    }

    /// The app icon. With a folder open, its tooltip names the folder and
    /// its full path.
    fn folder_logo(&self, ui: &mut egui::Ui) {
        let logo = crate::ui::brand::app_logo(ui, 16.0);
        if let Some(root) = &self.root {
            let name = root.file_name().unwrap_or_default().to_string_lossy();
            logo.on_hover_text(format!("{name}\n{}", root.display()));
        }
    }

    /// Open folder as a row-high icon, like Menu beside it.
    fn open_folder_button(&mut self, ui: &mut egui::Ui) {
        let name = "Open folder… (Ctrl/Cmd+O)";
        if bar_icon(
            ui,
            !self.busy,
            false,
            egui_phosphor::regular::FOLDER_OPEN,
            name,
        )
        .on_hover_text(name)
        .clicked()
        {
            self.select_folder();
        }
    }

    /// The one bar above the photos. Left: the app icon and the folder,
    /// Open folder and Menu (unless a sidebar beside the bar holds them),
    /// then the step's own controls. Right: the step's actions, a divider,
    /// then Review changes. When the window narrows, labels become icons,
    /// then captions go and Open folder moves into Menu, then the step's
    /// secondary controls move into a popover; nothing is clipped or
    /// overlaps. On Windows the window's buttons follow at the far right.
    pub(super) fn header(&mut self, root: &mut egui::Ui) {
        egui::Panel::top("header")
            .exact_size(HEADER_HEIGHT)
            .show_separator_line(false)
            .frame(
                egui::Frame::new()
                    .fill(CHROME)
                    .inner_margin(egui::Margin::symmetric(S3 as i8, 0)),
            )
            .show(root, |ui| {
                ui.spacing_mut().button_padding = egui::vec2(S2, 3.0);
                let mut bar = ui.max_rect();
                let whole = bar.expand2(egui::vec2(S3, 0.0));
                ui.painter()
                    .hline(whole.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, LINE));
                // A sidebar that runs up beside the bar holds the app's own
                // buttons, and on macOS the window's.
                let beside = whole.left() > ui.ctx().content_rect().left() + 0.5;
                // macOS: the window's buttons sit at the bar's left.
                self.window_buttons =
                    crate::ui::platform::place_window_buttons(HEADER_HEIGHT, BUTTONS_INSET);
                if self.window_buttons && !beside {
                    bar.min.x += BUTTONS_ROOM - S3;
                }
                // Windows: they sit at its right, drawn over everything.
                if self.window_frame {
                    bar.max.x = whole.right() - window_frame::BUTTONS_WIDTH - S2;
                }
                // The bar's empty space moves and maximises the window.
                if self.is_title_bar() {
                    title_bar(ui, whole, "title-bar");
                }
                if self.root.is_none() {
                    let mut ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(bar)
                            .layout(egui::Layout::left_to_right(Align::Center)),
                    );
                    ui.horizontal_centered(|ui| {
                        crate::ui::brand::app_logo(ui, 16.0);
                        ui.label(RichText::new("PhotoSort").font(semibold(12.5)));
                        ui.add_space(S1);
                        self.open_folder_button(ui);
                        self.app_menu(ui, true, false);
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            self.review_button(ui, Fit::Full);
                        });
                    });
                    return;
                }
                self.bar(ui, bar, beside);
            });
    }

    /// The bar's contents with a folder open, laid out at the roomiest fit
    /// that fits `bar`.
    fn bar(&mut self, ui: &mut egui::Ui, bar: egui::Rect, beside: bool) {
        let step = self.step.index();
        let fit = choose(&self.bar_needs[step], self.bar_fit, bar.width());
        self.bar_fit = fit;
        // Only the step's own controls and actions count as changing the
        // view; opening a folder from Menu brings its own saved view.
        let mut changed = false;
        let right = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(bar)
                    .layout(egui::Layout::right_to_left(Align::Center)),
                |ui| {
                    if self.review_button(ui, fit) {
                        divider(ui);
                    }
                    let before = self.view_state();
                    ui.add_enabled_ui(!self.busy, |ui| self.step_actions(ui, fit));
                    changed |= self.view_state() != before;
                },
            )
            .response
            .rect;
        let identity = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(bar)
                    .layout(egui::Layout::left_to_right(Align::Center)),
                |ui| self.identity(ui, fit, beside),
            )
            .response
            .rect;
        let start = identity.right() + ui.spacing().item_spacing.x;
        let room =
            egui::Rect::from_x_y_ranges(start..=(right.left() - S3).max(start), bar.y_range());
        let left = ui
            .scope_builder(
                egui::UiBuilder::new()
                    .max_rect(room)
                    .layout(egui::Layout::left_to_right(Align::Center)),
                |ui| {
                    // Never over the actions, even for the frame before a
                    // tighter fit takes over.
                    let clip = room.expand2(egui::vec2(1.0, S1));
                    ui.set_clip_rect(clip.intersect(ui.clip_rect()));
                    ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
                    let before = self.view_state();
                    ui.add_enabled_ui(!self.busy, |ui| self.step_controls(ui));
                    changed |= self.view_state() != before;
                },
            )
            .response
            .rect;
        let used = |rect: egui::Rect| {
            if rect.width() > 0.5 {
                rect.width()
            } else {
                0.0
            }
        };
        let need = used(identity) + ui.spacing().item_spacing.x + used(left) + S3 + used(right);
        record(&mut self.bar_needs[step], fit, need);
        if need > bar.width() + 0.5 && fit < Fit::Overflow {
            // Lay the bar out again at a tighter fit before it is shown.
            self.bar_fit = choose(&self.bar_needs[step], fit, bar.width());
            ui.ctx().request_discard("top bar fit");
        }
        if changed {
            self.pending_bookmark = None;
        }
    }

    /// What a saved view restores; a change made in the bar must not be
    /// undone by a view still waiting to be restored.
    #[allow(clippy::type_complexity)]
    fn view_state(
        &self,
    ) -> (
        String,
        Filter,
        crate::core::browsing::Options,
        bool,
        bool,
        GroupMode,
        HashSet<crate::core::delete_review::Category>,
        bool,
        Option<MediaId>,
    ) {
        (
            self.query.clone(),
            self.filter,
            self.browse.clone(),
            self.review,
            self.inspector,
            self.group_mode,
            self.delete_categories.clone(),
            self.rotation_queue_only,
            self.pick_current,
        )
    }

    /// Whether a sidebar beside the photos names the library folder in its
    /// top row: Cull's navigator or Organize's folders.
    fn sidebar_names_folder(&self, ctx: &egui::Context) -> bool {
        let frame = ctx.cumulative_frame_nr();
        self.navigator_sidebar_shown(ctx)
            || (self.step == Step::Organize
                && self.board.layout.sidebar
                && self
                    .board
                    .sidebar_frame
                    .is_some_and(|drawn| drawn.saturating_add(1) >= frame))
    }

    /// The bar's left: the app icon, the folder's name while there is room
    /// and no sidebar names it, then Open folder and Menu. A sidebar that
    /// runs up beside the bar holds Open folder and Menu instead, and the
    /// icon too where no window buttons take its place. The folder's full
    /// path is in a tooltip either way.
    fn identity(&mut self, ui: &mut egui::Ui, fit: Fit, beside: bool) {
        let Some(root) = self.root.clone() else {
            return;
        };
        let path = root.display().to_string();
        let name = root.file_name().unwrap_or_default().to_string_lossy();
        let widest = match fit {
            _ if self.sidebar_names_folder(ui.ctx()) => 0.0,
            Fit::Full => 220.0,
            Fit::Icons => 140.0,
            Fit::Tight | Fit::Overflow => 0.0,
        };
        let logo = !beside || self.window_buttons;
        if logo {
            self.folder_logo(ui);
        }
        if widest > 0.0 {
            let text = RichText::new(name.as_ref()).font(semibold(12.5));
            let natural = ui
                .painter()
                .layout_no_wrap(name.to_string(), semibold(12.5), INK)
                .size()
                .x;
            ui.add_sized(
                egui::vec2(natural.min(widest), HEADER_HEIGHT - 12.0),
                egui::Label::new(text).truncate(),
            )
            .on_hover_text(path);
        }
        if !beside {
            ui.add_space(S1);
            let tight = fit >= Fit::Tight;
            if !tight {
                self.open_folder_button(ui);
            }
            self.app_menu(ui, true, tight);
        }
        if logo || widest > 0.0 || !beside {
            ui.add_space(S1);
            divider(ui);
        }
    }

    /// Review changes, the bar's primary action, while there is a library
    /// or a plan to retry. True when it was shown.
    fn review_button(&mut self, ui: &mut egui::Ui, fit: Fit) -> bool {
        if self.catalog.is_none() && self.retry_plan.is_none() {
            return false;
        }
        // Accent only when there is something to review, so the bar
        // doubles as the staged-change indicator.
        let staged = self.staged_change_count();
        let name = if staged > 0 {
            format!(
                "Review {staged} {}",
                if staged == 1 { "change" } else { "changes" }
            )
        } else {
            "Review changes".into()
        };
        let shown = match (fit >= Fit::Tight, staged) {
            (true, 0) => "Review".into(),
            (true, _) => format!("Review {staged}"),
            (false, _) => name.clone(),
        };
        let button = if staged > 0 || self.retry_plan.is_some() {
            primary(shown)
        } else {
            egui::Button::new(RichText::new(shown).color(MUTED))
        };
        if named(
            ui.add_enabled(!self.busy, button),
            egui::WidgetType::Button,
            &name,
        )
        .on_hover_text("Shift+Enter")
        .clicked()
        {
            self.review_changes();
        }
        true
    }

    /// The step's actions at the right of the bar, before the divider:
    /// Details, Analyze, and the step's own (Organize's Show original and
    /// Reset changes, Cull's view switch and view options).
    fn step_actions(&mut self, ui: &mut egui::Ui, fit: Fit) {
        use egui_phosphor::regular as icon;
        let icons = fit >= Fit::Icons;
        let details = if icons {
            bar_icon(ui, true, self.inspector, icon::INFO, "Details")
        } else {
            icon_text_button(ui, true, self.inspector, icon::INFO, "Details")
        };
        if details.on_hover_text("Photo details · I").clicked() {
            self.inspector ^= true;
        }
        ui.add_space(S1);
        self.analyze_icon_button(ui);
        match self.step {
            Step::Organize => {
                ui.add_space(S2);
                self.reset_button(ui, icons);
                self.original_button(ui, icons);
            }
            // Easy Delete, Fix Rotation and Pick Best show the clicked card
            // beside their grid, so they have no view switch.
            Step::Cull => {
                ui.add_space(S1);
                // The tightest bar folds the view switch into a menu.
                if fit == Fit::Overflow {
                    self.view_menu(ui);
                } else {
                    self.view_switch(ui, icons);
                }
            }
            Step::EasyDelete | Step::Rotation | Step::PickBest => {}
        }
    }

    /// The step's own controls at the left of the bar.
    fn step_controls(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        if self.group_task.is_some() {
            ui.spinner();
            if self.bar_fit < Fit::Tight {
                ui.label(RichText::new("Updating groups…").color(MUTED));
            }
            if ui.button("Cancel group update").clicked() {
                self.cancel_group_edit();
            }
        }
        match self.step {
            Step::Organize => self.organize_controls(ui),
            Step::EasyDelete => self.delete_controls(ui),
            Step::Rotation => self.rotation_controls(ui),
            Step::PickBest => {}
            Step::Cull => self.browse_controls(ui),
        }
        if self.analysis_run.is_some() {
            let cancel = if self.bar_fit >= Fit::Icons {
                bar_icon(ui, true, false, icon::X_CIRCLE, "Cancel analysis")
                    .on_hover_text("Cancel analysis")
            } else {
                icon_text_button(ui, true, false, icon::X_CIRCLE, "Cancel analysis")
            };
            if cancel.clicked() {
                self.cancel_analysis();
            }
        }
        // Organize searches from its board, over the photos; Cull's search
        // leads its own controls.
        if matches!(self.step, Step::EasyDelete | Step::Rotation) {
            self.search_field(ui);
        }
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

    /// Cull's view switch as one menu button showing the current view, for
    /// a bar with no room for the segments.
    fn view_menu(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let views = [
            (false, icon::SQUARES_FOUR, "Grid"),
            (true, icon::IMAGE, "Review"),
        ];
        let (_, glyph, title) = views[usize::from(self.review)];
        let menu = ui.menu_button(
            RichText::new(format!("{glyph} {}", icon::CARET_DOWN)).size(14.0),
            |ui| {
                for (review, glyph, title) in views {
                    if ui
                        .selectable_label(self.review == review, format!("{glyph}  {title}"))
                        .clicked()
                    {
                        if review {
                            self.open_review();
                        } else {
                            self.show_grid();
                        }
                        ui.close();
                    }
                }
            },
        );
        named(
            menu.response,
            egui::WidgetType::Button,
            &format!("View: {title}"),
        );
    }

    /// One segmented view switch: Grid, then Review. `narrow` shows icons
    /// only; names stay for tooltips and AT.
    fn view_switch(&mut self, ui: &mut egui::Ui, narrow: bool) {
        use egui_phosphor::regular as icon;
        let segments: Vec<Segment> = [
            (icon::SQUARES_FOUR, "Grid", None),
            (icon::IMAGE, "Review", None),
        ]
        .into_iter()
        .map(
            |(glyph, title, hover): (&str, &str, Option<&str>)| Segment {
                shown: if narrow {
                    glyph.to_owned()
                } else {
                    format!("{glyph}  {title}")
                },
                name: title.to_owned(),
                hover: hover.or(narrow.then_some(title)).map(str::to_owned),
            },
        )
        .collect();
        if let Some(index) = segmented(
            ui,
            egui::Id::new(("view-switch", self.step == Step::Cull)),
            &segments,
            Some(usize::from(self.review)),
            button_row_height(ui),
        ) {
            if index == 1 {
                self.open_review();
            } else {
                self.show_grid();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bar_takes_the_roomiest_fit_that_fits_and_follows_its_contents() {
        let mut needs = [None; 4];
        // Nothing measured yet: try every label first.
        assert_eq!(choose(&needs, Fit::Full, 800.0), Fit::Full);
        record(&mut needs, Fit::Full, 1000.0);
        assert_eq!(choose(&needs, Fit::Full, 800.0), Fit::Icons);
        record(&mut needs, Fit::Icons, 900.0);
        record(&mut needs, Fit::Tight, 780.0);
        assert_eq!(choose(&needs, Fit::Icons, 800.0), Fit::Tight);
        // Going back to a roomier fit needs some slack.
        assert_eq!(choose(&needs, Fit::Tight, 905.0), Fit::Tight);
        assert_eq!(choose(&needs, Fit::Tight, 915.0), Fit::Icons);
        // A selection count appears: every fit needs that much more.
        record(&mut needs, Fit::Tight, 800.0);
        assert_eq!(needs, [Some(1020.0), Some(920.0), Some(800.0), None]);
    }
}
