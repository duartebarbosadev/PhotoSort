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
    #[test]
    fn clear_all_trash_does_not_resurrect_marks_from_confirmed_keep_all_reviews() {
        use crate::core::{delete_review, pick_review};
        let (dir, mut app) = fixture();
        app.root = Some(dir.path().into());
        app.review = true;
        for id in [MediaId(0), MediaId(2), MediaId(4)] {
            app.decisions.insert(
                id,
                Decision {
                    rejected: true,
                    rating: Some(4),
                    turns: 1,
                },
            );
        }
        app.delete_review.replace(vec![delete_review::Entry {
            subject: MediaId(0),
            companion: Some(MediaId(1)),
            category: delete_review::Category::Exact,
            reason: "fixture".into(),
        }]);
        app.delete_review
            .toggle(MediaId(0), MediaId(0), &mut app.decisions);
        app.delete_review.confirm(MediaId(0), &mut app.decisions);
        app.pick_review.replace(
            vec![pick_review::Cluster {
                key: 1,
                ordered: vec![MediaId(2), MediaId(3)],
                scores: HashMap::from([(MediaId(2), 0.2), (MediaId(3), 0.8)]),
            }],
            &mut app.decisions,
        );
        app.pick_review.confirm(true, &mut app.decisions);
        assert!(!app.decision(MediaId(0)).rejected && !app.decision(MediaId(2)).rejected);
        let ctx = egui::Context::default();
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::D,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::ALT,
                }],
                ..Default::default()
            },
            |ctx| app.keyboard(ctx),
        );
        assert!(app.decisions.values().all(|d| !d.rejected));
        app.delete_review.reset_all(&mut app.decisions);
        app.pick_review.reset_all(&mut app.decisions);
        assert!(
            app.decisions.values().all(|d| !d.rejected),
            "Reset replayed marks cleared by Alt+D"
        );
        for id in [MediaId(0), MediaId(2), MediaId(4)] {
            assert_eq!(app.decision(id).rating, Some(4));
            assert_eq!(app.decision(id).turns, 1);
        }
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        assert!(app.plan.is_none());
        app.runtime.shutdown();
    }
    fn clear_key(app: &mut PhotoSortApp, ctx: &egui::Context, modifiers: egui::Modifiers) {
        let _ = ctx.run(
            egui::RawInput {
                events: vec![egui::Event::Key {
                    key: egui::Key::D,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers,
                }],
                ..Default::default()
            },
            |ctx| app.keyboard(ctx),
        );
    }
    #[test]
    fn clear_all_trash_shared_action_covers_every_workflow_and_retains_unconfirmed_choices() {
        use crate::core::{delete_review, pick_review};
        for step in Step::ALL {
            let (dir, mut app) = fixture();
            app.root = Some(dir.path().into());
            app.step = step;
            app.media[7].kind = MediaKind::Video;
            Arc::get_mut(app.catalog.as_mut().unwrap())
                .unwrap()
                .supporting_fingerprints
                .insert(MediaId(100), Default::default());
            app.filtered = vec![2];
            app.selection
                .restore([MediaId(2), MediaId(4)], Some(MediaId(2)));
            app.active = Some(MediaId(2));
            app.comparison = vec![MediaId(2), MediaId(3)];
            app.inspection_view.zoom = 2.0;
            app.delete_review.replace(vec![delete_review::Entry {
                subject: MediaId(0),
                companion: Some(MediaId(1)),
                category: delete_review::Category::Exact,
                reason: "fixture".into(),
            }]);
            app.delete_review
                .toggle(MediaId(0), MediaId(0), &mut app.decisions);
            app.pick_review.replace(
                vec![pick_review::Cluster {
                    key: 1,
                    ordered: vec![MediaId(2), MediaId(3)],
                    scores: HashMap::from([(MediaId(2), 0.2), (MediaId(3), 0.8)]),
                }],
                &mut app.decisions,
            );
            app.pick_review.toggle(MediaId(2), &mut app.decisions);
            for id in [MediaId(4), MediaId(7), MediaId(100), MediaId(999)] {
                app.decisions.insert(
                    id,
                    Decision {
                        rejected: true,
                        ..Default::default()
                    },
                );
            }
            app.decisions.get_mut(&MediaId(4)).unwrap().rating = Some(5);
            app.decisions.get_mut(&MediaId(4)).unwrap().turns = 1;
            let catalog = app.catalog.clone().unwrap();
            let selected = app.selection.snapshot();
            let ctx = egui::Context::default();
            clear_key(&mut app, &ctx, egui::Modifiers::ALT);
            assert!(app.decisions.values().all(|d| !d.rejected), "{step:?}");
            assert_eq!(app.decision(MediaId(4)).rating, Some(5));
            assert_eq!(app.decision(MediaId(4)).turns, 1);
            assert_eq!(app.selection.snapshot(), selected);
            assert_eq!(app.active, Some(MediaId(2)));
            assert_eq!(app.comparison, vec![MediaId(2), MediaId(3)]);
            assert_eq!(app.inspection_view.zoom, 2.0);
            assert!(app.delete_review.unfinished(Some(MediaId(0))));
            assert!(app.delete_review.keeps(MediaId(0), MediaId(0)));
            assert!(app.pick_review.unfinished());
            assert!(app.pick_review.comparison().unwrap().keep_all());
            assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
            assert!(
                app.session_snapshot()
                    .decisions
                    .values()
                    .all(|d| !d.rejected)
            );
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
            assert!(
                app.requested.is_empty()
                    && app.analysis_run.is_none()
                    && app.metadata_run.is_none()
                    && app.plan.is_none()
            );
            let dirty = app.dirty;
            app.clear_all_trash_marks();
            assert_eq!(
                app.dirty, dirty,
                "repeated clear must not publish another update"
            );
            app.runtime.shutdown();
        }
    }
    #[test]
    fn clear_all_trash_respects_scanning_dialogs_and_pending_file_plans() {
        for blocked in 0..10 {
            let (dir, mut app) = fixture();
            app.root = Some(dir.path().into());
            app.decisions.insert(
                MediaId(0),
                Decision {
                    rejected: true,
                    rating: Some(4),
                    turns: 1,
                },
            );
            match blocked {
                0 => app.root = None,
                1 => app.catalog = None,
                2 => app.scanning = true,
                3 => app.busy = true,
                4 => app.help = true,
                5 => app.plan_open = true,
                6 => app.browse_filters_open = true,
                7 => app.retry_plan = Some(Default::default()),
                8 => app.close_authorized = true,
                _ => app.after_apply = Some(Destination::Close),
            }
            let before = app.decisions.clone();
            assert!(!app.can_clear_trash_marks());
            app.clear_all_trash_marks();
            clear_key(&mut app, &egui::Context::default(), egui::Modifiers::ALT);
            assert_eq!(app.decisions, before, "guard {blocked}");
            assert!(app.dirty.is_none());
            app.runtime.shutdown();
        }
    }
    #[test]
    fn clear_all_trash_after_keep_reviewing_invalidates_only_the_cached_plan() {
        use super::super::compact_cull_tests::{click_label, fixture};
        let (_dir, mut app) = fixture();
        app.step = Step::Organize;
        let id = app.media[0].id;
        app.decisions.insert(
            id,
            Decision {
                rejected: true,
                rating: Some(4),
                turns: 1,
            },
        );
        app.file_names.insert(id, "retained-name.JPG".into());
        app.set_review_plan(Some(Default::default()));
        app.plan_open = true;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(640., 600.);
        click_label(&mut app, &ctx, size, "Keep reviewing");
        assert!(!app.plan_open && app.plan.is_some());
        assert!(
            app.can_clear_trash_marks(),
            "A closed cached preview blocked clearing marks"
        );
        click_label(&mut app, &ctx, size, "Menu");
        click_label(&mut app, &ctx, size, "Clear all Trash marks · Alt+D");
        assert!(!app.decision(id).rejected);
        assert_eq!(app.decision(id).rating, Some(4));
        assert_eq!(app.decision(id).turns, 1);
        assert!(app.plan.is_none() && !app.plan_open);
        assert_eq!(
            app.file_names.get(&id).map(String::as_str),
            Some("retained-name.JPG")
        );
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        app.runtime.shutdown();
    }
    #[test]
    fn clear_all_trash_shortcut_preserves_text_and_requires_exact_modifiers() {
        for modifiers in [
            egui::Modifiers::ALT | egui::Modifiers::SHIFT,
            egui::Modifiers::ALT | egui::Modifiers::CTRL,
            egui::Modifiers::ALT | egui::Modifiers::COMMAND,
        ] {
            let (dir, mut app) = fixture();
            app.root = Some(dir.path().into());
            app.decisions.insert(
                MediaId(0),
                Decision {
                    rejected: true,
                    ..Default::default()
                },
            );
            clear_key(&mut app, &egui::Context::default(), modifiers);
            assert!(app.decision(MediaId(0)).rejected);
            app.runtime.shutdown();
        }
        let (dir, mut app) = fixture();
        app.root = Some(dir.path().into());
        app.decisions.insert(
            MediaId(0),
            Decision {
                rejected: true,
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        let field = egui::Id::new("clear-test-input");
        let mut text = "unfinished".to_string();
        for _ in 0..2 {
            let _ = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.add(egui::TextEdit::singleline(&mut text).id(field))
                        .request_focus();
                });
            });
        }
        assert!(ctx.wants_keyboard_input());
        clear_key(&mut app, &ctx, egui::Modifiers::ALT);
        assert!(app.decision(MediaId(0)).rejected);
        assert_eq!(text, "unfinished");
        app.runtime.shutdown();
    }
    #[test]
    fn clear_all_trash_reconciles_active_pick_round_without_reanalysis() {
        let (dir, mut app) = fixture();
        app.root = Some(dir.path().into());
        app.step = Step::PickBest;
        app.pick_review.replace(
            vec![crate::core::pick_review::Cluster {
                key: 1,
                ordered: (0..4).map(MediaId).collect(),
                scores: (0..4).map(|i| (MediaId(i), i as f64)).collect(),
            }],
            &mut app.decisions,
        );
        app.pick_review.confirm(false, &mut app.decisions);
        app.pick_review.confirm(false, &mut app.decisions);
        app.sync_pick_view();
        assert!(app.pick_review.current().unwrap().current > 0);
        app.clear_all_trash_marks();
        assert_eq!(app.pick_review.current().unwrap().current, 0);
        assert!(
            app.pick_review
                .comparison()
                .unwrap()
                .paths
                .contains(&app.active.unwrap())
        );
        assert_eq!(app.focused_slot, 0);
        assert!(app.decisions.values().all(|d| !d.rejected));
        app.pick_review.reset_all(&mut app.decisions);
        assert!(app.decisions.values().all(|d| !d.rejected));
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        assert!(app.analysis_run.is_none() && app.requested.is_empty());
        app.runtime.shutdown();
    }
    #[test]
    fn clear_all_trash_menu_clears_hidden_marks_at_compact_and_wide_sizes() {
        use super::super::compact_cull_tests::{click_label, rendered};
        for step in [Step::Organize, Step::Cull] {
            for size in [egui::vec2(640., 600.), egui::vec2(1440., 960.)] {
                let (_dir, mut app) = super::super::compact_cull_tests::fixture();
                app.step = step;
                app.query = "photo-0".into();
                app.filter_dirty = true;
                let ids = [app.media[0].id, app.media[8].id];
                for id in ids {
                    app.decisions.insert(
                        id,
                        Decision {
                            rejected: true,
                            rating: Some(4),
                            turns: 1,
                        },
                    );
                }
                let ctx = egui::Context::default();
                theme_install(&ctx);
                ctx.enable_accesskit();
                click_label(&mut app, &ctx, size, "Menu");
                let nodes = rendered(&mut app, &ctx, size);
                let label = "Clear all Trash marks · Alt+D";
                let rect = nodes
                    .iter()
                    .find(|(name, _)| name == label)
                    .expect("missing clear action")
                    .1;
                assert!(
                    rect.min.x >= 0.
                        && rect.min.y >= 0.
                        && rect.max.x <= size.x
                        && rect.max.y <= size.y
                );
                click_label(&mut app, &ctx, size, label);
                for id in ids {
                    assert!(!app.decision(id).rejected);
                    assert_eq!(app.decision(id).rating, Some(4));
                    assert_eq!(app.decision(id).turns, 1);
                }
                assert!(app.plan.is_none() && !app.plan_open);
                app.runtime.shutdown();
            }
        }
    }
}
