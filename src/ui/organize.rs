use super::*;

/// Why an analysis-based grouping has no result yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pending {
    Running,
    /// Not started: another task, the folder scan or AI models come first.
    Waiting,
    Cancelled,
    Failed,
}
impl PhotoSortApp {
    /// F2 renames the current photo in place and Ctrl/Cmd+Shift+N makes a
    /// new folder inside the folder shown, both in the Organize board.
    pub(super) fn organize_shortcut(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        let command = modifiers.command || modifiers.ctrl;
        let rename = key == egui::Key::F2 && modifiers.is_none();
        let new_folder = key == egui::Key::N && command && modifiers.shift && !modifiers.alt;
        if !(rename || new_folder)
            || self.step != Step::Organize
            || self.review
            || !self.board_editable()
        {
            return false;
        }
        if new_folder {
            let parent = self.board.focus.clone().unwrap_or_default();
            self.board_new_folder(&parent, &[]);
        } else if let Some(active) = self.active {
            self.board_rename_photo(active);
        }
        true
    }

    fn change_location_layout(&mut self, layout: crate::core::geography::Layout) {
        if self.step != Step::Organize
            || self.group_mode != GroupMode::Location
            || self.modal_active()
            || self.scanning
            || self.analysis_run.is_some()
            || self.retry_plan.is_some()
            || self.preferences.location_layout == layout
        {
            return;
        }
        if self.organize_drafts.contains_key(&GroupMode::Location) {
            self.pending_location_layout = Some(layout);
        } else {
            self.accept_location_layout(layout);
        }
    }
    fn accept_location_layout(&mut self, layout: crate::core::geography::Layout) {
        self.preferences.location_layout = layout;
        self.organize_drafts.remove(&GroupMode::Location);
        self.pending_bookmark = None;
        self.set_review_plan(None);
        self.save_preferences();
    }
    pub(super) fn location_layout_controls(&mut self, ui: &mut egui::Ui) {
        use crate::core::geography::Layout;
        let choices = [
            (Layout::Country, "Country"),
            (Layout::CountryCity, "Country / City"),
            (Layout::CountryRegionCity, "Country / Region / City"),
        ];
        let selected = choices
            .iter()
            .find(|(layout, _)| *layout == self.preferences.location_layout)
            .unwrap()
            .1;
        ui.add_enabled_ui(!self.modal_active() && !self.scanning && self.analysis_run.is_none()
            && self.retry_plan.is_none(), |ui| {
            egui::ComboBox::from_id_salt("location_layout").selected_text(selected)
                .show_ui(ui, |ui| {
                    for (layout, title) in choices {
                        if ui.selectable_label(self.preferences.location_layout == layout, title).clicked() {
                            self.change_location_layout(layout);
                            ui.close();
                        }
                    }
                }).response.on_hover_text("Choose location folder levels. Reuses cached metadata; no AI analysis needed to change levels.");
        });
    }
    /// Faces only: whether photos without a recognized person stay in their
    /// folders or go to one "No people" folder.
    pub(super) fn faceless_controls(&mut self, ui: &mut egui::Ui) {
        use crate::core::preferences::{Faceless, NO_PEOPLE};
        let choices = [
            (Faceless::Stay, "Keep in their folders".to_owned()),
            (Faceless::Folder, format!("Put in a “{NO_PEOPLE}” folder")),
        ];
        let selected = &choices
            .iter()
            .find(|(choice, _)| *choice == self.preferences.faceless)
            .unwrap()
            .1;
        ui.label(RichText::new("Photos without people").color(MUTED));
        ui.add_enabled_ui(
            !self.modal_active()
                && !self.scanning
                && self.analysis_run.is_none()
                && self.retry_plan.is_none(),
            |ui| {
                egui::ComboBox::from_id_salt("faceless")
                    .selected_text(selected.as_str())
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for (choice, title) in &choices {
                            if ui
                                .selectable_label(self.preferences.faceless == *choice, title)
                                .clicked()
                            {
                                self.change_faceless(*choice);
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_text(
                        "Photos where no person is recognized. Nothing moves until you review the changes and apply them.",
                    );
            },
        );
    }
    pub(super) fn change_faceless(&mut self, choice: crate::core::preferences::Faceless) {
        if self.step != Step::Organize
            || self.group_mode != GroupMode::Faces
            || self.modal_active()
            || self.scanning
            || self.analysis_run.is_some()
            || self.retry_plan.is_some()
            || self.preferences.faceless == choice
        {
            return;
        }
        if self.organize_drafts.contains_key(&GroupMode::Faces) {
            self.pending_faceless = Some(choice);
        } else {
            self.accept_faceless(choice);
        }
    }
    pub(super) fn accept_faceless(&mut self, choice: crate::core::preferences::Faceless) {
        self.preferences.faceless = choice;
        self.organize_drafts.remove(&GroupMode::Faces);
        self.pending_bookmark = None;
        self.set_review_plan(None);
        self.save_preferences();
    }
    pub(super) fn faceless_dialog(&mut self, ctx: &egui::Context) {
        let Some(choice) = self.pending_faceless else {
            return;
        };
        let mut action = None;
        modal(egui::Id::new("replace_faceless")).show(ctx, |ui| {
            ui.set_max_width(400.0_f32.min((ctx.content_rect().width() - 60.0).max(180.0)));
            ui.heading("Replace your people folder edits?");
            ui.label(match choice {
                crate::core::preferences::Faceless::Folder => format!(
                    "Photos without a recognized person will go to a “{}” folder. The people folders are proposed again, replacing the folder edits you made while grouping by faces. File names and other groupings keep their edits. Nothing changes on disk until you apply.",
                    crate::core::preferences::NO_PEOPLE
                ),
                crate::core::preferences::Faceless::Stay => "Photos without a recognized person will stay in their current folders. The people folders are proposed again, replacing the folder edits you made while grouping by faces. File names and other groupings keep their edits. Nothing changes on disk until you apply.".into(),
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Keep my edits").clicked() {
                    action = Some(false);
                }
                if ui.button("Replace folder edits").clicked() {
                    action = Some(true);
                }
            });
        });
        if let Some(replace) = action {
            self.pending_faceless = None;
            if replace {
                self.accept_faceless(choice);
            }
        }
    }
    pub(super) fn location_layout_dialog(&mut self, ctx: &egui::Context) {
        let Some(_) = self.pending_location_layout else {
            return;
        };
        let mut action = None;
        modal(egui::Id::new("replace_location_layout")).show(ctx, |ui| {
            ui.set_max_width(400.0_f32.min((ctx.content_rect().width() - 60.0).max(180.0)));
            ui.heading("Replace location folder edits?");
            ui.label("The new levels will regenerate location folders and replace their custom folder edits. File names, ratings and other grouping modes keep their edits. Your photos stay unchanged until Apply.");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Keep current layout").clicked() { action = Some(false); }
                if ui.button("Replace location layout").clicked() { action = Some(true); }
            });
        });
        if let Some(replace) = action {
            self.resolve_location_layout(replace);
        }
    }
    fn resolve_location_layout(&mut self, replace: bool) {
        if let Some(layout) = self.pending_location_layout.take()
            && replace
        {
            self.accept_location_layout(layout);
        }
    }
    pub(super) fn metadata_organize_mode(&self) -> bool {
        self.step == Step::Organize
            && matches!(
                self.group_mode,
                GroupMode::Date | GroupMode::Camera | GroupMode::Location
            )
    }
    pub(super) fn organize_metadata_ready(&self) -> bool {
        !self.metadata_organize_mode()
            || self.organize_drafts.contains_key(&self.group_mode)
            || self.metadata.len() == self.media.len()
    }
    pub(super) fn automatic_organize_mode(&self) -> bool {
        matches!(
            self.group_mode,
            GroupMode::Visual | GroupMode::Mixed | GroupMode::Faces
        )
    }
    pub(super) fn organize_labels(&self) -> HashMap<MediaId, String> {
        let mut labels = self
            .media
            .iter()
            .filter_map(|m| Some((m.id, self.organize_label(m)?)))
            .collect();
        if let Some(catalog) = &self.catalog {
            crate::core::organize::include_inventory(catalog, &mut labels);
        }
        labels
    }
    /// The folder the current grouping proposes for one item, before any
    /// folder edits; `None` keeps it in the folder it is in now.
    pub(super) fn organize_label(&self, m: &Media) -> Option<String> {
        let grouped = match self.group_mode {
            // Video GPS is not read, so a video without a resolved place would
            // otherwise be swept into "No GPS"; it stays where it is instead.
            GroupMode::Location => {
                m.kind != MediaKind::Video
                    || self
                        .metadata
                        .get(&m.id)
                        .is_some_and(|meta| meta.location.is_some())
            }
            GroupMode::Visual => self
                .visual_groups
                .as_ref()
                .is_some_and(|groups| groups.contains_key(&m.id)),
            // Photos without a recognized person (group 0) stay in place
            // unless the "No people" folder is chosen.
            GroupMode::Faces => match self.face_group(m.id) {
                Some(0) => {
                    return (m.kind != MediaKind::Video
                        && self.preferences.faceless
                            == crate::core::preferences::Faceless::Folder)
                        .then(|| crate::core::preferences::NO_PEOPLE.into());
                }
                group => group.is_some(),
            },
            GroupMode::Mixed => self
                .mixed_labels
                .as_ref()
                .is_some_and(|labels| labels.contains_key(&m.id)),
            _ => true,
        };
        if !grouped {
            None
        } else if self.group_mode == GroupMode::Current {
            self.current_label(m)
        } else {
            Some(self.label_for(m))
        }
    }
    fn face_group(&self, id: MediaId) -> Option<usize> {
        self.face_groups.as_ref()?.groups.get(&id).copied()
    }
    /// Why an item outside every proposed folder stays where it is, in the
    /// words of the current grouping: "without a recognized person".
    pub(super) fn unplaced_reason(&self, m: &Media) -> &'static str {
        if self.organize_pending().is_some() {
            return "not grouped yet";
        }
        match (self.group_mode, m.kind) {
            (GroupMode::Location, MediaKind::Video) => "video locations are not read",
            (GroupMode::Faces | GroupMode::Visual | GroupMode::Mixed, MediaKind::Video) => {
                "videos are not grouped this way"
            }
            (GroupMode::Faces, _) => "no recognized person",
            (GroupMode::Visual, _) => "no similar photos",
            (GroupMode::Current, _) => "its folder name cannot be read",
            _ => "not grouped",
        }
    }
    /// "58 photos without a recognized person and 2 videos": what stays in
    /// its current folders, said in the current grouping's terms.
    pub(super) fn unplaced_phrase(&self, photos: usize, videos: usize) -> String {
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let photos = match self.group_mode {
            GroupMode::Faces => plural(
                photos,
                "photo without a recognized person",
                "photos without a recognized person",
            ),
            GroupMode::Visual => plural(
                photos,
                "photo without similar photos",
                "photos without similar photos",
            ),
            _ => plural(photos, "photo", "photos"),
        };
        let videos = plural(videos, "video", "videos");
        match (photos.starts_with("0 "), videos.starts_with("0 ")) {
            (false, false) => format!("{photos} and {videos}"),
            (true, false) => videos,
            _ => photos,
        }
    }
    /// An analysis-based grouping is chosen, has no result or folder edits
    /// yet, and did not just fail or get cancelled.
    pub(super) fn organize_grouping_wanted(&self) -> bool {
        self.automatic_organize_mode()
            && !self.organize_grouping_ready()
            && !self.organize_drafts.contains_key(&self.group_mode)
            && self.organize_halted != Some(self.group_mode)
    }
    /// Ends the running Organize grouping without results; it waits for the
    /// user to choose it again instead of restarting into the same failure.
    pub(super) fn halt_organize_run(&mut self) {
        if let Some((_, mode)) = self.organize_run.take() {
            self.organize_halted = Some(mode);
            self.organize_cancelled = false;
        }
    }
    /// The chosen grouping is analyzing right now.
    pub(super) fn organize_running(&self) -> bool {
        self.analysis_run
            .is_some_and(|run| self.organize_run == Some((run, self.group_mode)))
    }
    /// An analysis-based grouping has no result or folder edits yet, so the
    /// board must not present its photos as staying in place.
    pub(super) fn organize_pending(&self) -> Option<Pending> {
        if self.step != Step::Organize
            || !self.automatic_organize_mode()
            || self.organize_grouping_ready()
            || self.organize_drafts.contains_key(&self.group_mode)
        {
            return None;
        }
        Some(if self.organize_running() {
            Pending::Running
        } else if self.organize_halted == Some(self.group_mode) {
            if self.organize_cancelled {
                Pending::Cancelled
            } else {
                Pending::Failed
            }
        } else {
            Pending::Waiting
        })
    }
    /// Choosing an analysis-based grouping starts it, or says why it waits.
    pub(super) fn start_organize_grouping(&mut self) {
        self.organize_halted = None;
        if self.step != Step::Organize || !self.organize_grouping_wanted() {
            return;
        }
        self.start_ready_analysis();
        let mode = self.group_mode;
        if self
            .analysis_run
            .is_some_and(|run| self.organize_run == Some((run, mode)))
            || self.scanning
            || self.media.is_empty()
        {
            return; // Started, or starts once the folder has been read.
        }
        let what = mode.title().to_lowercase();
        let message = if self.model_issue.is_some() || self.model_dialog {
            format!("Grouping by {what} needs AI models · open AI models to set them up")
        } else if self.analysis_run.is_some() || self.rotation_run.is_some() {
            format!("Grouping by {what} starts when the current analysis finishes")
        } else if self.busy || self.plan_open || self.retry_plan.is_some() {
            format!("Grouping by {what} waits for the current task · then press Review changes")
        } else {
            return; // `analyze` already reported why it could not start.
        };
        self.set_organize_status(mode, message);
    }
    /// What a finished people grouping found; `None` before it has run.
    pub(super) fn face_group_status(&self) -> Option<String> {
        let summary = crate::core::face_groups::summarize(self.face_groups.as_ref()?);
        Some(match summary {
            crate::core::face_groups::Summary { photos: 0, .. } => {
                "No faces found in these photos".into()
            }
            crate::core::face_groups::Summary { repeated: 0, .. } => {
                "Faces found, but no person appears in more than one photo".into()
            }
            crate::core::face_groups::Summary { repeated, .. } => format!(
                "People groups ready · {repeated} {} in more than one photo",
                if repeated == 1 { "person" } else { "people" }
            ),
        })
    }
    pub(super) fn organize_grouping_ready(&self) -> bool {
        match self.group_mode {
            GroupMode::Visual => self.visual_groups.is_some(),
            GroupMode::Mixed => self.mixed_labels.is_some(),
            GroupMode::Faces => self.face_groups.is_some(),
            GroupMode::Date | GroupMode::Camera | GroupMode::Location => {
                self.metadata.len() == self.media.len()
            }
            _ => true,
        }
    }
    pub(super) fn can_edit_folders(&self) -> bool {
        !self.scanning
            && self.retry_plan.is_none()
            && !self.busy
            && self.analysis_run.is_none()
            && self.catalog.is_some()
            && (self.organize_grouping_ready()
                || self.organize_drafts.contains_key(&self.group_mode))
    }
}

#[cfg(test)]
#[path = "organize/tests.rs"]
mod tests;
