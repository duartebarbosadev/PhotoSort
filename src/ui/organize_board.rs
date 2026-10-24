//! Organize's main view: the proposed folders on the left, the photos and
//! videos grouped under their destination folder in the middle, the current
//! photo large on the right. Photos are dragged onto folders, folders onto
//! each other; folders are made, renamed and removed in place. Every edit goes
//! into the grouping's folder draft, and nothing changes on disk until the
//! changes are reviewed and applied.
use super::organize::Pending;
use super::*;
use crate::core::organize::Draft;

/// What the board was arranged from; a change rearranges it.
#[derive(Clone, PartialEq)]
pub(super) struct BoardKey {
    mode: GroupMode,
    epoch: u64,
    media: usize,
    metadata: usize,
    draft: Option<(usize, usize)>,
    grouped: [bool; 3],
    layout: crate::core::geography::Layout,
    faceless: crate::core::preferences::Faceless,
    original: bool,
}
/// One destination folder's run of photos in `filtered` order.
pub(super) struct Section {
    /// Destination relative to the output folder; `None` stays in place.
    pub(super) path: Option<String>,
    pub(super) range: std::ops::Range<usize>,
    pub(super) videos: usize,
}
/// A row of the folder tree.
pub(super) struct FolderRow {
    pub(super) path: String,
    pub(super) depth: usize,
    /// Photos and videos directly in this folder, and including subfolders.
    pub(super) direct: usize,
    pub(super) total: usize,
}
#[derive(Default)]
pub(super) struct Board {
    pub(super) key: Option<BoardKey>,
    pub(super) sections: Vec<Section>,
    pub(super) folders: Vec<FolderRow>,
    /// Items that stay where they are (not part of any proposed folder),
    /// and how many of them are videos.
    pub(super) unplaced: usize,
    pub(super) unplaced_videos: usize,
    /// The folder shown in the middle; `None` shows every folder.
    pub(super) focus: Option<String>,
    /// A folder being renamed in place: (folder, typed name).
    pub(super) editing: Option<(String, String)>,
    pub(super) edit_focus: bool,
    /// The sidebar scrolls the folder being renamed into view once.
    pub(super) reveal_editing: bool,
    /// Thumbnail width chosen with the size slider.
    pub(super) tile: Option<f32>,
    /// The folder you are looking at: the active photo's while it is on
    /// screen, otherwise the topmost one shown. `None` before the board is
    /// drawn, or for items staying where they are. The sidebar highlights it.
    pub(super) current: Option<Option<String>>,
    /// A folder just chosen in the sidebar: highlighted until the board
    /// scrolls or the active photo changes from where they were (recorded
    /// on the first frame after the choice).
    pub(super) hold: Option<Option<(f32, Option<MediaId>)>>,
    /// The sidebar scrolls `current` into view once after it changes.
    pub(super) reveal_current: bool,
    /// The sidebar's scroll offset and height in the last frame.
    pub(super) tree_view: (f32, f32),
    /// The folders as they are on disk now, shown while "Show original" or
    /// the backslash key is held; the planned folders come back on release.
    pub(super) original: bool,
    /// "Show original" was held down in the last frame.
    pub(super) original_button: bool,
    /// The folder shown before the original layout was, restored after it.
    pub(super) focus_before: Option<Option<String>>,
    /// A selection rectangle being dragged from empty space in the board:
    /// where it started (in board coordinates) and the photos selected
    /// before it, kept when Shift or Ctrl/Cmd was held.
    pub(super) marquee: Option<(egui::Pos2, Vec<MediaId>)>,
    /// A photo being renamed in the preview's title: (photo, typed name).
    pub(super) renaming: Option<(MediaId, String)>,
    pub(super) rename_focus: bool,
    /// A folder or photo name field had focus in the last frame.
    pub(super) typing: bool,
    /// Planned changes when the board was last arranged: photos and videos
    /// that move or are renamed, and new folders.
    pub(super) changes: usize,
}

const HEADER: f32 = 44.0;
const TILE: f32 = 160.0;

fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}
/// Whether folder `source` can move into `target`: not into itself, a
/// folder inside it, or the parent it is already in.
fn can_nest(source: &str, target: &str) -> bool {
    !source.is_empty()
        && !within(target, source)
        && source.rsplit_once('/').map_or("", |(p, _)| p) != target
}
/// The end of every notice about an edit to the plan.
const STAGED: &str = " · nothing changes on disk until you review the changes";

/// A folder being dragged in the sidebar onto another folder.
pub(super) struct FolderDrag(pub(super) String);
/// What a folder's right-click menu, drag or the new-folder button asked for.
pub(super) enum FolderAction {
    NewInside(String),
    Rename(String),
    MoveSelected(String),
    /// Moves a folder into another (`""`: the top level).
    Nest(String, String),
    /// Puts a folder inside a new folder made where it is.
    Wrap(String),
    Reveal(PathBuf),
    Trash(String),
    /// Reviews moving a folder that exists on disk to Trash, with every
    /// file and empty folder inside it.
    TrashFolder(PathBuf),
    Remove(String),
    StartOver,
}
fn within(path: &str, folder: &str) -> bool {
    folder.is_empty()
        || path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The heading over photos outside every proposed folder. Until an
/// analysis-based grouping has a result, they are waiting, not staying.
pub(super) fn unplaced_title(pending: Option<Pending>) -> &'static str {
    match pending {
        None => "Not moving",
        Some(Pending::Running) => "Grouping…",
        Some(_) => "Not grouped yet",
    }
}

impl PhotoSortApp {
    /// The note under the folder tree about photos outside every folder.
    pub(super) fn board_note(&self) -> Option<String> {
        let unplaced = self.board.unplaced;
        if self.board.original {
            return Some("The folders as they are on disk now · release to see the plan".into());
        }
        let planned = (self.board.changes > 0).then(|| {
            format!(
                "{} planned {} · nothing changes on disk until you review and apply {}",
                self.board.changes,
                if self.board.changes == 1 {
                    "change"
                } else {
                    "changes"
                },
                if self.board.changes == 1 {
                    "it"
                } else {
                    "them"
                }
            )
        });
        Some(match self.board_pending() {
            None if unplaced > 0 => match planned {
                Some(planned) => format!("{planned}\n\n{}", self.unplaced_sentence()),
                None => self.unplaced_sentence(),
            },
            None if planned.is_some() => return planned,
            Some(Pending::Running) => "Folders appear when grouping finishes".into(),
            Some(Pending::Waiting) => {
                "Not grouped yet · nothing moves until you review the changes".into()
            }
            Some(Pending::Cancelled) => "Grouping cancelled · no folders proposed".into(),
            Some(Pending::Failed) => "Grouping stopped · no folders proposed".into(),
            None => return None,
        })
    }
    /// "58 photos without a recognized person stay in their current folders".
    pub(super) fn unplaced_sentence(&self) -> String {
        let (all, videos) = (self.board.unplaced, self.board.unplaced_videos);
        let what = self.unplaced_phrase(all - videos, videos);
        format!(
            "{what} {} in {} current {}",
            if all == 1 { "stays" } else { "stay" },
            if all == 1 { "its" } else { "their" },
            if all == 1 { "folder" } else { "folders" }
        )
    }
    /// The counts beside the board's title.
    pub(super) fn board_counts(&self, folders: usize, items: usize, all: bool) -> String {
        let items = format!("{items} {}", if items == 1 { "item" } else { "items" });
        match self.board_pending() {
            Some(Pending::Running) => format!("{items} · grouping…"),
            Some(_) => format!("{items} · not grouped yet"),
            None => {
                let mut summary = format!(
                    "{folders} {} · {items}",
                    if folders == 1 { "folder" } else { "folders" }
                );
                if all && self.board.unplaced > 0 {
                    summary.push_str(&format!(" · {} not moving", self.board.unplaced));
                }
                summary
            }
        }
    }
    pub(super) fn board_key(&self) -> BoardKey {
        BoardKey {
            mode: self.group_mode,
            epoch: self.board_epoch,
            media: self.media.len(),
            // Only metadata-based modes regroup as metadata arrives; elsewhere
            // arriving thumbnails and metadata must not reshuffle the tiles.
            metadata: if matches!(
                self.group_mode,
                GroupMode::Date | GroupMode::Camera | GroupMode::Location
            ) {
                self.metadata.len()
            } else {
                0
            },
            draft: self
                .organize_drafts
                .get(&self.group_mode)
                .map(|d| (d.labels().len(), d.folders().len())),
            grouped: [
                self.visual_groups.is_some(),
                self.mixed_labels.is_some(),
                self.face_groups.is_some(),
            ],
            layout: self.preferences.location_layout,
            faceless: self.preferences.faceless,
            original: self.board.original,
        }
    }
    /// Rearranges Organize when what it shows has changed.
    pub(super) fn check_board(&mut self) {
        if self.step == Step::Organize && self.board.key.as_ref() != Some(&self.board_key()) {
            self.filter_dirty = true;
        }
    }
    /// Why the grouping has no result yet; never while the original
    /// folders are shown, since they need no grouping.
    pub(super) fn board_pending(&self) -> Option<Pending> {
        self.organize_pending().filter(|_| !self.board.original)
    }
    /// Shows the folders as they are on disk now (`true`) or the plan.
    pub(super) fn show_original(&mut self, on: bool) {
        if self.board.original == on {
            return;
        }
        if on {
            self.board.focus_before = Some(self.board.focus.take());
            self.board.editing = None;
        } else {
            self.board.focus = self.board.focus_before.take().flatten();
        }
        self.board.original = on;
        self.board.current = None;
        self.filter_dirty = true;
    }
    /// Every item's folder on disk now, relative to the library folder.
    fn original_labels(&self) -> HashMap<MediaId, String> {
        self.media
            .iter()
            .filter_map(|m| Some((m.id, self.current_label(m)?)))
            .collect()
    }
    /// The output folder's own name, for the top of the tree (the library
    /// folder's while the original folders are shown).
    fn board_root_name(&self) -> String {
        let root = match (&self.catalog, self.board.original) {
            (Some(catalog), true) => catalog.root.clone(),
            _ => PathBuf::from(&self.output),
        };
        root.file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Destination".into())
    }
    /// Sorts `filtered` by destination folder, then capture time, and
    /// records the folder sections and tree.
    pub(super) fn arrange_board(&mut self) {
        let key = self.board_key();
        let original = self.board.original;
        let draft = self
            .organize_drafts
            .get(&self.group_mode)
            .filter(|_| !original);
        let generated;
        let labels = match draft {
            Some(draft) => draft.labels(),
            None => {
                generated = if original {
                    self.original_labels()
                } else {
                    self.organize_labels()
                };
                &generated
            }
        };
        let mut keyed: Vec<_> = self
            .filtered
            .iter()
            .map(|&i| {
                let media = &self.media[i];
                let path = labels.get(&media.id).cloned();
                let order = crate::core::dates::capture_order(media, self.metadata.get(&media.id));
                (path.is_none(), path, order, i)
            })
            .collect();
        keyed.sort();
        self.filtered = keyed.iter().map(|k| k.3).collect();
        if !original {
            let output = std::path::Path::new(&self.output);
            let mut changed: HashSet<MediaId> = labels
                .iter()
                .filter(|(id, label)| {
                    self.index.get(id).is_some_and(|&i| {
                        self.media[i].path.parent()
                            != Some(
                                crate::core::operations::folder_destination(output, label)
                                    .as_path(),
                            )
                    })
                })
                .map(|(id, _)| *id)
                .collect();
            changed.extend(self.file_names.keys().copied());
            self.board.changes = changed.len() + draft.map_or(0, |d| d.created_folders().len());
        }
        let mut sections: Vec<Section> = Vec::new();
        for (position, (_, path, _, index)) in keyed.iter().enumerate() {
            let video = usize::from(self.media[*index].kind == MediaKind::Video);
            match sections.last_mut() {
                Some(section) if section.path == *path => {
                    section.range.end = position + 1;
                    section.videos += video;
                }
                _ => sections.push(Section {
                    path: path.clone(),
                    range: position..position + 1,
                    videos: video,
                }),
            }
        }
        // The tree: the draft's folders (which keep empty ones), or every
        // proposed folder and its parents.
        let mut paths: std::collections::BTreeSet<String> = match (draft, &self.catalog) {
            (Some(draft), _) => draft.folders().clone(),
            // Every folder on disk, empty ones included.
            (None, Some(catalog)) if original => Draft::from_catalog(catalog, HashMap::new())
                .folders()
                .clone(),
            (None, _) => labels.values().cloned().collect(),
        };
        for path in paths.clone() {
            let mut parent = path.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                paths.insert(prefix.to_owned());
                parent = prefix;
            }
        }
        paths.insert(String::new());
        let direct: HashMap<&str, usize> = sections
            .iter()
            .filter_map(|s| Some((s.path.as_deref()?, s.range.len())))
            .collect();
        let folders: Vec<FolderRow> = paths
            .iter()
            .map(|path| FolderRow {
                path: path.clone(),
                depth: if path.is_empty() {
                    0
                } else {
                    path.matches('/').count() + 1
                },
                direct: direct.get(path.as_str()).copied().unwrap_or(0),
                total: direct
                    .iter()
                    .filter(|(p, _)| within(p, path))
                    .map(|(_, n)| n)
                    .sum(),
            })
            .collect();
        // A generated grouping lists the folders it fills, not the library's
        // existing folders it empties; folders made on the board stay.
        let folders = if self.group_mode == GroupMode::Current || original {
            folders
        } else {
            let created = draft.map(|d| d.created_folders());
            folders
                .into_iter()
                .filter(|f| {
                    f.path.is_empty()
                        || f.total > 0
                        || created.is_some_and(|c| c.iter().any(|p| within(p, &f.path)))
                })
                .collect()
        };
        self.board.unplaced = sections
            .iter()
            .filter(|s| s.path.is_none())
            .map(|s| s.range.len())
            .sum();
        self.board.unplaced_videos = sections
            .iter()
            .filter(|s| s.path.is_none())
            .map(|s| s.videos)
            .sum();
        self.board.sections = sections;
        self.board.folders = folders;
        self.board.key = Some(key);
        if let Some(focus) = &self.board.focus
            && !self.board.folders.iter().any(|f| &f.path == focus)
        {
            self.board.focus = None;
        }
    }
    /// The board section holding the active photo.
    fn active_section(&self) -> Option<&Section> {
        let index = self.index.get(&self.active?)?;
        let position = self.filtered.iter().position(|i| i == index)?;
        self.board
            .sections
            .iter()
            .find(|s| s.range.contains(&position))
    }
    /// Follows the folder you are looking at. Right after a folder is
    /// chosen in the sidebar, that folder stays highlighted until you scroll
    /// or move to another photo.
    fn board_look_at(&mut self, current: Option<Option<String>>, offset: f32) {
        let now = (offset, self.active);
        match self.board.hold {
            Some(None) => {
                self.board.hold = Some(Some(now));
                return;
            }
            Some(Some(then)) if then == now => return,
            Some(Some(_)) => self.board.hold = None,
            None => {}
        }
        if current.is_some() && current != self.board.current {
            self.board.current = current;
            self.board.reveal_current = true;
        }
    }
    /// The sidebar row to highlight: the folder you are looking at, or the
    /// folder shown (the library row when every item is).
    pub(super) fn board_highlight(&self) -> Option<String> {
        match &self.board.current {
            Some(Some(path)) => Some(path.clone()),
            _ => Some(self.board.focus.clone().unwrap_or_default()),
        }
    }
    /// Shows one folder (`None`: every item). Its first photo becomes the
    /// current one unless the current photo is already in it, so the arrow
    /// keys walk this folder's photos next.
    pub(super) fn board_select_folder(&mut self, focus: Option<String>) {
        let shown = |section: &Section| match (&focus, &section.path) {
            (None, _) => true,
            (Some(focus), Some(path)) => within(path, focus),
            (Some(_), None) => false,
        };
        let inside = self.active_section().is_some_and(shown);
        let first = self
            .board
            .sections
            .iter()
            .find(|s| shown(s))
            .and_then(|s| self.filtered.get(s.range.start))
            .map(|&i| self.media[i].id);
        self.board.focus = focus;
        if !inside && let Some(id) = first {
            if self.library_selection_mode() {
                self.selection.single(id);
                self.comparison.clear();
            }
            self.set_active(id);
        }
        self.library_reveal = false;
    }
    /// After the arrow keys move past the shown folder's last (or first)
    /// photo, the sidebar follows to the folder now holding it.
    pub(super) fn board_follow_active(&mut self) {
        let Some(focus) = self.board.focus.as_deref() else {
            return;
        };
        let Some(section) = self.active_section() else {
            return;
        };
        let next = match &section.path {
            Some(path) if within(path, focus) => return,
            Some(path) => Some(path.clone()),
            None => None, // Only "every item" shows those staying in place.
        };
        self.board.focus = next;
        self.library_reveal = true;
    }
    /// Whether the plan can be edited on the board now.
    pub(super) fn board_editable(&self) -> bool {
        self.can_edit_folders() && !self.board.original
    }
    /// The folder draft to edit, created from the current proposal when the
    /// first edit is made.
    fn board_draft(&mut self) -> Option<&mut Draft> {
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
    /// Moves photos into a proposed folder.
    pub(super) fn board_move(&mut self, ids: &[MediaId], target: &str) {
        // Items outside the current proposal (videos and ungrouped photos in
        // the analysis modes) stay where they are and cannot be moved.
        let movable: Vec<MediaId> = {
            let generated;
            let labels = match self.organize_drafts.get(&self.group_mode) {
                Some(draft) => draft.labels(),
                None => {
                    generated = self.organize_labels();
                    &generated
                }
            };
            ids.iter()
                .copied()
                .filter(|id| labels.contains_key(id))
                .collect()
        };
        let staying = ids.len() - movable.len();
        if movable.is_empty() {
            self.navigation_notice = Some(if staying == 1 {
                "This item stays in its current folder in this grouping and cannot be moved".into()
            } else {
                format!(
                    "These {staying} items stay in their current folders in this grouping and cannot be moved"
                )
            });
            return;
        }
        if self.board_draft().is_none() {
            return;
        }
        // A photo's RAW, JPEG and edit files move together when that is on.
        let selection = crate::core::organize::Selection {
            files: movable.iter().copied().collect(),
            ..Default::default()
        };
        let Some(draft) = self.organize_drafts.get_mut(&self.group_mode) else {
            return;
        };
        match draft.move_selection(&selection, target, &self.media, self.companions) {
            Ok(()) => {
                let title = if target.is_empty() {
                    self.board_root_name()
                } else {
                    target.to_owned()
                };
                let mut notice = format!(
                    "Moved {} {} to {title}",
                    movable.len(),
                    if movable.len() == 1 { "item" } else { "items" }
                );
                if staying > 0 {
                    notice.push_str(&format!(" · {staying} stay in their current folders"));
                }
                notice.push_str(STAGED);
                self.navigation_notice = Some(notice);
            }
            Err(error) => self.navigation_notice = Some(error.to_string()),
        }
    }
    fn board_finish_edit(&mut self) {
        let Some((path, name)) = self.board.editing.take() else {
            return;
        };
        let name = name.trim().to_owned();
        if name.is_empty() || name == leaf(&path) {
            return;
        }
        let Some(draft) = self.board_draft() else {
            return;
        };
        match draft.rename_folder(&path, &name) {
            Ok(renamed) => {
                if let Some(focus) = &self.board.focus
                    && within(focus, &path)
                {
                    self.board.focus = Some(format!("{renamed}{}", &focus[path.len()..]));
                }
            }
            Err(error) => self.navigation_notice = Some(error.to_string()),
        }
    }
    /// Starts renaming a folder in place in the sidebar.
    pub(super) fn board_rename_folder(&mut self, path: &str) {
        if !path.is_empty() {
            self.board.editing = Some((path.to_owned(), leaf(path).to_owned()));
            self.board.edit_focus = true;
            self.board.reveal_editing = true;
        }
    }
    /// Two to nine selected photos on the board, in board order: the preview
    /// compares them side by side. Larger selections preview the active photo.
    pub(super) fn board_selection_comparison(&self) -> Option<Vec<MediaId>> {
        if self.step != Step::Organize
            || !self.library_selection_mode()
            || !(2..=9).contains(&self.selection.len())
        {
            return None;
        }
        // Called several times a frame: one pass that stops at the last
        // selected tile, without hashing every shown photo's id.
        let wanted: Vec<usize> = self
            .selection
            .snapshot()
            .0
            .iter()
            .filter_map(|id| self.index.get(id).copied())
            .collect();
        let mut shown = Vec::with_capacity(wanted.len());
        for &index in &self.filtered {
            if wanted.contains(&index) {
                shown.push(self.media[index].id);
                if shown.len() == wanted.len() {
                    break;
                }
            }
        }
        (shown.len() > 1).then_some(shown)
    }
    /// Back to the single preview: keeps only the active photo selected.
    pub(super) fn end_board_comparison(&mut self) {
        self.comparison.clear();
        if let Some(active) = self.active {
            self.selection.single(active);
        }
        self.focused_slot = 0;
    }
    /// The photos a drag or a menu acts on: the selection when `id` is in it.
    fn board_targets(&self, id: MediaId) -> Vec<MediaId> {
        if self.selection.contains(id) {
            self.selection.ordered(&self.media, &self.filtered)
        } else {
            vec![id]
        }
    }
    /// Move, rename and Trash items for photo tiles.
    pub(super) fn board_move_menu(&mut self, ui: &mut egui::Ui, id: MediaId) {
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
            .on_hover_text("Nothing goes to Trash until you review the changes")
            .clicked()
        {
            self.stage_manual(ids, crate::core::decisions::Change::Trash(!marked));
            ui.close();
        }
    }
    /// The proposed folders as menu items that move `ids` there.
    pub(super) fn board_folder_list(&mut self, ui: &mut egui::Ui, ids: &[MediaId]) {
        let folders: Vec<String> = self.board.folders.iter().map(|f| f.path.clone()).collect();
        let root = self.board_root_name();
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
    /// Makes a new folder holding `ids` inside the folder shown, then starts
    /// naming it.
    pub(super) fn board_new_folder_with(&mut self, ids: &[MediaId]) {
        let parent = self.board.focus.clone().unwrap_or_default();
        self.board_new_folder(&parent, ids);
    }
    /// Makes "New folder" (numbered when taken) inside `parent` in the plan,
    /// moves `ids` into it and starts renaming it in place, as Finder does.
    /// Nothing is made on disk until the changes are reviewed and applied.
    pub(super) fn board_new_folder(&mut self, parent: &str, ids: &[MediaId]) {
        let name = self.unused_folder_name(parent);
        let Some(draft) = self.board_draft() else {
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
    /// inside `parent`.
    fn unused_folder_name(&self, parent: &str) -> String {
        (1..)
            .map(|n| {
                if n == 1 {
                    "New folder".to_owned()
                } else {
                    format!("New folder {n}")
                }
            })
            .find(|name| {
                let path = join(parent, name);
                // Hidden empty folders still take their names.
                !self.board.folders.iter().any(|f| f.path == path)
                    && !self
                        .organize_drafts
                        .get(&self.group_mode)
                        .is_some_and(|d| d.folders().contains(&path))
            })
            .unwrap_or_default()
    }
    /// Starts renaming a photo in the preview's title. Its RAW, JPEG and
    /// edit files take the same name when they move together.
    pub(super) fn board_rename_photo(&mut self, id: MediaId) {
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
    }
    /// Stages the typed name; the file is renamed when the changes are
    /// reviewed and applied.
    pub(super) fn board_finish_rename(&mut self) {
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
    pub(super) fn board_library_folder(&self, path: &str) -> Option<PathBuf> {
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
    pub(super) fn board_folder_on_disk(&self, path: &str) -> Option<PathBuf> {
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

    /// The folder panel: every proposed folder with its count. Folders are
    /// made, renamed, moved and removed right here, by right-click and drag.
    fn board_tree(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let editable = self.board_editable();
        let mut action: Option<FolderAction> = None;
        ui.horizontal(|ui| {
            ui.label(caption("Folders"));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if named(
                    ui.add_enabled(editable, egui::Button::new(icon::FOLDER_PLUS).frame(false)),
                    egui::WidgetType::Button,
                    "New folder",
                )
                .on_hover_text("New folder · inside the folder shown · Ctrl/Cmd+Shift+N")
                .clicked()
                {
                    action = Some(FolderAction::NewInside(
                        self.board.focus.clone().unwrap_or_default(),
                    ));
                }
            });
        });
        ui.add_space(S1);
        let root = self.board_root_name();
        let rows = self.board.folders.len();
        // Must equal the height `folder_row` allocates.
        let row_height = 26.0;
        let mut select: Option<Option<String>> = None;
        let mut drop: Option<(Vec<MediaId>, String)> = None;
        let mut finish = false;
        // What stays where it is, under the folders; the folders get the rest.
        let note = self.board_note().map(|note| {
            ui.painter().layout(
                note,
                egui::TextStyle::Small.resolve(ui.style()),
                MUTED,
                ui.available_width(),
            )
        });
        let reserve = note.as_ref().map_or(0.0, |g| g.size().y + S3);
        let highlighted = self.board_highlight();
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("organize_folders")
            .auto_shrink([false, false])
            .max_height((ui.available_height() - reserve).max(row_height));
        // Scroll a folder into view once: the one being renamed (a new
        // folder can be anywhere in a long tree), else the highlighted one.
        let editing = self.board.editing.as_ref().map(|(path, _)| path.clone());
        let reveal = if self.board.reveal_editing && editing.is_some() {
            editing.clone()
        } else if self.board.reveal_current {
            self.board.reveal_current = false;
            highlighted.clone()
        } else {
            None
        };
        if let Some(index) =
            reveal.and_then(|target| self.board.folders.iter().position(|f| f.path == target))
        {
            self.board.reveal_editing = false;
            let pitch = row_height + ui.spacing().item_spacing.y;
            let (offset, height) = self.board.tree_view;
            let top = index as f32 * pitch;
            if top < offset {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top + pitch > offset + height {
                scroll = scroll.vertical_scroll_offset(top + pitch - height);
            }
        }
        let output = scroll.show_rows(ui, row_height, rows, |ui, range| {
            for row in range {
                let folder = &self.board.folders[row];
                let path = folder.path.clone();
                let (depth, direct, total) = (folder.depth, folder.direct, folder.total);
                if let Some((editing, name)) = &mut self.board.editing
                    && *editing == path
                {
                    ui.horizontal(|ui| {
                        ui.set_height(row_height);
                        ui.add_space(depth as f32 * 14.0 + 6.0);
                        let mut field = egui::TextEdit::singleline(name)
                            .id_salt(("organize-folder-name", &path))
                            .desired_width(ui.available_width() - 8.0)
                            .show(ui);
                        if std::mem::take(&mut self.board.edit_focus) {
                            field.response.request_focus();
                            field.state.cursor.set_char_range(Some(
                                egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(name.chars().count()),
                                ),
                            ));
                            field.state.store(ui.ctx(), field.response.id);
                        }
                        finish |= field.response.lost_focus();
                        self.board.typing |=
                            field.response.has_focus() || field.response.lost_focus();
                    });
                    continue;
                }
                let title = if path.is_empty() {
                    root.clone()
                } else {
                    leaf(&path).to_owned()
                };
                // The library folder is the top row and shows every
                // item, those staying in place included.
                let top = path.is_empty();
                let selected = highlighted.as_deref() == Some(path.as_str());
                let response = folder_row(
                    ui,
                    &title,
                    if top {
                        icon::HARD_DRIVES
                    } else if total == 0 {
                        icon::FOLDER_DASHED
                    } else {
                        icon::FOLDER_SIMPLE
                    },
                    depth,
                    if top { self.filtered.len() } else { total },
                    selected,
                    editable && !top,
                )
                .on_hover_text(if top {
                    format!("{root} · every item · {direct} directly in it, {total} in its folders")
                } else {
                    format!(
                        "{path} · {direct} here, {total} including subfolders · drag onto another folder to move it"
                    )
                });
                if editable && !top && response.drag_started() {
                    response.dnd_set_drag_payload(FolderDrag(path.clone()));
                }
                let photos = response
                    .dnd_hover_payload::<super::grouping::PhotoDrag>()
                    .is_some();
                let folder_here = response
                    .dnd_hover_payload::<FolderDrag>()
                    .is_some_and(|source| can_nest(&source.0, &path));
                if editable && (photos || folder_here) {
                    ui.painter().rect_stroke(
                        response.rect,
                        R_CONTROL,
                        Stroke::new(1.5, ACCENT),
                        StrokeKind::Inside,
                    );
                }
                // Taking a payload of the other type would drop it.
                if editable
                    && photos
                    && let Some(payload) =
                        response.dnd_release_payload::<super::grouping::PhotoDrag>()
                {
                    drop = Some((payload.0.clone(), path.clone()));
                }
                if editable
                    && egui::DragAndDrop::has_payload_of_type::<FolderDrag>(ui.ctx())
                    && let Some(source) = response.dnd_release_payload::<FolderDrag>()
                    && can_nest(&source.0, &path)
                {
                    action = Some(FolderAction::Nest(source.0.clone(), path.clone()));
                }
                if response.clicked() {
                    select = Some((!top).then(|| path.clone()));
                    // Arrow keys then walk this folder's photos instead
                    // of moving between folder rows.
                    ui.memory_mut(|m| m.surrender_focus(response.id));
                }
                if response.double_clicked() && editable && !top {
                    action = Some(FolderAction::Rename(path.clone()));
                }
                response.context_menu(|ui| {
                    if let Some(chosen) = self.board_folder_menu(ui, &path, total) {
                        action = Some(chosen);
                    }
                });
            }
        });
        self.board.tree_view = (output.state.offset.y, output.inner_rect.height());
        if let Some(note) = note {
            ui.add_space(S2);
            ui.add(egui::Label::new(RichText::new(note.text()).small().color(MUTED)).wrap());
        }
        if let Some(focus) = select {
            self.board.current = Some(Some(focus.clone().unwrap_or_default()));
            self.board.hold = Some(None);
            self.board_select_folder(focus);
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("organize-board-top"), true));
        }
        if let Some((ids, target)) = drop {
            self.board_move(&ids, &target);
        }
        // Enter or clicking elsewhere keeps the name; Escape keeps the old one.
        if finish {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.board.editing = None;
            } else {
                self.board_finish_edit();
            }
        }
        if let Some(action) = action {
            self.board_folder_action(action);
        }
    }
    /// A folder's right-click menu. `total` counts the items in it,
    /// subfolders included.
    fn board_folder_menu(
        &mut self,
        ui: &mut egui::Ui,
        path: &str,
        total: usize,
    ) -> Option<FolderAction> {
        let mut action = None;
        let editable = self.board_editable();
        let top = path.is_empty();
        let mut choose = |ui: &mut egui::Ui, enabled: bool, label: &str, chosen: FolderAction| {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                action = Some(chosen);
                ui.close();
            }
        };
        choose(
            ui,
            editable,
            "New folder inside",
            FolderAction::NewInside(path.to_owned()),
        );
        if !top {
            choose(
                ui,
                editable,
                "Rename · double-click",
                FolderAction::Rename(path.to_owned()),
            );
        }
        let selected = self.selection.len();
        if selected > 0 && self.library_selection_mode() {
            choose(
                ui,
                editable,
                &format!("Move {selected} selected here"),
                FolderAction::MoveSelected(path.to_owned()),
            );
        }
        if !top {
            let targets: Vec<String> = self
                .board
                .folders
                .iter()
                .map(|f| f.path.clone())
                .filter(|target| !target.is_empty() && can_nest(path, target))
                .collect();
            ui.add_enabled_ui(editable && !targets.is_empty(), |ui| {
                ui.menu_button("Move into", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for target in targets {
                                if ui.button(target.replace('/', " / ")).clicked() {
                                    action = Some(FolderAction::Nest(path.to_owned(), target));
                                    ui.close();
                                }
                            }
                        });
                });
            });
            if path.contains('/')
                && ui
                    .add_enabled(editable, egui::Button::new("Move to the top level"))
                    .clicked()
            {
                action = Some(FolderAction::Nest(path.to_owned(), String::new()));
                ui.close();
            }
            if ui
                .add_enabled(editable, egui::Button::new("Put inside a new folder"))
                .on_hover_text("Makes a folder in the same place and moves this one into it")
                .clicked()
            {
                action = Some(FolderAction::Wrap(path.to_owned()));
                ui.close();
            }
        }
        if let Some(folder) = self.board_folder_on_disk(path) {
            ui.separator();
            let label = crate::core::desktop::Action::Reveal.label();
            if ui
                .add_enabled(self.desktop_available(), egui::Button::new(label))
                .clicked()
            {
                action = Some(FolderAction::Reveal(folder));
                ui.close();
            }
        }
        let ids = self.board_folder_items(path);
        let new = self
            .organize_drafts
            .get(&self.group_mode)
            .is_some_and(|d| d.is_new_folder(path));
        let library = self.board_library_folder(path);
        if !ids.is_empty() || new || library.is_some() {
            ui.separator();
        }
        if !ids.is_empty() {
            let marked = ids.iter().all(|id| self.decision(*id).rejected);
            if ui
                .add_enabled(
                    !self.busy,
                    egui::Button::new(if marked {
                        format!("Unmark {total} for Trash")
                    } else {
                        format!("Mark {total} for Trash")
                    }),
                )
                .on_hover_text(
                    "Marks every photo and video in this folder and its subfolders. Nothing goes to Trash until you review the changes.",
                )
                .clicked()
            {
                action = Some(FolderAction::Trash(path.to_owned()));
                ui.close();
            }
        }
        if let Some(folder) = library
            && ui
                .add_enabled(
                    !self.busy && !self.scanning,
                    egui::Button::new("Move folder to Trash…"),
                )
                .on_hover_text(
                    "Shows the whole folder, every file and empty folder in it, for review before anything goes to Trash",
                )
                .clicked()
        {
            action = Some(FolderAction::TrashFolder(folder));
            ui.close();
        }
        if new
            && ui
                .add_enabled(editable, egui::Button::new("Remove folder"))
                .on_hover_text("Removes this new folder from the plan. Its photos go back to where they were going.")
                .clicked()
        {
            action = Some(FolderAction::Remove(path.to_owned()));
            ui.close();
        }
        if top && self.organize_drafts.contains_key(&self.group_mode) {
            ui.separator();
            if ui
                .add_enabled(editable, egui::Button::new("Start over"))
                .on_hover_text("Undo every folder edit in this grouping")
                .clicked()
            {
                action = Some(FolderAction::StartOver);
                ui.close();
            }
        }
        action
    }
    /// The photos and videos in a sidebar folder and its subfolders.
    fn board_folder_items(&self, path: &str) -> Vec<MediaId> {
        self.board
            .sections
            .iter()
            .filter(|s| s.path.as_deref().is_some_and(|p| within(p, path)))
            .flat_map(|s| self.filtered[s.range.clone()].iter())
            .map(|&i| self.media[i].id)
            .collect()
    }
    pub(super) fn board_folder_action(&mut self, action: FolderAction) {
        match action {
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
                        if let Some(focus) = &self.board.focus
                            && within(focus, &source)
                        {
                            self.board.focus = Some(format!("{moved}{}", &focus[source.len()..]));
                        }
                        self.navigation_notice = Some(format!(
                            "Moved {} into {}{}",
                            leaf(&source),
                            if target.is_empty() {
                                self.board_root_name()
                            } else {
                                target.replace('/', " / ")
                            },
                            STAGED
                        ));
                    }
                    Err(error) => self.navigation_notice = Some(error.to_string()),
                }
            }
            FolderAction::Wrap(source) => {
                let parent = source.rsplit_once('/').map_or("", |(p, _)| p).to_owned();
                let name = self.unused_folder_name(&parent);
                let Some(draft) = self.board_draft() else {
                    return;
                };
                match draft.create_parent(&source, &name) {
                    Ok(_) => {
                        self.board.focus = None;
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
                let initial = self.organize_labels();
                let Some(draft) = self.board_draft() else {
                    return;
                };
                match draft.remove_folder(&path, &initial) {
                    Ok(()) => {
                        if self
                            .board
                            .focus
                            .as_deref()
                            .is_some_and(|f| within(f, &path))
                        {
                            self.board.focus = None;
                        }
                    }
                    Err(error) => self.navigation_notice = Some(error.to_string()),
                }
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

    /// The preview panel: the clicked photo (or the compared selection) at a
    /// useful size, with its name, folder and the usual mark, rating and zoom
    /// controls. Scroll to zoom, drag to pan, double-click for 1:1.
    fn board_preview(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let ids = self.comparison_ids();
        let Some(active) = self.active.filter(|id| self.index.contains_key(id)) else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Click a photo to preview it here").color(MUTED));
            });
            return;
        };
        let media = self.media[self.index[&active]].clone();
        if self
            .board
            .renaming
            .as_ref()
            .is_some_and(|(id, _)| *id != active)
        {
            self.board.renaming = None;
        }
        let mut finish = false;
        ui.horizontal(|ui| {
            if let Some((_, name)) = &mut self.board.renaming {
                let extension = media
                    .path
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                let mut field = egui::TextEdit::singleline(name)
                    .id_salt("organize-photo-name")
                    .desired_width((ui.available_width() - 60.0).max(80.0))
                    .show(ui);
                ui.label(RichText::new(extension).color(MUTED));
                if std::mem::take(&mut self.board.rename_focus) {
                    field.response.request_focus();
                    field
                        .state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(name.chars().count()),
                        )));
                    field.state.store(ui.ctx(), field.response.id);
                }
                finish = field.response.lost_focus();
                self.board.typing |= field.response.has_focus() || finish;
                return;
            }
            let title = self.file_names.get(&active).map_or_else(
                || media.name(),
                |stem| {
                    crate::core::organize::renamed_file_name(
                        &media.path,
                        std::ffi::OsStr::new(stem),
                    )
                    .to_string_lossy()
                    .into_owned()
                },
            );
            let label = ui
                .add(
                    egui::Label::new(RichText::new(title).font(semibold(13.0)))
                        .truncate()
                        .sense(Sense::click()),
                )
                .on_hover_text(format!(
                    "{} · double-click or F2 to rename",
                    media.path.display()
                ));
            if label.double_clicked() {
                self.board_rename_photo(active);
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ids.len() > 1 {
                    if ui
                        .small_button("End comparison")
                        .on_hover_text("Show only this photo · Esc")
                        .clicked()
                    {
                        self.end_board_comparison();
                    }
                } else if self.library_selection_mode() && self.selection.len() > 9 {
                    ui.label(
                        RichText::new("Select 2–9 photos to compare")
                            .small()
                            .color(MUTED),
                    );
                }
            });
        });
        // Enter or clicking elsewhere keeps the name; Escape keeps the old one.
        if finish {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.board.renaming = None;
            } else {
                self.board_finish_rename();
            }
        }
        let pending = self
            .board_pending()
            .filter(|_| !self.organize_drafts.contains_key(&self.group_mode));
        let line = match pending {
            None if self.board.original => format!(
                "{}  Now in {}",
                icon::FOLDER_SIMPLE,
                self.readable_folder(media.path.parent().unwrap_or(std::path::Path::new("")))
            ),
            Some(pending) => format!("{}  {}", icon::CIRCLE_DASHED, unplaced_title(Some(pending))),
            None => {
                let placement = self.placement(&media);
                if placement.moves {
                    format!("{}  Moves to {}", icon::ARROW_RIGHT, placement.folder)
                } else {
                    format!("{}  Stays in {}", icon::FOLDER_SIMPLE, placement.folder)
                }
            }
        };
        ui.add(egui::Label::new(RichText::new(line).small().color(MUTED)).truncate());
        ui.add_space(S1);
        // The mark, rating and zoom controls take two rows in a narrow panel,
        // so they never widen it over the board.
        let narrow = ui.available_width() < 560.0;
        let controls = if narrow { 70.0 } else { 34.0 };
        let size = egui::vec2(
            ui.available_width(),
            (ui.available_height() - controls).max(80.0),
        );
        let ids = if ids.is_empty() { vec![active] } else { ids };
        let gap = ui.spacing().item_spacing;
        let columns = self.preview_columns(&ids, size, gap);
        let rows = ids.len().div_ceil(columns);
        let cell = egui::vec2(
            (size.x - gap.x * (columns - 1) as f32) / columns as f32,
            ((size.y - gap.y * (rows - 1) as f32) / rows as f32).max(60.0),
        );
        ui.allocate_ui(size, |ui| {
            for row in 0..rows {
                ui.horizontal(|ui| {
                    for column in 0..columns {
                        let slot = row * columns + column;
                        if let Some(id) = ids.get(slot) {
                            self.photo_canvas(ui, *id, slot as u64, cell, true);
                        }
                    }
                });
            }
        });
        let row = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
            if narrow {
                ui.horizontal_wrapped(|ui| add(ui));
            } else {
                ui.horizontal_centered(|ui| add(ui));
            }
        };
        row(ui, &mut |ui| {
            let rejected = self.decision(active).rejected;
            if icon_text_button(
                ui,
                true,
                rejected,
                icon::TRASH,
                if rejected {
                    "Marked for Trash"
                } else {
                    "Trash"
                },
            )
            .on_hover_text(if rejected {
                "Unmark · D"
            } else {
                "Mark for Trash · D"
            })
            .clicked()
            {
                self.reject(active);
            }
            self.review_ratings(ui, active);
        });
    }
    /// Columns that show compared photos largest: their typical shape decides
    /// between side by side and stacked in the panel's shape.
    fn preview_columns(&self, ids: &[MediaId], size: Vec2, gap: Vec2) -> usize {
        let aspects: Vec<f32> = ids
            .iter()
            .filter_map(|id| self.metadata.get(id))
            .filter(|m| m.width > 0 && m.height > 0)
            .map(|m| {
                let (w, h) = if (5..=8).contains(&m.orientation) {
                    (m.height, m.width)
                } else {
                    (m.width, m.height)
                };
                w as f32 / h as f32
            })
            .collect();
        let aspect = if aspects.is_empty() {
            1.5
        } else {
            aspects.iter().sum::<f32>() / aspects.len() as f32
        };
        (1..=ids.len().max(1))
            .max_by(|a, b| {
                let width = |columns: usize| {
                    let rows = ids.len().div_ceil(columns);
                    let w = (size.x - gap.x * (columns - 1) as f32) / columns as f32;
                    let h = (size.y - gap.y * (rows - 1) as f32) / rows as f32;
                    w.min(h * aspect)
                };
                // Ties keep fewer columns.
                width(*a).total_cmp(&width(*b)).then(b.cmp(a))
            })
            .unwrap_or(1)
    }
    /// Above the photos: where you are, what it holds, and thumbnail size.
    fn board_summary(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let root = self.board_root_name();
        let focus = self.board.focus.clone();
        let (folders, items) = match &focus {
            None => (
                self.board
                    .sections
                    .iter()
                    .filter(|s| s.path.is_some())
                    .count(),
                self.filtered.len(),
            ),
            Some(focus) => {
                let shown: Vec<_> = self
                    .board
                    .sections
                    .iter()
                    .filter(|s| s.path.as_deref().is_some_and(|p| within(p, focus)))
                    .collect();
                (shown.len(), shown.iter().map(|s| s.range.len()).sum())
            }
        };
        let title = match &focus {
            None => root.clone(),
            Some(path) if path.is_empty() => root.clone(),
            Some(path) => format!("{root} / {}", path.replace('/', " / ")),
        };
        let summary = self.board_counts(folders, items, focus.is_none());
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            // The size slider is placed first, from the right; the title and
            // counts then get only the room left of it and truncate there.
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(icon::IMAGE).size(16.0).color(MUTED));
                let mut tile = self.board.tile.unwrap_or(TILE);
                ui.spacing_mut().slider_width = 90.0;
                ui.spacing_mut().interact_size.y = 14.0;
                if named(
                    ui.add(egui::Slider::new(&mut tile, 110.0..=300.0).show_value(false)),
                    egui::WidgetType::Slider,
                    "Thumbnail size",
                )
                .on_hover_text("Thumbnail size")
                .changed()
                {
                    self.board.tile = Some(tile);
                }
                ui.label(RichText::new(icon::IMAGE).size(11.0).color(MUTED));
                ui.add_space(S3);
                self.original_button(ui);
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
                    if focus.is_some()
                        && named(
                            ui.small_button(icon::ARROW_LEFT),
                            egui::WidgetType::Button,
                            "Back to all folders",
                        )
                        .on_hover_text(format!("Every item in {root}"))
                        .clicked()
                    {
                        self.board.focus = None;
                    }
                    ui.add(egui::Label::new(RichText::new(&title).font(semibold(13.0))).truncate());
                    ui.add(egui::Label::new(RichText::new(&summary).color(MUTED)).truncate())
                        .on_hover_text(&summary);
                });
            });
        });
    }
    /// Held down, shows the folders as they are on disk now; released, the
    /// plan again. Backslash does the same.
    fn original_button(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let on = self.board.original;
        // Only the icon when the board is narrow, so the counts stay readable.
        let text = if ui.available_width() < 440.0 {
            icon::CLOCK_COUNTER_CLOCKWISE.to_owned()
        } else {
            format!("{}  Show original", icon::CLOCK_COUNTER_CLOCKWISE)
        };
        let button =
            egui::Button::new(
                RichText::new(text)
                    .small()
                    .color(if on { ACCENT } else { INK }),
            )
            .selected(on)
            .sense(Sense::drag());
        let response = named(ui.add(button), egui::WidgetType::Button, "Show original")
            .on_hover_text("Hold to see the folders as they are on disk now · or hold \\");
        let held = response.is_pointer_button_down_on();
        if held != self.board.original_button {
            self.board.original_button = held;
            ui.ctx().request_repaint();
        }
    }
    /// Why the chosen grouping has no result, and the action that starts it.
    pub(super) fn organize_notice_text(
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
    fn organize_notice(&mut self, ui: &mut egui::Ui) {
        let Some((glyph, title, body, action)) = self.organize_notice_text() else {
            return;
        };
        let models = action == Some("AI models…");
        let enabled = models || self.can_start_analysis();
        if super::states::notice_card(ui, glyph, &title, &body, action.map(|a| (a, enabled))) {
            if models {
                self.open_model_setup();
            } else {
                self.start_organize_grouping();
            }
        }
    }
    /// The middle of Organize: photos grouped under their destination folder.
    pub(super) fn organize_board(&mut self, ui: &mut egui::Ui) {
        // A step switch earlier in this frame (the step tabs, a shortcut)
        // leaves `filtered` holding the previous step's photos, and the
        // sections from the last Organize visit, until the next refilter.
        // Sections always cover `filtered` exactly; rebuild both before drawing.
        // Holding "Show original" or backslash shows the folders on disk now.
        let key = ui.input(|i| i.focused && i.key_down(egui::Key::Backslash))
            && !ui.ctx().egui_wants_keyboard_input()
            && !self.modal_active();
        self.show_original(key || self.board.original_button);
        self.board.typing = false;
        if self.filter_dirty
            || self.board.key.is_none()
            || self.board.sections.last().map_or(0, |s| s.range.end) != self.filtered.len()
        {
            self.filter_dirty = true;
            self.refilter();
        }
        egui::Panel::left("organize_folder_panel")
            .resizable(true)
            .default_size(240.0)
            .size_range(180.0..=380.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(R_SURFACE)
                    .inner_margin(S2)
                    .outer_margin(egui::Margin {
                        left: 0,
                        right: 10,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| self.board_tree(ui));
        // The clicked photo, large, on the right.
        // The board keeps room for a row of tiles and its header; the preview
        // gives way in a narrow window.
        // Its controls need about 330 points in two rows.
        let widest = (ui.available_width() - 290.0).clamp(330.0, 1100.0);
        let preview_width = (ui.available_width() * 0.42).clamp(330.0, widest.min(900.0));
        egui::Panel::right("organize_preview_panel")
            .resizable(true)
            .default_size(preview_width)
            .size_range(330.0..=widest)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(R_SURFACE)
                    .inner_margin(S2)
                    .outer_margin(egui::Margin {
                        left: 10,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| self.board_preview(ui));
        let pending = self.board_pending();
        if self.analysis_run.is_some() {
            self.analysis_banner(ui);
            ui.add_space(S2);
        }
        if pending.is_some_and(|p| p != Pending::Running) {
            self.organize_notice(ui);
            ui.add_space(S2);
        }
        self.board_summary(ui);
        let gap = ui.spacing().item_spacing.x;
        let width = ui.available_width();
        let tile = self.board.tile.unwrap_or(TILE);
        let columns = ((width + gap) / tile).floor().max(1.0) as usize;
        let cell = (width - gap * (columns.saturating_sub(1) as f32)) / columns as f32;
        let tile_height = cell * 0.75 + library::CAPTION;
        let pitch = tile_height + ui.spacing().item_spacing.y;
        // Lay out the visible sections as header and tile rows.
        enum Row {
            Header(usize),
            Tiles(usize, usize),
        }
        let focus = self.board.focus.clone();
        let mut rows = Vec::new();
        let mut tops = Vec::new();
        let mut y = 0.0;
        for (index, section) in self.board.sections.iter().enumerate() {
            let shown = match (&focus, &section.path) {
                (None, _) => true,
                (Some(focus), Some(path)) => within(path, focus),
                (Some(_), None) => false,
            };
            if !shown {
                continue;
            }
            rows.push(Row::Header(index));
            tops.push(y);
            y += HEADER;
            for start in (section.range.start..section.range.end).step_by(columns) {
                rows.push(Row::Tiles(index, start));
                tops.push(y);
                y += pitch;
            }
        }
        let total = y;
        if rows.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("This folder is empty. Drag photos here from another folder.")
                        .color(MUTED),
                );
            });
            return;
        }
        // A tile row holds the photos from its start to the row's or the
        // section's end, whichever comes first.
        let ends: Vec<usize> = self.board.sections.iter().map(|s| s.range.end).collect();
        let holds = |row: &Row, position: usize| match row {
            Row::Tiles(index, start) => {
                (*start..(start + columns).min(ends[*index])).contains(&position)
            }
            Row::Header(_) => false,
        };
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("organize_board", self.group_mode))
            .auto_shrink([false, false]);
        if ui
            .ctx()
            .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("organize-board-top")))
            .is_some()
        {
            scroll = scroll.vertical_scroll_offset(0.0);
        } else if std::mem::take(&mut self.library_reveal)
            && let Some(position) = self
                .active
                .and_then(|id| self.index.get(&id))
                .and_then(|index| self.filtered.iter().position(|i| i == index))
            && let Some(row) = rows.iter().position(|row| holds(row, position))
        {
            let offset = egui::scroll_area::State::load(
                ui.ctx(),
                ui.make_persistent_id(egui::IdSalt::new(("organize_board", self.group_mode))),
            )
            .map_or(0.0, |state| state.offset.y);
            let top = tops[row];
            if top < offset {
                scroll = scroll.vertical_scroll_offset((top - HEADER).max(0.0));
            } else if top + tile_height > offset + ui.available_height() {
                scroll = scroll
                    .vertical_scroll_offset((top + tile_height - ui.available_height()).max(0.0));
            }
        }
        let editable = self.board_editable();
        let root = self.board_root_name();
        let dim = matches!(pending, Some(Pending::Running | Pending::Waiting));
        let mut drop: Option<(Vec<MediaId>, String)> = None;
        let mut select_section: Option<usize> = None;
        let mut faceless = false;
        let active_position = self
            .active
            .and_then(|id| self.index.get(&id))
            .and_then(|index| self.filtered.iter().position(|i| i == index));
        let selecting = self.library_selection_mode() && self.selection_ready();
        let mut marquee: Option<(egui::Rect, Vec<MediaId>)> = None;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            let origin = ui.max_rect().min;
            ui.set_min_size(egui::vec2(width, total));
            // Registered before the tiles, so a press on a tile still picks
            // or drags that photo; only empty space starts a rectangle.
            let background = ui.interact(
                egui::Rect::from_min_size(origin + viewport.min.to_vec2(), viewport.size()),
                egui::Id::new("organize-board-marquee"),
                if selecting {
                    Sense::drag()
                } else {
                    Sense::hover()
                },
            );
            let board_pos = |pos: egui::Pos2| pos - origin.to_vec2();
            if background.drag_started()
                && let Some(start) = ui.input(|i| i.pointer.press_origin())
            {
                let keep = ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);
                let base = if keep {
                    self.selection.ordered(&self.media, &self.filtered)
                } else {
                    Vec::new()
                };
                self.board.marquee = Some((board_pos(start), base));
            }
            if let Some((start, base)) = &self.board.marquee
                && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
            {
                let area = egui::Rect::from_two_pos(*start, board_pos(pointer));
                // Every tile the rectangle touches, drawn or not.
                let mut hit = Vec::new();
                for (row, top) in rows.iter().zip(&tops) {
                    let Row::Tiles(index, first) = row else {
                        continue;
                    };
                    if *top > area.max.y || top + tile_height < area.min.y {
                        continue;
                    }
                    let end = self.board.sections[*index].range.end.min(first + columns);
                    for (column, position) in (*first..end).enumerate() {
                        let left = column as f32 * (cell + gap);
                        if left <= area.max.x && left + cell >= area.min.x {
                            hit.push(self.media[self.filtered[position]].id);
                        }
                    }
                }
                let mut ids = base.clone();
                ids.extend(hit.into_iter().filter(|id| !base.contains(id)));
                marquee = Some((area.translate(origin.to_vec2()), ids));
                // Near or past the top or bottom edge, the board scrolls.
                let edge = 24.0;
                let shown = background.rect;
                let speed = if pointer.y < shown.top() + edge {
                    (shown.top() + edge - pointer.y).min(80.0) * 0.5
                } else if pointer.y > shown.bottom() - edge {
                    -(pointer.y - shown.bottom() + edge).min(80.0) * 0.5
                } else {
                    0.0
                };
                if speed != 0.0 {
                    ui.scroll_with_delta(egui::vec2(0.0, speed));
                    ui.ctx().request_repaint();
                }
            }
            // The folder you are looking at: the active photo's while its
            // tile is on screen, otherwise the topmost folder shown.
            let section = |row: &Row| match row {
                Row::Header(index) | Row::Tiles(index, _) => *index,
            };
            let height = |row: &Row| match row {
                Row::Header(_) => HEADER,
                Row::Tiles(..) => tile_height,
            };
            let on_screen = |row: &Row, top: f32| {
                top + height(row) > viewport.min.y + 8.0 && top < viewport.max.y - 8.0
            };
            let looking = active_position
                .and_then(|position| {
                    rows.iter()
                        .zip(&tops)
                        .find(|(row, _)| holds(row, position))
                })
                .filter(|(row, top)| on_screen(row, **top))
                .or_else(|| rows.iter().zip(&tops).find(|(row, top)| on_screen(row, **top)))
                .map(|(row, _)| section(row));
            let first = tops.partition_point(|top| *top + pitch < viewport.min.y);
            for (row, top) in rows.iter().zip(&tops).skip(first) {
                if *top > viewport.max.y {
                    break;
                }
                match row {
                    Row::Header(index) => {
                        let section = &self.board.sections[*index];
                        let rect = egui::Rect::from_min_size(
                            origin + egui::vec2(0.0, *top),
                            egui::vec2(width, HEADER),
                        );
                        let response = ui.interact(
                            rect,
                            egui::Id::new(("organize-section", &section.path)),
                            Sense::click(),
                        );
                        let count = section.range.len();
                        let photos = count - section.videos;
                        let counts = match (photos, section.videos) {
                            _ if section.path.is_none() && pending.is_none() => self
                                .unplaced_phrase(photos, section.videos),
                            (p, 0) => format!("{p} {}", if p == 1 { "photo" } else { "photos" }),
                            (0, v) => format!("{v} {}", if v == 1 { "video" } else { "videos" }),
                            (p, v) => format!(
                                "{p} {} · {v} {}",
                                if p == 1 { "photo" } else { "photos" },
                                if v == 1 { "video" } else { "videos" }
                            ),
                        };
                        // Faces offers its other choice right where those
                        // photos are: one "No people" folder.
                        let offer = section.path.is_none()
                            && pending.is_none()
                            && editable
                            && self.group_mode == GroupMode::Faces
                            && self.preferences.faceless
                                == crate::core::preferences::Faceless::Stay
                            && photos > 0
                            // In a narrow board the toolbar's choice is enough.
                            && rect.width() >= 480.0;
                        // The text stops short of the offer at the right.
                        let reserve = if offer { 210.0 } else { 0.0 };
                        section_header(
                            ui,
                            egui::Rect::from_min_max(
                                rect.min,
                                egui::pos2(rect.right() - reserve, rect.bottom()),
                            ),
                            section.path.as_deref(),
                            &root,
                            &counts,
                            response.hovered(),
                            pending,
                        );
                        if let Some(path) = &section.path {
                            if editable
                                && response
                                    .dnd_hover_payload::<super::grouping::PhotoDrag>()
                                    .is_some()
                            {
                                ui.painter().rect_stroke(
                                    rect.shrink2(egui::vec2(0.0, 4.0)),
                                    R_CONTROL,
                                    Stroke::new(1.5, ACCENT),
                                    StrokeKind::Inside,
                                );
                            }
                            if editable
                                && let Some(payload) =
                                    response.dnd_release_payload::<super::grouping::PhotoDrag>()
                            {
                                drop = Some((payload.0.clone(), path.clone()));
                            }
                        }
                        if offer {
                            let label = format!(
                                "Put them in a “{}” folder",
                                crate::core::preferences::NO_PEOPLE
                            );
                            let size = egui::vec2(200.0, 24.0);
                            let at = egui::Rect::from_min_size(
                                egui::pos2(rect.right() - size.x, rect.bottom() - 16.0 - size.y / 2.0),
                                size,
                            );
                            if ui
                                .put(at, egui::Button::new(RichText::new(&label).small()))
                                .on_hover_text("Photos without a recognized person go to one folder. You can switch back in the toolbar.")
                                .clicked()
                            {
                                faceless = true;
                            }
                        }
                        let name = match section.path.as_deref() {
                            None if pending.is_some() => "Photos not grouped yet".to_owned(),
                            None => format!("Not moving · {counts}"),
                            Some("") => format!("Folder {root}"),
                            Some(path) => format!("Folder {path}"),
                        };
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name)
                        });
                        if response
                            .on_hover_text("Click to select every item in this folder")
                            .clicked()
                        {
                            select_section = Some(*index);
                        }
                    }
                    Row::Tiles(index, start) => {
                        let end = self.board.sections[*index].range.end.min(start + columns);
                        for (column, position) in (*start..end).enumerate() {
                            let rect = egui::Rect::from_min_size(
                                origin + egui::vec2(column as f32 * (cell + gap), *top),
                                egui::vec2(cell, tile_height),
                            );
                            let Some(&media) = self.filtered.get(position) else {
                                continue;
                            };
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .id_salt(("board", position))
                                    .max_rect(rect),
                                |ui| {
                                    // Photos wait, dimmed, for the grouping
                                    // that will place them.
                                    if dim {
                                        ui.multiply_opacity(0.45);
                                    }
                                    self.card(ui, media, cell, tile_height, true)
                                },
                            );
                        }
                    }
                }
            }
            if let Some((area, _)) = &marquee {
                let painter = ui.painter();
                painter.rect_filled(*area, 2.0, ACCENT.gamma_multiply(0.12));
                painter.rect_stroke(*area, 2.0, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
            }
            looking
        });
        self.library_offset = output.state.offset.y;
        if let Some((_, ids)) = marquee {
            self.selection.replace(ids.iter().copied());
            self.comparison.clear();
            if !ui.input(|i| i.pointer.any_down()) {
                self.board.marquee = None;
                // The preview follows: the active photo stays when selected.
                if let Some(&first) = ids.first()
                    && !self.active.is_some_and(|a| ids.contains(&a))
                {
                    self.set_active(first);
                }
            }
        } else if !ui.input(|i| i.pointer.any_down()) {
            self.board.marquee = None;
        }
        self.board_look_at(
            output
                .inner
                .map(|index| self.board.sections[index].path.clone()),
            output.state.offset.y,
        );
        if self.board.reveal_current {
            ui.ctx().request_repaint(); // The sidebar was drawn first.
        }
        if let Some(index) = select_section
            && self.library_selection_mode()
        {
            let range = self.board.sections[index].range.clone();
            let ids: Vec<MediaId> = self
                .filtered
                .get(range)
                .unwrap_or_default()
                .iter()
                .map(|i| self.media[*i].id)
                .collect();
            self.selection.replace(ids.iter().copied());
            if let Some(first) = ids.first() {
                self.set_active(*first);
            }
        }
        if let Some((ids, target)) = drop {
            self.board_move(&ids, &target);
        }
        if faceless {
            self.change_faceless(crate::core::preferences::Faceless::Folder);
        }
    }
}

/// One row of the folder tree: indent, icon, name and count.
fn folder_row(
    ui: &mut egui::Ui,
    title: &str,
    glyph: &str,
    depth: usize,
    count: usize,
    selected: bool,
    draggable: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), 26.0),
        if draggable {
            Sense::click_and_drag()
        } else {
            Sense::click()
        },
    );
    let name = format!("{title}  ·  {count}");
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &name)
    });
    let hover = ui
        .ctx()
        .animate_bool_responsive(response.id.with("hover"), response.hovered());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, R_CONTROL, SELECTED);
    } else if hover > 0.0 {
        painter.rect_filled(rect, R_CONTROL, CARD.gamma_multiply(hover));
    }
    let x = rect.left() + 8.0 + depth as f32 * 14.0;
    painter.text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(14.0),
        if selected { ACCENT } else { MUTED },
    );
    let count_galley = painter.layout_no_wrap(count.to_string(), FontId::proportional(10.5), MUTED);
    let count_x = rect.right() - 8.0 - count_galley.size().x;
    painter.galley(
        egui::pos2(count_x, rect.center().y - count_galley.size().y / 2.0),
        count_galley,
        MUTED,
    );
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_owned(),
        if selected {
            medium(12.0)
        } else {
            FontId::proportional(12.0)
        },
        INK,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((count_x - x - 30.0).max(10.0));
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(x + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        INK,
    );
    response
}

/// A folder's header above its photos: name, parent path and counts.
fn section_header(
    ui: &egui::Ui,
    rect: egui::Rect,
    path: Option<&str>,
    root: &str,
    counts: &str,
    hovered: bool,
    pending: Option<Pending>,
) {
    use egui_phosphor::regular as icon;
    let painter = ui.painter_at(rect);
    let y = rect.bottom() - 16.0;
    let (glyph, title, parent) = match path {
        None => (
            match pending {
                None => icon::PUSH_PIN,
                Some(Pending::Running) => icon::HOURGLASS_MEDIUM,
                Some(_) => icon::CIRCLE_DASHED,
            },
            unplaced_title(pending).to_owned(),
            String::new(),
        ),
        Some("") => (icon::HARD_DRIVES, root.to_owned(), String::new()),
        Some(path) => (
            icon::FOLDER_SIMPLE,
            leaf(path).to_owned(),
            path.rsplit_once('/')
                .map(|(parent, _)| format!("{root} / {}", parent.replace('/', " / ")))
                .unwrap_or_else(|| root.to_owned()),
        ),
    };
    let color = if hovered { INK } else { ACCENT };
    painter.text(
        egui::pos2(rect.left() + 2.0, y),
        Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(16.0),
        color,
    );
    let title = painter.layout_no_wrap(title, semibold(14.0), INK);
    let mut x = rect.left() + 26.0;
    let title_width = title.size().x;
    painter.galley(egui::pos2(x, y - title.size().y / 2.0), title, INK);
    x += title_width + 10.0;
    if !parent.is_empty() {
        let parent =
            painter.layout_no_wrap(format!("in {parent}"), FontId::proportional(11.5), FAINT);
        let width = parent.size().x;
        painter.galley(egui::pos2(x, y - parent.size().y / 2.0), parent, FAINT);
        x += width + 10.0;
    }
    let mut job = egui::text::LayoutJob::simple_singleline(
        counts.to_owned(),
        FontId::proportional(11.5),
        MUTED,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - x).max(10.0));
    let counts = painter.layout_job(job);
    painter.galley(egui::pos2(x, y - counts.size().y / 2.0), counts, MUTED);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn organize() -> (tempfile::TempDir, PhotoSortApp) {
        let (temp, mut app) = super::super::compact_cull_tests::fixture();
        app.step = Step::Organize;
        app.review = false;
        app.output = app.root.as_ref().unwrap().display().to_string();
        app.filter_dirty = true;
        app.refilter();
        (temp, app)
    }

    /// Makes a folder as the sidebar does, then names it.
    fn new_folder(app: &mut PhotoSortApp, parent: &str, name: &str) {
        app.board_new_folder(parent, &[]);
        app.board.editing.as_mut().unwrap().1 = name.into();
        app.board_finish_edit();
    }

    #[test]
    fn board_groups_items_by_destination_folder_in_view_order() {
        let (_temp, mut app) = organize();
        let paths: Vec<_> = app
            .board
            .sections
            .iter()
            .map(|s| s.path.clone().unwrap())
            .collect();
        assert_eq!(paths, vec!["A/Day1".to_owned(), "B".to_owned()]);
        assert_eq!(app.board.sections[0].range, 0..4);
        assert_eq!(app.board.sections[1].range, 4..9);
        // `filtered` follows the board, so arrows and the strip match it.
        for (position, &index) in app.filtered.iter().enumerate() {
            let folder = if position < 4 { "A/Day1" } else { "B" };
            assert!(app.media[index].path.parent().unwrap().ends_with(folder));
        }
        let folders: Vec<_> = app
            .board
            .folders
            .iter()
            .map(|f| (f.path.as_str(), f.direct, f.total))
            .collect();
        assert!(folders.contains(&("", 0, 9)));
        assert!(folders.contains(&("A", 0, 4)));
        assert!(folders.contains(&("A/Day1", 4, 4)));
        assert!(folders.contains(&("B", 5, 5)));
        app.runtime.shutdown();
    }

    #[test]
    fn returning_from_another_step_mid_frame_draws_the_current_board() {
        let (_temp, mut app) = organize();
        assert_eq!(app.board.sections[1].range, 4..9);
        // Pick Best leaves its two-photo comparison in `filtered`; the board
        // keeps the sections it arranged on the last Organize visit.
        app.step = Step::PickBest;
        app.filtered.truncate(2);
        app.filter_dirty = false;
        app.board.focus = Some("B".into());
        // The step tabs switch steps after this frame's refilter has run,
        // so Organize draws before `filtered` is rebuilt.
        app.set_step(Step::Organize);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let _ = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| app.organize_board(ui),
        );
        assert_eq!(app.filtered.len(), 9);
        assert_eq!(app.board.sections.last().unwrap().range.end, 9);
        app.runtime.shutdown();
    }

    #[test]
    fn moving_creating_and_renaming_folders_edit_the_shared_draft_only() {
        let (_temp, mut app) = organize();
        let moved: Vec<MediaId> = app.filtered[..2].iter().map(|i| app.media[*i].id).collect();
        app.board_move(&moved, "B");
        assert!(app.organize_drafts.contains_key(&GroupMode::Current));
        app.check_board();
        app.refilter();
        let b = app
            .board
            .sections
            .iter()
            .find(|s| s.path.as_deref() == Some("B"))
            .unwrap();
        assert_eq!(b.range.len(), 7);
        // Creating and renaming a folder in place.
        new_folder(&mut app, "", "Trip");
        app.board.editing = Some(("Trip".into(), "Holiday".into()));
        app.board_finish_edit();
        app.board_move(&moved, "Holiday");
        app.check_board();
        app.refilter();
        assert!(
            app.board
                .sections
                .iter()
                .any(|s| s.path.as_deref() == Some("Holiday") && s.range.len() == 2)
        );
        // Nothing has moved on disk or been staged as a decision.
        assert!(app.decisions.is_empty());
        assert!(app.media.iter().all(|m| m.path.exists()));
        app.runtime.shutdown();
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]
    }

    #[test]
    fn shift_arrows_compare_the_selection_in_the_preview_and_escape_returns() {
        use super::super::compact_cull_tests::{frame, rendered};
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 900.0);
        app.select_library(ids[0], false, false);
        assert_eq!(app.comparison_ids(), vec![ids[0]]);
        for _ in 0..2 {
            frame(
                &mut app,
                &ctx,
                size,
                key(egui::Key::ArrowRight, egui::Modifiers::SHIFT),
            );
        }
        assert_eq!(app.selection.len(), 3);
        assert_eq!(app.comparison_ids(), ids[..3].to_vec());
        assert_eq!(app.active, Some(ids[2]));
        assert_eq!(app.focused_slot, 2);
        let nodes = rendered(&mut app, &ctx, size);
        assert!(nodes.iter().any(|(label, _)| label == "End comparison"));
        // Escape keeps the active photo and shows it alone.
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::Escape, Default::default()),
        );
        assert_eq!(app.comparison_ids(), vec![ids[2]]);
        assert_eq!(
            app.selection.ordered(&app.media, &app.filtered),
            vec![ids[2]]
        );
        assert_eq!(app.focused_slot, 0);
        let nodes = rendered(&mut app, &ctx, size);
        assert!(!nodes.iter().any(|(label, _)| label == "End comparison"));
        // A second Escape clears the selection as before.
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::Escape, Default::default()),
        );
        assert!(app.selection.is_empty() && app.active.is_none());
        app.runtime.shutdown();
    }

    #[test]
    fn the_comparison_follows_board_order_and_stops_past_nine() {
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        // Clicked out of order, compared in the order the board shows them.
        app.select_library(ids[6], false, false);
        app.select_library(ids[1], true, false);
        app.select_library(ids[4], true, false);
        assert_eq!(app.comparison_ids(), vec![ids[1], ids[4], ids[6]]);
        assert_eq!(app.active, Some(ids[4]));
        assert_eq!(app.focused_slot, 1);
        app.selection.all(&app.media, &app.filtered);
        assert_eq!(app.comparison_ids(), ids);
        // A tenth selected item (one this view does not show) ends the
        // comparison; the preview shows the active photo.
        let mut selected = ids.clone();
        selected.push(MediaId(u64::MAX));
        app.selection.replace(selected);
        assert_eq!(app.comparison_ids(), vec![ids[4]]);
        // Other steps keep their own comparisons.
        app.select_library(ids[0], false, false);
        app.select_library(ids[1], true, false);
        app.step = Step::Cull;
        assert_eq!(app.comparison_ids(), vec![ids[1]]);
        app.runtime.shutdown();
    }

    #[test]
    fn generated_groupings_hide_emptied_folders_but_keep_created_ones() {
        let (_temp, mut app) = organize();
        // Making a folder creates the draft, which knows every library folder.
        new_folder(&mut app, "", "Trip");
        app.check_board();
        app.refilter();
        let paths = |app: &PhotoSortApp| -> Vec<String> {
            app.board.folders.iter().map(|f| f.path.clone()).collect()
        };
        // Current folders lists them all, empty or not.
        assert_eq!(paths(&app), ["", "A", "A/Day1", "B", "Empty", "Trip"]);
        app.group_mode = GroupMode::Camera;
        new_folder(&mut app, "", "Picks");
        new_folder(&mut app, "Picks", "Best");
        app.check_board();
        app.refilter();
        assert_eq!(
            paths(&app),
            ["", "Picks", "Picks/Best", "Unknown camera"],
            "Camera lists the folders it fills and the ones made on the board"
        );
        // A folder emptied by a move disappears unless it was made by hand.
        let ids: Vec<MediaId> = app.media.iter().map(|m| m.id).collect();
        app.board_move(&ids, "Picks/Best");
        app.check_board();
        app.refilter();
        assert_eq!(paths(&app), ["", "Picks", "Picks/Best"]);
        app.runtime.shutdown();
    }

    fn labels(app: &mut PhotoSortApp, width: f32) -> Vec<(String, egui::Rect)> {
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        super::super::compact_cull_tests::rendered(app, &ctx, egui::vec2(width, 900.0))
    }
    fn shows(nodes: &[(String, egui::Rect)], text: &str) -> bool {
        nodes.iter().any(|(label, _)| label.contains(text))
    }

    #[test]
    fn a_running_grouping_shows_progress_instead_of_a_result() {
        let (_temp, mut app) = organize();
        app.group_mode = GroupMode::Visual;
        app.visual_groups = None;
        app.filter_dirty = true;
        app.refilter();
        app.analysis_run = Some(7);
        app.organize_run = Some((7, GroupMode::Visual));
        app.progress = Some((27, 502));
        assert_eq!(app.organize_pending(), Some(Pending::Running));
        assert_eq!(unplaced_title(app.organize_pending()), "Grouping…");
        assert_eq!(app.board_counts(0, 9, true), "9 items · grouping…");
        assert_eq!(
            app.board_note().as_deref(),
            Some("Folders appear when grouping finishes")
        );
        let nodes = labels(&mut app, 1440.0);
        assert!(shows(&nodes, "Grouping by visual similarity"));
        assert!(shows(&nodes, "27 of 502 photos"));
        assert!(nodes.iter().any(|(label, _)| label == "Cancel"));
        assert!(shows(&nodes, "Photos not grouped yet"));
        let sentence = app.unplaced_sentence();
        for claim in ["Not moving", "9 not moving", sentence.as_str()] {
            assert!(!shows(&nodes, claim), "{claim} shown while grouping");
        }
        // Model preparation is named while it runs.
        #[cfg(feature = "onnx")]
        {
            app.runtime
                .processing
                .preparing("dinov2", crate::core::processing::Backend::CoreMl);
            let nodes = labels(&mut app, 1440.0);
            assert!(shows(&nodes, "Preparing AI models · first run only"));
            app.runtime.processing.report(
                "dinov2",
                crate::core::processing::Backend::Cpu,
                "CPU fallback".into(),
            );
        }
        // Cancelled: says so and offers to group again; still no result.
        app.cancel_analysis();
        assert_eq!(app.organize_pending(), Some(Pending::Cancelled));
        // Cancelling another step's analysis later does not change that.
        app.analysis_run = Some(9);
        app.delete_run = Some(9);
        app.cancel_analysis();
        assert_eq!(app.organize_pending(), Some(Pending::Cancelled));
        let nodes = labels(&mut app, 1440.0);
        assert!(shows(&nodes, "Grouping cancelled"));
        assert!(nodes.iter().any(|(label, _)| label == "Group again"));
        assert!(!shows(&nodes, "27 of 502"));
        assert!(!shows(&nodes, &app.unplaced_sentence()));
        assert_eq!(
            app.board_note().as_deref(),
            Some("Grouping cancelled · no folders proposed")
        );
        // A failed run reads differently from a cancelled one.
        app.analysis_run = Some(8);
        app.organize_run = Some((8, GroupMode::Visual));
        app.halt_organize_run(); // As a failed run does.
        app.analysis_run = None;
        assert_eq!(app.organize_pending(), Some(Pending::Failed));
        assert!(
            app.organize_notice_text()
                .is_some_and(|(_, title, _, _)| title == "Grouping stopped")
        );
        // Finished without any group: now staying in place is the result.
        app.visual_groups = Some(HashMap::new());
        app.check_board();
        app.refilter();
        assert_eq!(app.organize_pending(), None);
        let nodes = labels(&mut app, 1440.0);
        assert!(shows(&nodes, "Not moving · "));
        let sentence = app.unplaced_sentence();
        assert!(
            sentence.ends_with("stay in their current folders"),
            "{sentence}"
        );
        assert!(shows(&nodes, &sentence));
        assert!(!shows(&nodes, "Grouping by visual similarity"));
        app.runtime.shutdown();
    }

    #[test]
    fn another_steps_analysis_shows_its_own_card_and_the_grouping_waits() {
        let (_temp, mut app) = organize();
        app.group_mode = GroupMode::Faces;
        app.analysis_run = Some(3);
        app.delete_run = Some(3);
        app.progress = Some((4, 9));
        assert_eq!(app.organize_pending(), Some(Pending::Waiting));
        let nodes = labels(&mut app, 1440.0);
        assert!(shows(&nodes, "Looking for photos to clean up"));
        assert!(shows(&nodes, "4 of 9 photos"));
        assert!(shows(&nodes, "Grouping is next"));
        assert!(!shows(&nodes, &app.unplaced_sentence()));
        // Cull keeps its grid and gets the same card above it.
        app.delete_run = None;
        app.cull_group_run = Some(3);
        app.step = Step::Cull;
        app.filter_dirty = true;
        app.refilter();
        let nodes = labels(&mut app, 1440.0);
        assert!(shows(&nodes, "Analyzing the library"));
        assert!(shows(&nodes, "4 of 9 photos"));
        app.runtime.shutdown();
    }

    #[test]
    fn board_counts_never_run_under_the_thumbnail_slider() {
        let (_temp, mut app) = organize();
        app.group_mode = GroupMode::Visual;
        app.visual_groups = Some(HashMap::new());
        app.filter_dirty = true;
        app.refilter();
        let summary = app.board_counts(0, 9, true);
        assert_eq!(summary, "0 folders · 9 items · 9 not moving");
        for width in [900.0, 1000.0, 1100.0, 1200.0, 1440.0, 1800.0] {
            let nodes = labels(&mut app, width);
            let rect = |name: &str| {
                nodes
                    .iter()
                    .find(|(label, _)| label == name)
                    .unwrap_or_else(|| panic!("{name} missing at {width}"))
                    .1
            };
            let (counts, slider) = (rect(&summary), rect("Thumbnail size"));
            assert!(
                counts.right() <= slider.left(),
                "counts {counts:?} overlap the slider {slider:?} at {width}"
            );
        }
        app.runtime.shutdown();
    }

    #[test]
    fn the_library_folder_is_one_row_that_shows_every_item() {
        let (_temp, mut app) = organize();
        let root = app.board_root_name();
        let nodes = labels(&mut app, 1400.0);
        let row = format!("{root}  ·  9");
        assert_eq!(nodes.iter().filter(|(label, _)| *label == row).count(), 1);
        assert!(!shows(&nodes, "All folders"));
        // Choosing it after a folder shows every item again.
        app.board_select_folder(Some("B".into()));
        app.board_select_folder(None);
        assert!(app.board.focus.is_none());
        // Dropping onto it still moves photos to the library folder.
        let id = app.media[app.filtered[0]].id;
        app.board_move(&[id], "");
        assert_eq!(
            app.organize_drafts[&GroupMode::Current].labels().get(&id),
            Some(&String::new())
        );
        app.runtime.shutdown();
    }

    #[test]
    fn details_say_where_a_photo_goes_on_disk_not_just_its_folder_name() {
        let (temp, mut app) = organize();
        let media = app.media[app.filtered[0]].clone();
        let placement = app.placement(&media);
        assert!(!placement.moves, "Current folders keep it where it is");
        assert_eq!(placement.folder, "photos / A / Day1");
        // Organized onto another drive, the same folder name still moves.
        let other = temp.path().join("SSD");
        app.output = other.display().to_string();
        let placement = app.placement(&media);
        assert!(placement.moves);
        assert_eq!(
            placement.folder,
            super::super::inspector::readable_path(&other.join("A").join("Day1"))
        );
        app.runtime.shutdown();
    }

    #[test]
    fn the_sidebar_highlights_the_folder_you_are_looking_at() {
        use super::super::compact_cull_tests::{click_label, frame, rendered};
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 900.0);
        rendered(&mut app, &ctx, size);
        // Every folder is shown; the photo you pick (scrolled into view, as a
        // click or the arrow keys do) names its folder.
        app.select_library(ids[5], false, false);
        app.library_reveal = true;
        rendered(&mut app, &ctx, size);
        assert!(app.board.focus.is_none());
        assert_eq!(app.board_highlight().as_deref(), Some("B"));
        app.select_library(ids[1], false, false);
        app.library_reveal = true;
        rendered(&mut app, &ctx, size);
        assert_eq!(app.board_highlight().as_deref(), Some("A/Day1"));
        // A folder chosen in the sidebar stays highlighted, though the photos
        // shown belong to its subfolder ...
        click_label(&mut app, &ctx, size, "A  ·  4");
        rendered(&mut app, &ctx, size);
        assert_eq!(app.board.focus.as_deref(), Some("A"));
        assert_eq!(app.board_highlight().as_deref(), Some("A"));
        // ... until you move to another photo.
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::ArrowDown, Default::default()),
        );
        rendered(&mut app, &ctx, size);
        assert_eq!(app.board_highlight().as_deref(), Some("A/Day1"));
        app.runtime.shutdown();
    }

    #[test]
    fn arrow_keys_walk_a_folders_photos_then_continue_into_the_next_folder() {
        use super::super::compact_cull_tests::{click_label, frame};
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 900.0);
        let press = |app: &mut PhotoSortApp, pressed: egui::Key| {
            frame(app, &ctx, size, key(pressed, Default::default()));
        };
        // Clicking a folder starts at its first photo; arrows stay in its photos.
        click_label(&mut app, &ctx, size, "Day1  ·  4");
        assert_eq!(app.board.focus.as_deref(), Some("A/Day1"));
        assert_eq!(app.active, Some(ids[0]));
        for &id in &ids[1..4] {
            press(&mut app, egui::Key::ArrowDown);
            assert_eq!(app.active, Some(id));
            assert_eq!(app.board.focus.as_deref(), Some("A/Day1"));
        }
        // Past the last photo: the next folder's first, and the sidebar follows.
        press(&mut app, egui::Key::ArrowDown);
        assert_eq!(app.active, Some(ids[4]));
        assert_eq!(app.board.focus.as_deref(), Some("B"));
        assert_eq!(
            app.selection.ordered(&app.media, &app.filtered),
            vec![ids[4]]
        );
        // And back.
        press(&mut app, egui::Key::ArrowUp);
        assert_eq!(app.active, Some(ids[3]));
        assert_eq!(app.board.focus.as_deref(), Some("A/Day1"));
        // The end of the last folder stays put.
        for _ in 0..8 {
            press(&mut app, egui::Key::ArrowDown);
        }
        assert_eq!(app.active, Some(ids[8]));
        assert_eq!(app.board.focus.as_deref(), Some("B"));
        // The library row walks every photo across folders without
        // narrowing the view.
        let root = format!("{}  ·  9", app.board_root_name());
        click_label(&mut app, &ctx, size, &root);
        assert!(app.board.focus.is_none());
        assert_eq!(app.active, Some(ids[8]));
        for &id in ids[2..8].iter().rev() {
            press(&mut app, egui::Key::ArrowUp);
            assert_eq!(app.active, Some(id));
            assert!(app.board.focus.is_none());
        }
        app.runtime.shutdown();
    }

    /// A right-click on the labelled node.
    fn context_click(app: &mut PhotoSortApp, ctx: &egui::Context, size: Vec2, label: &str) {
        use super::super::compact_cull_tests::{frame, rendered};
        let nodes = rendered(app, ctx, size);
        let pos = nodes
            .iter()
            .find(|(s, _)| s == label)
            .unwrap_or_else(|| panic!("Missing {label}"))
            .1
            .center();
        for pressed in [true, false] {
            frame(
                app,
                ctx,
                size,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Secondary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }

    #[test]
    fn new_folder_in_a_long_tree_is_made_in_the_plan_and_renamed_in_place() {
        use super::super::compact_cull_tests::{click_label, frame, rendered};
        let (_temp, mut app) = organize();
        // Enough folders that a new one lands far below the visible rows.
        for n in 1..=30 {
            new_folder(&mut app, "", &format!("M{n:02}"));
        }
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 420.0);
        let root = format!("{}  ·  9", app.board_root_name());
        context_click(&mut app, &ctx, size, &root);
        click_label(&mut app, &ctx, size, "New folder inside");
        let draft = &app.organize_drafts[&GroupMode::Current];
        assert!(draft.folders().contains("New folder"), "made in the plan");
        assert!(app.media.iter().all(|m| m.path.exists()));
        assert!(!app.root.as_ref().unwrap().join("New folder").exists());
        // Its name field is on screen, focused, with the name selected.
        let nodes = rendered(&mut app, &ctx, size);
        let field = nodes
            .iter()
            .find(|(label, _)| label == "New folder")
            .expect("the new folder's name field is shown");
        assert!(field.1.max.y <= size.y, "scrolled into view: {:?}", field.1);
        frame(&mut app, &ctx, size, vec![egui::Event::Text("Trip".into())]);
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::Enter, Default::default()),
        );
        rendered(&mut app, &ctx, size);
        let draft = &app.organize_drafts[&GroupMode::Current];
        assert!(draft.folders().contains("Trip"));
        assert!(!draft.folders().contains("New folder"));
        assert!(app.board.editing.is_none());
        // A second one inside a folder takes the next free name.
        app.board_new_folder("Trip", &[]);
        app.board_new_folder("Trip", &[]);
        let folders = app.organize_drafts[&GroupMode::Current].folders();
        assert!(folders.contains("Trip/New folder") && folders.contains("Trip/New folder 2"));
        app.runtime.shutdown();
    }

    #[test]
    fn show_in_finder_is_offered_only_for_folders_already_on_disk() {
        let (_temp, mut app) = organize();
        new_folder(&mut app, "", "Trip");
        app.check_board();
        app.refilter();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 900.0);
        let reveal = crate::core::desktop::Action::Reveal.label();
        context_click(&mut app, &ctx, size, "B  ·  5");
        let nodes = super::super::compact_cull_tests::rendered(&mut app, &ctx, size);
        assert!(shows(&nodes, reveal), "B exists on disk");
        assert!(shows(&nodes, "Move folder to Trash…"));
        assert_eq!(
            app.board_folder_on_disk("B"),
            Some(app.root.as_ref().unwrap().join("B"))
        );
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        context_click(&mut app, &ctx, size, "Trip  ·  0");
        let nodes = super::super::compact_cull_tests::rendered(&mut app, &ctx, size);
        assert!(!shows(&nodes, reveal), "Trip is only in the plan");
        assert!(shows(&nodes, "Remove folder"));
        assert!(!shows(&nodes, "Move folder to Trash"));
        assert_eq!(
            app.board_library_folder("B").as_deref(),
            app.catalog
                .as_ref()
                .unwrap()
                .directories
                .iter()
                .find(|d| d.ends_with("B"))
                .map(|d| d.as_path())
        );
        assert!(app.board_library_folder("").is_none());
        // A renamed folder is still found where it is on disk.
        app.board.editing = Some(("B".into(), "Beach".into()));
        app.board_finish_edit();
        assert_eq!(
            app.board_folder_on_disk("Beach"),
            Some(app.root.as_ref().unwrap().join("B"))
        );
        app.runtime.shutdown();
    }

    #[test]
    fn a_trackpad_pinch_zooms_the_preview_around_the_pointer() {
        use super::super::compact_cull_tests::{frame, rendered};
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        app.select_library(ids[0], false, false);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 900.0);
        let nodes = rendered(&mut app, &ctx, size);
        let canvas = nodes
            .iter()
            .find(|(label, _)| label.starts_with("Photo 1:"))
            .expect("the preview pane shows the photo")
            .1;
        let pointer = canvas.center() + egui::vec2(60.0, 40.0);
        let before = app.view_for(0).zoom;
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(1.5)],
        );
        let view = app.view_for(0);
        assert!((view.zoom - before * 1.5).abs() < 1e-3, "{view:?}");
        // The point under the fingers stays put: the image moved away from it.
        assert!(view.pan[0] < 0.0 && view.pan[1] < 0.0, "{view:?}");
        // Pinching in again goes back to fit, centred.
        frame(
            &mut app,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(pointer),
                egui::Event::Zoom(1.0 / 1.5),
            ],
        );
        let view = app.view_for(0);
        assert!((view.zoom - before).abs() < 1e-3 && view.pan == [0.0, 0.0]);
        // A pinch over the board does not zoom the preview.
        frame(
            &mut app,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(egui::pos2(500.0, 400.0)),
                egui::Event::Zoom(1.5),
            ],
        );
        assert!((app.view_for(0).zoom - before).abs() < 1e-3);
        app.runtime.shutdown();
    }

    #[test]
    fn holding_show_original_or_backslash_shows_the_folders_on_disk_until_release() {
        use super::super::compact_cull_tests::{frame, rendered};
        let (_temp, mut app) = organize();
        app.group_mode = GroupMode::Camera;
        app.filter_dirty = true;
        app.refilter();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 900.0);
        let sections = |app: &PhotoSortApp| -> Vec<Option<String>> {
            app.board.sections.iter().map(|s| s.path.clone()).collect()
        };
        let plan = vec![Some("Unknown camera".to_owned())];
        let disk = vec![Some("A/Day1".to_owned()), Some("B".to_owned())];
        rendered(&mut app, &ctx, size);
        assert_eq!(sections(&app), plan);
        let backslash = |pressed| egui::Event::Key {
            key: egui::Key::Backslash,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        };
        frame(&mut app, &ctx, size, vec![backslash(true)]);
        let nodes = rendered(&mut app, &ctx, size);
        assert!(app.board.original);
        assert_eq!(sections(&app), disk);
        assert!(
            shows(&nodes, "Empty  ·  0"),
            "empty folders on disk are listed"
        );
        assert!(!app.board_editable() && !app.photo_drag_enabled());
        // Edits wait for the plan to come back.
        let id = app.media[app.filtered[0]].id;
        app.board_move(&[id], "B");
        assert!(!app.organize_drafts.contains_key(&GroupMode::Camera));
        frame(&mut app, &ctx, size, vec![backslash(false)]);
        rendered(&mut app, &ctx, size);
        assert!(!app.board.original);
        assert_eq!(sections(&app), plan);
        // The button works the same way: down shows, up returns.
        let nodes = rendered(&mut app, &ctx, size);
        let pos = nodes
            .iter()
            .find(|(label, _)| label == "Show original")
            .expect("the Show original button")
            .1
            .center();
        let button = |pressed| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ]
        };
        frame(&mut app, &ctx, size, button(true));
        rendered(&mut app, &ctx, size);
        assert_eq!(sections(&app), disk);
        frame(&mut app, &ctx, size, button(false));
        rendered(&mut app, &ctx, size);
        assert_eq!(sections(&app), plan);
        // A folder chosen in the sidebar is shown again afterwards.
        app.board.focus = Some("Unknown camera".into());
        frame(&mut app, &ctx, size, vec![backslash(true)]);
        assert!(app.board.focus.is_none());
        frame(&mut app, &ctx, size, vec![backslash(false)]);
        assert_eq!(app.board.focus.as_deref(), Some("Unknown camera"));
        app.runtime.shutdown();
    }

    #[test]
    fn dragging_on_empty_space_selects_every_photo_the_rectangle_touches() {
        use super::super::compact_cull_tests::rendered;
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        let names: Vec<String> = app.filtered.iter().map(|i| app.media[*i].name()).collect();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 1100.0);
        let nodes = rendered(&mut app, &ctx, size);
        let tile = |name: &str| {
            nodes
                .iter()
                .find(|(label, _)| label == name)
                .unwrap_or_else(|| panic!("tile {name}"))
                .1
        };
        let tiles: Vec<egui::Rect> = names.iter().map(|n| tile(n)).collect();
        let step = |app: &mut PhotoSortApp, pos: egui::Pos2, press: Option<bool>, keep: bool| {
            let modifiers = if keep {
                egui::Modifiers::SHIFT
            } else {
                Default::default()
            };
            let mut events = vec![
                egui::Event::ModifiersChanged(modifiers),
                egui::Event::PointerMoved(pos),
            ];
            if let Some(pressed) = press {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers,
                });
            }
            ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |ui| app.update_workspace(ui, false),
            );
        };
        let drag = |app: &mut PhotoSortApp, from: egui::Pos2, to: egui::Pos2, keep: bool| {
            step(app, from, Some(true), keep);
            step(app, from.lerp(to, 0.5), None, keep);
            step(app, to, None, keep);
            step(app, to, Some(false), keep);
        };
        // From the empty slot after B's last tile (three columns: B fills
        // one row and two tiles of the next), up and left to the middle of
        // B's first tile: it and the tiles between are selected.
        assert_eq!(tiles[4].top(), tiles[6].top());
        assert_eq!(tiles[7].top(), tiles[8].top());
        let from = egui::pos2(tiles[6].center().x, tiles[8].center().y);
        let to = tiles[4].center();
        assert!(
            !tiles.iter().any(|t| t.contains(from)),
            "the drag starts on empty space"
        );
        drag(&mut app, from, to, false);
        let area = egui::Rect::from_two_pos(from, to);
        let expected: Vec<MediaId> = tiles
            .iter()
            .zip(&ids)
            .filter(|(t, _)| t.intersects(area))
            .map(|(_, id)| *id)
            .collect();
        assert!(expected.contains(&ids[4]) && expected.contains(&ids[8]));
        assert!(!expected.contains(&ids[0]));
        assert_eq!(app.selection.ordered(&app.media, &app.filtered), expected);
        assert!(app.active.is_some_and(|a| expected.contains(&a)));
        assert!(app.board.marquee.is_none(), "the rectangle ends on release");
        // Shift adds the next rectangle to the selection; this one starts
        // on A's header, which only takes clicks.
        let first = tiles[0];
        drag(
            &mut app,
            egui::pos2(first.left() + 10.0, first.top() - 8.0),
            first.center(),
            true,
        );
        let selected = app.selection.ordered(&app.media, &app.filtered);
        assert!(selected.contains(&ids[0]));
        assert!(expected.iter().all(|id| selected.contains(id)));
        // Without it, a new rectangle starts a new selection.
        drag(
            &mut app,
            egui::pos2(first.left() + 10.0, first.top() - 8.0),
            first.center(),
            false,
        );
        assert_eq!(
            app.selection.ordered(&app.media, &app.filtered),
            vec![ids[0]]
        );
        // Dragging a photo still drags the photo, not a rectangle.
        drag(&mut app, tiles[1].center(), tiles[2].center(), false);
        assert!(app.board.marquee.is_none());
        assert_eq!(
            app.selection.ordered(&app.media, &app.filtered),
            vec![ids[0]]
        );
        app.runtime.shutdown();
    }

    #[test]
    fn folders_are_moved_by_drag_and_wrapped_removed_and_marked_from_their_menu() {
        use super::super::compact_cull_tests::{frame, rendered};
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 900.0);
        let labels = |app: &PhotoSortApp, id: MediaId| {
            app.organize_drafts[&GroupMode::Current].labels()[&id].clone()
        };
        // Marking a folder marks everything in it; again unmarks.
        app.board_folder_action(FolderAction::Trash("B".into()));
        assert!(ids[4..].iter().all(|id| app.decision(*id).rejected));
        assert!(ids[..4].iter().all(|id| !app.decision(*id).rejected));
        app.board_folder_action(FolderAction::Trash("B".into()));
        assert!(ids.iter().all(|id| !app.decision(*id).rejected));
        // Drag Day1 onto B in the sidebar.
        let nodes = rendered(&mut app, &ctx, size);
        let at = |label: &str| {
            nodes
                .iter()
                .find(|(l, _)| l == label)
                .unwrap_or_else(|| panic!("{label}"))
                .1
                .center()
        };
        let (from, to) = (at("Day1  ·  4"), at("B  ·  5"));
        let pointer = |pos, pressed: Option<bool>| {
            let mut events = vec![egui::Event::PointerMoved(pos)];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                });
            }
            events
        };
        frame(&mut app, &ctx, size, pointer(from, Some(true)));
        for t in [0.2, 0.5, 0.8, 1.0] {
            frame(&mut app, &ctx, size, pointer(from.lerp(to, t), None));
        }
        frame(&mut app, &ctx, size, pointer(to, Some(false)));
        rendered(&mut app, &ctx, size);
        assert!(ids[..4].iter().all(|id| labels(&app, *id) == "B/Day1"));
        assert!(app.board.folders.iter().any(|f| f.path == "B/Day1"));
        // Back to the top level from the menu.
        app.board_folder_action(FolderAction::Nest("B/Day1".into(), String::new()));
        assert!(ids[..4].iter().all(|id| labels(&app, *id) == "Day1"));
        // Put B inside a new folder, named in place.
        app.board_folder_action(FolderAction::Wrap("B".into()));
        assert_eq!(
            app.board.editing.as_ref().map(|(p, _)| p.as_str()),
            Some("New folder")
        );
        assert!(
            ids[4..]
                .iter()
                .all(|id| labels(&app, *id) == "New folder/B")
        );
        // A folder holding an existing one cannot be removed ...
        app.board_folder_action(FolderAction::Remove("New folder".into()));
        assert!(
            app.organize_drafts[&GroupMode::Current]
                .folders()
                .contains("New folder")
        );
        assert!(app.navigation_notice.is_some());
        // ... but a new one can: its photos go back where they were going,
        // A/Day1, which now sits at the top level as Day1.
        app.check_board();
        app.refilter();
        app.board_new_folder("", &ids[..2]);
        let made = app.board.editing.clone().unwrap().0;
        assert_eq!(made, "New folder 2");
        assert!(ids[..2].iter().all(|id| labels(&app, *id) == made));
        app.board_folder_action(FolderAction::Remove(made.clone()));
        assert!(ids[..2].iter().all(|id| labels(&app, *id) == "Day1"));
        assert!(
            !app.organize_drafts[&GroupMode::Current]
                .folders()
                .contains(&made)
        );
        // Start over forgets every folder edit.
        app.board_folder_action(FolderAction::StartOver);
        assert!(!app.organize_drafts.contains_key(&GroupMode::Current));
        // None of it touched the disk.
        assert!(app.media.iter().all(|m| m.path.exists()));
        let root = app.root.clone().unwrap();
        assert!(!root.join("New folder").exists() && !root.join("Day1").exists());
        assert!(app.decisions.values().all(|d| d.is_empty()));
        app.runtime.shutdown();
    }

    #[test]
    fn f2_renames_the_current_photo_in_place_and_counts_as_a_change() {
        use super::super::compact_cull_tests::{frame, rendered};
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 900.0);
        app.select_library(ids[0], false, false);
        rendered(&mut app, &ctx, size);
        assert_eq!(app.board.changes, 0);
        assert_eq!(app.staged_change_count(), 0);
        frame(&mut app, &ctx, size, key(egui::Key::F2, Default::default()));
        let nodes = rendered(&mut app, &ctx, size);
        assert!(
            nodes.iter().any(|(label, _)| label == "photo-0"),
            "name field"
        );
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Text("Holiday".into())],
        );
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::Enter, Default::default()),
        );
        let nodes = rendered(&mut app, &ctx, size);
        assert_eq!(
            app.file_names.get(&ids[0]).map(String::as_str),
            Some("Holiday")
        );
        assert!(shows(&nodes, "Holiday.png"));
        assert_eq!(app.board.changes, 1);
        // Moving two photos adds them; the title bar counts it all.
        app.board_move(&ids[1..3], "B");
        rendered(&mut app, &ctx, size);
        assert_eq!(app.board.changes, 3);
        assert_eq!(app.staged_change_count(), 3);
        assert!(
            app.board_note()
                .is_some_and(|n| n.starts_with("3 planned changes")),
            "{:?}",
            app.board_note()
        );
        // Escape keeps the name it had.
        frame(&mut app, &ctx, size, key(egui::Key::F2, Default::default()));
        rendered(&mut app, &ctx, size);
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Text("Other".into())],
        );
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::Escape, Default::default()),
        );
        rendered(&mut app, &ctx, size);
        assert_eq!(
            app.file_names.get(&ids[0]).map(String::as_str),
            Some("Holiday")
        );
        assert!(app.board.renaming.is_none());
        assert!(app.media.iter().all(|m| m.path.exists()));
        app.runtime.shutdown();
    }
}
