//! Application ownership of durable group edits; all layouts use these commands.
use super::*;
use crate::core::manual_groups::{Edit, Resolved};
pub(super) struct Task {
    run: u64,
    scene: bool,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    pub source: Option<u64>,
    record_undo: bool,
}
impl PhotoSortApp {
    pub(super) fn scene_editing(&self) -> bool {
        self.step == Step::Organize && self.group_mode == GroupMode::Visual
    }
    pub(super) fn library_selection_mode(&self) -> bool {
        (self.step == Step::Cull || self.scene_editing()) && !self.review
    }
    fn groups_ready(&self) -> bool {
        (self.step == Step::Cull
            || (self.scene_editing()
                && self.visual_groups.is_some()
                && !self.organize_drafts.contains_key(&GroupMode::Visual)
                && self.organize_editor.is_none()))
            && !self.busy
            && !self.scanning
            && !self.filter_dirty
            && self.browse_run.is_none()
            && self.analysis_run.is_none()
            && self.group_task.is_none()
            && !self.plan_open
            && self.departure.is_none()
            && self.review_notice.is_none()
    }
    pub(super) fn publish_groups(&mut self, groups: Arc<HashMap<MediaId, usize>>) {
        // Restore a group filter by photo identity, not yesterday's AI numbering.
        let anchor = self.browse.group.and_then(|group| {
            self.active
                .filter(|id| self.cull_groups.get(id) == Some(&group))
                .or_else(|| {
                    self.media
                        .iter()
                        .find(|m| self.cull_groups.get(&m.id) == Some(&group))
                        .map(|m| m.id)
                })
        });
        self.cull_groups = groups;
        if self.browse.group.is_some() {
            self.browse.group = anchor.and_then(|id| self.cull_groups.get(&id).copied());
        }
        // Numeric collapsed group keys must not attach to a different AI cohort.
        self.navigator
            .collapsed
            .retain(|key| !matches!(key, crate::core::browsing::tree::Key::Group(_)));
        self.runtime.browsing.groups(self.cull_groups.clone());
        self.filter_dirty = true;
        self.library_reveal = true;
    }
    pub(super) fn resolve_groups(
        &mut self,
        automatic: Arc<HashMap<MediaId, usize>>,
        edit: Edit,
        source: Option<u64>,
        record_undo: bool,
    ) {
        self.resolve_group_edit(false, automatic, edit, source, record_undo);
    }
    fn resolve_group_edit(
        &mut self,
        scene: bool,
        automatic: Arc<HashMap<MediaId, usize>>,
        edit: Edit,
        source: Option<u64>,
        record_undo: bool,
    ) {
        let Some(catalog) = self.catalog.clone() else {
            return;
        };
        self.group_serial += 1;
        let run = self.group_serial;
        let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        match self.runtime.io(IoWork::EditGroups {
            cancelled: cancelled.clone(),
            run,
            catalog,
            automatic,
            manual: if scene {
                self.scene_groups.clone()
            } else {
                self.manual_groups.clone()
            },
            scene,
            edit,
        }) {
            Ok(()) => {
                self.group_task = Some(Task {
                    cancelled,
                    run,
                    scene,
                    source,
                    record_undo,
                });
                self.busy = true;
                self.set_status("Updating photo groups…".into());
            }
            Err(error) => self.error = Some(error),
        }
    }
    pub(super) fn cancel_group_edit(&mut self) {
        if let Some(task) = self.group_task.take() {
            task.cancelled
                .store(true, std::sync::atomic::Ordering::Release);
            self.busy = false;
            if task.source.is_some() {
                self.cancel_analysis();
            }
            self.set_status("Group update cancelled · previous groups retained".into());
        }
    }
    fn edit_groups(&mut self, edit: Edit, record_undo: bool) {
        if !self.groups_ready() {
            return;
        }
        self.pending_bookmark = None;
        let scene = self.scene_editing();
        let automatic = if scene {
            self.automatic_scenes.clone()
        } else {
            self.automatic_groups.clone()
        };
        self.resolve_group_edit(scene, automatic, edit, None, record_undo);
    }
    pub(super) fn finish_group_edit(&mut self, result: Resolved) {
        if self
            .group_task
            .as_ref()
            .is_none_or(|task| task.run != result.run)
        {
            return;
        }
        let task = self.group_task.take().unwrap();
        self.busy = false;
        if task.scene {
            if result.changed {
                self.scene_undo = task.record_undo.then_some(result.previous);
            }
            self.scene_groups = result.manual;
            self.automatic_scenes = result.automatic;
            self.scene_choices = result.choices;
            self.visual_groups = Some(result.groups);
            self.filter_dirty = true;
            if result.changed {
                self.save_session();
            }
            self.set_organize_status(GroupMode::Visual, format!("Scenes updated · {} photos manually grouped · used by the next Easy Delete analysis", self.scene_groups.len()));
            return;
        }
        if task.record_undo && result.changed {
            self.group_undo = Some(result.previous);
        }
        let changed = result.changed;
        if changed && !task.record_undo {
            self.group_undo = None;
        }
        self.manual_groups = result.manual;
        self.automatic_groups = result.automatic;
        self.publish_groups(Arc::new(result.groups));
        if changed {
            self.save_session();
        }
        self.set_workflow_status(
            Step::Cull,
            if changed && self.manual_groups.is_empty() && self.automatic_groups.is_empty() {
                "Manual groups removed · Analyze to build automatic groups".into()
            } else {
                format!(
                    "Groups ready · {} photos manually grouped",
                    self.manual_groups.len()
                )
            },
        );
        if task.source.is_some() && self.pick_run == task.source && self.analysis_run.is_none() {
            self.finish_pick_analysis();
        } else {
            self.start_ready_pick_analysis();
        }
        if self.step == Step::Cull || self.metadata_organize_mode() {
            self.start_metadata_index();
        }
    }
    fn selected_group_members(&self) -> Vec<MediaId> {
        if self.scene_editing() && self.review {
            if self.comparison.is_empty() {
                self.active.into_iter().collect()
            } else {
                self.comparison.clone()
            }
        } else {
            self.selection.ordered(&self.media, &self.filtered)
        }
    }
    fn move_selected_group(&mut self, target: Option<usize>) {
        let mut selected = self.selected_group_members();
        if self.scene_editing() {
            selected.retain(|id| self.automatic_scenes.contains_key(id));
        }
        self.edit_groups(Edit::Move { selected, target }, true);
    }
    fn group_targets(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().max_height(240.).show_rows(
            ui,
            button_row_height(ui),
            if self.scene_editing() {
                self.scene_choices.len()
            } else {
                self.browse_groups.len()
            },
            |ui, rows| {
                for row in rows {
                    let scene = self.scene_editing();
                    let group = if scene {
                        self.scene_choices[row].0
                    } else {
                        self.browse_groups[row]
                    };
                    let label = if scene {
                        let (_, count, example) = self.scene_choices[row];
                        let name = self
                            .index
                            .get(&example)
                            .map(|i| self.media[*i].name())
                            .unwrap_or_default();
                        format!("Scene {group:03} · {count} photos · {name}")
                    } else {
                        self.group_choices.get(&group).map_or_else(
                            || format!("Group {group}"),
                            |choice| {
                                let name = self
                                    .index
                                    .get(&choice.example)
                                    .map(|i| self.media[*i].name())
                                    .unwrap_or_default();
                                format!("Group {group} · {} photos · {name}", choice.count)
                            },
                        )
                    };
                    if ui
                        .add(egui::Button::new(&label).truncate())
                        .on_hover_text(&label)
                        .clicked()
                    {
                        self.move_selected_group(Some(group));
                        ui.close();
                    }
                }
            },
        );
    }
    pub(super) fn group_controls(&mut self, ui: &mut egui::Ui) {
        let scene = self.scene_editing();
        let undo = if scene {
            self.scene_undo.clone()
        } else {
            self.group_undo.clone()
        };
        let empty = if scene {
            self.scene_groups.is_empty()
        } else {
            self.manual_groups.is_empty()
        };
        ui.add_enabled_ui(self.groups_ready(), |ui| {
            ui.menu_button(if scene { "Scenes" } else { "Groups" }, |ui| {
                let selected = self.selected_group_members();
                ui.set_max_width(300.);
                ui.label(if scene { "Scene corrections survive reanalysis and guide the next Easy Delete analysis. Mixed date grouping is independent." } else { "Manual groups stay together after reanalysis. They also guide new Pick Best reviews." });
                ui.label(format!("{} selected", selected.len()));
                ui.label("Photos only. Folder locations are unchanged.");
                ui.separator();
                ui.add_enabled_ui(!selected.is_empty(), |ui| {
                    if ui.button("Move selected to new group").clicked() {
                        self.move_selected_group(None); ui.close();
                    }
                    ui.menu_button("Move selected to existing group", |ui| {
                        ui.label("The target’s current members stay together.");
                        self.group_targets(ui);

                    });
                    if ui.button("Restore automatic grouping for selected").clicked() {
                        let selected = selected.clone();
                        self.edit_groups(Edit::Restore(selected),true); ui.close();
                    }
                });
                ui.separator();
                if ui.add_enabled(undo.is_some(),egui::Button::new("Undo last group edit")).clicked()
                    && let Some(previous) = undo.clone() {
                        self.edit_groups(Edit::Replace(previous),false); ui.close();
                    }
                if ui.add_enabled(!empty,egui::Button::new("Restore all automatic groups")).clicked() {
                    self.edit_groups(Edit::RestoreAll,true); ui.close();
                }
            });
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
        let dir = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        app.media = (0..6)
            .map(|i| Media {
                id: MediaId(i),
                path: dir.path().join(format!("{i}.jpg")),
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
        app.metadata = app
            .media
            .iter()
            .map(|m| (m.id, Metadata::default()))
            .collect();
        app.catalog = Some(Arc::new(Catalog {
            root: dir.path().into(),
            media: app.media.clone(),
            ..Default::default()
        }));
        app.root = Some(dir.path().into());
        app.step = Step::Cull;
        app.automatic_groups = Arc::new(
            app.media
                .iter()
                .map(|m| (m.id, m.id.0 as usize / 2 + 1))
                .collect(),
        );
        app.cull_groups = app.automatic_groups.clone();
        settle(&mut app);
        (dir, app)
    }
    fn settle(app: &mut PhotoSortApp) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let ctx = egui::Context::default();
        loop {
            app.poll(&ctx);
            app.refilter();
            if !app.busy
                && app.group_task.is_none()
                && app.browse_run.is_none()
                && !app.filter_dirty
            {
                break;
            }
            assert!(Instant::now() < deadline, "{:?}", app.error);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.error.is_none(), "{:?}", app.error);
    }
    #[test]
    fn correction_updates_every_layout_and_saved_session_without_image_work_or_losing_decisions() {
        let (dir, mut app) = fixture();
        app.decisions.insert(
            MediaId(2),
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        );
        for layout in crate::core::views::Layout::ALL {
            app.library_layout = layout;
            app.selection.single(MediaId(2));
            app.active = Some(MediaId(2));
            app.move_selected_group(Some(app.cull_groups[&MediaId(0)]));
            assert!(app.busy);
            settle(&mut app);
            assert_eq!(app.cull_groups[&MediaId(0)], app.cull_groups[&MediaId(2)]);
            assert_eq!(app.decisions[&MediaId(2)].rating, Some(5));
        }
        app.runtime.shutdown();
        let saved = Session::load(dir.path()).unwrap();
        assert_eq!(saved.manual_groups.len(), 3);
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        assert!(app.plan.is_none());
    }
    #[test]
    fn undo_reset_and_regrouping_preserve_filtered_focus_and_existing_pick_review() {
        let (_dir, mut app) = fixture();
        app.pick_review.replace(
            vec![crate::core::pick_review::Cluster {
                key: 1,
                ordered: vec![MediaId(0), MediaId(1)],
                scores: HashMap::from([(MediaId(0), 1.), (MediaId(1), 2.)]),
            }],
            &mut app.decisions,
        );
        let pair = app.pick_review.comparison().unwrap().paths.clone();
        app.selection.single(MediaId(0));
        app.active = Some(MediaId(0));
        app.browse.group = Some(1);
        app.filter_dirty = true;
        settle(&mut app);
        app.move_selected_group(None);
        settle(&mut app);
        assert_eq!(app.browse.group, app.cull_groups.get(&MediaId(0)).copied());
        assert_eq!(app.active, Some(MediaId(0)));
        assert_eq!(app.pick_review.comparison().unwrap().paths, pair);
        let undo = app.group_undo.clone().unwrap();
        app.edit_groups(Edit::Replace(undo), false);
        settle(&mut app);
        assert!(app.manual_groups.is_empty());
        assert_eq!(app.browse.group, Some(1));
        app.move_selected_group(None);
        settle(&mut app);
        let regenerated = Arc::new(app.media.iter().map(|m| (m.id, 90)).collect());
        app.resolve_groups(regenerated, Edit::Resolve, None, false);
        settle(&mut app);
        assert_ne!(app.cull_groups[&MediaId(0)], app.cull_groups[&MediaId(1)]);
        app.edit_groups(Edit::RestoreAll, true);
        settle(&mut app);
        assert!(app.manual_groups.is_empty());
        assert_eq!(app.browse.group, Some(90));
    }
    #[test]
    fn edits_reject_pending_views_other_workflows_and_analysis_and_discard_cancelled_results() {
        let (_dir, mut app) = fixture();
        app.selection.single(MediaId(0));
        for step in [
            Step::Organize,
            Step::EasyDelete,
            Step::Rotation,
            Step::PickBest,
        ] {
            app.step = step;
            app.move_selected_group(None);
            assert!(app.group_task.is_none());
        }
        app.step = Step::Cull;
        app.analysis_run = Some(1);
        app.move_selected_group(None);
        assert!(app.group_task.is_none());
        app.analysis_run = None;
        app.filter_dirty = true;
        app.move_selected_group(None);
        assert!(app.group_task.is_none());
        settle(&mut app);
        app.browse_run = Some(44);
        app.move_selected_group(None);
        assert!(app.group_task.is_none());
        app.browse_run = None;
        let before = app.cull_groups.clone();
        app.resolve_groups(Arc::default(), Edit::Resolve, Some(1), false);
        let run = app.group_task.as_ref().unwrap().run;
        let stale = crate::core::manual_groups::edit(
            run,
            Arc::default(),
            Arc::default(),
            &app.media,
            Edit::Resolve,
            || false,
        )
        .unwrap();
        app.cancel_analysis();
        app.finish_group_edit(stale);
        settle(&mut app);
        assert_eq!(app.cull_groups, before);
        app.move_selected_group(None);
        let task = app.group_task.as_ref().unwrap();
        let token = task.cancelled.clone();
        let stale = crate::core::manual_groups::edit(
            task.run,
            app.automatic_groups.clone(),
            app.manual_groups.clone(),
            &app.media,
            Edit::Move {
                selected: vec![MediaId(0)],
                target: None,
            },
            || false,
        )
        .unwrap();
        app.cancel_group_edit();
        assert!(token.load(std::sync::atomic::Ordering::Acquire));
        app.finish_group_edit(stale);
        assert_eq!(app.cull_groups, before);
        assert!(app.manual_groups.is_empty());
    }
    #[test]
    fn fresh_pick_review_uses_corrected_members_after_async_completion() {
        let (_dir, mut app) = fixture();
        app.selection.single(MediaId(2));
        app.move_selected_group(Some(1));
        settle(&mut app);
        app.step = Step::PickBest;
        app.analysis = app
            .media
            .iter()
            .map(|m| {
                (
                    m.id,
                    Analysis {
                        aesthetic: Some(5.),
                        faces: Some(Default::default()),
                        score: 80.,
                        ..Default::default()
                    },
                )
            })
            .collect();
        app.pick_run = Some(42);
        app.analysis_run = None;
        app.resolve_groups(app.automatic_groups.clone(), Edit::Resolve, Some(42), false);
        settle(&mut app);
        assert!(app.pick_run.is_none());
        let cluster = app
            .pick_review
            .tournaments()
            .iter()
            .find(|t| t.cluster.ordered.contains(&MediaId(0)))
            .unwrap();
        assert!(cluster.cluster.ordered.contains(&MediaId(2)));
    }
    #[test]
    fn group_target_picker_virtualizes_large_libraries_at_compact_and_wide_sizes() {
        let (_dir, mut app) = fixture();
        app.browse_groups = (1..=100_000).collect();
        let ctx = egui::Context::default();
        for width in [320., 480., 1200.] {
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.group_targets(ui));
                },
            );
            assert!(
                out.shapes.len() < 180,
                "Only visible group targets may create widgets"
            );
        }
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
    #[test]
    fn real_application_reopen_restores_groups_and_bookmark_without_ai() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..3 {
            image::RgbaImage::from_pixel(4, 4, image::Rgba([i, 0, 0, 255]))
                .save(dir.path().join(format!("{i}.png")))
                .unwrap();
        }
        let catalog = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
        let selected = vec![catalog.media[0].id, catalog.media[1].id];
        let manual = crate::core::manual_groups::edit(
            1,
            Arc::default(),
            Arc::default(),
            &catalog.media,
            Edit::Move {
                selected: selected.clone(),
                target: None,
            },
            || false,
        )
        .unwrap()
        .manual;
        let fingerprints: HashMap<_, _> = catalog
            .media
            .iter()
            .map(|m| (m.id, m.fingerprint))
            .collect();
        Session {
            manual_groups: manual,
            scene_groups: Default::default(),
            decisions: Default::default(),
            fingerprints: fingerprints.clone(),
        }
        .save(&catalog.root)
        .unwrap();
        crate::core::views::Saved {
            step: Some(Step::Cull),
            bookmarks: HashMap::from([(
                Step::Cull,
                crate::core::views::Bookmark {
                    active: Some(selected[0]),
                    selected: selected.clone(),
                    group_anchor: Some(selected[0]),
                    browse: crate::core::browsing::Options {
                        group: Some(900),
                        ..Default::default()
                    },
                    ..Default::default()
                },
            )]),
            fingerprints,
        }
        .save(&catalog.root)
        .unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        app.scan_folder(catalog.root.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            app.poll(&egui::Context::default());
            app.refilter();
            app.restore_bookmark();
            assert!(app.error.is_none(), "{:?}", app.error);
            assert!(
                app.analysis_run.is_none(),
                "Restoring manual membership must not launch AI"
            );
            if !app.scanning
                && !app.busy
                && app.metadata_run.is_none()
                && app.browse_run.is_none()
                && app.pending_bookmark.is_none()
                && !app.filter_dirty
            {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(app.manual_groups.len(), 2);
        assert_eq!(app.filtered.len(), 2);
        assert_eq!(app.active, Some(selected[0]));
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(app.stats.metadata_reads, 3);
        app.runtime.shutdown();
    }
    #[test]
    fn every_workflow_preserves_both_namespaces_on_save_exit_and_folder_reset() {
        for step in Step::ALL {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path().join("library");
            std::fs::create_dir(&root).unwrap();
            for name in ["a.jpg", "b.jpg", "c.jpg"] {
                std::fs::write(root.join(name), b"Not decoded for session ownership").unwrap();
            }
            let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
            let group = |selected| {
                crate::core::manual_groups::edit(
                    1,
                    Default::default(),
                    Default::default(),
                    &catalog.media,
                    Edit::Move {
                        selected,
                        target: None,
                    },
                    || false,
                )
                .unwrap()
                .manual
            };
            let subjects = group(vec![catalog.media[0].id, catalog.media[1].id]);
            let scenes = group(vec![catalog.media[1].id, catalog.media[2].id]);
            Session {
                manual_groups: subjects.clone(),
                scene_groups: scenes.clone(),
                fingerprints: catalog
                    .media
                    .iter()
                    .map(|m| (m.id, m.fingerprint))
                    .collect(),
                ..Default::default()
            }
            .save(&catalog.root)
            .unwrap();
            let mut app = PhotoSortApp::with_runtime(
                Runtime::with_cache(|| {}, dir.path().join("cache")),
                None,
            );
            app.scan_folder(catalog.root.clone());
            let deadline = Instant::now() + Duration::from_secs(10);
            while app.scanning || app.busy {
                app.poll(&egui::Context::default());
                assert!(app.error.is_none(), "{:?}", app.error);
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            app.step = step;
            assert_eq!(app.manual_groups, subjects);
            assert_eq!(app.scene_groups, scenes);
            let intent = app.session_intent();
            assert!(
                intent.fingerprints.is_empty(),
                "Recovery captures only the indexed user intent"
            );
            assert_eq!(intent.scene_groups, scenes);
            app.save_session();
            eframe::App::on_exit(&mut app);
            let saved = Session::load(&catalog.root).unwrap();
            assert_eq!(saved.manual_groups, subjects);
            assert_eq!(saved.scene_groups, scenes);
            assert_eq!(saved.fingerprints.len(), 3);
            assert_eq!(app.stats.decodes, 0);
            // Reset is application-owned too; no previous library's membership
            // may leak into a new or failed scan, even after shutdown.
            app.scan_folder(dir.path().join("other"));
            assert!(app.manual_groups.is_empty() && app.scene_groups.is_empty());
        }
    }
    #[test]
    fn scene_edits_share_worker_save_undo_and_selection_without_touching_subjects() {
        let (dir, mut app) = fixture();
        let subjects = app.cull_groups.clone();
        app.step = Step::Organize;
        app.group_mode = GroupMode::Visual;
        app.automatic_scenes = subjects.clone();
        app.visual_groups = Some((*subjects).clone());
        app.scene_choices = crate::core::manual_groups::group_choices(&subjects, || false).unwrap();
        app.filter_dirty = true;
        settle(&mut app);
        app.select_library(MediaId(0), false, false);
        app.select_library(MediaId(2), true, false);
        assert_eq!(app.selection.len(), 2);
        app.move_selected_group(None);
        assert!(app.busy);
        settle(&mut app);
        let groups = app.visual_groups.as_ref().unwrap();
        assert_eq!(groups[&MediaId(0)], groups[&MediaId(2)]);
        assert_ne!(groups[&MediaId(0)], groups[&MediaId(1)]);
        assert_eq!(app.cull_groups, subjects);
        assert!(app.manual_groups.is_empty());
        assert_eq!(app.scene_groups.len(), 2);
        let undo = app.scene_undo.clone().unwrap();
        app.edit_groups(Edit::Replace(undo), false);
        settle(&mut app);
        assert!(app.scene_groups.is_empty());
        assert_eq!(app.visual_groups.as_ref().unwrap(), &*subjects);
        app.move_selected_group(Some(3));
        settle(&mut app);
        assert_eq!(app.scene_groups.len(), 4);
        app.runtime.shutdown();
        let saved = Session::load(dir.path()).unwrap();
        assert_eq!(saved.scene_groups, app.scene_groups);
        assert!(saved.manual_groups.is_empty());
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        assert!(app.plan.is_none());
    }
    #[test]
    fn scene_edit_cancel_keeps_completed_groups_and_folder_drafts_are_guarded() {
        let (_dir, mut app) = fixture();
        app.step = Step::Organize;
        app.group_mode = GroupMode::Visual;
        app.automatic_scenes = app.automatic_groups.clone();
        app.visual_groups = Some((*app.automatic_scenes).clone());
        app.filter_dirty = true;
        settle(&mut app);
        app.select_library(MediaId(0), false, false);
        app.move_selected_group(None);
        let task = app.group_task.as_ref().unwrap();
        let late = crate::core::manual_groups::edit(
            task.run,
            app.automatic_scenes.clone(),
            app.scene_groups.clone(),
            &app.media,
            Edit::Move {
                selected: vec![MediaId(0)],
                target: None,
            },
            || false,
        )
        .unwrap();
        app.cancel_group_edit();
        app.finish_group_edit(late);
        assert!(app.scene_groups.is_empty());
        assert_eq!(app.visual_groups.as_ref().unwrap(), &*app.automatic_scenes);
        app.organize_drafts.insert(
            GroupMode::Visual,
            crate::core::organize::Draft::from_catalog(
                app.catalog.as_ref().unwrap(),
                HashMap::new(),
            ),
        );
        app.move_selected_group(None);
        assert!(app.group_task.is_none());
        app.organize_drafts.clear();
        app.group_mode = GroupMode::Mixed;
        app.move_selected_group(None);
        assert!(app.group_task.is_none());
        app.group_mode = GroupMode::Visual;
        app.scene_choices = (1..=100_000).map(|g| (g, 1, MediaId(0))).collect();
        let ctx = egui::Context::default();
        for width in [320., 600., 1200.] {
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.group_targets(ui));
                },
            );
            assert!(output.shapes.len() < 180);
        }
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
}
