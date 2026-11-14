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
    /// new folder inside the folder clicked or shown, both in the Organize board.
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
            let parent = self.board_focus_folder();
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
        ui.add_enabled_ui(
            !self.modal_active()
                && !self.scanning
                && self.analysis_run.is_none()
                && self.retry_plan.is_none(),
            |ui| {
                egui::ComboBox::from_id_salt("location_layout")
                    .selected_text(selected)
                    .show_ui(ui, |ui| {
                        for (layout, title) in choices {
                            if ui
                                .selectable_label(self.preferences.location_layout == layout, title)
                                .clicked()
                            {
                                self.change_location_layout(layout);
                                ui.close();
                            }
                        }
                    });
            },
        );
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
                    });
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
            ui.label("The people folders are proposed again. Your other edits stay.");
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
            ui.label("The location folders are proposed again. Your other edits stay.");
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
    /// A grouping by metadata. Its folder changes are planned from any
    /// step, so its metadata is read whatever the step.
    pub(super) fn metadata_organize_mode(&self) -> bool {
        matches!(
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
    /// Choosing an analysis-based grouping starts it; when it waits, the
    /// board's notice says why.
    pub(super) fn start_organize_grouping(&mut self) {
        self.organize_halted = None;
        if self.step != Step::Organize || !self.organize_grouping_wanted() {
            return;
        }
        self.start_ready_analysis();
    }
    /// Why a finished people grouping proposes no folders; `None` before it
    /// has run, and when its folders are on the board.
    pub(super) fn face_group_status(&self) -> Option<String> {
        let summary = crate::core::face_groups::summarize(self.face_groups.as_ref()?);
        match summary {
            crate::core::face_groups::Summary { photos: 0, .. } => {
                Some("No faces found in these photos".into())
            }
            crate::core::face_groups::Summary { repeated: 0, .. } => {
                Some("No person appears in more than one photo".into())
            }
            _ => None,
        }
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
    /// Organize's own controls, inline in the context bar: how photos are
    /// grouped and where they go. Folders are edited on the board itself and
    /// every change is reviewed with Review changes.
    pub(super) fn organize_controls(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let fit = self.bar_fit;
        // A tight bar drops the label; the tooltip names the menu instead.
        let labelled = fit < top_bar::Fit::Tight;
        if labelled {
            ui.label(RichText::new("Group by").color(MUTED));
        }
        wrap_before(ui, ui.spacing().combo_width + 2.0 * S3);
        let combo = egui::ComboBox::from_id_salt("grouping")
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
            })
            .response;
        if !labelled {
            combo.on_hover_text("Group by");
        }
        if let Some(hint) = self.organize_hint() {
            named(
                ui.label(RichText::new(icon::INFO).color(MUTED)),
                egui::WidgetType::Label,
                hint,
            )
            .on_hover_text(hint);
        }
        let more = (self.automatic_organize_mode() && self.model_issue.is_some())
            || matches!(self.group_mode, GroupMode::Location | GroupMode::Faces)
            || self.scene_editing();
        if fit == top_bar::Fit::Overflow && more {
            let id = egui::Id::new("organize-more");
            top_bar::overflow_popover(ui, id, "More grouping options", |ui| {
                self.organize_more_controls(ui)
            });
            // A choice there that opens a dialog (AI models, or the question
            // before a grouping is made again) is not left open over it.
            if self.modal_active() {
                ui.ctx().data_mut(|data| data.insert_temp(id, false));
            }
        } else {
            self.organize_more_controls(ui);
        }
        self.destination_chip(ui);
    }
    /// The grouping's own options: AI models when missing, location levels,
    /// photos without people, and scene corrections.
    fn organize_more_controls(&mut self, ui: &mut egui::Ui) {
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
        // The library folder is already named in the top bar.
        let label = if here {
            "Organize into this folder".to_owned()
        } else {
            format!("Organize into {name}")
        };
        // A tight bar keeps only the folder icon; the name is in the tooltip.
        let shown = if self.bar_fit >= top_bar::Fit::Tight {
            format!("{}  {}", icon::FOLDER_SIMPLE, icon::CARET_DOWN)
        } else {
            format!("{}  {label}  {}", icon::FOLDER_SIMPLE, icon::CARET_DOWN)
        };
        let chip = ui
            .add(egui::Button::new(shown))
            .on_hover_text(super::inspector::readable_path(&output));
        chip.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
        egui::Popup::menu(&chip)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(400.0)
            .show(|ui| {
                ui.set_width(400.0);
                if let Some(root) = self.root.clone()
                    && ui
                        .radio(here, "This folder")
                        .on_hover_text(super::inspector::readable_path(&root))
                        .clicked()
                {
                    self.set_output(&root);
                }
                ui.horizontal(|ui| {
                    let other = ui.radio(!here, "Another folder…");
                    let change = !here && ui.small_button("Change…").clicked();
                    if (other.clicked() || change)
                        && let Some(path) = native_dialog::pick(
                            ui.ctx(),
                            rfd::FileDialog::new(),
                            rfd::FileDialog::pick_folder,
                        )
                    {
                        self.set_output(&path);
                        // The dialog held this frame up: draw the new plan.
                        ui.ctx().request_repaint();
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
                        "Keep RAW, JPEG and edit files together",
                    ))
                    .on_hover_text(
                        "IMG_0042.ARW, .JPG and .xmp go to the same folder, under the same name",
                    )
                    .changed()
                {
                    self.preferences.keep_companions = self.companions;
                    self.save_preferences();
                }
            });
    }
    /// Chooses the folder the organized folders are made in. What moves
    /// depends on it, so the board counts its planned changes again.
    pub(super) fn set_output(&mut self, folder: &std::path::Path) {
        let output = folder.display().to_string();
        if self.output != output {
            self.output = output;
            self.board_epoch += 1;
        }
    }
    /// One-line explanation of the current Organize mode, shown on hover.
    fn organize_hint(&self) -> Option<&'static str> {
        match self.group_mode {
            GroupMode::Mixed => Some(
                "Similar photos get a folder inside their date; single photos stay in the date folder.",
            ),
            GroupMode::Faces => Some(
                "Group portraits share one folder, such as People 001 + 003. Videos stay in their folders.",
            ),
            GroupMode::Visual => Some(
                "Photos without similar photos, and videos, stay in their folders. Select photos, then use Scenes to correct groups.",
            ),
            GroupMode::Location => Some("Place names are found offline."),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "organize/tests.rs"]
mod tests;

/// Organize's pending changes, as the bar and the step tabs count them.
#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct Changes {
    /// Photos that move or get a new name, and new folders that stay
    /// empty: what the Review step lists for Organize, counted its way.
    count: usize,
    /// The files that move or get a new name, each with the number of the
    /// photo it belongs to: a RAW and the JPEG it stays with are one photo.
    photos: HashMap<MediaId, usize>,
}

/// What [`PhotoSortApp::organize_change_count`] was last worked out from.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct ChangeCountKey {
    epoch: u64,
    mode: GroupMode,
    drafts: usize,
    names: u64,
    output: String,
    catalog: usize,
    metadata: usize,
    ready: bool,
    companions: bool,
}

impl PhotoSortApp {
    /// Organize's planned moves, renames and new folders, from any step.
    /// Worked out again only when the folders, names or grouping change,
    /// since the bar and the step tabs ask every frame.
    pub(super) fn organize_change_count(&self) -> usize {
        // Only a grouping by date, camera or place with no folder edited by
        // hand reads the photos' metadata. Any other count stays the same
        // while metadata arrives, photo by photo, in a large library.
        let reads_metadata =
            self.metadata_organize_mode() && !self.organize_drafts.contains_key(&self.group_mode);
        let names = self.file_names_stamp();
        let key = ChangeCountKey {
            epoch: self.board_epoch,
            mode: self.group_mode,
            drafts: self.organize_drafts.len(),
            names,
            output: self.output.clone(),
            catalog: self.catalog.as_ref().map_or(0, |c| Arc::as_ptr(c) as usize),
            metadata: if reads_metadata {
                self.metadata.len()
            } else {
                0
            },
            ready: self.organize_grouping_ready(),
            companions: self.companions,
        };
        if let Some((cached, changes)) = &*self.organize_count.borrow()
            && *cached == key
        {
            return changes.count;
        }
        let changes = self.count_organize_changes();
        let count = changes.count;
        *self.organize_count.borrow_mut() = Some((key, changes));
        count
    }
    /// Photos marked for Trash that Organize also moves or renames. The
    /// Review step lists each once, under Trash, so they count once: a RAW
    /// and JPEG that are both marked are still one photo that moves.
    pub(super) fn moved_to_trash_count(&self) -> usize {
        // Brings the photos Organize changes up to date.
        if self.organize_change_count() == 0 {
            return 0;
        }
        let cached = self.organize_count.borrow();
        let Some((_, changes)) = &*cached else {
            return 0;
        };
        let mut counted = HashSet::new();
        self.decisions
            .iter()
            .filter(|(id, d)| d.rejected && self.index.contains_key(id))
            .filter_map(|(id, _)| changes.photos.get(id))
            .filter(|photo| counted.insert(**photo))
            .count()
    }
    /// The staged new names as one number, to tell when any changed.
    pub(super) fn file_names_stamp(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        // Map order is arbitrary: each name is hashed alone and combined.
        self.file_names.iter().fold(0u64, |all, entry| {
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            entry.hash(&mut hasher);
            all ^ hasher.finish()
        })
    }
    fn count_organize_changes(&self) -> Changes {
        if self.catalog.is_none() || self.scanning {
            return Changes::default();
        }
        let draft = self.organize_drafts.get(&self.group_mode);
        // Current folders, none edited, in the library itself: every photo
        // keeps its folder, so only the new names count. No label is built.
        if draft.is_none()
            && self.group_mode == GroupMode::Current
            && self
                .catalog
                .as_ref()
                .is_some_and(|c| c.root.as_path() == std::path::Path::new(&self.output))
        {
            let mut photos = HashMap::new();
            self.each_organize_change(&HashMap::new(), |id, photo| {
                photos.insert(id, photo);
            });
            return Changes {
                count: photos.values().collect::<HashSet<_>>().len(),
                photos,
            };
        }
        let generated;
        let labels = match draft {
            Some(draft) => draft.labels(),
            None if self.organize_grouping_ready() => {
                generated = self.organize_labels();
                &generated
            }
            None => {
                generated = HashMap::new();
                &generated
            }
        };
        let mut photos = HashMap::new();
        self.each_organize_change(labels, |id, photo| {
            photos.insert(id, photo);
        });
        Changes {
            count: photos.values().collect::<HashSet<_>>().len()
                + draft.map_or(0, |d| empty_new_folders(labels, d.created_folders())),
            photos,
        }
    }
    /// Photos that move to the folder `labels` gives them, photos renamed
    /// where they are, and the folders of `created` that stay empty.
    pub(super) fn organize_changes_in(
        &self,
        labels: &HashMap<MediaId, String>,
        created: &std::collections::BTreeSet<String>,
    ) -> usize {
        let mut changed = 0;
        self.each_organize_change(labels, |_, photo| changed = changed.max(photo + 1));
        changed + empty_new_folders(labels, created)
    }
    /// Each file that moves to the folder `labels` gives it or is renamed
    /// where it is, with the number of the photo it belongs to, counted from
    /// zero. The Review step lists a photo once with its RAW, so a RAW and
    /// the JPEG it stays with share a number. A file that only follows its
    /// folder (a lone XMP, a notes file) is listed on its own and has its
    /// own number.
    fn each_organize_change(
        &self,
        labels: &HashMap<MediaId, String>,
        mut each: impl FnMut(MediaId, usize),
    ) {
        if labels.is_empty() && self.file_names.is_empty() {
            return;
        }
        let output = std::path::Path::new(&self.output);
        // Many photos share a folder: each destination is worked out once.
        let mut destinations: HashMap<&str, PathBuf> = HashMap::new();
        for label in labels.values() {
            destinations
                .entry(label)
                .or_insert_with(|| crate::core::operations::folder_destination(output, label));
        }
        // A folder renamed on the board to other capital letters does move
        // its files. Any other label that spells an existing folder that way
        // names the folder itself where the volume ignores letter case.
        let respelled: Vec<&str> = self
            .organize_drafts
            .get(&self.group_mode)
            .map(|draft| draft.respelled_folders().collect())
            .unwrap_or_default();
        let within = |label: &str, folder: &str| {
            label
                .strip_prefix(folder)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        };
        let moves = |path: &std::path::Path, label: &str| {
            let parent = path.parent();
            let destination = destinations.get(label).map(PathBuf::as_path);
            parent != destination
                && !parent
                    .zip(destination)
                    .zip(self.catalog.as_deref())
                    .is_some_and(|((parent, destination), catalog)| {
                        catalog.same_folder(parent, destination)
                            && !respelled.iter().any(|folder| within(label, folder))
                    })
        };
        // The files that follow their folder without being photos. They
        // move only when a folder does, so they are looked up only then.
        let supporting = match self.catalog.as_deref() {
            Some(catalog) if !labels.is_empty() => catalog.supporting_files(),
            _ => Vec::new(),
        };
        let files = self
            .media
            .iter()
            .filter(|m| self.index.contains_key(&m.id))
            .map(|m| (m.id, m.path.as_path()))
            .chain(supporting);
        let mut photos = HashMap::new();
        for (id, path) in files {
            if labels.get(&id).is_some_and(|label| moves(path, label))
                || self.file_names.contains_key(&id)
            {
                let next = photos.len();
                let key = crate::core::organize::name_key(path, self.companions);
                each(id, *photos.entry(key).or_insert(next));
            }
        }
    }
}

/// The folders of `created`, made by hand on the board, that the Review
/// step lists as created and empty: no file goes into them or below them,
/// and no other new folder is inside. One that receives photos is named
/// with them instead.
fn empty_new_folders(
    labels: &HashMap<MediaId, String>,
    created: &std::collections::BTreeSet<String>,
) -> usize {
    if created.is_empty() {
        return 0;
    }
    // Every folder a file goes into, with the folders above it.
    let mut used = HashSet::new();
    for label in labels.values() {
        let mut folder = label.as_str();
        while used.insert(folder) {
            match folder.rsplit_once('/') {
                Some((parent, _)) => folder = parent,
                None => break,
            }
        }
    }
    created
        .iter()
        .filter(|folder| {
            !used.contains(folder.as_str())
                && !created.iter().any(|other| {
                    other
                        .strip_prefix(folder.as_str())
                        .is_some_and(|rest| rest.starts_with('/'))
                })
        })
        .count()
}
