//! Shared presentation of immutable operation plans. Rendering never rescans files.
use super::library::shimmer;
use super::*;
use crate::core::metadata::WriteTarget;
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
    /// Names the list's scroll area: a new list starts at the top.
    generation: u64,
    /// The generation the next list takes instead of a new one: the list
    /// made again after an Undo stays where it was scrolled to.
    keep_scroll: Option<u64>,
    summary: Summary,
    /// The plain view, as plan groups and staged media: never a copy of paths.
    folders: Vec<Folder>,
    trash: Vec<Item>,
    rotations: Vec<Item>,
    ratings: Vec<Item>,
    renamed_folders: Vec<usize>,
    /// How each staged rating or rotation would be written, from a plan of
    /// them made only to say so on the Review step.
    targets: HashMap<MediaId, WriteTarget>,
    /// The files that go to the Trash with a staged Trash mark, as a short
    /// note: "+ RAW, XMP".
    followers: HashMap<MediaId, String>,
    /// The files those notes name.
    follower_ids: HashSet<MediaId>,
    /// Everything listed as going to the Trash: the marked photos and the
    /// files that follow them.
    going: HashSet<MediaId>,
    /// Photos marked for Trash that the plan moves or renames first: the
    /// plan group that does it, and which of its files the photo is. They
    /// are listed once, under Trash.
    moves_first: HashMap<MediaId, (usize, usize)>,
    /// The files listed for the Trash on the Review step, where they are
    /// now: the marked photos and the files that follow them.
    trash_paths: Vec<PathBuf>,
    /// Folders that were there before and that the listed Trash marks leave
    /// empty, each with the leftovers that go to the Trash with it. Asked
    /// of the file worker once the list is made; "Tidying up" names them
    /// and Apply removes no other.
    emptied: Vec<(PathBuf, Vec<String>)>,
    /// RAW files the user rated, rotated or chose to keep whose JPEG is
    /// listed for the Trash: Apply asks what happens to them.
    wanted_raws: usize,
    /// Shown on the Review step: each photo has an Undo button, and the
    /// folder sections a link back to Organize.
    pub(super) editable: bool,
    /// The plain view laid out for `columns` thumbnails per row, or
    /// `pair_columns` before-and-after pairs in a list with rotations.
    layout: Vec<Placed>,
    columns: usize,
    pair_columns: usize,
    width: f32,
}

/// The files that go to the Trash with the listed Trash marks.
#[derive(Default)]
struct Followers {
    /// "+ RAW, XMP", by the marked photo they follow.
    notes: HashMap<MediaId, String>,
    ids: HashSet<MediaId>,
    /// The same files, as paths.
    paths: Vec<PathBuf>,
    /// The RAW files among them, or left out of them, that the user rated,
    /// rotated or chose to keep.
    wanted: HashSet<MediaId>,
}

/// One photo in the plain view: a plan group, or a choice staged for after
/// the moves of the same review (its Trash mark, rating or turns).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Item {
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
    /// What each of `groups` is made of: photos, videos or other files.
    tallies: Vec<Tally>,
    /// False when every photo here keeps its folder and only gets a new name.
    arrivals: bool,
}
impl Folder {
    fn tally(&self) -> Tally {
        self.tallies
            .iter()
            .fold(Tally::default(), |all, tally| all.plus(*tally))
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Section {
    Moving,
    Folders,
    Trash,
    Rotate,
    Rate,
}
#[derive(Clone, Copy, Debug, PartialEq, Hash)]
pub(super) enum List {
    /// The photos of `folders[i]` from its source folder `g`.
    Folder(usize, usize),
    Trash,
    Rotate,
    Rate,
}

/// What the Review step's list was asked to do this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum ReviewAction {
    /// Take back the change of one photo in one section.
    Undo(List, Item),
    /// Go back to a step to change things there.
    Open(Step),
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

/// What a count of listed changes is made of, so a video or a notes file
/// that moves with its folder is never called a photo. A RAW is a photo.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Tally {
    pub(super) photos: usize,
    pub(super) videos: usize,
    pub(super) others: usize,
}
impl Tally {
    /// One file, by its name: `lead` is the file a person thinks of as the
    /// photo among those that move together.
    fn of(lead: &Path) -> Self {
        let mut tally = Self::default();
        match MediaKind::from_path(lead) {
            _ if crate::core::model::is_apple_double(lead) => tally.others = 1,
            Some(MediaKind::Video) => tally.videos = 1,
            Some(_) => tally.photos = 1,
            None => tally.others = 1,
        }
        tally
    }
    fn plus(self, other: Self) -> Self {
        Self {
            photos: self.photos + other.photos,
            videos: self.videos + other.videos,
            others: self.others + other.others,
        }
    }
    pub(super) fn total(&self) -> usize {
        self.photos + self.videos + self.others
    }
    /// "3 photos", "1 video", "2 photos and 1 video", "1 photo and 2 other
    /// files", "2 files". The numbers add up to `total`.
    pub(super) fn text(&self) -> String {
        let mut parts = Vec::new();
        if self.photos > 0 {
            parts.push(photos(self.photos));
        }
        if self.videos > 0 {
            parts.push(count(self.videos, "video", "videos"));
        }
        if self.others > 0 && parts.is_empty() {
            parts.push(count(self.others, "file", "files"));
        } else if self.others > 0 {
            parts.push(count(self.others, "other file", "other files"));
        }
        if parts.is_empty() {
            return photos(0);
        }
        and_list(&parts)
    }
}
/// "a", "a and b", "a, b and c".
fn and_list(parts: &[String]) -> String {
    match parts {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// Plain counts: photos, never file operations. The sentence, the Apply
/// button and the Review badge all read these same numbers.
#[derive(Default)]
struct Summary {
    moved: Tally,
    renamed: Tally,
    destinations: usize,
    new_destinations: usize,
    empty_folders: usize,
    trashed: Tally,
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
    /// Every change the sentence counts, added up: what the Review badge
    /// shows while this list is on screen.
    fn total(&self) -> usize {
        self.moved.total()
            + self.renamed.total()
            + self.folder_renames
            + self.empty_folders
            + self.updated
            + self.trashed.total()
            + self.trashed_folders
    }
    /// The button that applies the plan, named for what it does.
    fn apply_label(&self) -> String {
        let (moved, renamed, trashed) = (
            self.moved.total(),
            self.renamed.total(),
            self.trashed.total(),
        );
        let kinds = [
            moved,
            renamed,
            self.folder_renames,
            self.empty_folders,
            self.updated,
            trashed + self.trashed_folders,
        ];
        let capital = |text: String| {
            let mut chars = text.chars();
            chars.next().map_or_else(String::new, |c| {
                c.to_uppercase().chain(chars).collect::<String>()
            })
        };
        match kinds.iter().filter(|n| **n > 0).count() {
            0 => "Apply".into(),
            1 if moved > 0 => format!("Move {}", self.moved.text()),
            1 if renamed > 0 => format!("Rename {}", self.renamed.text()),
            1 if self.updated > 0 => capital(self.updates_text(photos(self.updated))),
            1 if trashed > 0 && self.trashed_folders == 0 => {
                format!("Move {} to the Trash", self.trashed.text())
            }
            2 if moved > 0 && trashed > 0 && self.trashed_folders == 0 => {
                format!("Move {moved} and trash {trashed}")
            }
            _ => "Apply all changes".into(),
        }
    }
}

impl Rows {
    /// Index `plan`, and the choices staged to apply right after its moves.
    /// `followers` are the files that go to the Trash with a staged mark,
    /// `videos` the staged choices that are about a video.
    fn replace(
        &mut self,
        plan: Option<&Plan>,
        staged: &[(MediaId, Decision)],
        followers: &HashSet<MediaId>,
        videos: &HashSet<MediaId>,
    ) {
        self.generation = match self.keep_scroll.filter(|_| plan.is_some()) {
            Some(kept) => {
                self.keep_scroll = None;
                kept
            }
            None => self.generation.wrapping_add(1),
        };
        self.ends.clear();
        self.movable.clear();
        self.folders.clear();
        self.trash.clear();
        self.rotations.clear();
        self.ratings.clear();
        self.renamed_folders.clear();
        self.targets.clear();
        self.followers.clear();
        self.follower_ids.clear();
        self.going.clear();
        self.moves_first.clear();
        self.trash_paths.clear();
        self.emptied.clear();
        self.wanted_raws = 0;
        self.layout.clear();
        self.columns = 0;
        self.pair_columns = 0;
        self.width = 0.0;
        self.summary = Summary::default();
        let Some(plan) = plan else {
            return;
        };
        let summary = &mut self.summary;
        // A photo marked for Trash that also moves is moved first and trashed
        // from its new folder. It is listed once, under Trash, with the files
        // that follow it there, never also as moved.
        let trashed: HashSet<MediaId> = staged
            .iter()
            .filter(|(_, d)| d.rejected)
            .map(|(id, _)| *id)
            .collect();
        self.going = trashed.union(followers).copied().collect();
        // Folders that only such photos pass through: not listed, and
        // removed again by the same Apply when they are new.
        let mut passing = std::collections::BTreeSet::<&Path>::new();
        // Destination folder -> (photos arrive, plan groups by source folder).
        type Sources<'a> = BTreeMap<&'a Path, (Vec<Item>, Tally)>;
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
            // What moves and stays is the photo here: a RAW the user keeps
            // while its JPEG goes to the Trash is still listed as moving.
            let mut kept = Vec::new();
            let staying = if self.going.is_empty() {
                &pairs
            } else {
                for (member, pair) in pairs.iter().enumerate() {
                    let id = MediaId::for_path(pair.0);
                    if trashed.contains(&id) {
                        self.moves_first.insert(id, (group, member));
                    } else if !followers.contains(&id) {
                        kept.push(*pair);
                    }
                }
                &kept
            };
            let sources: Vec<&Path> = staying.iter().map(|(s, _)| *s).collect();
            let lead = staying
                .get(lead_and_extras(&sources).0)
                .filter(|(source, _)| staying.len() == pairs.len() || !is_sidecar(source));
            if lead.is_none()
                && let Some(folder) = pairs.first().and_then(|(_, to)| to.parent())
            {
                passing.insert(folder);
            }
            if let Some((source, to)) = lead
                && let Some(folder) = to.parent()
            {
                let arrives = staying.iter().any(|(s, to)| s.parent() != to.parent());
                let entry = into.entry(folder).or_default();
                entry.0 |= arrives;
                let from = lead
                    .filter(|_| arrives)
                    .and_then(|(source, _)| source.parent());
                let kind = Tally::of(source);
                let listed = entry.1.entry(from.unwrap_or(folder)).or_default();
                listed.0.push(Item::Group(group));
                listed.1 = listed.1.plus(kind);
                if arrives {
                    summary.moved = summary.moved.plus(kind);
                } else {
                    summary.renamed = summary.renamed.plus(kind);
                }
            }
            match action {
                Action::Trash { files } if !files.is_empty() => {
                    let sources: Vec<&Path> = files.iter().map(|s| s.path.as_path()).collect();
                    let lead = sources[lead_and_extras(&sources).0];
                    summary.trashed = summary.trashed.plus(Tally::of(lead));
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
                    if orientation.is_some() {
                        self.rotations.push(Item::Group(group));
                    }
                    if rating.is_some() {
                        self.ratings.push(Item::Group(group));
                    }
                }
                Action::RenameDirectory(_) => {
                    summary.folder_renames += 1;
                    self.renamed_folders.push(group);
                }
                Action::CreateDirectory { path } => created.push(path.as_path()),
                Action::RemoveEmptyDirectory { leftovers, .. } => {
                    summary.cleanup.push(group);
                    // With leftovers in it, the folder goes to the Trash.
                    summary.trashed_folders += usize::from(!leftovers.is_empty());
                }
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
                summary.trashed = summary.trashed.plus(Tally {
                    photos: usize::from(!videos.contains(&id)),
                    videos: usize::from(videos.contains(&id)),
                    others: 0,
                });
                self.trash.push(item);
            } else if decision.rating.is_some() || decision.turns % 4 != 0 {
                summary.updated += 1;
                summary.rated += usize::from(decision.rating.is_some());
                summary.rotated += usize::from(decision.turns % 4 != 0);
                if decision.turns % 4 != 0 {
                    self.rotations.push(item);
                }
                if decision.rating.is_some() {
                    self.ratings.push(item);
                }
            }
        }
        // A new folder that only holds other new folders is shown through them.
        // Paths order by component, so a folder's subfolders sort right after
        // it: one look past it finds any, however many folders the plan makes.
        let made: std::collections::BTreeSet<&Path> = created.iter().copied().collect();
        let empty: Vec<&Path> = created
            .iter()
            .copied()
            .filter(|path| {
                !into
                    .range::<&Path, _>(path..)
                    .next()
                    .is_some_and(|(folder, _)| folder.starts_with(path))
                    && !passing
                        .range::<&Path, _>(path..)
                        .next()
                        .is_some_and(|folder| folder.starts_with(path))
                    && !made
                        .range::<&Path, _>((
                            std::ops::Bound::Excluded(path),
                            std::ops::Bound::Unbounded,
                        ))
                        .next()
                        .is_some_and(|c| c.starts_with(path))
            })
            .collect();
        summary.empty_folders = empty.len();
        for path in empty {
            into.insert(path, (false, Sources::new()));
        }
        for (path, (arrivals, sources)) in into {
            let new = made.contains(path);
            if arrivals {
                summary.destinations += 1;
                summary.new_destinations += usize::from(new);
            }
            let (groups, tallies) = sources
                .into_iter()
                .map(|(from, (items, tally))| ((from.to_path_buf(), items), tally))
                .unzip();
            self.folders.push(Folder {
                path: path.to_path_buf(),
                new,
                groups,
                tallies,
                arrivals,
            });
        }
    }
    fn len(&self) -> usize {
        self.ends.last().copied().unwrap_or(0)
    }
    /// The list made next is this one with a change taken back: it keeps
    /// its scroll position instead of starting at the top.
    fn hold_scroll(&mut self) {
        self.keep_scroll.get_or_insert(self.generation);
    }
    /// The next list is a new one, from the top.
    pub(super) fn release_scroll(&mut self) {
        self.keep_scroll = None;
    }
    /// The files listed for the Trash, where they are now.
    pub(super) fn trash_paths(&self) -> Vec<PathBuf> {
        self.trash_paths.clone()
    }
    /// The folders "Tidying up" names as left empty by the Trash marks.
    pub(super) fn emptied(&self) -> Vec<(PathBuf, Vec<String>)> {
        self.emptied.clone()
    }
    /// Takes the file worker's answer: `plan` holds one removal per folder
    /// the listed Trash marks leave empty.
    pub(super) fn set_emptied(&mut self, plan: &Plan) {
        let folders = plan
            .actions
            .iter()
            .filter_map(|action| match action {
                Action::RemoveEmptyDirectory {
                    path, leftovers, ..
                } => Some((path.clone(), leftovers.clone())),
                _ => None,
            })
            .collect();
        self.restore_emptied(folders);
    }
    /// Names `folders` in "Tidying up". One that holds leftovers goes to
    /// the Trash with them, and counts as a folder sent there.
    pub(super) fn restore_emptied(&mut self, folders: Vec<(PathBuf, Vec<String>)>) {
        let trashed = |folders: &[(PathBuf, Vec<String>)]| {
            folders.iter().filter(|(_, left)| !left.is_empty()).count()
        };
        self.summary.trashed_folders -= trashed(&self.emptied);
        self.summary.trashed_folders += trashed(&folders);
        self.emptied = folders;
        // The line appears, changes or goes: laid out again, scroll kept.
        self.layout.clear();
    }
    /// Every change listed, as the sentence counts them.
    pub(super) fn total(&self) -> usize {
        self.summary.total()
    }
    /// The RAW and XMP files listed as going to the Trash with a staged mark.
    pub(super) fn follower_ids(&self) -> HashSet<MediaId> {
        self.follower_ids.clone()
    }
    /// How many RAW files Apply asks about before it trashes their JPEG.
    pub(super) fn wanted_raws(&self) -> usize {
        self.wanted_raws
    }
    /// Nothing to show: no file operation and no staged choice.
    pub(super) fn is_empty(&self) -> bool {
        self.len() == 0
            && self.trash.is_empty()
            && self.rotations.is_empty()
            && self.ratings.is_empty()
    }
    /// What applying it did, said once it is done: "Moved 3 photos and
    /// sent 2 photos to the Trash."
    pub(super) fn applied_sentence(&self) -> String {
        let summary = &self.summary;
        let mut parts = Vec::new();
        if summary.moved.total() > 0 {
            parts.push(format!("moved {}", summary.moved.text()));
        }
        if summary.renamed.total() > 0 {
            parts.push(format!("renamed {}", summary.renamed.text()));
        }
        if summary.folder_renames > 0 {
            parts.push(format!(
                "renamed {}",
                count(summary.folder_renames, "folder", "folders")
            ));
        }
        if summary.empty_folders > 0 {
            parts.push(format!(
                "created {}",
                count(summary.empty_folders, "empty folder", "empty folders")
            ));
        }
        if summary.rotated > 0 {
            parts.push(format!("rotated {}", photos(summary.rotated)));
        }
        if summary.rated > 0 {
            parts.push(format!("rated {}", photos(summary.rated)));
        }
        let mut trash = Vec::new();
        if summary.trashed.total() > 0 {
            trash.push(summary.trashed.text());
        }
        if summary.trashed_folders > 0 {
            trash.push(count(summary.trashed_folders, "folder", "folders"));
        }
        if !trash.is_empty() {
            parts.push(format!("sent {} to the Trash", and_list(&trash)));
        }
        let text = and_list(&parts);
        let mut chars = text.chars();
        match chars.next() {
            Some(first) => format!("{}{}.", first.to_uppercase(), chars.as_str()),
            None => "Your changes are applied.".into(),
        }
    }
    /// Remembers how `plan`, the staged ratings and rotations planned, would
    /// write each of them.
    pub(super) fn set_targets(&mut self, plan: &Plan) {
        self.targets = plan
            .actions
            .iter()
            .filter_map(|action| match action {
                Action::Metadata { source, target, .. } => {
                    Some((MediaId::for_path(&source.path), *target))
                }
                _ => None,
            })
            .collect();
    }
    /// The button that applies everything listed.
    pub(super) fn apply_label(&self) -> String {
        self.summary.apply_label()
    }
    fn at(&self, row: usize) -> (usize, usize) {
        let group = self.ends.partition_point(|end| *end <= row);
        let start = group.checked_sub(1).map_or(0, |i| self.ends[i]);
        (group, row - start)
    }
    fn items(&self, list: List) -> &[Item] {
        match list {
            List::Trash => &self.trash,
            List::Rotate => &self.rotations,
            List::Rate => &self.ratings,
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
            let (columns, pair_columns) = (self.columns, self.pair_columns);
            let columns = |list: List| {
                if list == List::Rotate {
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
            for (section, list) in [
                (Section::Trash, List::Trash),
                (Section::Rotate, List::Rotate),
                (Section::Rate, List::Rate),
            ] {
                let len = self.items(list).len();
                if len > 0 {
                    place(Block::Section(section), SECTION_HEIGHT, false, false);
                    thumbs(&mut place, list, len, true, true);
                }
            }
            if !self.summary.cleanup.is_empty() || !self.emptied.is_empty() {
                place(Block::Tidy, TIDY_HEIGHT, false, false);
            }
            self.layout = layout;
        }
        self.layout.last().map_or(0.0, |p| p.top + p.height)
    }
    /// How wide one photo's place in `list` is: a pair where rotations are.
    fn slot(&self, list: List) -> f32 {
        if list == List::Rotate { PAIR } else { TILE.x }
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
fn is_sidecar(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xmp"))
}
/// Which of a photo's files a person thinks of as the photo, and a short
/// note for the rest: a RAW twin or XMP sidecar is "+ RAW, XMP".
fn lead_and_extras(paths: &[&Path]) -> (usize, String) {
    let kind = |p: &Path| MediaKind::from_path(p);
    let lead = paths
        .iter()
        .position(|p| matches!(kind(p), Some(MediaKind::Photo | MediaKind::Video)))
        .or_else(|| paths.iter().position(|p| !is_sidecar(p)))
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

/// What Undo says for a photo in `list`, named `file`.
fn undo_text(list: List, file: &str) -> String {
    match list {
        List::Trash => format!("Keep {file}"),
        List::Rotate => format!("Don't rotate {file}"),
        List::Rate => format!("Don't change the rating of {file}"),
        List::Folder(..) => format!("Leave {file} where it is"),
    }
}

/// One thumbnail of the plain view: what it shows and says.
struct Thumb {
    id: MediaId,
    folder: bool,
    /// The file's name as it is now, for Undo.
    file: String,
    caption: String,
    name: String,
    hover: String,
    extras: String,
    /// Quarter turns shown beside the photo as it is now, "before → after".
    turns: u8,
    /// On a photo listed for the Trash that moves first: where to, or its
    /// new name. Empty for every other photo.
    first: String,
    /// The same as a sentence: "Moves to 2024 / Lisbon first".
    first_said: String,
}

/// What a plan tidies away afterwards, in one line. Folders that still hold
/// thumbnail folders or Finder leftovers come first: they go to the Trash
/// with them, named up to `limit` each. Empty folders are simply removed;
/// one inside another that is removed is not named apart from it.
/// `every` names each leftover; otherwise the "._name" file macOS keeps
/// beside a "name" that is listed is not named a second time.
fn cleanup_text(
    plan: &Plan,
    index: &Rows,
    root: Option<&Path>,
    limit: usize,
    every: bool,
) -> String {
    let (mut trashed, mut removed) = (Vec::new(), Vec::new());
    // The folders the moves empty, then those the Trash marks empty.
    let by_moves = index
        .summary
        .cleanup
        .iter()
        .filter_map(|&group| match &plan.actions[group] {
            Action::RemoveEmptyDirectory {
                path, leftovers, ..
            } => Some((path, leftovers)),
            _ => None,
        });
    let by_marks = index.emptied.iter().map(|(path, left)| (path, left));
    let folders: Vec<_> = by_moves.chain(by_marks).collect();
    // Each folder once: an empty folder inside one that is removed goes
    // with it, and naming it too would say its parent a second time.
    let gone: HashSet<&Path> = folders
        .iter()
        .filter(|(_, leftovers)| leftovers.is_empty())
        .map(|(path, _)| path.as_path())
        .collect();
    for (path, leftovers) in folders {
        if leftovers.is_empty() && path.ancestors().skip(1).any(|above| gone.contains(above)) {
            continue;
        }
        let folder = folder_text(path, root);
        let names: Vec<String> = leftovers
            .iter()
            .filter(|name| {
                every
                    || !name
                        .strip_prefix("._")
                        .is_some_and(|owner| leftovers.iter().any(|other| other == owner))
            })
            .cloned()
            .collect();
        if leftovers.is_empty() {
            removed.push(folder);
        } else if names.len() <= limit {
            trashed.push(format!("{folder}, with {}", and_list(&names)));
        } else {
            trashed.push(format!("{folder}, with {} leftover items", leftovers.len()));
        }
    }
    let listed = |names: &[String], separator: &str| {
        let shown = limit.min(names.len());
        let mut text = names[..shown].join(separator);
        if names.len() > shown {
            text.push_str(&format!(" and {} more", names.len() - shown));
        }
        text
    };
    let mut parts = Vec::new();
    if !trashed.is_empty() {
        parts.push(format!("Sent to the Trash: {}", listed(&trashed, "; ")));
    }
    if !removed.is_empty() {
        parts.push(format!(
            "Folders left empty are removed: {}",
            listed(&removed, ", ")
        ));
    }
    parts.join(". ")
}

/// File `member` of plan group `group`, when that group moves or renames
/// files: where it is and where it goes.
fn plan_pair(plan: &Plan, group: usize, member: usize) -> Option<(&Path, &Path)> {
    match plan.actions.get(group)? {
        Action::Move { files } | Action::FinishMove { files } => files
            .get(member)
            .map(|(source, to)| (source.path.as_path(), to.as_path())),
        Action::RenameFile { source, to } => Some((source.path.as_path(), to.as_path())),
        _ => None,
    }
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
        if summary.moved.total() > 0 {
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
            parts.push(format!("move {} into {into}", summary.moved.text()));
        }
        if summary.renamed.total() > 0 {
            parts.push(format!("rename {}", summary.renamed.text()));
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
        if summary.trashed.total() > 0 {
            trash.push(summary.trashed.text());
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

    fn thumb(&self, plan: &Plan, rows: &Rows, item: Item, list: List) -> Option<Thumb> {
        let root = self.root.as_deref();
        let blank = |id: MediaId, name: String| Thumb {
            id,
            folder: false,
            file: name.clone(),
            caption: name.clone(),
            hover: name.clone(),
            name,
            extras: String::new(),
            turns: 0,
            first: String::new(),
            first_said: String::new(),
        };
        match item {
            Item::Staged { id, rating, turns } => {
                let media = &self.media[*self.index.get(&id)?];
                let mut thumb = blank(id, file_name(&media.path));
                if let Some(from) = media.path.parent() {
                    thumb.hover = format!("{}\nin {}", thumb.name, folder_text(from, root));
                }
                if list == List::Rotate {
                    let pixels = rows.targets.get(&id) == Some(&WriteTarget::Pixels);
                    thumb.caption = update_text(None, turned(turns), pixels);
                    thumb.turns = turns;
                    thumb.hover = format!("{}: {}", thumb.name, thumb.caption);
                } else if list == List::Rate {
                    thumb.caption = update_text(rating, None, false);
                    thumb.hover = format!("{}: {}", thumb.name, thumb.caption);
                } else {
                    if let Some(extras) = rows.followers.get(&id) {
                        thumb.extras = extras.clone();
                    }
                    // Moved first, then trashed from there: said on the tile.
                    if let Some((source, to)) = rows
                        .moves_first
                        .get(&id)
                        .and_then(|&(group, member)| plan_pair(plan, group, member))
                    {
                        let renamed = source.file_name() != to.file_name();
                        let folder = to
                            .parent()
                            .filter(|folder| source.parent() != Some(*folder));
                        let base = plan.output_root.as_deref().or(root);
                        let line = match folder {
                            Some(folder) if renamed => format!(
                                "Moves to {} as {} first",
                                folder_text(folder, base),
                                file_name(to)
                            ),
                            Some(folder) => format!("Moves to {} first", folder_text(folder, base)),
                            None => format!("Renamed {} first", file_name(to)),
                        };
                        thumb.first = file_name(folder.unwrap_or(to));
                        thumb.hover = format!("{}\n{line}", thumb.hover);
                        thumb.first_said = line;
                    }
                }
                Some(thumb)
            }
            Item::Group(group) => match &plan.actions[group] {
                Action::Move { files } | Action::FinishMove { files } if !files.is_empty() => {
                    // Files listed for the Trash are named there, not here.
                    let staying: Vec<_> = files
                        .iter()
                        .filter(|(s, _)| {
                            rows.going.is_empty()
                                || !rows.going.contains(&MediaId::for_path(&s.path))
                        })
                        .collect();
                    let sources: Vec<&Path> =
                        staying.iter().map(|(s, _)| s.path.as_path()).collect();
                    let (lead, extras) = lead_and_extras(&sources);
                    let (source, to) = staying.get(lead).copied()?;
                    Some(self.moved_thumb(&source.path, to, extras))
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
                    let system: Vec<String> = folder
                        .inventory
                        .system_folders
                        .iter()
                        .flatten()
                        .map(|path| folder_text(path, Some(&folder.path)))
                        .collect();
                    thumb.hover = format!(
                        "{}, with everything in it ({}{})",
                        folder_text(&folder.path, root),
                        count(folder.inventory.files.len(), "file", "files"),
                        if system.is_empty() {
                            String::new()
                        } else {
                            format!(", and {}", and_list(&system))
                        }
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
                    let pixels = *target == WriteTarget::Pixels;
                    if list == List::Rate {
                        thumb.caption = update_text(*rating, None, false);
                    } else {
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
                        thumb.caption = update_text(None, orientation, pixels);
                    }
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
            file: before.clone(),
            // A rename reads "old → new"; the caption fits the new name.
            name: hover.lines().next().unwrap_or_default().to_string(),
            caption: after,
            hover,
            extras,
            turns: 0,
            first: String::new(),
            first_said: String::new(),
        }
    }

    /// The plain view: sections of photo thumbnails by what happens to them.
    /// Only the pieces in view are drawn and only their thumbnails load.
    pub(super) fn plain_view(
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
                // The counts are in the sentence at the top.
                let (glyph, title, color) = match section {
                    Section::Moving => (icon::FOLDER_SIMPLE, "Moving", ACCENT),
                    Section::Folders => (icon::PENCIL_SIMPLE, "Renaming folders", INK),
                    Section::Trash => (icon::TRASH, "Going to the Trash", RED),
                    Section::Rotate => (icon::ARROW_CLOCKWISE, "Rotating", INK),
                    Section::Rate => (icon::STAR, "Rating", INK),
                };
                // On the Review step, folder changes are edited in Organize.
                let organize =
                    rows.editable && matches!(section, Section::Moving | Section::Folders);
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
                        if organize {
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if ui
                                    .link(format!("{}  Edit in Organize", icon::FOLDERS))
                                    .clicked()
                                {
                                    self.review_action = Some(ReviewAction::Open(Step::Organize));
                                }
                            });
                        }
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
                let (from, _) = &folder.groups[g];
                let listed = folder.tallies[g].text();
                let text = if *from == folder.path {
                    format!("Already here, with a new name · {listed}")
                } else {
                    format!(
                        "From {} · {listed}",
                        folder_text(from, self.root.as_deref())
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
                            RichText::new(cleanup_text(plan, rows, root, 3, false))
                                .small()
                                .color(MUTED),
                        )
                        .on_hover_text(cleanup_text(plan, rows, root, 40, true));
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
        let tally = folder.tally();
        let one = tally.total() == 1;
        let listed = tally.text();
        match folder.groups.as_slice() {
            [] => format!("{dest} is created and stays empty for now"),
            [(from, _)] if *from == folder.path => format!(
                "{listed} in {dest} {}",
                if one {
                    "gets a new name"
                } else {
                    "get new names"
                }
            ),
            [(from, _)] => format!(
                "{listed} from {} {} to {dest}",
                folder_text(from, self.root.as_deref()),
                if one { "moves" } else { "move" }
            ),
            groups => format!(
                "{dest} gets {listed} from {}",
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
            if let Some(thumb) = self.thumb(plan, rows, item, list) {
                let undo = (rows.editable && !thumb.folder).then(|| undo_text(list, &thumb.file));
                if self.paint_thumb(ui, &thumb, tile, slot, (list, k), undo.as_deref()) {
                    self.review_action = Some(ReviewAction::Undo(list, item));
                }
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
        undo: Option<&str>,
    ) -> bool {
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
        if !thumb.first.is_empty() {
            // Top left, beside Undo: where the photo goes before the Trash.
            let mut job = LayoutJob::single_section(
                format!("{} {}", icon::ARROW_RIGHT, thumb.first),
                TextFormat {
                    font_id: egui::TextStyle::Small.resolve(ui.style()),
                    color: INK,
                    ..Default::default()
                },
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(slot - 24.0 - 3.0 * S1 - S2);
            let galley = painter.layout_job(job);
            let pill = egui::Rect::from_min_size(
                tile.left_top() + egui::vec2(S1, S1 + 2.0),
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
        let mut spoken = if thumb.name.ends_with(&thumb.caption) {
            thumb.name.clone()
        } else {
            format!("{}: {}", thumb.name, thumb.caption)
        };
        if !thumb.first_said.is_empty() {
            spoken = format!("{spoken}. {}", thumb.first_said);
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Image, true, &spoken));
        response.on_hover_text(&thumb.hover);
        // The Review step's Undo, over the top-right corner of the photo's place.
        let Some(text) = undo else {
            return false;
        };
        let size = 24.0;
        let button = egui::Rect::from_min_size(
            egui::pos2(tile.left() + slot - size - S1, tile.top() + S1),
            egui::vec2(size, size),
        );
        let response = ui.interact(button, ui.id().with(("undo", key)), Sense::click());
        let hovered = response.hovered() && ui.is_enabled();
        painter.circle(
            button.center(),
            size / 2.0,
            Color32::from_black_alpha(if hovered { 230 } else { 170 }),
            Stroke::new(
                1.0,
                if hovered {
                    INK.gamma_multiply(0.5)
                } else {
                    LINE
                },
            ),
        );
        painter.text(
            button.center(),
            Align2::CENTER_CENTER,
            icon::ARROW_COUNTER_CLOCKWISE,
            FontId::proportional(13.0),
            if hovered {
                INK
            } else {
                lerp_color(MUTED, INK, 0.5)
            },
        );
        response.widget_info(|| {
            egui::WidgetInfo::labeled(egui::WidgetType::Button, ui.is_enabled(), text)
        });
        response.on_hover_text(text).clicked()
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
        // The Review step lists Organize's plan with every staged choice.
        let listing = self.step == Step::Review && self.retry_plan.is_none();
        let staged = if listing {
            self.staged_choices()
        } else {
            Vec::new()
        };
        let follow = self.trash_followers(&staged);
        // A RAW that goes to the Trash with its JPEG is listed there only:
        // its rating or rotation is never written.
        let listed: Vec<(MediaId, Decision)> = staged
            .into_iter()
            .filter(|(id, _)| !follow.ids.contains(id))
            .collect();
        let videos: HashSet<MediaId> = listed
            .iter()
            .filter(|(id, _)| {
                self.index
                    .get(id)
                    .is_some_and(|i| self.media[*i].kind == MediaKind::Video)
            })
            .map(|(id, _)| *id)
            .collect();
        self.plan_rows
            .replace(plan.as_ref(), &listed, &follow.ids, &videos);
        // Where each file listed for the Trash is now, for "Tidying up".
        self.plan_rows.trash_paths = listed
            .iter()
            .filter(|(_, decision)| decision.rejected)
            .filter_map(|(id, _)| self.index.get(id))
            .map(|i| self.media[*i].path.clone())
            .chain(follow.paths)
            .collect();
        self.plan_rows.editable = listing && plan.is_some();
        self.plan = plan;
        self.plan_rows.followers = follow.notes;
        self.plan_rows.follower_ids = follow.ids;
        self.plan_rows.wanted_raws = follow.wanted.len();
    }

    /// The files Apply sends to the Trash with each listed Trash mark: its
    /// XMP sidecars ("IMG.xmp", "IMG.JPG.xmp") and RAW twin, a video's XMP. A file with a mark of its own is listed on
    /// its own. A RAW the user rated, rotated or chose to keep follows only
    /// once they said so; kept, it keeps the XMP it shares with the photo.
    fn trash_followers(&self, staged: &[(MediaId, Decision)]) -> Followers {
        use crate::core::preferences::WantedRaw;
        let mut found = Followers::default();
        let rejected: HashSet<MediaId> = staged
            .iter()
            .filter(|(_, d)| d.rejected)
            .map(|(id, _)| *id)
            .collect();
        let Some(catalog) = self.catalog.as_ref().filter(|_| !rejected.is_empty()) else {
            return found;
        };
        let index = catalog.companion_index();
        let keep = self
            .raw_answer
            .or(self.preferences.limit_prompts.wanted_raw)
            != Some(WantedRaw::Trash);
        // Kept over another photo in Pick Best or Easy Delete. Only a RAW
        // beside a JPEG follows, and only with companions on.
        let mut chosen = HashSet::new();
        if self.companions {
            chosen.extend(self.pick_review.chosen(&self.decisions));
            chosen.extend(self.delete_review.chosen(&self.decisions));
        }
        let is_xmp = |p: &Path| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xmp"));
        let wanted = |p: &Path| {
            let id = MediaId::for_path(p);
            MediaKind::from_path(p) == Some(MediaKind::Raw)
                && (chosen.contains(&id)
                    || self
                        .decisions
                        .get(&id)
                        .is_some_and(|d| d.rating.is_some() || d.turns % 4 != 0))
        };
        for id in &rejected {
            let Some(media) = self.index.get(id).map(|i| &self.media[*i]) else {
                continue;
            };
            let mut followers: Vec<PathBuf> = crate::core::operations::trash_followers_for(
                &index,
                &media.path,
                self.companions,
                &rejected,
            )
            .into_iter()
            .filter(|path| !rejected.contains(&MediaId::for_path(path)))
            .collect();
            let kept: Vec<MediaId> = followers
                .iter()
                .filter(|p| wanted(p))
                .map(|p| MediaId::for_path(p))
                .collect();
            if keep && !kept.is_empty() {
                // A kept RAW keeps its own "IMG.CR2.xmp" and the "IMG.xmp" it
                // shares. A sidecar naming a file that goes still goes.
                let going: Vec<PathBuf> = std::iter::once(media.path.clone())
                    .chain(followers.iter().cloned())
                    .filter(|p| !is_xmp(p) && !kept.contains(&MediaId::for_path(p)))
                    .collect();
                followers.retain(|p| going.contains(p) || going.contains(&p.with_extension("")));
            }
            found.wanted.extend(kept);
            if followers.is_empty() {
                continue;
            }
            // "+ RAW, XMP", whatever order the folder lists them in.
            followers.sort_by_key(|p| is_xmp(p));
            let paths: Vec<&Path> = std::iter::once(media.path.as_path())
                .chain(followers.iter().map(PathBuf::as_path))
                .collect();
            let (_, extras) = lead_and_extras(&paths);
            found.notes.insert(*id, extras);
            found
                .ids
                .extend(followers.iter().map(|p| MediaId::for_path(p)));
            found.paths.extend(followers);
        }
        found
    }

    /// Takes back the change of one photo listed on the Review step.
    pub(super) fn undo_review_item(&mut self, list: List, item: Item) {
        if self.busy || self.scanning {
            return;
        }
        // Already out of date: it is listed again from scratch anyway.
        let stale = self.review_stale;
        // The list is made again without this photo, where it was scrolled to.
        if matches!(
            (list, item),
            (List::Trash | List::Rotate | List::Rate, Item::Staged { .. })
                | (List::Folder(..), Item::Group(_))
        ) {
            self.plan_rows.hold_scroll();
        }
        match (list, item) {
            (List::Trash, Item::Staged { id, .. }) => {
                // With its RAW, JPEG and XMP companions, as it was marked.
                self.stage_manual([id], crate::core::decisions::Change::Trash(false));
            }
            (List::Rotate, Item::Staged { id, .. }) => {
                crate::core::rotation::set_turns(&mut self.decisions, id, 0);
                self.changed_item(id);
            }
            (List::Rate, Item::Staged { id, .. }) => {
                if let Some(decision) = self.decisions.get_mut(&id) {
                    decision.rating = None;
                    if decision.is_empty() {
                        self.decisions.remove(&id);
                    }
                }
                self.changed_item(id);
            }
            (List::Folder(..), Item::Group(group)) => {
                let ids: HashSet<MediaId> =
                    match self.plan.as_ref().and_then(|plan| plan.actions.get(group)) {
                        Some(Action::Move { files } | Action::FinishMove { files }) => files
                            .iter()
                            .map(|(source, _)| MediaId::for_path(&source.path))
                            .collect(),
                        Some(Action::RenameFile { source, .. }) => {
                            [MediaId::for_path(&source.path)].into_iter().collect()
                        }
                        _ => HashSet::new(),
                    };
                self.keep_organize_items_in_place(&ids);
                return;
            }
            _ => return,
        }
        // The planned moves stay as they are: a Trash mark, rating or turn
        // never changes them. Only the staged choices are listed again, with
        // what Apply compares against, so nothing is planned a second time.
        // A choice staged by anything else still marks the list out of date.
        if !stale && let Some(plan) = self.plan.take() {
            self.review_stale = false;
            self.list_review(plan);
        }
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
            ui.heading(if retry { "Unfinished file operations" } else { "Review changes" });
            ui.add_space(S1);
            ui.add(egui::Label::new(RichText::new(self.review_sentence(&plan, &rows)).font(medium(15.0)).color(INK)).wrap());
            if retry {
                ui.label(RichText::new("Changes that finished are already applied. These are the ones left.").color(MUTED));
            }
            if let Some(output) = &plan.output_root && self.root.as_ref() != Some(output) {
                ui.add(egui::Label::new(RichText::new(format!("Photos go to {}", folder_text(output, Some(output)))).color(MUTED)).truncate())
                    .on_hover_text(output.display().to_string());
            }
            if retry && self.error.is_some() { ui.colored_label(RED, "Some operations need attention. See Operation details."); }
            // Details swap the plain view for every file operation, so the
            // list keeps one height budget. Retries start there.
            let details = egui::CollapsingHeader::new("Operation details").id_salt(("plan_details", retry)).default_open(retry).show(ui, |ui| {
                egui::ScrollArea::vertical().id_salt("plan_explanation").max_height(70.0).show(ui, |ui| {
                    ui.label(&plan.description);
                    if retry {
                        if let Some(error) = &self.error { ui.colored_label(RED, error); }
                    } else if !self.rotation_plan {
                        ui.label("Moves never overwrite existing files.");
                    }
                });
            });
            ui.separator();
            // Account for actual wrapped heading/notice heights and reserve the footer.
            let footer = if retry && matches!(self.after_apply, Some(Destination::Close)) { 230.0 } else { 100.0 };
            let height = (ctx.content_rect().height() - footer - (ui.cursor().top() - top)).clamp(40.0, 520.0);
            if details.openness > 0.0 || rows.is_empty() {
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
            self.review_after_moves = None;
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
                    let reviewed: Vec<&PathBuf> = files.iter().map(|(_, to)| to).collect();
                    let renamed = crate::core::organize::renamed_file_names(&reviewed, edit.stem.as_ref());
                    egui::ScrollArea::both().id_salt("pending_names_preview").max_height(48.).show_rows(ui, ui.text_style_height(&egui::TextStyle::Monospace), files.len(), |ui, rows| {
                        for row in rows {
                            let old = &files[row].1;
                            let name = if edit.stem.is_empty() { old.file_name().unwrap_or_default().to_owned() }
                                else { renamed[row].clone() };
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
