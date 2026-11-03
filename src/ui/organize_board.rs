//! Organize's main view: the proposed folders on the left, the photos and
//! videos grouped under their destination folder in the middle, the current
//! photo large on the right. Photos are dragged onto folders, folders onto
//! each other; folders are made, renamed and removed in place. Every edit goes
//! into the grouping's folder draft, and nothing changes on disk until the
//! changes are reviewed and applied.
use super::organize::Pending;
use super::*;
use crate::core::organize::Draft;

#[path = "organize_board/arrange.rs"]
mod arrange;
#[path = "organize_board/edits.rs"]
mod edits;
#[path = "organize_board/grid.rs"]
mod grid;
#[path = "organize_board/header.rs"]
mod header;
#[path = "organize_board/navigation.rs"]
mod navigation;
#[path = "organize_board/preview.rs"]
mod preview;
#[path = "organize_board/sidebar.rs"]
mod sidebar;
#[path = "organize_board/state.rs"]
mod state;
pub(super) use sidebar::folder_row;
use state::*;
pub(super) use state::{BOARD_GAP, Board, FolderAction};

#[cfg(test)]
#[path = "organize_board/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "organize_board/multi_folder_tests.rs"]
mod multi_folder_tests;
