//! The board's header and Organize's top-bar controls: title and counts,
//! search, filter, Show original and Reset changes.
use super::*;

impl PhotoSortApp {
    /// Above the photos: where you are, what it holds, and thumbnail size.
    pub(super) fn board_summary(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let root = self.board_root_name();
        let focus = self.board.focus.clone();
        let (folders, items) = if focus.is_empty() {
            (
                self.board
                    .sections
                    .iter()
                    .filter(|s| s.path.is_some())
                    .count(),
                self.filtered.len(),
            )
        } else {
            let shown: Vec<_> = self
                .board
                .sections
                .iter()
                .filter(|s| s.path.is_some() && shows(&focus, s.path.as_deref()))
                .collect();
            (shown.len(), shown.iter().map(|s| s.range.len()).sum())
        };
        let title = match focus.as_slice() {
            [] => root.clone(),
            [path] if path.is_empty() => root.clone(),
            [path] => format!("{root} / {}", path.replace('/', " / ")),
            several => format!("{root} / {} folders", several.len()),
        };
        // Several folders shown are named on hover over the title.
        let names = (focus.len() > 1).then(|| {
            focus
                .iter()
                .map(|path| path.replace('/', " / "))
                .collect::<Vec<_>>()
                .join("\n")
        });
        let summary = self.board_counts(folders, items, focus.is_empty());
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            // The size slider is placed first, from the right; the title and
            // counts then get only the room left of it and truncate there.
            // Folded away in a narrow window: the folders at the left, the
            // preview at the right.
            if !self.board.layout.sidebar {
                self.board_folders_button(ui);
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if !self.board.layout.preview {
                    self.board_preview_toggle(ui);
                    ui.add_space(S2);
                }
                self.size_slider(ui);
                ui.add_space(S3);
                self.board_filter(ui);
                self.board_search(ui);
                ui.add_space(S2);
                // The selection count only when it fits beside the title.
                let small = egui::TextStyle::Small.resolve(ui.style());
                let width = |text: &str, font: FontId| {
                    ui.painter()
                        .layout_no_wrap(text.to_owned(), font, ACCENT)
                        .size()
                        .x
                };
                // The title and counts come first; the count of selected
                // photos only fills what they leave.
                let body = egui::TextStyle::Body.resolve(ui.style());
                let reserved = width(&title, semibold(13.0)) + width(&summary, body) + 40.0;
                if self.library_selection_mode() && !self.selection.is_empty() {
                    let count = self.selection.len();
                    let full = format!("{count} selected · drag onto a folder to move");
                    let short = format!("{count} selected");
                    let room = ui.available_width() - reserved;
                    let text = if width(&full, small.clone()) <= room {
                        Some(full)
                    } else if width(&short, small.clone()) <= room {
                        Some(short)
                    } else {
                        None
                    };
                    if let Some(text) = text {
                        ui.label(RichText::new(text).small().color(ACCENT))
                            .on_hover_text("Drag onto a folder to move");
                        ui.add_space(S3);
                    }
                }
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    if !focus.is_empty()
                        && named(
                            ui.small_button(icon::ARROW_LEFT),
                            egui::WidgetType::Button,
                            "Back to all folders",
                        )
                        .on_hover_text(format!("Every item in {root}"))
                        .clicked()
                    {
                        self.board.focus.clear();
                    }
                    let title = ui.add(
                        egui::Label::new(RichText::new(&title).font(semibold(13.0))).truncate(),
                    );
                    if let Some(names) = &names {
                        title.on_hover_text(format!("Showing these folders together:\n{names}"));
                    }
                    ui.add(egui::Label::new(RichText::new(&summary).color(MUTED)).truncate())
                        .on_hover_text(&summary);
                });
            });
        });
    }
    /// Filters the photos on the board by name or folder. A magnifier button
    /// stands in for the field when the header is narrow; it opens on click
    /// or Ctrl/Cmd+F and stays open while it has text or focus.
    fn board_search(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let id = egui::Id::new("library_search");
        // The title keeps about this much room before the field takes it.
        let room = ui.available_width() - 160.0;
        let focused = ui.memory(|m| m.has_focus(id));
        if room >= 110.0 || focused || self.board.searching || !self.query.is_empty() {
            // The board is on BG; the field stands out like a tile.
            let field = self.search_box(ui, room.clamp(100.0, 180.0), ELEVATED);
            if field.lost_focus() && self.query.is_empty() {
                self.board.searching = false;
            }
        } else if icon_button(ui, icon::MAGNIFYING_GLASS, "Search · Ctrl/Cmd+F").clicked() {
            self.board.searching = true;
            ui.memory_mut(|m| m.request_focus(id));
        }
    }
    /// Which photos the board shows: all, unmarked, marked for Trash, rated
    /// and so on. Highlighted while it hides some.
    fn board_filter(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let on = self.filter != Filter::All;
        let button = ui
            .add(
                egui::Button::new(RichText::new(icon::FUNNEL).color(if on { ACCENT } else { INK }))
                    .frame(on)
                    .selected(on)
                    .min_size(egui::vec2(28.0, 0.0)),
            )
            .on_hover_text(format!("Show: {}", self.filter.title()));
        let button = named(button, egui::WidgetType::Button, "Show");
        egui::Popup::menu(&button)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(180.0);
                self.filter_choices(ui);
            });
    }
    /// Held down, shows the folders as they are on disk now; released, the
    /// plan again. Backslash does the same. Only the icon in a narrow bar.
    pub(in crate::ui::workspace) fn original_button(&mut self, ui: &mut egui::Ui, compact: bool) {
        use egui_phosphor::regular as icon;
        let on = self.board.original;
        let text = if compact {
            icon::CLOCK_COUNTER_CLOCKWISE.to_owned()
        } else {
            format!("{}  Show original", icon::CLOCK_COUNTER_CLOCKWISE)
        };
        let button = egui::Button::new(RichText::new(text).color(if on { ACCENT } else { INK }))
            .selected(on)
            .sense(Sense::drag());
        // With nothing planned, the plan is the folders on disk.
        let response = named(
            ui.add_enabled(self.organize_planned(), button),
            egui::WidgetType::Button,
            "Show original",
        )
        .on_hover_text("Hold to see the folders as they are on disk now · or hold \\")
        .on_disabled_hover_text("Nothing planned yet: the folders already look like this on disk");
        let held = response.is_pointer_button_down_on();
        if held != self.board.original_button {
            self.board.original_button = held;
            ui.ctx().request_repaint();
        }
    }
    /// Whether the plan differs from the folders on disk: photos that move
    /// or are renamed, whether by hand or by the grouping, new folders, and
    /// folders renamed or moved. Show original and Reset changes both ask.
    pub(in crate::ui::workspace) fn organize_planned(&self) -> bool {
        self.board.changes > 0
            || !self.file_names.is_empty()
            || self
                .organize_drafts
                .get(&self.group_mode)
                .is_some_and(|draft| {
                    let edits = draft.directory_edits();
                    !edits.renamed.is_empty() || !edits.moved_empty.is_empty()
                })
    }
    /// Whether there are planned changes to reset.
    pub(in crate::ui::workspace) fn organize_resettable(&self) -> bool {
        self.organize_planned() && self.retry_plan.is_none()
    }
    /// Asks before dropping every planned Organize change. Only the icon in
    /// a narrow bar.
    pub(in crate::ui::workspace) fn reset_button(&mut self, ui: &mut egui::Ui, compact: bool) {
        use egui_phosphor::regular as icon;
        let text = if compact {
            icon::ARROW_U_UP_LEFT.to_owned()
        } else {
            format!("{}  Reset changes", icon::ARROW_U_UP_LEFT)
        };
        let response = named(
            ui.add_enabled(self.organize_resettable(), egui::Button::new(text)),
            egui::WidgetType::Button,
            "Reset changes",
        )
        .on_hover_text("Discard every planned move, rename and new folder")
        .on_disabled_hover_text("No planned changes to reset");
        if response.clicked() {
            self.board.confirm_reset = true;
        }
    }
    /// Drops every planned Organize change and shows the folders as they are
    /// on disk. Trash marks, ratings and rotations stay.
    pub(in crate::ui::workspace) fn reset_organize(&mut self) {
        self.board.confirm_reset = false;
        self.set_group_mode(GroupMode::Current);
        self.discard_organize_changes();
        self.pending_bookmark = None;
        self.board.editing = None;
        self.board.renaming = None;
        self.board.marquee = None;
        self.board.focus.clear();
        self.board_epoch += 1;
        self.navigation_notice = Some("Changes reset · the folders are as they are on disk".into());
    }
    pub(in crate::ui::workspace) fn organize_reset_dialog(&mut self, ctx: &egui::Context) {
        if !self.board.confirm_reset {
            return;
        }
        let changes = self.board.changes;
        let mut reset = None;
        let response = modal(egui::Id::new("reset_organize")).show(ctx, |ui| {
            ui.set_max_width(420.0_f32.min((ctx.content_rect().width() - 60.0).max(180.0)));
            ui.heading(if changes > 0 {
                format!(
                    "Reset {changes} planned {}?",
                    if changes == 1 { "change" } else { "changes" }
                )
            } else {
                "Reset planned changes?".into()
            });
            ui.label(format!(
                "Every move, rename and new folder planned in Organize is dropped, in every grouping, and Group by goes back to {}. Nothing on disk changes.",
                GroupMode::Current.title()
            ));
            ui.label(RichText::new("Trash marks, ratings and rotations stay.").color(MUTED));
            ui.add_space(S2);
            ui.horizontal_wrapped(|ui| {
                if ui.button("Keep changes").clicked() {
                    reset = Some(false);
                }
                if ui.button("Reset changes").clicked() {
                    reset = Some(true);
                }
            });
        });
        if response.should_close() && reset.is_none() {
            reset = Some(false);
        }
        match reset {
            Some(true) => self.reset_organize(),
            Some(false) => self.board.confirm_reset = false,
            None => {}
        }
    }
    /// Why the chosen grouping has no result, and the action that starts it.
    pub(in crate::ui::workspace) fn organize_notice_text(
        &self,
    ) -> Option<(&'static str, String, String, Option<&'static str>)> {
        use egui_phosphor::regular as icon;
        let what = self.group_mode.title().to_lowercase();
        Some(match self.organize_pending()? {
            Pending::Running => return None,
            Pending::Cancelled => (
                icon::PROHIBIT,
                "Grouping cancelled".into(),
                format!(
                    "Grouping by {what} was cancelled. Photos stay where they are; group again to propose folders."
                ),
                Some("Group again"),
            ),
            Pending::Failed => (
                icon::WARNING,
                "Grouping stopped".into(),
                format!("Grouping by {what} could not finish. Group again to retry."),
                Some("Group again"),
            ),
            Pending::Waiting if self.model_issue.is_some() || self.model_dialog => (
                icon::CPU,
                "AI models needed".into(),
                format!("Grouping by {what} needs AI models. Set them up, then group again."),
                Some("AI models…"),
            ),
            Pending::Waiting if self.analysis_run.is_some() || self.rotation_run.is_some() => (
                icon::HOURGLASS_MEDIUM,
                "Grouping is next".into(),
                format!("Grouping by {what} starts when the current analysis finishes."),
                None,
            ),
            Pending::Waiting if self.scanning || self.media.is_empty() => (
                icon::HOURGLASS_MEDIUM,
                "Grouping is next".into(),
                format!("Grouping by {what} starts once the folder has been read."),
                None,
            ),
            Pending::Waiting if self.busy || self.plan_open || self.retry_plan.is_some() => (
                icon::HOURGLASS_MEDIUM,
                "Grouping is next".into(),
                format!("Grouping by {what} waits for the current task to finish."),
                None,
            ),
            Pending::Waiting => (
                icon::CIRCLE_DASHED,
                "Not grouped yet".into(),
                format!(
                    "Group by {what} to propose folders. Nothing moves until you review the changes and apply them."
                ),
                Some("Group photos"),
            ),
        })
    }
    pub(super) fn organize_notice(&mut self, ui: &mut egui::Ui) {
        let Some((glyph, title, body, action)) = self.organize_notice_text() else {
            return;
        };
        let models = action == Some("AI models…");
        let enabled = models || self.can_start_analysis();
        if states::notice_card(ui, glyph, &title, &body, action.map(|a| (a, enabled))) {
            if models {
                self.open_model_setup();
            } else {
                self.start_organize_grouping();
            }
        }
    }
}
