//! Edits to the plan from the board: moving photos, making, renaming,
//! nesting and removing folders, and renaming photos.
use super::*;

impl PhotoSortApp {
    /// Whether the plan can be edited on the board now.
    pub(in crate::ui::workspace) fn board_editable(&self) -> bool {
        self.can_edit_folders() && !self.board.original
    }
    /// The folder draft to edit, created from the current proposal when the
    /// first edit is made.
    pub(in crate::ui::workspace) fn board_draft(&mut self) -> Option<&mut Draft> {
        if !self.board_editable() {
            return None;
        }
        if !self.organize_drafts.contains_key(&self.group_mode) {
            let draft = Draft::from_catalog(self.catalog.as_ref()?, self.organize_labels());
            self.organize_drafts.insert(self.group_mode, draft);
        }
        self.dirty = Some(Instant::now());
        self.board_epoch += 1;
        self.organize_drafts.get_mut(&self.group_mode)
    }
    /// Moves photos into a proposed folder. Items the grouping leaves where
    /// they are move too, which is how a missed face or place is corrected;
    /// moved back to the folder they are in, they are not moving again.
    pub(in crate::ui::workspace) fn board_move(&mut self, ids: &[MediaId], target: &str) {
        if ids.is_empty() || self.board_draft().is_none() {
            return;
        }
        let initial = self.proposed_labels();
        let title = if target.is_empty() {
            self.board_root_name()
        } else {
            target.to_owned()
        };
        let home =
            crate::core::operations::folder_destination(std::path::Path::new(&self.output), target);
        // A photo's RAW, JPEG and edit files move together when that is on.
        let selection = crate::core::organize::Selection {
            files: ids.iter().copied().collect(),
            ..Default::default()
        };
        let Some(draft) = self.organize_drafts.get_mut(&self.group_mode) else {
            return;
        };
        if let Err(error) = draft.move_selection(&selection, target, &self.media, self.companions) {
            self.navigation_notice = Some(error.to_string());
            return;
        }
        let back: HashSet<MediaId> = draft
            .labels()
            .iter()
            .filter(|(id, label)| {
                label.as_str() == target
                    && !initial.contains_key(id)
                    && self
                        .index
                        .get(id)
                        .is_some_and(|&i| self.media[i].path.parent() == Some(home.as_path()))
            })
            .map(|(id, _)| *id)
            .collect();
        draft.keep_in_place(&back, &self.media, false);
        let what = |n: usize| format!("{n} {}", if n == 1 { "item" } else { "items" });
        self.navigation_notice = Some(if back.len() >= ids.len() {
            format!("{} not moving · already in {title}", what(ids.len()))
        } else {
            format!("Moved {} to {title}", what(ids.len()))
        });
        self.board_hold_view();
    }
    /// Leaves items in the folders they are in now: they join "Not moving".
    pub(in crate::ui::workspace) fn board_keep_in_place(&mut self, ids: &[MediaId]) {
        if self.board_draft().is_none() {
            return;
        }
        if let Some(draft) = self.organize_drafts.get_mut(&self.group_mode) {
            draft.keep_in_place(&ids.iter().copied().collect(), &self.media, self.companions);
        }
        self.navigation_notice = Some(format!(
            "{} {} in place",
            ids.len(),
            if ids.len() == 1 {
                "item stays"
            } else {
                "items stay"
            },
        ));
        self.board_hold_view();
    }
    pub(super) fn board_finish_edit(&mut self) {
        let Some((path, name)) = self.board.editing.take() else {
            return;
        };
        let name = name.trim().to_owned();
        if name.is_empty() || name == leaf(&path) {
            return;
        }
        if self.board_draft().is_none() {
            return;
        }
        // Naming a person renames every folder they are in.
        match self.rename_person(&path, &name) {
            Ok(renamed) => self.board_refocus(&path, &renamed),
            Err(error) => self.navigation_notice = Some(error.to_string()),
        }
    }
    /// Folder `from` is now `to`: the folders shown and highlighted inside
    /// it follow.
    fn board_refocus(&mut self, from: &str, to: &str) {
        let moved = |folder: &mut String| {
            if within(folder, from) {
                *folder = format!("{to}{}", &folder[from.len()..]);
            }
        };
        self.board.focus.iter_mut().for_each(moved);
        self.board.folder_anchor.iter_mut().for_each(moved);
        self.board.current.iter_mut().flatten().for_each(moved);
        self.board.focus = outermost(std::mem::take(&mut self.board.focus));
    }
    /// Starts renaming a folder in place in the sidebar. A folded-away
    /// sidebar opens its popover for it, so the name field is in view.
    pub(in crate::ui::workspace) fn board_rename_folder(&mut self, path: &str) {
        if !path.is_empty() {
            self.board.editing = Some((path.to_owned(), leaf(path).to_owned()));
            self.board.edit_focus = true;
            self.board.reveal_editing = true;
            if !self.board.layout.sidebar {
                self.board.folders_open = true;
            }
        }
    }
    /// Move, rename and Trash items for photo tiles.
    pub(in crate::ui::workspace) fn board_move_menu(&mut self, ui: &mut egui::Ui, id: MediaId) {
        let ids = self.board_targets(id);
        let enabled = self.board_editable();
        ui.add_enabled_ui(enabled, |ui| {
            if ids.len() == 1 && ui.button("Rename…  F2").clicked() {
                self.board_rename_photo(id);
                ui.close();
            }
            ui.menu_button(
                format!(
                    "Move {} to folder",
                    if ids.len() == 1 {
                        "item".to_owned()
                    } else {
                        format!("{} items", ids.len())
                    }
                ),
                |ui| self.board_folder_list(ui, &ids),
            );
            if ui.button("New folder from selection…").clicked() {
                self.board_new_folder_with(&ids);
                ui.close();
            }
        });
        let marked = ids.iter().all(|id| self.decision(*id).rejected);
        let what = if ids.len() == 1 {
            "item".to_owned()
        } else {
            format!("{} items", ids.len())
        };
        if ui
            .add_enabled(
                !self.busy,
                egui::Button::new(if marked {
                    format!("Unmark {what} for Trash")
                } else {
                    format!("Mark {what} for Trash")
                }),
            )
            .clicked()
        {
            self.stage_manual(ids, crate::core::decisions::Change::Trash(!marked));
            ui.close();
        }
    }
    /// The proposed folders as menu items that move `ids` there.
    pub(in crate::ui::workspace) fn board_folder_list(
        &mut self,
        ui: &mut egui::Ui,
        ids: &[MediaId],
    ) {
        let folders: Vec<String> = self.board.folders.iter().map(|f| f.path.clone()).collect();
        let root = self.board_root_name();
        // A generated grouping can also leave them where they are.
        let placed = ids.iter().any(|id| {
            self.index
                .get(id)
                .is_some_and(|&i| self.placement(&self.media[i]).label.is_some())
        });
        if self.group_mode != GroupMode::Current && placed {
            if ui
                .button(format!("{}  ·  stays in its folder", unplaced_title(None)))
                .clicked()
            {
                self.board_keep_in_place(ids);
                ui.close();
            }
            ui.separator();
        }
        egui::ScrollArea::vertical()
            .max_height(320.0)
            .show(ui, |ui| {
                for path in folders {
                    let label = if path.is_empty() {
                        root.clone()
                    } else {
                        path.replace('/', " / ")
                    };
                    if ui.button(label).clicked() {
                        self.board_move(ids, &path);
                        ui.close();
                    }
                }
            });
    }
    /// Makes a new folder holding `ids` inside the folder clicked or shown
    /// (see [`Self::board_focus_folder`]), then starts
    /// naming it.
    pub(in crate::ui::workspace) fn board_new_folder_with(&mut self, ids: &[MediaId]) {
        let parent = self.board_focus_folder();
        self.board_new_folder(&parent, ids);
    }
    /// Makes "New folder" (numbered when taken) inside `parent` in the plan,
    /// moves `ids` into it and starts renaming it in place, as Finder does.
    /// Nothing is made on disk until the changes are reviewed and applied.
    pub(in crate::ui::workspace) fn board_new_folder(&mut self, parent: &str, ids: &[MediaId]) {
        // The draft first: it knows every folder on disk, shown or not.
        if self.board_draft().is_none() {
            return;
        }
        let name = self.unused_folder_name(parent);
        let Some(draft) = self.organize_drafts.get_mut(&self.group_mode) else {
            return;
        };
        match draft.create_folder(parent, &name) {
            Ok(path) => {
                if !ids.is_empty() {
                    self.board_move(ids, &path);
                }
                self.board_rename_folder(&path);
            }
            Err(error) => self.navigation_notice = Some(error.to_string()),
        }
    }
    /// "New folder", or "New folder 2" and so on when that name is taken
    /// inside `parent`. A name is taken in any letter case and either form
    /// of its accents, as it is when the folder is made: "New Folder" and
    /// "New folder" are one folder on macOS and Windows.
    fn unused_folder_name(&self, parent: &str) -> String {
        let draft = self.organize_drafts.get(&self.group_mode);
        (1..)
            .map(|n| {
                if n == 1 {
                    "New folder".to_owned()
                } else {
                    format!("New folder {n}")
                }
            })
            .find(|name| {
                let folded = crate::core::text::folded(&join(parent, name));
                let taken = |path: &str| crate::core::text::folded(path) == folded;
                // Hidden empty folders still take their names.
                !self.board.folders.iter().any(|f| taken(&f.path))
                    && !draft.is_some_and(|d| d.folders().iter().any(|f| taken(f)))
            })
            .unwrap_or_default()
    }
    /// A photo's name as its tile, the strip and the preview's title show
    /// it: the new name staged for it in Organize, or the one on disk.
    pub(in crate::ui::workspace) fn shown_name(&self, media: &Media) -> String {
        match self.file_names.get(&media.id) {
            Some(stem) if !stem.trim().is_empty() => {
                crate::core::organize::renamed_file_name(&media.path, std::ffi::OsStr::new(stem))
                    .to_string_lossy()
                    .into_owned()
            }
            _ => media.name(),
        }
    }
    /// Starts renaming a photo in the preview's title. Its RAW, JPEG and
    /// edit files take the same name when they move together.
    pub(in crate::ui::workspace) fn board_rename_photo(&mut self, id: MediaId) {
        let Some(media) = self.index.get(&id).map(|&i| &self.media[i]) else {
            return;
        };
        if !self.board_editable() {
            return;
        }
        let stem = self.file_names.get(&id).cloned().unwrap_or_else(|| {
            media
                .path
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default()
        });
        if self.active != Some(id) {
            self.set_active(id);
        }
        self.board.renaming = Some((id, stem));
        self.board.rename_focus = true;
        // The name is edited in the preview's title, so a folded-away
        // preview opens for it.
        if !self.board.layout.preview {
            self.board.preview_open = true;
            self.inspector = false;
        }
    }
    /// Stages the typed name; the file is renamed when the changes are
    /// reviewed and applied.
    pub(in crate::ui::workspace) fn board_finish_rename(&mut self) {
        let Some((id, name)) = self.board.renaming.take() else {
            return;
        };
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        match crate::core::organize::set_file_name(
            &catalog,
            &mut self.file_names,
            id,
            name.trim(),
            self.companions,
        ) {
            Ok(()) => {
                self.dirty = Some(Instant::now());
                self.board_epoch += 1;
            }
            Err(error) => self.navigation_notice = Some(error.to_string()),
        }
    }
    /// The library folder a sidebar folder started as, when that is exactly
    /// one folder below the library folder (not the library itself, a new
    /// folder or several merged ones), for moving it to Trash whole.
    pub(in crate::ui::workspace) fn board_library_folder(&self, path: &str) -> Option<PathBuf> {
        if self.board.original || path.is_empty() {
            return None;
        }
        let catalog = self.catalog.as_ref()?;
        let original = match self.organize_drafts.get(&self.group_mode) {
            Some(draft) => draft.directory_source(path)?,
            None => path,
        };
        if original.is_empty() {
            return None;
        }
        catalog
            .directories
            .iter()
            .find(|folder| {
                folder.strip_prefix(&catalog.root).is_ok_and(|relative| {
                    crate::core::names::relative_label(relative).as_deref() == Some(original)
                })
            })
            .cloned()
    }
    /// Where a sidebar folder is on disk today, when it already exists there:
    /// its original folder, followed through renames and moves in the plan.
    pub(in crate::ui::workspace) fn board_folder_on_disk(&self, path: &str) -> Option<PathBuf> {
        let root = &self.catalog.as_ref()?.root;
        let candidates = match self.organize_drafts.get(&self.group_mode) {
            Some(draft) => vec![crate::core::operations::folder_destination(
                root,
                draft.directory_source(path)?,
            )],
            None => vec![
                crate::core::operations::folder_destination(root, path),
                crate::core::operations::folder_destination(
                    std::path::Path::new(&self.output),
                    path,
                ),
            ],
        };
        candidates.into_iter().find(|folder| folder.is_dir())
    }
    /// The photos and videos in a sidebar folder and its subfolders; the
    /// library folder holds every item, those not moving included.
    pub(super) fn board_folder_items(&self, path: &str) -> Vec<MediaId> {
        self.board
            .sections
            .iter()
            .filter(|s| in_folder(path, s.path.as_deref()))
            .flat_map(|s| self.filtered[s.range.clone()].iter())
            .map(|&i| self.media[i].id)
            .collect()
    }
    pub(in crate::ui::workspace) fn board_folder_action(&mut self, action: FolderAction) {
        match action {
            FolderAction::ShowOnly(path) => self.board_select_folder(path),
            FolderAction::NewInside(parent) => self.board_new_folder(&parent, &[]),
            FolderAction::Rename(path) => self.board_rename_folder(&path),
            FolderAction::MoveSelected(path) => {
                let ids = self.selection.ordered(&self.media, &self.filtered);
                self.board_move(&ids, &path);
            }
            FolderAction::Nest(source, target) => {
                let Some(draft) = self.board_draft() else {
                    return;
                };
                match draft.nest_folder(&source, &target) {
                    Ok(moved) => {
                        self.board_refocus(&source, &moved);
                        self.navigation_notice = Some(format!(
                            "Moved {} into {}",
                            leaf(&source),
                            if target.is_empty() {
                                self.board_root_name()
                            } else {
                                target.replace('/', " / ")
                            },
                        ));
                    }
                    Err(error) => self.navigation_notice = Some(error.to_string()),
                }
            }
            FolderAction::Wrap(source) => {
                let parent = source.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
                if self.board_draft().is_none() {
                    return;
                }
                let name = self.unused_folder_name(&parent);
                let Some(draft) = self.organize_drafts.get_mut(&self.group_mode) else {
                    return;
                };
                match draft.create_parent(&source, &name) {
                    Ok(_) => {
                        self.board.focus.clear();
                        self.board_rename_folder(&join(&parent, &name));
                    }
                    Err(error) => self.navigation_notice = Some(error.to_string()),
                }
            }
            FolderAction::Reveal(folder) => {
                match crate::core::desktop::Request::for_path(
                    &folder,
                    crate::core::desktop::Action::Reveal,
                ) {
                    Ok(request) => self.dispatch_desktop(request),
                    Err(error) => self.error = Some(error.to_string()),
                }
            }
            FolderAction::Trash(path) => {
                let ids = self.board_folder_items(&path);
                let marked = !ids.iter().all(|id| self.decision(*id).rejected);
                self.stage_manual(ids, crate::core::decisions::Change::Trash(marked));
            }
            FolderAction::TrashFolder(folder) => {
                self.review_trash(crate::core::operations::TrashRequest {
                    files: Vec::new(),
                    folders: vec![folder],
                });
            }
            FolderAction::Remove(path) => {
                let initial = self.proposed_labels();
                let Some(draft) = self.board_draft() else {
                    return;
                };
                match draft.remove_folder(&path, &initial) {
                    Ok(()) => self.board.focus.retain(|f| !within(f, &path)),
                    Err(error) => self.navigation_notice = Some(error.to_string()),
                }
            }
            FolderAction::SamePerson(from, into) => {
                self.navigation_notice = Some(match self.same_person(&from, &into) {
                    Ok(name) => format!("{} is now {name}", leaf(&from)),
                    Err(error) => error.to_string(),
                });
            }
            FolderAction::StartOver => {
                if self.board_editable() {
                    self.organize_drafts.remove(&self.group_mode);
                    self.board.editing = None;
                    self.board_epoch += 1;
                    self.dirty = Some(Instant::now());
                    self.navigation_notice = Some("Folder edits undone".into());
                }
            }
        }
    }
}
