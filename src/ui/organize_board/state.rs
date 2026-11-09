//! The board's state and the small vocabulary every part of it shares:
//! sections, folder rows, the folder actions and the path helpers.
use super::grid::BoardRows;
use super::*;

/// What the board was arranged from; a change rearranges it.
#[derive(Clone, PartialEq)]
pub(in crate::ui::workspace) struct BoardKey {
    pub(super) mode: GroupMode,
    pub(super) epoch: u64,
    pub(super) media: usize,
    pub(super) metadata: usize,
    pub(super) draft: Option<(usize, usize)>,
    pub(super) grouped: [bool; 3],
    pub(super) layout: crate::core::geography::Layout,
    pub(super) faceless: crate::core::preferences::Faceless,
    pub(super) original: bool,
}

/// One destination folder's run of photos in `filtered` order.
pub(in crate::ui::workspace) struct Section {
    /// Destination relative to the output folder; `None` stays in place.
    pub(in crate::ui::workspace) path: Option<String>,
    pub(in crate::ui::workspace) range: std::ops::Range<usize>,
    pub(in crate::ui::workspace) videos: usize,
}

/// A row of the folder tree.
pub(in crate::ui::workspace) struct FolderRow {
    pub(in crate::ui::workspace) path: String,
    pub(in crate::ui::workspace) depth: usize,
    /// Photos and videos directly in this folder, and including subfolders.
    pub(in crate::ui::workspace) direct: usize,
    pub(in crate::ui::workspace) total: usize,
}

/// The folder clicked in the sidebar that the board is scrolling to, eased
/// over a few frames.
pub(in crate::ui::workspace) struct Glide(pub(in crate::ui::workspace) String);

#[derive(Default)]
pub(in crate::ui::workspace) struct Board {
    pub(in crate::ui::workspace) key: Option<BoardKey>,
    pub(in crate::ui::workspace) sections: Vec<Section>,
    pub(in crate::ui::workspace) folders: Vec<FolderRow>,
    /// Items that stay where they are (not part of any proposed folder),
    /// and how many of them are videos.
    pub(in crate::ui::workspace) unplaced: usize,
    pub(in crate::ui::workspace) unplaced_videos: usize,
    /// The folders shown in the middle, chosen in the sidebar; empty shows
    /// every folder. None is inside another.
    pub(in crate::ui::workspace) focus: Vec<String>,
    /// The folder a Shift-click in the sidebar spans from: the last one
    /// clicked without Shift.
    pub(in crate::ui::workspace) folder_anchor: Option<String>,
    /// A folder being renamed in place: (folder, typed name).
    pub(in crate::ui::workspace) editing: Option<(String, String)>,
    pub(in crate::ui::workspace) edit_focus: bool,
    /// The sidebar scrolls the folder being renamed into view once.
    pub(in crate::ui::workspace) reveal_editing: bool,
    /// The folder you are looking at: the active photo's while it is on
    /// screen, otherwise the topmost one shown. `None` before the board is
    /// drawn, or for items staying where they are. The sidebar highlights it.
    pub(in crate::ui::workspace) current: Option<Option<String>>,
    /// A folder just chosen in the sidebar: highlighted until the board
    /// scrolls or the active photo changes from where they were (recorded
    /// on the first frame after the choice).
    pub(in crate::ui::workspace) hold: Option<Option<(f32, Option<MediaId>)>>,
    /// The sidebar scrolls `current` into view once after it changes.
    pub(in crate::ui::workspace) reveal_current: bool,
    /// A folder clicked in the sidebar that the board is scrolling to.
    pub(in crate::ui::workspace) glide: Option<Glide>,
    /// The board's scroll offset and active photo when the sidebar last
    /// followed them: a hand scroll follows the folder at the top, a new
    /// active photo its own folder.
    pub(in crate::ui::workspace) spy: Option<(f32, Option<MediaId>)>,
    /// The board was last scrolled by hand rather than by a new active photo.
    pub(in crate::ui::workspace) scrolled: bool,
    /// The board scrolls back to its top once, after the folders shown change.
    pub(in crate::ui::workspace) to_top: bool,
    /// The sidebar's scroll offset and height in the last frame.
    pub(in crate::ui::workspace) tree_view: (f32, f32),
    /// The folders as they are on disk now, shown while "Show original" or
    /// the backslash key is held; the planned folders come back on release.
    pub(in crate::ui::workspace) original: bool,
    /// "Show original" was held down in the last frame.
    pub(in crate::ui::workspace) original_button: bool,
    /// The folders shown before the original layout was, restored after it.
    pub(in crate::ui::workspace) focus_before: Option<Vec<String>>,
    /// A selection rectangle being dragged from empty space in the board:
    /// where it started (in board coordinates) and the photos selected
    /// before it, kept when Shift or Ctrl/Cmd was held.
    pub(in crate::ui::workspace) marquee: Option<(egui::Pos2, Vec<MediaId>)>,
    /// A photo being renamed in the preview's title: (photo, typed name).
    pub(in crate::ui::workspace) renaming: Option<(MediaId, String)>,
    pub(in crate::ui::workspace) rename_focus: bool,
    /// A folder or photo name field had focus in the last frame.
    pub(in crate::ui::workspace) typing: bool,
    /// Planned changes when the board was last arranged: photos and videos
    /// that move or are renamed, and new folders.
    pub(in crate::ui::workspace) changes: usize,
    /// "Reset changes" is asking for confirmation.
    pub(in crate::ui::workspace) confirm_reset: bool,
    /// Faces: who each folder shows and each person's face.
    pub(in crate::ui::workspace) people: people::People,
    /// The single preview was chosen over the contact sheet for a selection
    /// of this many photos; another selection shows the sheet again.
    pub(in crate::ui::workspace) single: Option<usize>,
    /// The board's search field is open in a narrow header, where it is
    /// otherwise a magnifier button.
    pub(in crate::ui::workspace) searching: bool,
    /// The frame in which the folder sidebar was drawn at the window's edge;
    /// the board then leaves it out.
    pub(in crate::ui::workspace) sidebar_frame: Option<u64>,
    /// What fits beside the board in this window.
    pub(in crate::ui::workspace) layout: BoardLayout,
    /// The preview, when it does not fit beside the board, is open over it.
    pub(in crate::ui::workspace) preview_open: bool,
    /// The folders, when the sidebar does not fit, are open in a popover.
    pub(in crate::ui::workspace) folders_open: bool,
    /// Counts the times the board was arranged, so its row layout knows
    /// when the sections or the photos' shapes and turns may have changed.
    pub(in crate::ui::workspace) arranged: u64,
    /// The board's rows as last laid out; reused while nothing they depend
    /// on has changed.
    pub(in crate::ui::workspace) rows: Option<Arc<BoardRows>>,
}

/// What sits beside Organize's board. As the window narrows the preview
/// gives way first and opens over the board on request; then the folder
/// sidebar folds into a button on the board's header.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::ui::workspace) struct BoardLayout {
    pub sidebar: bool,
    pub preview: bool,
}

impl Default for BoardLayout {
    fn default() -> Self {
        Self {
            sidebar: true,
            preview: true,
        }
    }
}

/// The board keeps at least this width beside the preview: its header and
/// two rows of small tiles.
pub(super) const BOARD_MIN: f32 = 300.0;

/// Beside the sidebar alone the board keeps a little more, so a narrow
/// window gives the photos the room rather than the folder list.
pub(super) const BOARD_MIN_BESIDE_SIDEBAR: f32 = 360.0;

/// The narrowest the preview gets: its controls need about this in two rows.
pub(super) const PREVIEW_MIN: f32 = 330.0;

/// What fits in `room` (the width for the sidebar, board and preview) with a
/// `sidebar` this wide.
pub(in crate::ui::workspace) fn board_layout(room: f32, sidebar: f32) -> BoardLayout {
    let sidebar_fits = room >= sidebar + BOARD_MIN_BESIDE_SIDEBAR;
    BoardLayout {
        sidebar: sidebar_fits,
        preview: sidebar_fits && room >= sidebar + BOARD_MIN + BOARD_GAP + PREVIEW_MIN,
    }
}

/// The space between Organize's panels, and under the bar above them.
pub(in crate::ui::workspace) const BOARD_GAP: f32 = 10.0;

pub(super) const HEADER: f32 = 44.0;

/// The inset around a board tile's photo, as `library::card` draws it.
pub(super) const TILE_INSET: f32 = 6.0;

/// The narrowest a board tile gets, so its name and details stay readable.
pub(super) const MIN_TILE: f32 = 104.0;

pub(super) fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

pub(super) fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}

/// Whether folder `source` can move into `target`: not into itself, a
/// folder inside it, or the parent it is already in.
pub(super) fn can_nest(source: &str, target: &str) -> bool {
    !source.is_empty()
        && !within(target, source)
        && source.rsplit_once('/').map_or("", |(p, _)| p) != target
}

/// A folder being dragged in the sidebar onto another folder.
pub(in crate::ui::workspace) struct FolderDrag(pub(in crate::ui::workspace) String);

/// What a folder's right-click menu, drag or the new-folder button asked for.
pub(in crate::ui::workspace) enum FolderAction {
    /// Shows only this folder (`None`: every folder again).
    ShowOnly(Option<String>),
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
    /// Grouped by faces: the first folder's person is the second's.
    SamePerson(String, String),
}

/// Whether a section in folder `path` (`None`: staying in place) is shown
/// while the `focus` folders are chosen.
pub(super) fn shows(focus: &[String], path: Option<&str>) -> bool {
    focus.is_empty() || path.is_some_and(|path| focus.iter().any(|f| within(path, f)))
}

/// The folders to show, without those inside another one: they are shown
/// with it already.
pub(super) fn outermost(focus: Vec<String>) -> Vec<String> {
    let mut chosen: Vec<String> = Vec::new();
    for folder in focus {
        if !chosen.iter().any(|c| within(&folder, c)) {
            chosen.retain(|c| !within(c, &folder));
            chosen.push(folder);
        }
    }
    chosen
}

/// Whether sidebar folder `folder` holds a section in folder `path`
/// (`None`: staying in place). The library folder, the top row, holds
/// every item.
pub(super) fn in_folder(folder: &str, path: Option<&str>) -> bool {
    folder.is_empty() || path.is_some_and(|path| within(path, folder))
}

pub(super) fn within(path: &str, folder: &str) -> bool {
    folder.is_empty()
        || path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// The heading over photos outside every proposed folder. Until an
/// analysis-based grouping has a result, they are waiting, not staying.
pub(in crate::ui::workspace) fn unplaced_title(pending: Option<Pending>) -> &'static str {
    match pending {
        None => "Not moving",
        Some(Pending::Running) => "Grouping…",
        Some(_) => "Not grouped yet",
    }
}
