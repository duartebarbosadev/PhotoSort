//! One browser selection/action owner shared by Cull's list, icons and grid.
use super::*;
use crate::core::decisions::{self, Change};

pub(super) fn clear_trash_shortcut(key: egui::Key, modifiers: egui::Modifiers) -> bool {
    key == egui::Key::D
        && modifiers.alt
        && !modifiers.command
        && !modifiers.ctrl
        && !modifiers.shift
}

impl PhotoSortApp {
    pub(super) fn clear_trash_menu_item(&mut self, ui: &mut egui::Ui) {
        if ui.add_enabled(self.can_clear_trash_marks(), egui::Button::new("Clear all Trash marks · Alt+D"))
            .on_hover_text("Clears marks across the whole library, including filtered-out and supporting files. Ratings and rotations are kept.")
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
        let pick_before = self.pick_review.comparison().map(|c| c.paths.clone());
        let changed = decisions::clear_trash_marks(&mut self.decisions);
        let count = changed.len();
        self.delete_review.clear_prior_trash_marks();
        self.pick_review.clear_prior_trash_marks();
        self.pending_bookmark = None;
        // A closed preview is a derived snapshot; unfinished recovery stays guarded.
        self.set_review_plan(None);
        self.changed_items(changed);
        if self.step == Step::PickBest
            && pick_before != self.pick_review.comparison().map(|c| c.paths.clone())
        {
            self.sync_pick_view();
        }
        self.set_status(format!(
            "Cleared {count} Trash marks · ratings and rotations kept"
        ));
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
        self.changed_items(changed);
    }
    fn selection_ready(&self) -> bool {
        self.step == Step::Cull && !self.busy && !self.scanning && self.browse_run.is_none()
    }
    fn scene_selection_ready(&self) -> bool {
        self.scene_editing() && !self.busy && !self.scanning && !self.filter_dirty
    }
    pub(super) fn select_library(&mut self, id: MediaId, toggle: bool, extend: bool) {
        if !self.selection_ready() && !self.scene_selection_ready() {
            return;
        }
        self.pending_bookmark = None;
        if self.selection.is_empty()
            && extend
            && let Some(active) = self.active
        {
            self.selection.single(active);
        }
        self.selection
            .select(id, &self.media, &self.filtered, toggle, extend);
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
        self.selection.all(&self.media, &self.filtered);
        if self.active.is_none()
            && let Some(&i) = self.filtered.first()
        {
            self.set_active(self.media[i].id);
        }
    }
    fn clear_library_selection(&mut self) {
        self.selection.clear();
        self.comparison.clear();
        self.active = None;
    }
    fn stage_selection(&mut self, change: Change) {
        if !self.selection_ready() {
            return;
        }
        let ids = self.selection.ordered(&self.media, &self.filtered);
        self.stage_manual(ids, change);
    }
    pub(super) fn compare_selection(&mut self) {
        if (!self.selection_ready() && !self.scene_selection_ready())
            || !(2..=9).contains(&self.selection.len())
        {
            return;
        }
        self.comparison = self.selection.ordered(&self.media, &self.filtered);
        self.review = true;
        if let Some(id) = self
            .active
            .filter(|id| self.comparison.contains(id))
            .or_else(|| self.comparison.first().copied())
        {
            self.set_active(id);
        }
    }
    /// Inline, so it wraps with whatever row it is placed in.
    pub(super) fn selection_controls(&mut self, ui: &mut egui::Ui) {
        {
            ui.label(RichText::new(format!("{} selected", self.selection.len())).color(MUTED));
            ui.add_enabled_ui(self.selection_ready(), |ui| {
                ui.menu_button("Selection", |ui| {
                    if ui.button("Select all visible · Ctrl/Cmd+A").clicked() { self.select_all_visible(); ui.close(); }
                    if ui.button("Clear selection").clicked() { self.clear_library_selection(); ui.close(); }
                    ui.separator();
                    self.selection_actions(ui);
                });
                self.group_controls(ui);
                if ui.add_enabled((2..=9).contains(&self.selection.len()), egui::Button::new("Compare selected"))
                    .on_hover_text("Compare 2–9 selected photos. Larger selections can still be rated, rotated or marked together.").clicked() { self.compare_selection(); }
            });
        }
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
        if self.scene_editing() && !self.review {
            match key {
                egui::Key::A if modifiers.command || modifiers.ctrl => {
                    self.select_all_visible();
                    return true;
                }
                egui::Key::Escape => {
                    self.clear_library_selection();
                    return true;
                }
                _ => return false,
            }
        }
        if self.step != Step::Cull {
            return false;
        }
        let command = modifiers.command || modifiers.ctrl;
        if !self.review {
            match key {
                egui::Key::A if command => {
                    self.select_all_visible();
                    return true;
                }
                egui::Key::Escape => {
                    self.clear_library_selection();
                    return true;
                }
                egui::Key::D if !modifiers.alt => {
                    self.stage_selection(Change::ToggleTrash);
                    return true;
                }
                egui::Key::R => {
                    self.stage_selection(Change::Rotate(if modifiers.alt {
                        2
                    } else if modifiers.shift {
                        3
                    } else {
                        1
                    }));
                    return true;
                }
                egui::Key::Delete | egui::Key::Backspace => {
                    if self.selection_ready() && !self.selection.is_empty() {
                        self.stage_selection(Change::Trash(true));
                        self.review_changes();
                    }
                    return true;
                }
                _ => {}
            }
        }
        if command && !self.selection.is_empty() {
            let rating = match key {
                egui::Key::Num0 => Some(0),
                egui::Key::Num1 => Some(1),
                egui::Key::Num2 => Some(2),
                egui::Key::Num3 => Some(3),
                egui::Key::Num4 => Some(4),
                egui::Key::Num5 => Some(5),
                _ => None,
            };
            if let Some(n) = rating {
                self.stage_selection(Change::Rating(n));
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
#[path = "selection/tests.rs"]
mod tests;
