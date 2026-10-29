use super::*;
use crate::test_support::worker_patience;
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
    let deadline = Instant::now() + worker_patience();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    loop {
        let expired = Instant::now() >= deadline;
        app.poll(&ctx);
        app.refilter();
        if !app.busy && app.group_task.is_none() && app.browse_run.is_none() && !app.filter_dirty {
            break;
        }
        assert!(!expired, "{:?}", app.error);
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
    app.pick_review
        .replace(vec![crate::core::pick_review::Cluster::new(
            1,
            vec![MediaId(0), MediaId(1)],
            HashMap::from([(MediaId(0), 1.), (MediaId(1), 2.)]),
        )]);
    let groups = app.pick_review.clusters().to_vec();
    app.selection.single(MediaId(0));
    app.active = Some(MediaId(0));
    app.browse.group = Some(1);
    app.filter_dirty = true;
    settle(&mut app);
    app.move_selected_group(None);
    settle(&mut app);
    assert_eq!(app.browse.group, app.cull_groups.get(&MediaId(0)).copied());
    assert_eq!(app.active, Some(MediaId(0)));
    assert_eq!(app.pick_review.clusters(), groups);
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
    let cluster = app.pick_review.cluster_of(MediaId(0)).unwrap();
    assert!(cluster.ordered.contains(&MediaId(2)));
}
#[test]
fn group_target_picker_virtualizes_large_libraries_at_compact_and_wide_sizes() {
    let (_dir, mut app) = fixture();
    app.browse_groups = (1..=100_000).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    for width in [320., 480., 1200.] {
        let out = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| app.group_targets(ui));
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
        organize: None,
        manual_groups: manual,
        scene_groups: Default::default(),
        decisions: Default::default(),
        fingerprints: fingerprints.clone(),
        suggested: Default::default(),
        pick_suggested: Default::default(),
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
    let deadline = Instant::now() + worker_patience();
    loop {
        let expired = Instant::now() >= deadline;
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
        assert!(!expired);
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
            organize: None,
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
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        app.scan_folder(catalog.root.clone());
        let deadline = Instant::now() + worker_patience();
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
        crate::core::organize::Draft::from_catalog(app.catalog.as_ref().unwrap(), HashMap::new()),
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
    theme_install(&ctx);
    for width in [320., 600., 1200.] {
        let output = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 600.),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| app.group_targets(ui));
            },
        );
        assert!(output.shapes.len() < 180);
    }
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}

#[test]
fn each_group_with_a_choice_marks_its_best_scoring_photo() {
    let groups: HashMap<MediaId, usize> = [(1, 7), (2, 7), (3, 7), (4, 8), (5, 9), (6, 9)]
        .into_iter()
        .map(|(id, g)| (MediaId(id), g))
        .collect();
    let analysis: HashMap<MediaId, crate::core::analysis::Analysis> = [
        (1, 40.0),
        (2, 81.5),
        (3, 60.0),
        (4, 99.0),
        (5, 70.0),
        (6, 70.0),
    ]
    .into_iter()
    .map(|(id, score)| {
        (
            MediaId(id),
            crate::core::analysis::Analysis {
                score,
                ..Default::default()
            },
        )
    })
    .collect();
    let best = super::group_best(&groups, &analysis);
    // Group 8 has one photo, so nothing to choose; group 9 ties on score
    // and resolves to the lower id every time.
    assert_eq!(best, [MediaId(2), MediaId(5)].into_iter().collect());
    assert!(super::group_best(&groups, &HashMap::new()).is_empty());
}

#[test]
fn dropping_photos_on_a_group_moves_them_like_the_groups_menu() {
    let (_dir, mut app) = fixture();
    assert!(app.photo_drag_enabled());
    // Photos 0 and 1 start in group 1; drop photo 0 onto photo 4's group.
    let target = app.cull_groups[&MediaId(4)];
    app.drop_on_group(&super::super::grouping::PhotoDrag(vec![MediaId(0)]), target);
    settle(&mut app);
    // Group numbers are reassigned after an edit; membership is what moved.
    assert_eq!(app.cull_groups[&MediaId(0)], app.cull_groups[&MediaId(4)]);
    assert!(app.manual_groups.contains(MediaId(0)));
    assert_ne!(app.cull_groups[&MediaId(1)], app.cull_groups[&MediaId(4)]);
    // In review, or with nothing grouped yet, tiles are not draggable.
    app.review = true;
    assert!(!app.photo_drag_enabled());
    app.runtime.shutdown();
}

#[test]
fn dragging_a_tile_onto_a_navigator_group_moves_it() {
    use crate::core::browsing::tree::{self, Key, Mode};
    let (_dir, mut app) = fixture();
    app.navigator = tree::State {
        enabled: true,
        mode: Mode::Groups,
        ..Default::default()
    };
    let entries: Vec<_> = app
        .media
        .iter()
        .map(|m| tree::Entry {
            path: &m.path,
            date: None,
            group: app.cull_groups.get(&m.id).copied(),
        })
        .collect();
    let built = tree::build(Mode::Groups, &[], entries.into_iter(), || false).unwrap();
    app.browse_tree = Some(Arc::new(
        Arc::new(built).project(&app.navigator, || false).unwrap(),
    ));
    app.filtered = (0..app.media.len()).collect();
    let target = app.cull_groups[&MediaId(4)];
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut time = 0.0;
    let mut frame = |app: &mut PhotoSortApp, events: Vec<egui::Event>| {
        time += 0.05;
        ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1200.0, 800.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    app.navigator_panel(ui);
                    app.library(ui);
                });
                app.photo_drag_overlay(ui.ctx());
            },
        )
    };
    let mut output = frame(&mut app, vec![]);
    for _ in 0..2 {
        output = frame(&mut app, vec![]);
    }
    let tree = output.platform_output.accesskit_update.unwrap();
    let tile = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("0.jpg"))
        .and_then(|(_, n)| n.bounds())
        .expect("tile 0.jpg");
    let from = egui::pos2(
        ((tile.x0 + tile.x1) / 2.0) as f32,
        ((tile.y0 + tile.y1) / 2.0) as f32,
    );
    let row = ctx
        .data(|d| {
            d.get_temp::<egui::Rect>(egui::Id::new(("test_navigator_node", &Key::Group(target))))
        })
        .expect("group row");
    let to = row.center();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(
        &mut app,
        vec![egui::Event::PointerMoved(from), button(from, true)],
    );
    for step in 1..=10 {
        let pos = from + (to - from) * (step as f32 / 10.0);
        frame(&mut app, vec![egui::Event::PointerMoved(pos)]);
    }
    frame(&mut app, vec![button(to, false)]);
    frame(&mut app, vec![]);
    settle(&mut app);
    assert_eq!(app.cull_groups[&MediaId(0)], app.cull_groups[&MediaId(4)]);
    app.runtime.shutdown();
}
