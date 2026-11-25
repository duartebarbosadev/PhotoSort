//! One browser selection/action owner shared by Cull's grid and strips and
//! Organize's board.
use super::*;
use crate::core::decisions::{self, Change};

pub(super) fn clear_trash_shortcut(key: egui::Key, modifiers: egui::Modifiers) -> bool {
    key == egui::Key::D
        && modifiers.alt
        && !modifiers.command
        && !modifiers.ctrl
        && !modifiers.shift
}

const TRASH_LABELS: [&str; 4] = [
    "Unmark all",
    "Mark all for Trash",
    "Marked for Trash",
    "Trash",
];

/// The Trash button's label and hover: for every selected photo (`all`) or
/// for one photo, marked or not. `several` photos shown can be clicked to
/// pick one.
pub(super) fn trash_button_text(
    all: bool,
    rejected: bool,
    several: bool,
) -> (&'static str, String) {
    let label = TRASH_LABELS[match (all, rejected) {
        (true, true) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (false, false) => 3,
    }];
    let hover = match (all, rejected) {
        (true, _) if several => format!(
            "Every selected photo · D\nClick a photo to {} only that one",
            if rejected { "unmark" } else { "mark" }
        ),
        (true, _) => "Every selected photo · D".to_owned(),
        (false, true) => "Unmark · D".to_owned(),
        (false, false) => "Mark for Trash · D".to_owned(),
    };
    (label, hover)
}
/// The widest Trash label's button width, so switching between them never
/// moves the controls beside it. Without `all`, only the one-photo labels.
pub(super) fn trash_button_width(ui: &egui::Ui, all: bool) -> f32 {
    let font = egui::TextStyle::Button.resolve(ui.style());
    TRASH_LABELS[if all { 0 } else { 2 }..]
        .iter()
        .map(|label| {
            let text = format!("{}  {label}", egui_phosphor::regular::TRASH);
            ui.painter()
                .layout_no_wrap(text, font.clone(), INK)
                .size()
                .x
        })
        .fold(0.0, f32::max)
        + 2.0 * ui.spacing().button_padding.x
}

/// The menu item that clears every Trash mark, with its key.
pub(super) fn clear_trash_marks_label() -> String {
    format!("Clear all Trash marks · {}+D", super::dialogs::ALT)
}

impl PhotoSortApp {
    pub(super) fn clear_trash_menu_item(&mut self, ui: &mut egui::Ui) {
        if ui.add_enabled(self.can_clear_trash_marks(), egui::Button::new(clear_trash_marks_label()))
            .on_hover_text("Across the whole library, including photos hidden by filters. Ratings and rotations stay.")
            .clicked() {
            self.clear_all_trash_marks();
            ui.close();
        }
    }
    pub(super) fn can_clear_trash_marks(&self) -> bool {
        self.root.is_some()
            && self.catalog.is_some()
            && !self.scanning
            && !self.modal_active()
            && !self.close_authorized
            && self.after_apply.is_none()
            && self.retry_plan.is_none()
            && self.decisions.values().any(|decision| decision.rejected)
    }
    pub(super) fn clear_all_trash_marks(&mut self) {
        if !self.can_clear_trash_marks() {
            return;
        }
        let changed = decisions::clear_trash_marks(&mut self.decisions);
        let count = changed.len();
        // Easy Delete and Pick Best stage their suggestions again the next
        // time they open.
        self.delete_review.restore_suggested(Default::default());
        self.pick_review.restore_suggested(Default::default());
        // Starting over: nothing counts as decided by hand any more.
        self.by_hand.clear();
        self.pending_bookmark = None;
        // A closed preview is a derived snapshot; unfinished recovery stays guarded.
        self.set_review_plan(None);
        self.changed_items(changed);
        self.set_status(format!("Cleared {count} Trash marks"));
    }
    pub(super) fn stage_manual(&mut self, ids: impl IntoIterator<Item = MediaId>, change: Change) {
        if self.busy {
            return;
        }
        self.pending_bookmark = None;
        let empty = HashMap::new();
        let supporting = self
            .catalog
            .as_ref()
            .map_or(&empty, |c| &c.supporting_fingerprints);
        let changed = decisions::stage_files(
            &self.media,
            &self.index,
            supporting,
            &mut self.decisions,
            ids,
            change,
        );
        if changed.is_empty() {
            return;
        }
        // A Trash mark set or taken back here is the user's own choice.
        if matches!(change, Change::Trash(_) | Change::ToggleTrash) {
            self.by_hand.extend(changed.iter().copied());
        }
        // So is a turn: Fix Rotation's Accept all leaves it.
        if matches!(change, Change::Rotate(_)) {
            self.rotation_review
                .decided_by_hand(changed.iter().copied());
        }
        self.changed_items(changed);
    }
    pub(super) fn selection_ready(&self) -> bool {
        let step = match self.step {
            Step::Cull => self.browse_run.is_none(),
            Step::Organize => !self.review && !self.filter_dirty,
            _ => false,
        };
        step && !self.busy && !self.scanning
    }
    pub(super) fn scene_selection_ready(&self) -> bool {
        self.scene_editing() && !self.busy && !self.scanning && !self.filter_dirty
    }
    pub(super) fn select_library(&mut self, id: MediaId, toggle: bool, extend: bool) {
        if !self.selection_ready() && !self.scene_selection_ready() {
            return;
        }
        self.pending_bookmark = None;
        self.comparison_focus = None;
        if self.selection.is_empty()
            && extend
            && let Some(active) = self.active
        {
            self.selection.single(active);
        }
        let shown = self.board_shown_order();
        let order = shown.as_deref().unwrap_or(&self.filtered);
        self.selection
            .select(id, &self.media, order, toggle, extend);
        self.comparison.clear();
        let active = if self.selection.contains(id) {
            Some(id)
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
        if let Some(active) = active {
            self.set_active(active);
        } else {
            self.active = None;
        }
    }
    fn select_all_visible(&mut self) {
        if !self.selection_ready() && !self.scene_selection_ready() {
            return;
        }
        let shown = self.board_shown_order();
        let order = shown.as_deref().unwrap_or(&self.filtered);
        self.selection.all(&self.media, order);
        self.comparison_focus = None;
        if self.active.is_none_or(|id| !self.selection.contains(id))
            && let Some(&i) = order.first()
        {
            self.set_active(self.media[i].id);
        }
    }
    fn clear_library_selection(&mut self) {
        self.selection.clear();
        self.comparison.clear();
        self.comparison_focus = None;
        self.active = None;
    }
    fn stage_selection(&mut self, change: Change) {
        // Cull's browser and Organize's board stage batch ratings, turns and
        // marks; the guided steps have no selection of their own.
        if !matches!(self.step, Step::Cull | Step::Organize) || !self.selection_ready() {
            return;
        }
        let ids = self.selection.ordered(&self.media, &self.filtered);
        self.stage_manual(ids, change);
    }
    /// D on several selected photos marks them all for Trash, or unmarks them
    /// all when every one is already marked; a mixed selection never flips
    /// each photo separately.
    pub(super) fn toggle_selection_trash(&mut self) {
        if !self.selection_ready() || self.selection.is_empty() {
            return;
        }
        let (ids, _) = self.selection.snapshot();
        let all_marked = ids
            .iter()
            .all(|id| self.decisions.get(id).is_some_and(|d| d.rejected));
        self.stage_manual(ids, Change::Trash(!all_marked));
    }
    /// Whether the photos shown side by side (or on Organize's contact sheet)
    /// are the selection itself, so D can mark them all together.
    pub(super) fn selection_compared(&self) -> bool {
        if self.selection.len() < 2 {
            return false;
        }
        match self.step {
            Step::Organize => self.library_selection_mode(),
            Step::Cull => {
                self.review
                    && self.comparison.len() == self.selection.len()
                    && self
                        .comparison
                        .iter()
                        .all(|id| self.selection.contains(*id))
            }
            _ => false,
        }
    }
    /// What R, D, Delete and a Ctrl/Cmd+digit rating act on. Selecting
    /// several photos aims them at all of them; clicking one among them
    /// afterwards (or picking it with its digit) aims them at that photo
    /// alone, until Escape or a click on empty space lets go of it.
    pub(super) fn key_target(&self) -> KeyTarget {
        if let Some(id) = self.focused_comparison_photo() {
            return KeyTarget::Picked(id);
        }
        let together = match self.step {
            // Cull's browser always acts on its selection, even one photo.
            Step::Cull if !self.review => !self.selection.is_empty(),
            // Organize's preview and Cull's comparison: one selected photo
            // stays with the canvas, as does a comparison that is not the
            // selection (a similarity group).
            Step::Cull | Step::Organize => self.selection_compared(),
            _ => false,
        };
        if together {
            KeyTarget::Selection
        } else {
            KeyTarget::Canvas
        }
    }
    /// The photo clicked among compared selected photos, while it is shown.
    pub(super) fn focused_comparison_photo(&self) -> Option<MediaId> {
        let id = self.comparison_focus?;
        if !self.selection_compared() || !self.selection.contains(id) {
            return None;
        }
        (self.board_sheet() || self.comparison_ids().contains(&id)).then_some(id)
    }
    /// D beside compared selected photos: the clicked photo alone, otherwise
    /// every selected photo together.
    pub(super) fn trash_compared(&mut self) {
        match self.focused_comparison_photo() {
            Some(id) => self.reject(id),
            None => self.toggle_selection_trash(),
        }
    }
    /// The Trash button next to a preview or comparison: whether it acts on
    /// every selected photo, and whether the photos it acts on are marked.
    pub(super) fn trash_button_state(&self, active: MediaId) -> (bool, bool) {
        if !self.selection_compared() {
            return (false, self.decision(active).rejected);
        }
        if let Some(id) = self.focused_comparison_photo() {
            return (false, self.decision(id).rejected);
        }
        let (ids, _) = self.selection.snapshot();
        (true, ids.iter().all(|id| self.decision(*id).rejected))
    }
    pub(super) fn trash_button_clicked(&mut self, active: MediaId) {
        if self.selection_compared() {
            self.trash_compared();
        } else {
            self.reject(active);
        }
    }
    pub(super) fn compare_selection(&mut self) {
        if (!self.selection_ready() && !self.scene_selection_ready())
            || !(2..=9).contains(&self.selection.len())
        {
            return;
        }
        self.comparison = self.selection.ordered(&self.media, &self.filtered);
        self.comparison_focus = None;
        // Organize compares in its preview panel and keeps the board.
        self.review = self.step != Step::Organize;
        if let Some(id) = self
            .active
            .filter(|id| self.comparison.contains(id))
            .or_else(|| self.comparison.first().copied())
        {
            self.set_active(id);
        }
    }
    /// The shown photos of one similarity group, in view order.
    pub(super) fn group_members(&self, group: usize) -> Vec<MediaId> {
        self.filtered
            .iter()
            .map(|i| self.media[*i].id)
            .filter(|id| self.cull_groups.get(id) == Some(&group))
            .collect()
    }
    /// Compares a whole similarity group (2–9 shown photos) side by side.
    pub(super) fn compare_group(&mut self, group: usize) {
        let members = self.group_members(group);
        if !(2..=9).contains(&members.len()) {
            return;
        }
        self.comparison = members;
        self.comparison_focus = None;
        self.single_view_stash.clear();
        self.review = true;
        if let Some(first) = self.comparison.first().copied() {
            self.set_active(first);
        }
    }
    /// F1: show only the active photo, keeping the comparison to return to.
    pub(super) fn toggle_single_view(&mut self) {
        if self.comparison.len() > 1 {
            self.single_view_stash = std::mem::take(&mut self.comparison);
        } else if self.review && !self.single_view_stash.is_empty() {
            let stash = std::mem::take(&mut self.single_view_stash);
            // Only photos still shown in this view can return.
            self.comparison = stash
                .into_iter()
                .filter(|id| {
                    self.index
                        .get(id)
                        .is_some_and(|i| self.filtered.contains(i))
                })
                .collect();
            if self.comparison.len() < 2 {
                self.comparison.clear();
            }
        }
        self.review = true;
    }
    /// Review from the grid compares a 2–9 photo selection directly.
    pub(super) fn open_review(&mut self) {
        if self.step == Step::Cull
            && !self.review
            && self.library_selection_mode()
            && (2..=9).contains(&self.selection.len())
        {
            self.compare_selection();
        }
        self.review = true;
    }
    /// Inline, so it wraps with whatever row it is placed in.
    pub(super) fn selection_controls(&mut self, ui: &mut egui::Ui) {
        // The enabled scope below is one block, which cannot wrap inside a
        // wrapping row; start a new row when the whole group does not fit.
        let font = egui::TextStyle::Button.resolve(ui.style());
        let pad = 2.0 * ui.spacing().button_padding.x + ui.spacing().item_spacing.x;
        let label = format!("{} selected", self.selection.len());
        // An overflowing bar leaves Compare to the selection menu.
        let compare =
            (2..=9).contains(&self.selection.len()) && self.bar_fit < super::top_bar::Fit::Overflow;
        let menu = format!("{label}  {}", egui_phosphor::regular::CARET_DOWN);
        let needed: f32 = [menu.clone(), "Compare selected".into()]
            .into_iter()
            .map(|text| {
                ui.painter()
                    .layout_no_wrap(text, font.clone(), INK)
                    .size()
                    .x
                    + pad
            })
            .sum();
        wrap_before(ui, needed);
        // Selection, group and batch actions share one menu; comparing is
        // offered directly once it is possible.
        ui.add_enabled_ui(self.selection_ready(), |ui| {
            ui.menu_button(menu, |ui| {
                if ui.button("Select all visible · Ctrl/Cmd+A").clicked() {
                    self.select_all_visible();
                    ui.close();
                }
                if ui.button("Clear selection").clicked() {
                    self.clear_library_selection();
                    ui.close();
                }
                ui.separator();
                self.selection_actions(ui);
            });
            if compare && ui.button("Compare selected").clicked() {
                self.compare_selection();
            }
        });
    }
    pub(super) fn selection_actions(&mut self, ui: &mut egui::Ui) {
        self.group_controls(ui);
        ui.add_enabled_ui(self.selection_ready() && !self.selection.is_empty(), |ui| {
            if ui
                .add_enabled(
                    (2..=9).contains(&self.selection.len()),
                    egui::Button::new("Compare selected"),
                )
                .clicked()
            {
                self.compare_selection();
                ui.close();
            }
            ui.menu_button("Rate selected", |ui| {
                for n in 0..=5 {
                    if ui
                        .button(if n == 0 {
                            "Clear rating".into()
                        } else {
                            format!("{n} stars")
                        })
                        .clicked()
                    {
                        self.stage_selection(Change::Rating(n));
                        ui.close();
                    }
                }
            });
            for (label, change) in [
                ("Mark selected for Trash", Change::Trash(true)),
                ("Unmark selected", Change::Trash(false)),
                ("Rotate selected clockwise", Change::Rotate(1)),
                ("Rotate selected counterclockwise", Change::Rotate(3)),
                ("Rotate selected 180°", Change::Rotate(2)),
            ] {
                if ui.button(label).clicked() {
                    self.stage_selection(change);
                    ui.close();
                }
            }
        });
    }
    pub(super) fn cull_selection_key(
        &mut self,
        key: egui::Key,
        modifiers: egui::Modifiers,
    ) -> bool {
        let command = modifiers.command || modifiers.ctrl;
        match self.step {
            // Organize's folder board selects photos to move or compare them.
            Step::Organize if !self.review => match key {
                egui::Key::A if command => {
                    self.select_all_visible();
                    return true;
                }
                egui::Key::Escape => {
                    // A clicked photo lets go first, then the contact sheet
                    // and a comparison return to the single preview, keeping
                    // the selection.
                    if self.focused_comparison_photo().is_some() {
                        self.comparison_focus = None;
                    } else if self.board_sheet() {
                        self.board.single = Some(self.selection.len());
                    } else if self.active.is_some() && self.comparison_ids().len() > 1 {
                        self.end_board_comparison();
                    } else {
                        self.clear_library_selection();
                    }
                    return true;
                }
                _ => {}
            },
            Step::Cull if !self.review => match key {
                egui::Key::A if command => {
                    self.select_all_visible();
                    return true;
                }
                egui::Key::Escape => {
                    self.clear_library_selection();
                    return true;
                }
                _ => {}
            },
            // A comparison of selected photos lets go of the one clicked
            // first, so the keys below reach them all again.
            Step::Cull => {
                if key == egui::Key::Escape
                    && !modifiers.alt
                    && !command
                    && self.focused_comparison_photo().is_some()
                {
                    self.comparison_focus = None;
                    return true;
                }
            }
            _ => return false,
        }
        self.selection_action_key(key, modifiers)
    }
    /// R, D, Delete and a Ctrl/Cmd+digit rating on the photos
    /// [`Self::key_target`] names; the canvas's photo is left to
    /// `photo_key` and `numeric_key`.
    fn selection_action_key(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        let command = modifiers.command || modifiers.ctrl;
        let change = match key {
            // D marks them all, or unmarks them all when every one is marked.
            egui::Key::D if !modifiers.alt && !command => None,
            egui::Key::R if !command => Some(Change::Rotate(if modifiers.alt {
                2
            } else if modifiers.shift {
                3
            } else {
                1
            })),
            egui::Key::Delete | egui::Key::Backspace if !modifiers.alt && !command => {
                Some(Change::Trash(true))
            }
            _ if command && !modifiers.alt => {
                match super::shortcuts::digit(key).filter(|n| *n <= 5) {
                    Some(n) => Some(Change::Rating(n as u8)),
                    None => return false,
                }
            }
            _ => return false,
        };
        match (self.key_target(), change) {
            (KeyTarget::Canvas, _) => return false,
            (KeyTarget::Picked(id), Some(change)) => self.stage_manual([id], change),
            (KeyTarget::Picked(id), None) => self.reject(id),
            (KeyTarget::Selection, Some(change)) => self.stage_selection(change),
            (KeyTarget::Selection, None) => self.toggle_selection_trash(),
        }
        true
    }
}

/// See [`PhotoSortApp::key_target`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum KeyTarget {
    /// The one photo on the canvas, as when nothing is selected.
    Canvas,
    /// The photo clicked among compared selected photos.
    Picked(MediaId),
    /// Every selected photo.
    Selection,
}

#[cfg(test)]
#[path = "selection/tests.rs"]
mod tests;
