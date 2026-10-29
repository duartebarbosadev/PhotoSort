//! Shared presentation of immutable operation plans. Rendering never rescans files.
use super::library::shimmer;
use super::*;
use crate::core::operations::Action;
use egui::text::{LayoutJob, TextFormat};
use egui_phosphor::regular as icon;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Default)]
pub(super) struct Rows {
    // One cumulative offset per operation group, not a second copy of file paths.
    ends: Vec<usize>,
    movable: Vec<usize>,
    generation: u64,
    summary: Summary,
    /// The plain view, as plan groups and staged media: never a copy of paths.
    folders: Vec<Folder>,
    trash: Vec<Item>,
    updates: Vec<Item>,
    renamed_folders: Vec<usize>,
    /// The plain view laid out for `columns` thumbnails per row, or
    /// `pair_columns` before-and-after pairs in a list with rotations.
    layout: Vec<Placed>,
    columns: usize,
    pair_columns: usize,
    width: f32,
}

/// One photo in the plain view: a plan group, or a choice staged for after
/// the moves of the same review (its Trash mark, rating or turns).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Item {
    Group(usize),
    Staged {
        id: MediaId,
        rating: Option<u8>,
        turns: u8,
    },
}

/// A destination folder and the photos going into it.
struct Folder {
    path: PathBuf,
    new: bool,
    /// Its photos by the folder they come from, in folder order. A photo
    /// renamed where it is comes "from" this folder itself.
    groups: Vec<(PathBuf, Vec<Item>)>,
    /// False when every photo here keeps its folder and only gets a new name.
    arrivals: bool,
}
impl Folder {
    fn photos(&self) -> usize {
        self.groups.iter().map(|(_, items)| items.len()).sum()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Section {
    Moving,
    Folders,
    Trash,
    Updates,
}
#[derive(Clone, Copy, Debug, PartialEq, Hash)]
enum List {
    /// The photos of `folders[i]` from its source folder `g`.
    Folder(usize, usize),
    Trash,
    Updates,
}
/// A piece of the plain view. Thumbnail rows are laid out per width, and
/// only the pieces in view are drawn, so any number of photos scrolls.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Block {
    Section(Section),
    Card(usize),
    /// "From 2025-10-24 / group-1 · 4 photos" over that folder's photos.
    SourceHead(usize, usize),
    Thumbs(List, usize),
    FolderRenamed(usize),
    Tidy,
}
#[derive(Clone, Copy, Debug)]
struct Placed {
    block: Block,
    top: f32,
    height: f32,
    card_top: bool,
    card_bottom: bool,
}

const SECTION_HEIGHT: f32 = 40.0;
const CARD_HEAD: f32 = 40.0;
const SOURCE_HEAD: f32 = 26.0;
const TILE: egui::Vec2 = egui::vec2(104.0, 78.0);
const CAPTION: f32 = 20.0;
const THUMB_ROW: f32 = TILE.y + CAPTION + S2;
/// The gap a "before → after" arrow sits in between a pair's two tiles.
const ARROW: f32 = 28.0;
/// A rotation reads as the photo as it is, an arrow, then the photo turned.
const PAIR: f32 = 2.0 * TILE.x + ARROW;
const TEXT_ROW: f32 = 30.0;
const TIDY_HEIGHT: f32 = 48.0;
/// Space above each card, part of its first piece so pieces stay touching.
const CARD_GAP: f32 = 8.0;

fn count(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}
fn photos(n: usize) -> String {
    count(n, "photo", "photos")
}
/// "a", "a and b", "a, b and c".
fn and_list(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Plain counts: photos, never file operations. The sentence, the section
/// headings and the Apply button all read these same numbers.
#[derive(Default)]
struct Summary {
    moved: usize,
    renamed: usize,
    destinations: usize,
    new_destinations: usize,
    empty_folders: usize,
    trashed: usize,
    trashed_folders: usize,
    updated: usize,
    rotated: usize,
    rated: usize,
    folder_renames: usize,
    /// Folders removed afterwards, only if nothing is left in them (plan groups).
    cleanup: Vec<usize>,
}
impl Summary {
    fn updates_text(&self, n: String) -> String {
        if self.rotated == self.updated && self.rated == 0 {
            format!("rotate {n}")
        } else if self.rated == self.updated && self.rotated == 0 {
            format!("rate {n}")
        } else {
            format!("change the rating or rotation of {n}")
        }
    }
    /// The button that applies the plan, named for what it does.
    fn apply_label(&self) -> String {
        let kinds = [
            self.moved,
            self.renamed,
            self.folder_renames,
            self.empty_folders,
            self.updated,
            self.trashed + self.trashed_folders,
        ];
        let capital = |text: String| {
            let mut chars = text.chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_uppercase().chain(chars).collect::<String>()
            })
        };
        match kinds.iter().filter(|n| **n > 0).count() {
            0 => "Apply".into(),
            1 if self.moved > 0 => format!("Move {}", photos(self.moved)),
            1 if self.renamed > 0 => format!("Rename {}", photos(self.renamed)),
            1 if self.updated > 0 => capital(self.updates_text(photos(self.updated))),
            1 if self.trashed > 0 && self.trashed_folders == 0 => {
                format!("Move {} to the Trash", photos(self.trashed))
            }
            2 if self.moved > 0 && self.trashed > 0 && self.trashed_folders == 0 => {
                format!("Move {} and trash {}", self.moved, self.trashed)
            }
            _ => "Apply all changes".into(),
        }
    }
}

impl Rows {
    /// Index `plan`, and the choices staged to apply right after its moves.
    fn replace(&mut self, plan: Option<&Plan>, staged: &[(MediaId, Decision)]) {
        self.generation = self.generation.wrapping_add(1);
        self.ends.clear();
        self.movable.clear();
        self.folders.clear();
        self.trash.clear();
        self.updates.clear();
        self.renamed_folders.clear();
        self.layout.clear();
        self.columns = 0;
        self.pair_columns = 0;
        self.width = 0.0;
        self.summary = Summary::default();
        let Some(plan) = plan else {
            return;
        };
        let summary = &mut self.summary;
        // Photos going to the Trash are shown there, not also as moved.
        let trashed: HashSet<MediaId> = staged
            .iter()
            .filter(|(_, d)| d.rejected)
            .map(|(id, _)| *id)
            .collect();
        // Destination folder -> (photos arrive, plan groups by source folder).
        type Sources<'a> = BTreeMap<&'a Path, Vec<Item>>;
        let mut into = BTreeMap::<&Path, (bool, Sources)>::new();
        let mut created = Vec::new();
        let mut end = 0;
        for (group, action) in plan.actions.iter().enumerate() {
            if matches!(action, Action::Move { files } if !files.is_empty()) {
                self.movable.push(group);
            }
            let pairs: Vec<(&Path, &Path)> = match action {
                Action::Move { files } | Action::FinishMove { files } => files
                    .iter()
                    .map(|(s, to)| (s.path.as_path(), to.as_path()))
                    .collect(),
                Action::RenameFile { source, to } => {
                    vec![(source.path.as_path(), to.as_path())]
                }
                _ => Vec::new(),
            };
            let sources: Vec<&Path> = pairs.iter().map(|(s, _)| *s).collect();
            let lead = pairs.get(lead_and_extras(&sources).0);
            if let Some((source, to)) = lead
                && let Some(folder) = to.parent()
                && !trashed.contains(&MediaId::for_path(source))
            {
                let arrives = pairs.iter().any(|(s, to)| s.parent() != to.parent());
                let entry = into.entry(folder).or_default();
                entry.0 |= arrives;
                let from = if arrives { source.parent() } else { None };
                entry
                    .1
                    .entry(from.unwrap_or(folder))
                    .or_default()
                    .push(Item::Group(group));
                if arrives {
                    summary.moved += 1;
                } else {
                    summary.renamed += 1;
                }
            }
            match action {
                Action::Trash { files } if !files.is_empty() => {
                    summary.trashed += 1;
                    self.trash.push(Item::Group(group));
                }
                Action::TrashDirectory(_) => {
                    summary.trashed_folders += 1;
                    self.trash.push(Item::Group(group));
                }
                Action::Metadata {
                    rating,
                    orientation,
                    ..
                } => {
                    summary.updated += 1;
                    summary.rated += usize::from(rating.is_some());
                    summary.rotated += usize::from(orientation.is_some());
                    self.updates.push(Item::Group(group));
                }
                Action::RenameDirectory(_) => {
                    summary.folder_renames += 1;
                    self.renamed_folders.push(group);
                }
                Action::CreateDirectory { path } => created.push(path.as_path()),
                Action::RemoveEmptyDirectory { .. } => summary.cleanup.push(group),
                _ => {}
            }
            end += match action {
                Action::Move { files } | Action::FinishMove { files } => files.len().max(1),
                Action::Trash { files } => files.len().max(1),
                Action::TrashDirectory(folder) => 1 + folder.inventory.files.len(),
                _ => 1,
            };
            self.ends.push(end);
        }
        for &(id, ref decision) in staged {
            let item = Item::Staged {
                id,
                rating: decision.rating,
                turns: decision.turns % 4,
            };
            if decision.rejected {
                summary.trashed += 1;
                self.trash.push(item);
            } else if decision.rating.is_some() || decision.turns % 4 != 0 {
                summary.updated += 1;
                summary.rated += usize::from(decision.rating.is_some());
                summary.rotated += usize::from(decision.turns % 4 != 0);
                self.updates.push(item);
            }
        }
        // A new folder that only holds other new folders is shown through them.
        let empty: Vec<&Path> = created
            .iter()
            .copied()
            .filter(|path| {
                !into.keys().any(|folder| folder.starts_with(path))
                    && !created.iter().any(|c| c != path && c.starts_with(path))
            })
            .collect();
        summary.empty_folders = empty.len();
        for path in empty {
            into.insert(path, (false, Sources::new()));
        }
        for (path, (arrivals, sources)) in into {
            let new = created.contains(&path);
            if arrivals {
                summary.destinations += 1;
                summary.new_destinations += usize::from(new);
            }
            self.folders.push(Folder {
                path: path.to_path_buf(),
                new,
                groups: sources
                    .into_iter()
                    .map(|(from, items)| (from.to_path_buf(), items))
                    .collect(),
                arrivals,
            });
        }
    }
    fn len(&self) -> usize {
        self.ends.last().copied().unwrap_or(0)
    }
    fn at(&self, row: usize) -> (usize, usize) {
        let group = self.ends.partition_point(|end| *end <= row);
        let start = group.checked_sub(1).map_or(0, |i| self.ends[i]);
        (group, row - start)
    }
    fn items(&self, list: List) -> &[Item] {
        match list {
            List::Trash => &self.trash,
            List::Updates => &self.updates,
            List::Folder(i, g) => &self.folders[i].groups[g].1,
        }
    }
    /// Lays the plain view out for `width`, once per plan and width.
    /// Returns the total height.
    fn lay_out(&mut self, width: f32) -> f32 {
        if self.width != width || self.layout.is_empty() {
            let fit = |slot: f32| (((width - 2.0 * S3 + S2) / (slot + S2)).floor() as usize).max(1);
            self.columns = fit(TILE.x);
            self.pair_columns = fit(PAIR);
            self.width = width;
            let pairs = self.summary.rotated > 0;
            let (columns, pair_columns) = (self.columns, self.pair_columns);
            let columns = |list: List| {
                if pairs && list == List::Updates {
                    pair_columns
                } else {
                    columns
                }
            };
            let mut layout = Vec::new();
            let mut top = 0.0;
            let mut place = |block, height, card_top, card_bottom| {
                layout.push(Placed {
                    block,
                    top,
                    height,
                    card_top,
                    card_bottom,
                });
                top += height;
            };
            let thumbs = |place: &mut dyn FnMut(Block, f32, bool, bool),
                          list: List,
                          len: usize,
                          first: bool,
                          last: bool| {
                let columns = columns(list);
                let rows = len.div_ceil(columns);
                for row in 0..rows {
                    let height = THUMB_ROW + if first && row == 0 { CARD_GAP } else { 0.0 };
                    place(
                        Block::Thumbs(list, row * columns),
                        height,
                        first && row == 0,
                        last && row + 1 == rows,
                    );
                }
            };
            let len = self.items(List::Trash).len();
            if len > 0 {
                place(Block::Section(Section::Trash), SECTION_HEIGHT, false, false);
                thumbs(&mut place, List::Trash, len, true, true);
            }
            if !self.folders.is_empty() {
                place(
                    Block::Section(Section::Moving),
                    SECTION_HEIGHT,
                    false,
                    false,
                );
                // One card per destination; inside, photos by source folder.
                for (i, folder) in self.folders.iter().enumerate() {
                    let groups = folder.groups.len();
                    place(Block::Card(i), CARD_HEAD + CARD_GAP, true, groups == 0);
                    for (g, (_, items)) in folder.groups.iter().enumerate() {
                        // One source is already named by the card's sentence.
                        if groups > 1 {
                            place(Block::SourceHead(i, g), SOURCE_HEAD, false, false);
                        }
                        let list = List::Folder(i, g);
                        thumbs(&mut place, list, items.len(), false, g + 1 == groups);
                    }
                }
            }
            if !self.renamed_folders.is_empty() {
                place(
                    Block::Section(Section::Folders),
                    SECTION_HEIGHT,
                    false,
                    false,
                );
                let last = self.renamed_folders.len() - 1;
                for (i, &group) in self.renamed_folders.iter().enumerate() {
                    let gap = if i == 0 { CARD_GAP } else { 0.0 };
                    place(
                        Block::FolderRenamed(group),
                        TEXT_ROW + gap,
                        i == 0,
                        i == last,
                    );
                }
            }
            let len = self.items(List::Updates).len();
            if len > 0 {
                place(
                    Block::Section(Section::Updates),
                    SECTION_HEIGHT,
                    false,
                    false,
                );
                thumbs(&mut place, List::Updates, len, true, true);
            }
            if !self.summary.cleanup.is_empty() {
                place(Block::Tidy, TIDY_HEIGHT, false, false);
            }
            self.layout = layout;
        }
        self.layout.last().map_or(0.0, |p| p.top + p.height)
    }
    /// How wide one photo's place in `list` is: a pair where rotations are.
    fn slot(&self, list: List) -> f32 {
        if self.summary.rotated > 0 && list == List::Updates {
            PAIR
        } else {
            TILE.x
        }
    }
}

/// A folder as a person reads it: `2025-10-25 / group-2 / h`, relative to
/// `base`, or the base folder's own name when it is the base.
fn folder_text(path: &Path, base: Option<&Path>) -> String {
    match base.and_then(|base| path.strip_prefix(base).ok()) {
        Some(relative) if relative.as_os_str().is_empty() => path.file_name().map_or_else(
            || path.display().to_string(),
            |n| n.to_string_lossy().into(),
        ),
        Some(relative) => relative
            .iter()
            .map(|c| c.to_string_lossy())
            .collect::<Vec<_>>()
            .join(" / "),
        None => path.display().to_string(),
    }
}
fn file_name(path: &Path) -> String {
    path.file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into()
}
/// Which of a photo's files a person thinks of as the photo, and a short
/// note for the rest: a RAW twin or XMP sidecar is "+ RAW, XMP".
fn lead_and_extras(paths: &[&Path]) -> (usize, String) {
    let kind = |p: &Path| MediaKind::from_path(p);
    let sidecar = |p: &Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xmp"));
    let lead = paths
        .iter()
        .position(|p| matches!(kind(p), Some(MediaKind::Photo | MediaKind::Video)))
        .or_else(|| paths.iter().position(|p| !sidecar(p)))
        .unwrap_or(0);
    let mut extras: Vec<String> = Vec::new();
    for (i, path) in paths.iter().enumerate() {
        let tag = if kind(path) == Some(MediaKind::Raw) {
            "RAW".to_string()
        } else {
            path.extension()
                .map_or_else(String::new, |e| e.to_string_lossy().to_uppercase())
        };
        if i != lead && !tag.is_empty() && !extras.contains(&tag) {
            extras.push(tag);
        }
    }
    let note = if extras.is_empty() {
        String::new()
    } else {
        format!("+ {}", extras.join(", "))
    };
    (lead, note)
}
/// A rating or rotation in everyday words.
fn update_text(rating: Option<u8>, orientation: Option<u8>, pixels: bool) -> String {
    let mut parts = Vec::new();
    if let Some(orientation) = orientation {
        parts.push(
            match orientation {
                1 => "Set upright",
                2 => "Mirror",
                3 => "Turn upside down",
                4 => "Flip",
                5 | 7 => "Flip and turn",
                6 => "Rotate right",
                8 => "Rotate left",
                _ => "Change orientation",
            }
            .to_string(),
        );
    }
    match rating {
        Some(0) => parts.push("Clear rating".into()),
        Some(n) => parts.push("★".repeat(n.into())),
        None => {}
    }
    let mut text = parts.join(" · ");
    if pixels {
        text.push_str(" (re-saves the image)");
    }
    text
}
/// The clockwise quarter turns from one orientation to another, when a
/// turn alone gets there (not a mirror).
fn turns_between(from: u8, to: u8) -> Option<u8> {
    (0..4).find(|&turns| crate::core::metadata::rotate_orientation(from, turns) == to)
}
/// The orientation that clockwise quarter `turns` give an upright photo.
fn turned(turns: u8) -> Option<u8> {
    match turns % 4 {
        1 => Some(6),
        2 => Some(3),
        3 => Some(8),
        _ => None,
    }
}

/// One thumbnail of the plain view: what it shows and says.
struct Thumb {
    id: MediaId,
    folder: bool,
    caption: String,
    name: String,
    hover: String,
    extras: String,
    /// Quarter turns shown beside the photo as it is now, "before → after".
    turns: u8,
}

/// The folders a plan removes afterwards if nothing is left in them, in one line.
fn cleanup_text(plan: &Plan, index: &Rows, root: Option<&Path>, limit: usize) -> String {
    let names: Vec<String> = index
        .summary
        .cleanup
        .iter()
        .filter_map(|&group| match &plan.actions[group] {
            Action::RemoveEmptyDirectory { path, .. } => Some(folder_text(path, root)),
            _ => None,
        })
        .collect();
    let shown = limit.min(names.len());
    let mut text = names
        .iter()
        .take(shown)
        .cloned()
        .collect::<Vec<_>>()
        .join(", ");
    if names.len() > shown {
        text.push_str(&format!(" and {} more", names.len() - shown));
    }
    text
}

/// A label that reads the same to assistive technology as it looks.
fn line_label(ui: &mut egui::Ui, text: RichText) -> egui::Response {
    ui.add(egui::Label::new(text).truncate().selectable(false))
}

impl PhotoSortApp {
    /// The sentence at the top of the review: what is about to happen.
    pub(super) fn review_sentence(&self, plan: &Plan, rows: &Rows) -> String {
        let summary = &rows.summary;
        let output = plan.output_root.as_deref().or(self.root.as_deref());
        let mut parts = Vec::new();
        if summary.moved > 0 {
            let mut destinations = rows
                .folders
                .iter()
                .filter(|f| f.arrivals && !f.groups.is_empty());
            let into = match (destinations.next(), destinations.next()) {
                (Some(folder), None) => {
                    let name = folder_text(&folder.path, Some(&folder.path));
                    if folder.new {
                        format!("a new folder \"{name}\"")
                    } else if Some(folder.path.as_path()) == output {
                        format!("\"{name}\"")
                    } else {
                        format!("the folder \"{name}\"")
                    }
                }
                _ if summary.new_destinations == summary.destinations => {
                    count(summary.destinations, "new folder", "new folders")
                }
                _ if summary.new_destinations > 0 => format!(
                    "{} ({} new)",
                    count(summary.destinations, "folder", "folders"),
                    summary.new_destinations
                ),
                _ => count(summary.destinations, "folder", "folders"),
            };
            parts.push(format!("move {} into {into}", photos(summary.moved)));
        }
        if summary.renamed > 0 {
            parts.push(format!("rename {}", photos(summary.renamed)));
        }
        if summary.folder_renames > 0 {
            parts.push(format!(
                "rename {}",
                count(summary.folder_renames, "folder", "folders")
            ));
        }
        if summary.empty_folders > 0 {
            parts.push(format!(
                "create {}",
                count(summary.empty_folders, "empty folder", "empty folders")
            ));
        }
        if summary.updated > 0 {
            parts.push(summary.updates_text(photos(summary.updated)));
        }
        let mut trash = Vec::new();
        if summary.trashed > 0 {
            trash.push(photos(summary.trashed));
        }
        if summary.trashed_folders > 0 {
            trash.push(count(summary.trashed_folders, "folder", "folders"));
        }
        if !trash.is_empty() {
            parts.push(format!("send {} to the Trash", and_list(&trash)));
        }
        if parts.is_empty() {
            return if summary.cleanup.is_empty() {
                "There is nothing to change.".into()
            } else {
                "You're about to tidy up empty folders.".into()
            };
        }
        format!("You're about to {}.", and_list(&parts))
    }

    fn thumb(&self, plan: &Plan, item: Item, list: List) -> Option<Thumb> {
        let root = self.root.as_deref();
        let blank = |id: MediaId, name: String| Thumb {
            id,
            folder: false,
            caption: name.clone(),
            hover: name.clone(),
            name,
            extras: String::new(),
            turns: 0,
        };
        match item {
            Item::Staged { id, rating, turns } => {
                let media = &self.media[*self.index.get(&id)?];
                let mut thumb = blank(id, file_name(&media.path));
                if let Some(from) = media.path.parent() {
                    thumb.hover = format!("{}\nin {}", thumb.name, folder_text(from, root));
                }
                if list == List::Updates {
                    thumb.caption = update_text(rating, turned(turns), false);
                    thumb.turns = turns;
                    thumb.hover = format!("{}: {}", thumb.name, thumb.caption);
                }
                Some(thumb)
            }
            Item::Group(group) => match &plan.actions[group] {
                Action::Move { files } | Action::FinishMove { files } if !files.is_empty() => {
                    let sources: Vec<&Path> = files.iter().map(|(s, _)| s.path.as_path()).collect();
                    let (lead, extras) = lead_and_extras(&sources);
                    let (source, to) = (&files[lead].0.path, &files[lead].1);
                    Some(self.moved_thumb(source, to, extras))
                }
                Action::RenameFile { source, to } => {
                    Some(self.moved_thumb(&source.path, to, String::new()))
                }
                Action::Trash { files } if !files.is_empty() => {
                    let sources: Vec<&Path> = files.iter().map(|s| s.path.as_path()).collect();
                    let (lead, extras) = lead_and_extras(&sources);
                    let path = sources[lead];
                    let mut thumb = blank(MediaId::for_path(path), file_name(path));
                    if let Some(from) = path.parent() {
                        thumb.hover = format!("{}\nin {}", thumb.name, folder_text(from, root));
                    }
                    thumb.extras = extras;
                    Some(thumb)
                }
                Action::TrashDirectory(folder) => {
                    let mut thumb = blank(
                        MediaId::for_path(&folder.path),
                        format!("Folder {}", file_name(&folder.path)),
                    );
                    thumb.folder = true;
                    thumb.hover = format!(
                        "{}, with everything in it ({})",
                        folder_text(&folder.path, root),
                        count(folder.inventory.files.len(), "file", "files")
                    );
                    Some(thumb)
                }
                Action::Metadata {
                    source,
                    rating,
                    orientation,
                    target,
                    ..
                } => {
                    let id = MediaId::for_path(&source.path);
                    let mut thumb = blank(id, file_name(&source.path));
                    // Said as a turn of the photo as it looks now, not as
                    // the orientation tag it ends with.
                    let from = self.metadata.get(&id).map(|m| m.orientation);
                    thumb.turns = from
                        .zip(*orientation)
                        .and_then(|(from, to)| turns_between(from, to))
                        .unwrap_or(0);
                    let orientation = if thumb.turns == 0 {
                        *orientation
                    } else {
                        turned(thumb.turns)
                    };
                    thumb.caption = update_text(
                        *rating,
                        orientation,
                        *target == crate::core::metadata::WriteTarget::Pixels,
                    );
                    thumb.hover = format!("{}: {}", thumb.name, thumb.caption);
                    Some(thumb)
                }
                _ => None,
            },
        }
    }
    fn moved_thumb(&self, source: &Path, to: &Path, extras: String) -> Thumb {
        let (before, after) = (file_name(source), file_name(to));
        let mut hover = if before == after {
            after.clone()
        } else {
            format!("{before} → {after}")
        };
        if source.parent() != to.parent()
            && let Some(from) = source.parent()
        {
            hover.push_str(&format!(
                "\nfrom {}",
                folder_text(from, self.root.as_deref())
            ));
        }
        Thumb {
            id: MediaId::for_path(source),
            folder: false,
            // A rename reads "old → new"; the caption fits the new name.
            name: hover.lines().next().unwrap_or_default().to_string(),
            caption: after,
            hover,
            extras,
            turns: 0,
        }
    }

    /// The plain view: sections of photo thumbnails by what happens to them.
    /// Only the pieces in view are drawn and only their thumbnails load.
    fn plain_view(
        &mut self,
        ui: &mut egui::Ui,
        plan: &Plan,
        rows: &mut Rows,
        height: f32,
    ) -> egui::scroll_area::ScrollAreaOutput<usize> {
        egui::ScrollArea::vertical()
            .id_salt(("plan_plain", rows.generation))
            .max_height(height)
            .auto_shrink([false, true])
            .show_viewport(ui, |ui, viewport| {
                let width = ui.available_width();
                let total = rows.lay_out(width);
                ui.set_height(total);
                let origin = ui.max_rect().min;
                let first = rows
                    .layout
                    .partition_point(|p| p.top + p.height < viewport.min.y);
                let mut rendered = 0;
                for i in first..rows.layout.len() {
                    let placed = rows.layout[i];
                    if placed.top > viewport.max.y {
                        break;
                    }
                    rendered += 1;
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(0.0, placed.top),
                        egui::vec2(width, placed.height),
                    );
                    ui.push_id(i, |ui| self.plain_block(ui, plan, rows, placed, rect));
                }
                rendered
            })
    }

    fn plain_block(
        &mut self,
        ui: &mut egui::Ui,
        plan: &Plan,
        rows: &Rows,
        placed: Placed,
        rect: egui::Rect,
    ) {
        let summary = &rows.summary;
        // Cards: one rounded panel across touching pieces.
        let in_card = !matches!(placed.block, Block::Section(_) | Block::Tidy);
        let mut card = rect;
        if placed.card_top {
            card.min.y += CARD_GAP;
        }
        if in_card {
            let r = R_TILE;
            let corners = egui::CornerRadius {
                nw: if placed.card_top { r } else { 0 },
                ne: if placed.card_top { r } else { 0 },
                sw: if placed.card_bottom { r } else { 0 },
                se: if placed.card_bottom { r } else { 0 },
            };
            ui.painter().rect_filled(card, corners, CARD);
        }
        let inner = card.shrink2(egui::vec2(S3, 0.0));
        match placed.block {
            Block::Section(section) => {
                let (glyph, title, color, note) = match section {
                    Section::Moving => (
                        icon::FOLDER_SIMPLE,
                        "Moving",
                        ACCENT,
                        photos(summary.moved + summary.renamed),
                    ),
                    Section::Folders => (
                        icon::PENCIL_SIMPLE,
                        "Renaming folders",
                        INK,
                        count(summary.folder_renames, "folder", "folders"),
                    ),
                    Section::Trash => (
                        icon::TRASH,
                        "Going to the Trash",
                        RED,
                        format!(
                            "{} · you can restore them from the Trash",
                            photos(summary.trashed)
                        ),
                    ),
                    Section::Updates => (
                        icon::STAR,
                        "Ratings and rotations",
                        INK,
                        photos(summary.updated),
                    ),
                };
                let row = egui::Rect::from_min_size(
                    inner.min + egui::vec2(-S3, S3),
                    egui::vec2(inner.width() + S3, SECTION_HEIGHT - S3),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(row)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.label(RichText::new(glyph).size(16.0).color(color));
                        ui.label(RichText::new(title).font(semibold(15.0)).color(color));
                        ui.add_space(S1);
                        line_label(ui, RichText::new(note).color(MUTED));
                    },
                );
            }
            Block::Card(i) => {
                let heading = egui::Rect::from_min_size(
                    inner.min + egui::vec2(0.0, S2),
                    egui::vec2(inner.width(), CARD_HEAD - S2),
                );
                self.card_heading(ui, plan, rows, i, heading);
            }
            Block::SourceHead(i, g) => {
                let folder = &rows.folders[i];
                let (from, items) = &folder.groups[g];
                let text = if *from == folder.path {
                    format!("Already here, with a new name · {}", photos(items.len()))
                } else {
                    format!(
                        "From {} · {}",
                        folder_text(from, self.root.as_deref()),
                        photos(items.len())
                    )
                };
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(inner)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| line_label(ui, RichText::new(text).small().color(MUTED)),
                );
            }
            Block::Thumbs(list, start) => {
                let columns = if rows.slot(list) == PAIR {
                    rows.pair_columns
                } else {
                    rows.columns
                };
                let end = (start + columns).min(rows.items(list).len());
                let top = rect.top() + if placed.card_top { CARD_GAP + S2 } else { 0.0 };
                self.thumb_row(ui, plan, rows, list, start, end, inner.left(), top);
            }
            Block::FolderRenamed(group) => {
                if let Action::RenameDirectory(rename) = &plan.actions[group] {
                    let root = self.root.as_deref();
                    let text = format!(
                        "{}  →  {}",
                        folder_text(&rename.from, root),
                        folder_text(&rename.to, root)
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new()
                            .max_rect(inner)
                            .layout(egui::Layout::left_to_right(egui::Align::Center)),
                        |ui| line_label(ui, RichText::new(text).color(INK)),
                    );
                }
            }
            Block::Tidy => {
                let root = self.root.as_deref();
                let row = egui::Rect::from_min_size(
                    rect.min + egui::vec2(0.0, S3),
                    egui::vec2(rect.width(), TIDY_HEIGHT - S3),
                );
                ui.scope_builder(
                    egui::UiBuilder::new()
                        .max_rect(row)
                        .layout(egui::Layout::left_to_right(egui::Align::Center)),
                    |ui| {
                        ui.label(RichText::new(icon::BROOM).color(MUTED));
                        ui.label(RichText::new("Tidying up").font(medium(13.0)).color(MUTED));
                        line_label(
                            ui,
                            RichText::new(format!(
                                "Folders left empty afterwards are removed: {}",
                                cleanup_text(plan, rows, root, 3)
                            ))
                            .small()
                            .color(MUTED),
                        )
                        .on_hover_text(cleanup_text(plan, rows, root, 40));
                    },
                );
            }
        }
    }

    /// A destination card in one sentence, the shortest form that says
    /// where its photos come from and where they go:
    /// "3 photos from 2025-10-24 / group-1 move to group-5 (in 2025-10-25)" or
    /// "group-5 (in 2025-10-25) gets 16 photos from 5 folders".
    fn card_sentence(&self, plan: &Plan, folder: &Folder) -> String {
        let output = plan.output_root.as_deref().or(self.root.as_deref());
        let leaf = folder_text(&folder.path, Some(&folder.path));
        let dest = match folder.path.parent() {
            Some(parent) if Some(folder.path.as_path()) != output && Some(parent) != output => {
                format!("{leaf} (in {})", folder_text(parent, output))
            }
            _ => leaf,
        };
        let n = folder.photos();
        let one = n == 1;
        match folder.groups.as_slice() {
            [] => format!("{dest} is created and stays empty for now"),
            [(from, _)] if *from == folder.path => format!(
                "{} in {dest} {}",
                photos(n),
                if one {
                    "gets a new name"
                } else {
                    "get new names"
                }
            ),
            [(from, _)] => format!(
                "{} from {} {} to {dest}",
                photos(n),
                folder_text(from, self.root.as_deref()),
                if one { "moves" } else { "move" }
            ),
            groups => format!(
                "{dest} gets {} from {}",
                photos(n),
                count(groups.len(), "folder", "folders")
            ),
        }
    }

    fn card_heading(
        &self,
        ui: &mut egui::Ui,
        plan: &Plan,
        rows: &Rows,
        i: usize,
        rect: egui::Rect,
    ) {
        let folder = &rows.folders[i];
        let sentence = self.card_sentence(plan, folder);
        let mut job = LayoutJob::default();
        job.append(
            &sentence,
            0.0,
            TextFormat {
                font_id: semibold(14.0),
                color: INK,
                ..Default::default()
            },
        );
        if folder.new {
            job.append(
                "new folder",
                S2,
                TextFormat {
                    font_id: egui::TextStyle::Small.resolve(ui.style()),
                    color: GREEN,
                    valign: Align::Center,
                    ..Default::default()
                },
            );
        }
        ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(rect)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if folder.new {
                    ui.label(RichText::new(icon::FOLDER_PLUS).color(GREEN));
                }
                let spoken = if folder.new {
                    format!("{sentence}, new folder")
                } else {
                    sentence.clone()
                };
                let response = ui.add(egui::Label::new(job).truncate().selectable(false));
                response.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken)
                });
                response.on_hover_text(folder.path.display().to_string());
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    fn thumb_row(
        &mut self,
        ui: &mut egui::Ui,
        plan: &Plan,
        rows: &Rows,
        list: List,
        start: usize,
        end: usize,
        left: f32,
        top: f32,
    ) {
        let slot = rows.slot(list);
        for k in start..end {
            let item = rows.items(list)[k];
            let column = (k - start) as f32;
            let tile =
                egui::Rect::from_min_size(egui::pos2(left + column * (slot + S2), top), TILE);
            if let Some(thumb) = self.thumb(plan, item, list) {
                self.paint_thumb(ui, &thumb, tile, slot, (list, k));
            }
        }
    }

    fn paint_thumb(
        &mut self,
        ui: &mut egui::Ui,
        thumb: &Thumb,
        tile: egui::Rect,
        slot: f32,
        key: (List, usize),
    ) {
        let painter = ui.painter().clone();
        self.paint_tile(ui, thumb, tile, 0);
        if thumb.turns != 0 {
            let after = tile.translate(egui::vec2(TILE.x + ARROW, 0.0));
            painter.text(
                egui::pos2(tile.right() + ARROW / 2.0, tile.center().y),
                Align2::CENTER_CENTER,
                icon::ARROW_RIGHT,
                FontId::proportional(16.0),
                MUTED,
            );
            self.paint_tile(ui, thumb, after, thumb.turns);
        }
        if !thumb.extras.is_empty() {
            let galley = painter.layout_no_wrap(
                thumb.extras.clone(),
                egui::TextStyle::Small.resolve(ui.style()),
                INK,
            );
            let pill = egui::Rect::from_min_size(
                tile.right_bottom() - galley.size() - egui::vec2(S2 + 4.0, S1 + 4.0),
                galley.size() + egui::vec2(S2, 4.0),
            );
            painter.rect_filled(pill, 6.0, Color32::from_black_alpha(170));
            painter.galley(pill.min + egui::vec2(S1, 2.0), galley, INK);
        }
        let mut job = LayoutJob::single_section(
            thumb.caption.clone(),
            TextFormat {
                font_id: egui::TextStyle::Small.resolve(ui.style()),
                color: MUTED,
                ..Default::default()
            },
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(slot);
        let galley = painter.layout_job(job);
        painter.galley(egui::pos2(tile.left(), tile.bottom() + 4.0), galley, MUTED);
        let area = egui::Rect::from_min_size(tile.min, egui::vec2(slot, TILE.y + CAPTION));
        let response = ui.interact(area, ui.id().with(("thumb", key)), Sense::hover());
        let spoken = if thumb.name.ends_with(&thumb.caption) {
            thumb.name.clone()
        } else {
            format!("{}: {}", thumb.name, thumb.caption)
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, &spoken));
        response.on_hover_text(&thumb.hover);
    }

    /// One photo tile, turned `turns` quarter turns from how it is now.
    fn paint_tile(&mut self, ui: &mut egui::Ui, thumb: &Thumb, tile: egui::Rect, turns: u8) {
        let painter = ui.painter().clone();
        let radius = R_CONTROL as f32;
        painter.rect_filled(tile, radius, BG);
        let texture = (!thumb.folder)
            .then(|| self.texture(thumb.id, false))
            .flatten();
        if let Some(texture) = texture {
            let drawn =
                paint_image_tinted(ui, &texture, tile, 1.0, Vec2::ZERO, turns, Color32::WHITE);
            round_corners(&painter, drawn, radius, BG);
            round_corners(&painter, tile, radius, CARD);
        } else if self.requested.contains(&(thumb.id, false)) {
            shimmer(ui, tile);
        } else {
            let glyph = if thumb.folder {
                icon::FOLDER
            } else if self.failures.contains_key(&thumb.id) {
                icon::IMAGE_BROKEN
            } else {
                icon::IMAGE
            };
            painter.text(
                tile.center(),
                Align2::CENTER_CENTER,
                glyph,
                FontId::proportional(22.0),
                FAINT,
            );
        }
    }
}

const ROW_HEIGHT: f32 = 46.0;

/// One path line. `spoken` prefixes the accessible text ("From  ", "To    ")
/// so screen readers keep the direction the arrow shows visually.
fn path_line(
    ui: &mut egui::Ui,
    spoken: &str,
    shown: &str,
    path: &Path,
    root: Option<&Path>,
    color: Color32,
) {
    let relative = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let display = if relative.as_os_str().is_empty() {
        ".".into()
    } else {
        relative.display().to_string()
    };
    // Extend inside the two-axis scroll area: long paths can be read without hover.
    let response = ui
        .add(egui::Label::new(RichText::new(format!("{shown}{display}")).color(color)).extend())
        .on_hover_text(path.display().to_string());
    let spoken = format!("{spoken}{display}");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
}
fn path_label(ui: &mut egui::Ui, prefix: &str, path: &Path, root: Option<&Path>) {
    let to = prefix.trim() == "To";
    path_line(
        ui,
        prefix,
        if to { "→  " } else { "" },
        path,
        root,
        if to { MUTED } else { INK },
    );
}
/// The first line of a row: what happens, then the file it happens to.
fn source_line(
    ui: &mut egui::Ui,
    kind: &str,
    color: Color32,
    prefix: &str,
    path: &Path,
    root: Option<&Path>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = S2;
        ui.label(RichText::new(kind).small().color(color));
        path_line(ui, prefix, "", path, root, INK);
    });
}
fn note_line(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(MUTED)).extend());
}

fn row_ui(
    ui: &mut egui::Ui,
    action: &Action,
    _group: usize,
    file: usize,
    root: Option<&Path>,
    output: Option<&Path>,
) {
    match action {
        Action::Move { files } | Action::FinishMove { files } if !files.is_empty() => {
            let (source, to) = &files[file];
            let kind = if matches!(action, Action::FinishMove { .. }) {
                "Finish staged move (remove verified source link)"
            } else if source.path.parent() == to.parent() {
                "Rename"
            } else if source.path.file_name() == to.file_name() {
                "Move"
            } else {
                "Move and rename"
            };
            source_line(ui, kind, ACCENT, "From  ", &source.path, root);
            path_label(ui, "To    ", to, output.or(root));
        }
        Action::Trash { files } if !files.is_empty() => {
            source_line(ui, "Trash", RED, "From  ", &files[file].path, root);
            note_line(ui, "→  System Trash");
        }
        Action::TrashDirectory(folder) => {
            if file == 0 {
                source_line(
                    ui,
                    &format!(
                        "Trash folder · {} files, {} subfolders",
                        folder.inventory.files.len(),
                        folder.inventory.directory_count().saturating_sub(1)
                    ),
                    RED,
                    "From  ",
                    &folder.path,
                    root,
                );
            } else {
                source_line(
                    ui,
                    &format!("In folder · {} of {}", file, folder.inventory.files.len()),
                    RED,
                    "File  ",
                    &folder.inventory.files[file - 1].path,
                    root,
                );
            }
            note_line(ui, "→  System Trash, with the whole folder");
        }
        Action::RenameFile { source, to } => {
            source_line(ui, "Rename", INK, "From  ", &source.path, root);
            path_label(ui, "To    ", to, output.or(root));
        }
        Action::RenameDirectory(rename) => {
            source_line(ui, "Rename folder", INK, "From  ", &rename.from, root);
            path_label(ui, "To    ", &rename.to, root);
        }
        Action::Metadata { source, .. } => {
            source_line(ui, "Metadata", INK, "File  ", &source.path, root);
            note_line(ui, &action.label());
        }
        Action::CreateDirectory { path } | Action::RemoveEmptyDirectory { path, .. } => {
            let create = matches!(action, Action::CreateDirectory { .. });
            source_line(
                ui,
                if create {
                    "Create folder"
                } else {
                    "Remove folder if empty"
                },
                if create { GREEN } else { MUTED },
                "Folder  ",
                path,
                output.or(root),
            );
        }
        _ => {
            ui.label("Empty file operation");
        }
    }
}

fn file_rows(
    ui: &mut egui::Ui,
    plan: &Plan,
    index: &Rows,
    root: Option<&Path>,
    height: f32,
) -> egui::scroll_area::ScrollAreaOutput<usize> {
    egui::ScrollArea::both()
        .id_salt(("plan_files", index.generation))
        .max_height(height)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, index.len(), |ui, rows| {
            let rendered = rows.len();
            for row in rows {
                let (group, file) = index.at(row);
                ui.push_id(row, |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            row_ui(
                                ui,
                                &plan.actions[group],
                                group,
                                file,
                                root,
                                plan.output_root.as_deref(),
                            );
                        },
                    );
                });
            }
            rendered
        })
}

impl PhotoSortApp {
    pub(super) fn set_review_plan(&mut self, plan: Option<Plan>) {
        self.retry_edit = None;
        // A review that also covers moves shows the staged choices that are
        // applied right after them, with the same click.
        let mut staged: Vec<(MediaId, Decision)> = Vec::new();
        if self.review_after_moves && self.retry_plan.is_none() {
            staged = self
                .decisions
                .iter()
                .filter(|(id, d)| !d.is_empty() && self.index.contains_key(id))
                .map(|(id, d)| (*id, d.clone()))
                .collect();
            staged.sort_by_key(|(id, _)| self.index[id]);
        }
        self.plan_rows.replace(plan.as_ref(), &staged);
        self.plan = plan;
    }

    pub(super) fn plan_review_dialog(&mut self, ctx: &egui::Context) {
        if !self.plan_open {
            return;
        }
        if self.retry_edit.is_some() {
            self.destination_editor(ctx);
            return;
        }
        let Some(plan) = self.plan.take() else {
            return;
        };
        let mut rows = std::mem::take(&mut self.plan_rows);
        let mut edit = false;
        let mut apply = false;
        let mut cancel = false;
        let mut discard = false;
        let mut save_quit = false;
        modal(egui::Id::new("review_changes")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(240.0, 760.0));
            let top = ui.cursor().top();
            let retry = self.retry_plan.is_some();
            ui.heading(if retry { "Unfinished file operations" } else { "Review your changes" });
            ui.add_space(S1);
            ui.add(egui::Label::new(RichText::new(self.review_sentence(&plan, &rows)).font(medium(15.0)).color(INK)).wrap());
            // Reassurance; the library is named, its full path is on hover.
            let library = self.root.as_deref().map(|root| (folder_text(root, Some(root)), root.to_path_buf()));
            let note = if retry {
                "Changes that finished are already applied. These are the ones left.".to_string()
            } else {
                match &library {
                    Some((name, _)) => format!("Nothing in {name} has changed yet."),
                    None => "Nothing has changed yet.".into(),
                }
            };
            let response = ui.add(egui::Label::new(RichText::new(note).color(MUTED)).wrap());
            if let Some((_, root)) = &library { response.on_hover_text(root.display().to_string()); }
            if let Some(output) = &plan.output_root && self.root.as_ref() != Some(output) {
                ui.add(egui::Label::new(RichText::new(format!("Photos go to {}", folder_text(output, Some(output)))).color(MUTED)).truncate())
                    .on_hover_text(output.display().to_string());
            }
            if retry && self.error.is_some() { ui.colored_label(RED, "Some operations need attention. Open details before retrying."); }
            // Details swap the plain view for every file operation, so the
            // list keeps one height budget. Retries start there.
            let details = egui::CollapsingHeader::new("Operation details").id_salt(("plan_details", retry)).default_open(retry).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("plan_explanation").max_height(70.0).show(ui, |ui| {
                    ui.label(&plan.description);
                    if retry {
                        ui.label("Successful operations are already applied. Finish staged move rechecks that the destination is the same file before removing its source link. Edit destination saves a revised folder or base name for a pending move and all its companions. Retry uses the saved destinations and source checks. Resolve remaining conflicts first. Discard abandons only unfinished operations; the audit record is retained.");
                        if let Some(error) = &self.error { ui.colored_label(RED, error); }
                    } else {
                        ui.label(if self.rotation_plan { "Only confirmed corrections are included. Metadata rows identify embedded updates, XMP sidecars or lossless pixel rotations." } else { "All listed companion files are included. Metadata rows identify embedded updates, XMP sidecars or lossless pixel rotations. Trash entries go to your system's Trash. Moves never overwrite existing files. Folders are removed only when nothing is left in them." });
                        if !rows.trash.is_empty() || !rows.updates.is_empty() {
                            ui.label("Trash marks, ratings and rotations are planned and applied right after the moves, for the files at their new places.");
                        }
                    }
                });
            });
            ui.separator();
            // Account for actual wrapped heading/notice heights and reserve the footer.
            let footer = if retry && matches!(self.after_apply, Some(Destination::Close)) { 230.0 } else { 100.0 };
            let height = (ctx.content_rect().height() - footer - (ui.cursor().top() - top)).clamp(40.0, 520.0);
            if details.openness > 0.0 || rows.len() == 0 && rows.trash.is_empty() && rows.updates.is_empty() {
                file_rows(ui, &plan, &rows, self.root.as_deref(), height);
            } else {
                self.plain_view(ui, &plan, &mut rows, height);
            }
            ui.separator();
            if self.close_save_pending { ui.label("Saving before quitting…"); }
            if retry && matches!(self.after_apply, Some(Destination::Close)) && self.recovery_ticket.is_some() {
                ui.label("Save your plan and review progress to resume next time. No remaining file operations will run.");
                if self.has_unsaved_plan_extras() { ui.label("Go back to apply or reset other unsaved organization or confirmed rotation choices first."); }
            }
            let apply_label = if retry { "Retry unfinished changes".to_string() } else { rows.summary.apply_label() };
            ui.add_enabled_ui(!self.busy, |ui| { ui.horizontal_wrapped(|ui| {
                if retry && self.recovery_ticket.is_some() && matches!(self.after_apply, Some(Destination::Close))
                    && ui.add_enabled(!self.has_unsaved_plan_extras(), egui::Button::new("Save plan and quit")).clicked() { save_quit = true; }
                if ui.button("Back").clicked() { cancel = true; }
                if retry && ui.add_enabled(!self.busy && self.recovery_ticket.is_some() && !rows.movable.is_empty(), egui::Button::new("Edit destination…")).clicked() { edit = true; }
                if retry && ui.button("Discard unfinished plan").clicked() { discard = true; }
                if ui.add_enabled(!plan.actions.is_empty() && !self.busy, primary(apply_label)).clicked() { apply = true; }
            }); });
        });
        self.plan = Some(plan);
        self.plan_rows = rows;
        if save_quit {
            self.save_plan_and_quit();
        }
        if edit {
            self.begin_destination_edit();
        }
        if cancel {
            self.plan_open = false;
            self.after_apply = None;
            self.review_after_moves = false;
            if self.retry_plan.is_none() {
                self.rotation_plan = false;
            }
        }
        if discard {
            self.discard_remaining_operations();
        }
        if apply {
            self.apply_reviewed_plan();
        }
    }
}

pub(super) struct Edit {
    pub(super) group: usize,
    pub(super) folder: String,
    pub(super) stem: String,
    pub saving: bool,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    pub error: Option<String>,
}
impl PhotoSortApp {
    pub(super) fn begin_destination_edit(&mut self) {
        if self.busy || self.scanning || self.recovery_ticket.is_none() {
            return;
        }
        let Some(&group) = self.plan_rows.movable.first() else {
            return;
        };
        self.select_destination_group(group);
    }
    fn select_destination_group(&mut self, group: usize) {
        let Some(plan) = &self.plan else {
            return;
        };
        let Some(Action::Move { files }) = plan.actions.get(group) else {
            return;
        };
        let Some((_, destination)) = files.first() else {
            return;
        };
        let base = plan.output_root.as_deref().or(self.root.as_deref());
        let folder = destination
            .parent()
            .and_then(|p| base.and_then(|b| p.strip_prefix(b).ok()))
            .unwrap_or(Path::new(""));
        self.retry_edit = Some(Edit {
            group,
            folder: folder.display().to_string(),
            stem: String::new(),
            saving: false,
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            error: None,
        });
    }
    pub(super) fn save_destination_edit(&mut self) {
        if self.busy || self.scanning {
            return;
        }
        let (Some(root), Some(ticket), Some(edit)) =
            (&self.root, &self.recovery_ticket, &mut self.retry_edit)
        else {
            return;
        };
        edit.cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        match self.runtime.io(IoWork::AmendRecovery {
            root: root.clone(),
            ticket: ticket.clone(),
            group: edit.group,
            folder: PathBuf::from(&edit.folder),
            stem: (!edit.stem.is_empty()).then(|| edit.stem.clone()),
            cancelled: edit.cancelled.clone(),
        }) {
            Ok(()) => {
                edit.saving = true;
                edit.error = None;
                self.busy = true;
            }
            Err(error) => edit.error = Some(error),
        }
    }
    fn destination_editor(&mut self, ctx: &egui::Context) {
        let mut select = None;
        let mut save = false;
        let mut cancel = false;
        let Some(edit) = &mut self.retry_edit else {
            return;
        };
        let Some(plan) = &self.plan else {
            return;
        };
        modal(egui::Id::new("edit_pending_destination")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.).clamp(240., 720.));
            ui.heading("Edit pending destination");
            ui.label("Choose a move. Photo companions stay together; extensions keep their original spelling.");
            ui.add_enabled_ui(!edit.saving, |ui| {
                egui::ScrollArea::vertical().id_salt("pending_move_groups")
                    .max_height((ctx.content_rect().height() * 0.22).clamp(50., 150.))
                    .show_rows(ui, button_row_height(ui), self.plan_rows.movable.len(), |ui, rows| {
                        for row in rows {
                            let group = self.plan_rows.movable[row];
                            if let Action::Move { files } = &plan.actions[group] {
                                let name = files[0].1.file_name().unwrap_or_default().to_string_lossy();
                                if ui.add(egui::Button::new(format!("Group {} · {} · {} file(s)", group + 1, name, files.len())).selected(edit.group == group).truncate()).clicked() { select = Some(group); }
                            }
                        }
                    });
                let base = plan.output_root.as_deref().or(self.root.as_deref()).unwrap_or(Path::new(""));
                ui.add(egui::Label::new(format!("Folder within {}", base.display())).truncate()).on_hover_text(base.display().to_string());
                ui.add(egui::TextEdit::singleline(&mut edit.folder).hint_text("Leave empty for the destination root").desired_width(f32::INFINITY));
                ui.label("New base name (optional)");
                ui.add(egui::TextEdit::singleline(&mut edit.stem).hint_text("Leave empty to keep reviewed names").desired_width(f32::INFINITY));
                if let Some(Action::Move { files }) = plan.actions.get(edit.group) {
                    egui::ScrollArea::both().id_salt("pending_names_preview").max_height(48.).show_rows(ui, ui.text_style_height(&egui::TextStyle::Monospace), files.len(), |ui, rows| {
                        for row in rows {
                            let old = &files[row].1;
                            let name = if edit.stem.is_empty() { old.file_name().unwrap_or_default().to_owned() }
                                else { crate::core::organize::renamed_file_name(old, edit.stem.as_ref()) };
                            let to = base.join(&edit.folder).join(name);
                            path_label(ui, "To  ", &to, Some(base));
                        }
                    });
                }
            });
            if let Some(error) = &edit.error {
                egui::ScrollArea::vertical().id_salt("destination_edit_error").max_height(55.).show(ui, |ui| { ui.colored_label(RED, error); });
            }
            ui.small("Saving updates the plan only. Existing files stay untouched until you review and retry.");
            if edit.saving { ui.label("Checking sources and saving…"); }
            ui.horizontal_wrapped(|ui| {
                if ui.button(if edit.saving { "Cancel saving" } else { "Cancel" }).clicked() { cancel = true; }
                if ui.add_enabled(!edit.saving, primary("Save destination")).clicked() { save = true; }
            });
        });
        if cancel {
            if edit.saving {
                edit.cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
            } else {
                self.retry_edit = None;
            }
        } else if let Some(group) = select {
            self.select_destination_group(group);
        } else if save {
            self.save_destination_edit();
        }
    }
}

#[cfg(test)]
#[path = "plan_review/tests.rs"]
mod tests;
