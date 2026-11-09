//! The board's header and Organize's top-bar controls: the folder shown,
//! search, filter, and the menu with Show original and Reset changes.
use super::*;

impl PhotoSortApp {
    /// Above the photos: the folder shown, when it is not every folder, and
    /// search, filter and thumbnail size. The folders and their counts are
    /// in the sidebar and the section headers.
    pub(super) fn board_summary(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let focus = self.board.focus.clone();
        let title = match focus.as_slice() {
            [] => None,
            [path] if path.is_empty() => Some(self.board_root_name()),
            [path] => Some(path.replace('/', " / ")),
            several => Some(format!("{} folders", several.len())),
        };
        // Several folders shown are named on hover over the title.
        let names = (focus.len() > 1).then(|| {
            focus
                .iter()
                .map(|path| path.replace('/', " / "))
                .collect::<Vec<_>>()
                .join("\n")
        });
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            // The size slider is placed first, from the right; the title
            // then gets only the room left of it and truncates there.
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
                // The count of selected photos only fills what the title
                // leaves.
                if self.library_selection_mode() && !self.selection.is_empty() {
                    let text = format!("{} selected", self.selection.len());
                    let width = |text: &str, font: FontId| {
                        ui.painter()
                            .layout_no_wrap(text.to_owned(), font, ACCENT)
                            .size()
                            .x
                    };
                    let reserved = title
                        .as_deref()
                        .map_or(0.0, |title| width(title, semibold(13.0)) + 40.0);
                    let small = egui::TextStyle::Small.resolve(ui.style());
                    if width(&text, small) <= ui.available_width() - reserved {
                        ui.label(RichText::new(text).small().color(ACCENT))
                            .on_hover_text("Drag onto a folder to move");
                        ui.add_space(S3);
                    }
                }
                let Some(title) = &title else {
                    return;
                };
                ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                    if named(
                        ui.small_button(icon::ARROW_LEFT),
                        egui::WidgetType::Button,
                        "Back to all folders",
                    )
                    .on_hover_text("All folders")
                    .clicked()
                    {
                        self.board.focus.clear();
                    }
                    let title = ui.add(
                        egui::Label::new(RichText::new(title).font(semibold(13.0))).truncate(),
                    );
                    if let Some(names) = &names {
                        title.on_hover_text(names);
                    }
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
    /// Show original and Reset changes, in a "⋯" menu at the right of the
    /// bar. Lit while the folders on disk are shown.
    pub(in crate::ui::workspace) fn organize_menu(&mut self, ui: &mut egui::Ui) {
        let name = "Planned changes";
        let button = top_bar::bar_icon(
            ui,
            self.organize_planned(),
            self.board.original,
            egui_phosphor::regular::DOTS_THREE,
            name,
        )
        .on_hover_text(name)
        .on_disabled_hover_text("Nothing planned yet");
        egui::Popup::menu(&button).show(|ui| {
            ui.set_min_width(200.0);
            // Stays on until chosen again; holding backslash shows it only
            // while held.
            if ui
                .add(egui::Button::selectable(
                    self.board.original_on,
                    "Show original · hold \\",
                ))
                .clicked()
            {
                self.board.original_on ^= true;
                ui.close();
            }
            if ui
                .add_enabled(
                    self.organize_resettable(),
                    egui::Button::new("Reset changes…"),
                )
                .clicked()
            {
                self.board.confirm_reset = true;
                ui.close();
            }
        });
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
            ui.label("Every planned move, rename and new folder is dropped, in every grouping.");
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
                "Photos stay where they are.".into(),
                Some("Group again"),
            ),
            Pending::Failed => (
                icon::WARNING,
                "Grouping stopped".into(),
                format!("Grouping by {what} could not finish."),
                Some("Group again"),
            ),
            Pending::Waiting if self.model_issue.is_some() || self.model_dialog => (
                icon::CPU,
                "AI models needed".into(),
                format!("Grouping by {what} needs them."),
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
                format!("Group by {what} to propose folders."),
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
