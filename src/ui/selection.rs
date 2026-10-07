//! One browser selection/action owner shared by Cull's list, icons and grid.
use super::*;
use crate::core::decisions::{self, Change};

impl PhotoSortApp {
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
    pub(super) fn selection_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            ui.label(format!("{} selected", self.selection.len()));
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
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
        let dir = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        app.media = (0..12)
            .map(|i| Media {
                id: MediaId(i),
                path: dir.path().join(format!("{i:03}.jpg")),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint::default(),
            })
            .collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.catalog = Some(Arc::new(Catalog {
            root: dir.path().into(),
            media: app.media.clone(),
            ..Default::default()
        }));
        app.step = Step::Cull;
        app.active = Some(MediaId(0));
        app.filtered = (0..12).collect();
        (dir, app)
    }
    fn settle(app: &mut PhotoSortApp) {
        let until = Instant::now() + Duration::from_secs(5);
        loop {
            app.refilter();
            if app.browse_run.is_none() {
                break;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn browser_selection_bulk_staging_and_comparison_share_one_filtered_order() {
        let (_dir, mut app) = fixture();
        settle(&mut app);
        app.select_library(MediaId(1), false, false);
        app.select_library(MediaId(4), false, true);
        assert_eq!(app.selection.len(), 4);
        assert_eq!(app.active, Some(MediaId(4)));
        assert!(app.comparison.is_empty() && !app.review);
        app.select_library(MediaId(2), true, false);
        assert_eq!(app.selection.len(), 3);
        app.stage_selection(Change::Rating(5));
        settle(&mut app);
        assert_eq!(app.decisions.len(), 3);
        assert!(!app.decisions.contains_key(&MediaId(2)));
        app.compare_selection();
        assert!(app.review);
        assert_eq!(app.comparison.len(), 3);
        let active = app.active.unwrap();
        app.rating(active, 1);
        assert_eq!(
            app.decisions[&MediaId(1)].rating,
            Some(5),
            "viewer stars target one photo"
        );
        assert_eq!(app.decisions[&active].rating, Some(1));
        app.review = false;
        app.browse.rating = crate::core::browsing::RatingFilter::Exactly(5);
        app.filter_dirty = true;
        settle(&mut app);
        assert_eq!(app.selection.len(), 2);
        app.stage_selection(Change::Trash(true));
        settle(&mut app);
        assert!(app.decisions[&MediaId(1)].rejected);
        assert!(!app.decisions[&active].rejected);
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(app.stats.metadata_reads, 0);
        assert!(app.plan.is_none(), "staging must not start file operations");
    }
    #[test]
    fn keyboard_ranges_select_all_and_explicit_compare_keep_bulk_selection_unbounded() {
        let (_dir, mut app) = fixture();
        settle(&mut app);
        app.navigate_selection(1, true, false);
        assert_eq!(app.selection.len(), 2);
        assert!(app.comparison.is_empty());
        assert!(app.cull_selection_key(egui::Key::A, egui::Modifiers::COMMAND));
        assert_eq!(app.selection.len(), 12);
        app.compare_selection();
        assert!(!app.review);
        app.cull_selection_key(egui::Key::Num3, egui::Modifiers::COMMAND);
        settle(&mut app);
        assert_eq!(app.decisions.len(), 12);
        app.cull_selection_key(egui::Key::R, egui::Modifiers::NONE);
        settle(&mut app);
        assert!(app.decisions.values().all(|d| d.turns == 1));
        app.cull_selection_key(egui::Key::D, egui::Modifiers::NONE);
        settle(&mut app);
        assert!(app.decisions.values().all(|d| d.rejected));
        app.cull_selection_key(egui::Key::Escape, egui::Modifiers::NONE);
        assert!(app.selection.is_empty() && app.active.is_none());
        app.cull_selection_key(egui::Key::D, egui::Modifiers::NONE);
        assert!(
            app.decisions.values().all(|d| d.rejected),
            "cleared selection must not target old focus"
        );
    }
    #[test]
    fn single_review_navigation_rates_current_photo_and_comparison_rates_its_members() {
        let (_dir, mut app) = fixture();
        settle(&mut app);
        app.review = true;
        app.navigate_selection(1, false, false);
        assert_eq!(app.active, Some(MediaId(1)));
        assert_eq!(
            app.selection.ordered(&app.media, &app.filtered),
            [MediaId(1)]
        );
        app.cull_selection_key(egui::Key::Num5, egui::Modifiers::COMMAND);
        settle(&mut app);
        assert_eq!(app.decisions.len(), 1);
        assert_eq!(app.decisions[&MediaId(1)].rating, Some(5));
        app.compare_with(MediaId(3));
        app.cull_selection_key(egui::Key::Num2, egui::Modifiers::COMMAND);
        settle(&mut app);
        assert_eq!(app.decisions[&MediaId(1)].rating, Some(2));
        assert_eq!(app.decisions[&MediaId(3)].rating, Some(2));
        assert!(
            !app.cull_selection_key(egui::Key::D, egui::Modifiers::NONE),
            "review must use focused mark handler"
        );
        app.reject(app.active.unwrap());
        assert!(app.decisions[&MediaId(1)].rejected);
        assert!(!app.decisions[&MediaId(3)].rejected);
    }
    #[test]
    fn selection_does_not_act_on_pending_views_busy_mutations_or_other_workflows() {
        let (_dir, mut app) = fixture();
        settle(&mut app);
        app.select_all_visible();
        app.browse_run = Some(123);
        app.stage_selection(Change::Trash(true));
        app.select_library(MediaId(2), false, false);
        assert_eq!(app.selection.len(), 12);
        assert!(app.decisions.is_empty());
        app.browse_run = None;
        app.busy = true;
        app.stage_selection(Change::Rating(5));
        assert!(app.decisions.is_empty());
        app.busy = false;
        for step in [
            Step::Organize,
            Step::EasyDelete,
            Step::Rotation,
            Step::PickBest,
        ] {
            app.step = step;
            assert!(!app.cull_selection_key(egui::Key::D, egui::Modifiers::NONE));
            app.stage_selection(Change::Trash(true));
            assert!(app.decisions.is_empty());
        }
        app.step = Step::Cull;
        app.set_step(Step::Organize);
        assert!(app.selection.is_empty());
        assert_eq!(app.stats.decodes, 0);
    }
}
