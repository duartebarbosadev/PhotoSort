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
    /// Photos that move or get a new name, folders renamed to other
    /// capital letters, folders made that stay empty, and folders the moves
    /// leave with only leftover files, which go to the Trash: what the
    /// Review step lists for Organize, counted its way.
    count: usize,
    /// The files that move or get a new name, each with the number of the
    /// photo it belongs to: a RAW and the JPEG it stays with are one photo.
    photos: HashMap<MediaId, usize>,
    /// The files among them that go to another folder.
    moved: HashSet<MediaId>,
    /// What the moves do to the folders: kept to tell which folders the
    /// Trash marks then finish emptying.
    folders: FolderMoves,
}

/// The folders a plan's moves touch, by path: what the last scan
/// (`Census::left_with_leftovers`) is asked with, to tell which of them end
/// up with only leftover files.
#[derive(Clone, Debug, Default, PartialEq)]
struct FolderMoves {
    /// How many files leave each folder.
    leaving: HashMap<PathBuf, u32>,
    /// Empty folders the plan makes again somewhere else.
    emptied: Vec<PathBuf>,
    /// Folders the plan keeps or makes.
    required: std::collections::BTreeSet<PathBuf>,
    /// Folders a file goes into.
    receiving: std::collections::BTreeSet<PathBuf>,
    respelling: Respelling,
    /// How many folders these moves alone leave for the Trash.
    trashed: usize,
    /// Another folder is the destination: Trash marks are applied there,
    /// and no folder of the library is tidied for them.
    elsewhere: bool,
}
impl FolderMoves {
    /// The plan keeps `path`, makes a folder in it or moves a file into
    /// it. Paths order by component, so what is inside a folder sorts right
    /// after it: one lookup each.
    fn stays(&self, path: &std::path::Path) -> bool {
        use std::ops::Bound::{Included, Unbounded};
        let path = self.respelling.rebase(path);
        let holds = |folders: &std::collections::BTreeSet<PathBuf>| {
            folders
                .range::<std::path::Path, _>((Included(&*path), Unbounded))
                .next()
                .is_some_and(|folder| folder.starts_with(&path))
        };
        holds(&self.required) || holds(&self.receiving)
    }
    /// How many folders go to the Trash with their leftovers once these
    /// moves are done and `also` more files are gone from each folder.
    fn left_for_trash(&self, catalog: &Catalog, also: &HashMap<&std::path::Path, u32>) -> usize {
        let mut leaving: HashMap<&std::path::Path, u32> = self
            .leaving
            .iter()
            .map(|(folder, files)| (folder.as_path(), *files))
            .collect();
        for (folder, files) in also {
            *leaving.entry(folder).or_default() += files;
        }
        catalog
            .census()
            .left_with_leftovers(&catalog.root, &leaving, &self.emptied, |path| {
                self.stays(path)
            })
    }
}

/// The folders a draft gives other capital letters, or the other form of
/// their accents: where each is now and what it is called afterwards. The
/// plan renames such a folder as one, with everything in it
/// (`operations::case_rename::prepare`), so its files do not move and its
/// empty folders are neither made again nor left behind. Only in the
/// library itself: into another folder everything is moved.
#[derive(Clone, Debug, Default, PartialEq)]
struct Respelling {
    root: PathBuf,
    /// By the folder's path with every rename above it already applied.
    to: HashMap<PathBuf, PathBuf>,
}
impl Respelling {
    fn of(
        draft: Option<&crate::core::organize::Draft>,
        catalog: &Catalog,
        output: &std::path::Path,
    ) -> Self {
        let mut found = Self {
            root: catalog.root.clone(),
            to: HashMap::new(),
        };
        let Some(draft) = draft.filter(|_| output == catalog.root) else {
            return found;
        };
        let mut renamed: Vec<(&str, &str)> = draft.respellings().collect();
        // A folder before the folders inside it, as the plan renames them.
        renamed.sort_by_key(|(from, _)| from.split('/').count());
        for (original, target) in renamed {
            let original = crate::core::operations::folder_destination(output, original);
            let from = found.rebase(&original).into_owned();
            let to = crate::core::operations::folder_destination(output, target);
            // Inside a renamed folder and otherwise unchanged: no rename
            // of its own.
            if from != to {
                found.to.insert(from, to);
            }
        }
        found
    }
    /// How many folders are renamed.
    fn len(&self) -> usize {
        self.to.len()
    }
    /// `path` once the folders at or above it have their new spelling.
    fn rebase<'a>(&self, path: &'a std::path::Path) -> std::borrow::Cow<'a, std::path::Path> {
        use std::borrow::Cow;
        if self.to.is_empty() {
            return Cow::Borrowed(path);
        }
        let Ok(inside) = path.strip_prefix(&self.root) else {
            return Cow::Borrowed(path);
        };
        let mut current = self.root.clone();
        for component in inside.components() {
            current.push(component);
            if let Some(to) = self.to.get(&current) {
                current.clone_from(to);
            }
        }
        Cow::Owned(current)
    }
}

/// What [`PhotoSortApp::marks_folder_count`] was last worked out from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct MarksKey {
    /// Which count of Organize's changes it was added to.
    organize: u64,
    /// The staged choices, as one number.
    marks: u64,
    keep_raws: bool,
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
        // What is worked out from this count is out of date with it.
        self.organize_generation
            .set(self.organize_generation.get() + 1);
        count
    }
    /// The folders the Review step lists as going to the Trash because of
    /// the Trash marks: those left with only leftover files once the marked
    /// photos are gone, with the RAW files that go with them, on top of the
    /// folders Organize's moves empty by themselves. Worked out from the
    /// last scan, as the Review step later does from the disk, and again
    /// only when a staged choice or Organize's plan changes.
    pub(super) fn marks_folder_count(&self) -> usize {
        use crate::core::preferences::WantedRaw;
        // Brings Organize's own count up to date: this one builds on it.
        self.organize_change_count();
        let key = MarksKey {
            organize: self.organize_generation.get(),
            marks: self.staged_choices_stamp(),
            keep_raws: self
                .raw_answer
                .or(self.preferences.limit_prompts.wanted_raw)
                != Some(WantedRaw::Trash),
        };
        if let Some((cached, count)) = self.marks_folders.get()
            && cached == key
        {
            return count;
        }
        let count = self.count_marks_folders(key.keep_raws);
        self.marks_folders.set(Some((key, count)));
        count
    }
    /// The staged choices as one number, to tell when any changed: which
    /// photos are marked for the Trash, rated or rotated. A few operations
    /// per choice, since the bar and the step tabs ask every frame.
    fn staged_choices_stamp(&self) -> u64 {
        self.decisions
            .iter()
            .filter(|(_, decision)| !decision.is_empty())
            .fold(0u64, |all, (id, decision)| {
                let kind = u64::from(decision.rejected)
                    | u64::from(decision.rating.is_some()) << 1
                    | u64::from(decision.turns % 4 != 0) << 2;
                // Map order is arbitrary: each choice is mixed alone.
                all.wrapping_add(
                    (id.0 ^ kind.wrapping_mul(0x9E37_79B9_7F4A_7C15))
                        .wrapping_mul(0xD6E8_FEB8_6659_FD93),
                )
            })
    }
    fn count_marks_folders(&self, keep_raws: bool) -> usize {
        use std::path::Path;
        let cached = self.organize_count.borrow();
        let (Some(catalog), Some((_, changes))) = (self.catalog.as_deref(), &*cached) else {
            return 0;
        };
        if self.scanning || changes.folders.elsewhere {
            return 0;
        }
        let rejected = |id: &MediaId| {
            self.decisions
                .get(id)
                .is_some_and(|decision| decision.rejected)
        };
        // A RAW the user rated, rotated or chose to keep stays unless they
        // said otherwise. Looked up only when a RAW could go with a photo.
        let mut chosen: Option<HashSet<MediaId>> = None;
        let mut wanted = |id: &MediaId| {
            self.decisions
                .get(id)
                .is_some_and(|d| d.rating.is_some() || d.turns % 4 != 0)
                || chosen
                    .get_or_insert_with(|| {
                        let mut chosen = self.pick_review.chosen(&self.decisions);
                        chosen.extend(self.delete_review.chosen(&self.decisions));
                        chosen
                    })
                    .contains(id)
        };
        // The files that go to the Trash from the folder they are in. One
        // that Organize moves first goes from where it was moved to, and
        // one in a folder that is renamed is not tidied after.
        let respelling = &changes.folders.respelling;
        let in_place = |id: &MediaId, folder: &Path| {
            !changes.moved.contains(id)
                && (respelling.len() == 0 || *respelling.rebase(folder) == *folder)
        };
        let mut gone: HashMap<&Path, u32> = HashMap::new();
        let mut followers = HashSet::new();
        for (id, decision) in &self.decisions {
            let Some(&at) = self.index.get(id).filter(|_| decision.rejected) else {
                continue;
            };
            let media = &self.media[at];
            let Some(folder) = media.path.parent() else {
                continue;
            };
            if in_place(id, folder) {
                *gone.entry(folder).or_default() += 1;
            }
            // Its RAW goes with a photo once every photo of that name goes.
            if !self.companions || media.kind != MediaKind::Photo {
                continue;
            }
            let group = self.same_name(at);
            let photos_go = group
                .iter()
                .filter(|other| other.kind == MediaKind::Photo)
                .all(|other| rejected(&other.id));
            for raw in group.iter().filter(|other| other.kind == MediaKind::Raw) {
                if photos_go
                    && !rejected(&raw.id)
                    && !(keep_raws && wanted(&raw.id))
                    && in_place(&raw.id, folder)
                    && followers.insert(raw.id)
                {
                    *gone.entry(folder).or_default() += 1;
                }
            }
        }
        if gone.is_empty() {
            return 0;
        }
        changes
            .folders
            .left_for_trash(catalog, &gone)
            .saturating_sub(changes.folders.trashed)
    }
    /// The other photos, RAW files and videos in the folder of the one at
    /// `at` that have its name with another extension. The list is in path
    /// order, so they are found right beside it.
    fn same_name(&self, at: usize) -> Vec<&Media> {
        let media = &self.media[at];
        let (Some(folder), Some(stem)) = (media.path.parent(), media.path.file_stem()) else {
            return Vec::new();
        };
        let stem = stem.as_encoded_bytes();
        // Every name that starts with "stem." sorts together, a folder of
        // such a name with what is in it included.
        let near = |other: &&Media| {
            other
                .path
                .strip_prefix(folder)
                .ok()
                .and_then(|inside| inside.components().next())
                .is_some_and(|name| {
                    let name = name.as_os_str().as_encoded_bytes();
                    name.starts_with(stem) && name.get(stem.len()) == Some(&b'.')
                })
        };
        let twin = |other: &&Media| {
            other.path.parent() == Some(folder)
                && other.path.file_stem().map(|s| s.as_encoded_bytes()) == Some(stem)
        };
        self.media[..at]
            .iter()
            .rev()
            .take_while(near)
            .chain(self.media[at + 1..].iter().take_while(near))
            .filter(twin)
            .collect()
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
            self.each_organize_change(&HashMap::new(), &Respelling::default(), |id, photo, _| {
                photos.insert(id, photo);
            });
            return Changes {
                count: photos.values().collect::<HashSet<_>>().len(),
                photos,
                ..Default::default()
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
        self.organize_changes_with(labels, draft)
    }
    /// Photos that move to the folder `labels` gives them, photos renamed
    /// where they are, the folders of `draft` that are made and stay empty,
    /// and the folders the moves leave with only leftover files.
    pub(super) fn organize_changes_in(
        &self,
        labels: &HashMap<MediaId, String>,
        draft: Option<&crate::core::organize::Draft>,
    ) -> usize {
        self.organize_changes_with(labels, draft).count
    }
    fn organize_changes_with(
        &self,
        labels: &HashMap<MediaId, String>,
        draft: Option<&crate::core::organize::Draft>,
    ) -> Changes {
        use std::path::Path;
        let respelling = self
            .catalog
            .as_deref()
            .map_or_else(Respelling::default, |catalog| {
                Respelling::of(draft, catalog, Path::new(&self.output))
            });
        let mut photos = HashMap::new();
        let mut moved = HashSet::new();
        // The files that leave each folder, and the folders files go into.
        let mut leaving: HashMap<&Path, u32> = HashMap::new();
        let mut receiving = std::collections::BTreeSet::<PathBuf>::new();
        self.each_organize_change(labels, &respelling, |id, photo, to| {
            photos.insert(id, photo);
            if let Some((from, to)) = to {
                moved.insert(id);
                *leaving.entry(from).or_default() += 1;
                if !receiving.contains(to) {
                    receiving.insert(to.to_path_buf());
                }
            }
        });
        let renamed = respelling.len();
        let (made, folders) =
            self.organize_folder_changes(labels, draft, &leaving, receiving, respelling);
        Changes {
            count: photos.values().collect::<HashSet<_>>().len() + renamed + made + folders.trashed,
            photos,
            moved,
            folders,
        }
    }
    /// The folders the Review step lists for these moves: how many are
    /// made and stay empty (a folder made by hand, or an empty one made
    /// again where its folder went), and, in the folders the moves touch,
    /// how many are left with only leftover files and go to the Trash with
    /// them. Worked out from the plan and the last scan, as the Review step
    /// later does from the disk. In another destination folder every empty
    /// folder is taken as made: one already there is only found when the
    /// changes are listed.
    fn organize_folder_changes(
        &self,
        labels: &HashMap<MediaId, String>,
        draft: Option<&crate::core::organize::Draft>,
        leaving: &HashMap<&std::path::Path, u32>,
        receiving: std::collections::BTreeSet<PathBuf>,
        respelling: Respelling,
    ) -> (usize, FolderMoves) {
        use crate::core::operations::folder_destination;
        use std::path::Path;
        let Some(catalog) = self.catalog.as_deref() else {
            return (0, FolderMoves::default());
        };
        let output = Path::new(&self.output);
        let elsewhere = output != catalog.root;
        let edits = draft.map(|draft| draft.directory_edits());
        // Without a board edit the plan keeps every empty folder as it is.
        let required: std::collections::BTreeSet<String> = match &edits {
            Some(edits) => edits.required.clone(),
            None if elsewhere || !leaving.is_empty() => {
                catalog.census().empty_folders().iter().cloned().collect()
            }
            None => Default::default(),
        };
        // An empty folder that only gets other capital letters, or is in a
        // folder that does, is renamed with it: it is neither made again
        // nor left behind.
        let moved_empty: Vec<&(String, String)> = edits
            .iter()
            .flat_map(|edits| &edits.moved_empty)
            .filter(|(source, target)| {
                respelling.len() == 0
                    || *respelling.rebase(&folder_destination(&catalog.root, source))
                        != *folder_destination(output, target)
            })
            .collect();
        let made = match draft {
            _ if elsewhere => empty_new_folders(labels, &required),
            Some(draft) if !moved_empty.is_empty() => {
                let mut made = draft.created_folders().clone();
                made.extend(moved_empty.iter().map(|(_, target)| target.clone()));
                empty_new_folders(labels, &made)
            }
            Some(draft) => empty_new_folders(labels, draft.created_folders()),
            None => 0,
        };
        let mut folders = FolderMoves {
            leaving: leaving
                .iter()
                .map(|(folder, files)| (folder.to_path_buf(), *files))
                .collect(),
            emptied: moved_empty
                .iter()
                .map(|(source, _)| folder_destination(&catalog.root, source))
                .collect(),
            required: required
                .iter()
                .map(|label| folder_destination(output, label))
                .collect(),
            receiving,
            respelling,
            trashed: 0,
            elsewhere,
        };
        if !folders.leaving.is_empty() || !folders.emptied.is_empty() {
            folders.trashed = folders.left_for_trash(catalog, &HashMap::new());
        }
        (made, folders)
    }
    /// Each file that moves to the folder `labels` gives it or is renamed
    /// where it is, with the number of the photo it belongs to, counted from
    /// zero. The Review step lists a photo once with its RAW, so a RAW and
    /// the JPEG it stays with share a number. A file that only follows its
    /// folder (a lone XMP, a notes file) is listed on its own and has its
    /// own number. A file that leaves its folder comes with that folder
    /// and the one it goes to. A file in a folder of `respelling` that goes
    /// where its folder goes does not move: the folder is renamed as one.
    fn each_organize_change<'a>(
        &'a self,
        labels: &HashMap<MediaId, String>,
        respelling: &Respelling,
        mut each: impl FnMut(MediaId, usize, Option<(&'a std::path::Path, &std::path::Path)>),
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
        // Where each folder is once the renamed folders have their new
        // spelling: many files share a folder, so each is worked out once.
        let mut respelt: HashMap<&std::path::Path, PathBuf> = HashMap::new();
        let mut moves = |path: &'a std::path::Path, label: &str| {
            let parent = path.parent();
            let destination = destinations.get(label).map(PathBuf::as_path);
            if parent == destination {
                return false;
            }
            if respelling.len() > 0
                && let Some((parent, destination)) = parent.zip(destination)
                && *respelt
                    .entry(parent)
                    .or_insert_with(|| respelling.rebase(parent).into_owned())
                    == *destination
            {
                return false;
            }
            !parent
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
            let moved = labels
                .get(&id)
                .filter(|label| moves(path, label))
                .and_then(|label| path.parent().zip(destinations.get(label.as_str())))
                .map(|(from, to)| (from, to.as_path()));
            if moved.is_some() || self.file_names.contains_key(&id) {
                let next = photos.len();
                let key = crate::core::organize::name_key(path, self.companions);
                each(id, *photos.entry(key).or_insert(next), moved);
            }
        }
    }
}

/// The folders of `created`, made by hand on the board or made again
/// where their folder went, that the Review step lists as created and
/// empty: no file goes into them or below them, and no other new folder is
/// inside. One that receives photos is named with them instead.
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
    // The folders inside "a" all start with "a/" and so sort together:
    // one lookup tells whether there is any, however many are made.
    let mut inside = String::new();
    created
        .iter()
        .filter(|folder| {
            if used.contains(folder.as_str()) {
                return false;
            }
            inside.clear();
            inside.push_str(folder);
            inside.push('/');
            !created
                .range::<str, _>((
                    std::ops::Bound::Included(inside.as_str()),
                    std::ops::Bound::Unbounded,
                ))
                .next()
                .is_some_and(|other| other.starts_with(inside.as_str()))
        })
        .count()
}
