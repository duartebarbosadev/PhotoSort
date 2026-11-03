//! Choosing what the board shows from the sidebar and selecting whole
//! folders, keeping the selection to the photos shown.
use super::*;

impl PhotoSortApp {
    /// The folder new folders go inside: the one shown, or of several shown
    /// the last one clicked. With every folder shown, the one clicked while
    /// it is still highlighted, otherwise the library folder.
    pub(in crate::ui::workspace) fn board_focus_folder(&self) -> String {
        let focus = &self.board.focus;
        if focus.is_empty() {
            let clicked = self.board.folder_anchor.clone();
            return clicked
                .filter(|path| self.board_highlight().as_ref() == Some(path))
                .unwrap_or_default();
        }
        self.board
            .folder_anchor
            .as_ref()
            .filter(|anchor| focus.contains(anchor))
            .or(focus.last())
            .cloned()
            .unwrap_or_default()
    }
    /// Shows only one folder (`None`: every item), from the top. Its first
    /// photo becomes the current one unless the current photo is already in
    /// it, so the arrow keys walk this folder's photos next.
    pub(in crate::ui::workspace) fn board_select_folder(&mut self, focus: Option<String>) {
        self.board.folder_anchor = focus.clone();
        self.board.glide = None;
        self.board.to_top = true;
        self.board_show_folders(focus.into_iter().collect());
        self.board.current = Some(self.board.focus.first().cloned().or(Some(String::new())));
        self.board.hold = Some(None);
    }
    /// A plain click on a sidebar folder: the board scrolls to it, hiding
    /// nothing; an empty folder leaves the board where it is. A folder that
    /// only some folders shown leave out shows every folder again. Its first
    /// photo becomes the current one unless the current photo is already in
    /// it, so the arrow keys walk on from there.
    pub(in crate::ui::workspace) fn board_jump_to(&mut self, path: &str) {
        self.board.folder_anchor = Some(path.to_owned());
        let focus = &self.board.focus;
        if !shows(focus, Some(path)) && !focus.iter().any(|f| within(f, path)) {
            self.board.focus.clear();
        }
        let focus = &self.board.focus;
        let target = |section: &Section| {
            shows(focus, section.path.as_deref())
                && section.path.as_deref().is_some_and(|p| within(p, path))
        };
        let inside = self.active_section().is_some_and(target);
        let first = self
            .board
            .sections
            .iter()
            .find(|s| target(s))
            .and_then(|s| self.filtered.get(s.range.start))
            .map(|&i| self.media[i].id);
        if !inside && let Some(id) = first {
            self.board_start_at(id);
        }
        self.library_reveal = false;
        self.board.to_top = false;
        self.board.glide = first.is_some().then(|| Glide(path.to_owned()));
        self.board.current = Some(Some(path.to_owned()));
        self.board.hold = Some(None);
    }
    /// A click on a sidebar folder. A plain click scrolls the board to it;
    /// Ctrl/Cmd shows only the folders chosen, adding it or taking it away
    /// again; Shift shows every folder from the last one clicked through
    /// this one, added to those shown with Ctrl/Cmd. The library row shows
    /// every item.
    pub(in crate::ui::workspace) fn board_choose_folder(
        &mut self,
        path: &str,
        modifiers: egui::Modifiers,
    ) {
        let toggle = modifiers.command || modifiers.ctrl;
        if path.is_empty() {
            self.board_select_folder(None);
            return;
        }
        if !(toggle || modifiers.shift) {
            self.board_jump_to(path);
            return;
        }
        let rows = &self.board.folders;
        let row = |target: &str| rows.iter().position(|f| f.path == target);
        let mut focus = self.board.focus.clone();
        let span = self
            .board
            .folder_anchor
            .as_deref()
            .filter(|_| modifiers.shift)
            .and_then(row)
            .zip(row(path));
        if let Some((from, to)) = span {
            if !toggle {
                focus.clear();
            }
            let span = rows[from.min(to)..=from.max(to)].iter();
            focus.extend(span.filter(|f| !f.path.is_empty()).map(|f| f.path.clone()));
        } else if toggle && focus.iter().any(|f| f == path) {
            focus.retain(|f| f != path);
            self.board.folder_anchor = Some(path.to_owned());
        } else {
            if !toggle {
                focus.clear();
            }
            focus.push(path.to_owned());
            self.board.folder_anchor = Some(path.to_owned());
        }
        self.board.glide = None;
        self.board.to_top = true;
        self.board_show_folders(outermost(focus));
        // The folder now highlighted, when only one is shown.
        let shown = match self.board.focus.as_slice() {
            [] => String::new(),
            [one] => one.clone(),
            _ => path.to_owned(),
        };
        self.board.current = Some(Some(shown));
        self.board.hold = Some(None);
    }
    /// Makes `id`, the first photo of the folders just shown or scrolled to,
    /// the current photo and the only one selected.
    fn board_start_at(&mut self, id: MediaId) {
        if self.library_selection_mode() {
            self.selection.single(id);
            self.comparison.clear();
            self.comparison_focus = None;
        }
        self.set_active(id);
    }
    /// Shows these folders (none: every item), as [`Self::board_select_folder`].
    /// A folder hidden leaves none of its photos selected unseen, so a mark,
    /// move or drag never reaches a photo you cannot see.
    fn board_show_folders(&mut self, focus: Vec<String>) {
        let shown = |section: &Section| shows(&focus, section.path.as_deref());
        let inside = self.active_section().is_some_and(shown);
        let first = self
            .board
            .sections
            .iter()
            .find(|s| shown(s))
            .and_then(|s| self.filtered.get(s.range.start))
            .map(|&i| self.media[i].id);
        self.board.focus = focus;
        if !inside {
            match first {
                Some(id) => self.board_start_at(id),
                // Only empty folders shown: no photo stays current unseen,
                // for D, F2 or a rating to act on.
                None => {
                    self.active = None;
                    self.comparison.clear();
                    self.comparison_focus = None;
                    self.swipe = None;
                }
            }
        }
        if !self.board.focus.is_empty() {
            let shown = self.board_focus_positions();
            let order: Vec<usize> = shown.iter().map(|p| self.filtered[*p]).collect();
            self.selection.reconcile(&self.media, &order);
            if self
                .comparison_focus
                .is_some_and(|id| !self.selection.contains(id))
            {
                self.comparison_focus = None;
            }
        }
        self.library_reveal = false;
    }
    /// A click on a folder's header selects every item in it. Ctrl/Cmd adds
    /// them to the selection, or removes them when all are already selected;
    /// Shift selects from the anchor through the folder, as with photos.
    pub(in crate::ui::workspace) fn board_select_section(
        &mut self,
        index: usize,
        modifiers: egui::Modifiers,
    ) {
        let Some(range) = self.board.sections.get(index).map(|s| s.range.clone()) else {
            return;
        };
        let ids: Vec<MediaId> = self
            .filtered
            .get(range.clone())
            .unwrap_or_default()
            .iter()
            .map(|i| self.media[*i].id)
            .collect();
        let toggle = modifiers.command || modifiers.ctrl;
        self.pending_bookmark = None;
        self.comparison.clear();
        self.comparison_focus = None;
        // An empty folder has nothing to add; a plain click still clears.
        if ids.is_empty() {
            if !toggle && !modifiers.shift {
                self.selection.clear();
            }
            return;
        }
        let anchor = if modifiers.shift {
            self.board_anchor_span()
        } else {
            None
        };
        let added = if let Some(anchor) = anchor {
            let span = anchor.start.min(range.start)..anchor.end.max(range.end);
            match self.board_shown_positions() {
                // The span counted in the photos shown only.
                Some(shown) => {
                    let order: Vec<usize> = shown.iter().map(|p| self.filtered[*p]).collect();
                    let at = |position: usize| shown.partition_point(|p| *p < position);
                    let span = at(span.start)..at(span.end);
                    self.selection
                        .select_span(span, &self.media, &order, toggle);
                }
                None => self
                    .selection
                    .select_span(span, &self.media, &self.filtered, toggle),
            }
            false
        } else if toggle {
            self.selection.toggle_group(&ids)
        } else {
            self.selection.replace(ids.iter().copied());
            true
        };
        // The preview follows the folder just added; otherwise it keeps a
        // photo that is still selected.
        let next = if added {
            ids.first().copied()
        } else {
            self.active
                .filter(|id| self.selection.contains(*id))
                .or_else(|| {
                    self.selection
                        .ordered(&self.media, &self.filtered)
                        .first()
                        .copied()
                })
        };
        match next {
            Some(id) => self.set_active(id),
            None => self.active = None,
        }
    }
    /// Where a Shift-click range starts: the anchor photo, or its whole
    /// folder when the anchor came from clicking that folder's header.
    fn board_anchor_span(&self) -> Option<std::ops::Range<usize>> {
        let anchor = self.selection.anchor()?;
        let index = *self.index.get(&anchor)?;
        let at = self.filtered.iter().position(|i| *i == index)?;
        let whole = self
            .board
            .sections
            .iter()
            .find(|s| s.range.start == at)
            .map(|s| s.range.clone())
            .filter(|range| {
                self.filtered[range.clone()]
                    .iter()
                    .all(|i| self.selection.contains(self.media[*i].id))
            });
        Some(whole.unwrap_or(at..at + 1))
    }
    /// Only some folders shown in Organize: the positions in `filtered` of
    /// their photos, in board order.
    pub(in crate::ui::workspace) fn board_shown_positions(&self) -> Option<Vec<usize>> {
        if self.step != Step::Organize || self.board.focus.is_empty() {
            return None;
        }
        Some(self.board_focus_positions())
    }
    /// The positions in `filtered` of the photos in the folders shown.
    fn board_focus_positions(&self) -> Vec<usize> {
        let shown = self
            .board
            .sections
            .iter()
            .filter(|s| shows(&self.board.focus, s.path.as_deref()));
        shown
            .flat_map(|s| s.range.clone())
            .filter(|p| *p < self.filtered.len())
            .collect()
    }
    /// Only some folders shown in Organize: their photos in board order,
    /// which the arrow keys, Shift ranges and Ctrl/Cmd+A keep to, so a photo
    /// of a folder not shown is never selected unseen. `None` (every folder
    /// shown) uses `filtered`.
    pub(in crate::ui::workspace) fn board_shown_order(&self) -> Option<Vec<usize>> {
        let positions = self.board_shown_positions()?;
        Some(positions.iter().map(|p| self.filtered[*p]).collect())
    }
    /// Two to nine selected photos on the board, in board order: the preview
    /// compares them side by side. Larger selections preview the active photo.
    pub(in crate::ui::workspace) fn board_selection_comparison(&self) -> Option<Vec<MediaId>> {
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
    pub(in crate::ui::workspace) fn end_board_comparison(&mut self) {
        self.comparison.clear();
        self.comparison_focus = None;
        if let Some(active) = self.active {
            self.selection.single(active);
        }
        self.focused_slot = 0;
    }
    /// The photos a drag or a menu acts on: the selection when `id` is in it.
    pub(super) fn board_targets(&self, id: MediaId) -> Vec<MediaId> {
        if self.selection.contains(id) {
            self.selection.ordered(&self.media, &self.filtered)
        } else {
            vec![id]
        }
    }
}
