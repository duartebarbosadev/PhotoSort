use super::*;
#[test]
fn metadata_cancel_survives_browse_completion_between_pointer_down_and_up() {
    for width in [640.0, 1440.0] {
        let (_temp, mut app) = fixture();
        app.metadata_run = Some(73);
        app.metadata_progress = Some((1, 65));
        app.browse_run = Some(99);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let frame = |app: &mut PhotoSortApp, events| {
            ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 300.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.metadata_status(ui));
                },
            )
        };
        frame(&mut app, vec![]);
        let output = frame(&mut app, vec![]);
        let rect = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .find(|(_, n)| n.label() == Some("Cancel metadata reading"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
        let pos = egui::pos2(
            ((rect.x0 + rect.x1) / 2.0) as f32,
            ((rect.y0 + rect.y1) / 2.0) as f32,
        );
        for pressed in [true, false] {
            if !pressed {
                app.browse_run = None;
            }
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        assert!(
            app.metadata_run.is_none(),
            "Cancel moved or changed identity when browsing completed at {width} points"
        );
        assert_eq!(app.stats.decodes, 0);
        app.runtime.shutdown();
    }
}
fn settle(app: &mut PhotoSortApp) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.refilter();
        if app.browse_run.is_none() {
            return;
        }
        assert!(Instant::now() < deadline, "Browse worker did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
    let temp = tempfile::tempdir().unwrap();
    for n in 0..65 {
        std::fs::write(
            temp.path().join(format!("{n:03}.jpg")),
            b"metadata-only fixture",
        )
        .unwrap();
    }
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.root = Some(catalog.root.clone());
    app.catalog = Some(Arc::new(catalog));
    (temp, app)
}
#[test]
fn folder_filter_preserves_focus_and_stays_in_cull_without_more_metadata_work() {
    let (_temp, mut app) = fixture();
    app.step = Step::Cull;
    app.active = Some(app.media[12].id);
    settle(&mut app);
    let folders = app.browse_folders.clone();
    app.browse.folder.path = app.root.clone();
    app.browse.folder.recursive = false;
    app.filter_dirty = true;
    settle(&mut app);
    assert_eq!(app.filtered.len(), 65);
    assert_eq!(app.active, Some(app.media[12].id));
    assert!(Arc::ptr_eq(&folders, &app.browse_folders));
    app.browse.folder.path = Some(app.root.as_ref().unwrap().join("missing"));
    app.filter_dirty = true;
    settle(&mut app);
    assert!(app.filtered.is_empty());
    assert_eq!(app.active, None);
    app.set_step(Step::Organize);
    settle(&mut app);
    assert_eq!(app.filtered.len(), 65);
    assert_eq!(app.stats.decodes, 0);
    assert_eq!(app.stats.metadata_reads, 0);
}
#[test]
fn cull_indexes_once_without_decoding_and_keeps_richer_metadata_and_staged_ratings() {
    let (_temp, mut app) = fixture();
    let id = app.media[0].id;
    app.set_step(Step::Cull);
    let run = app.metadata_run;
    assert!(run.is_some());
    app.start_metadata_index();
    assert_eq!(app.metadata_run, run);
    app.metadata.insert(
        id,
        Metadata {
            width: 6000,
            height: 4000,
            rating: 4,
            ..Default::default()
        },
    );
    app.decisions.insert(
        id,
        Decision {
            rating: Some(2),
            ..Default::default()
        },
    );
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.metadata_run.is_some() && Instant::now() < deadline {
        app.poll(&ctx);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.metadata_run.is_none());
    assert_eq!(app.metadata.len(), 65);
    assert_eq!(app.stats.decodes, 0);
    assert_eq!(app.stats.metadata_reads, 65);
    assert_eq!(app.metadata[&id].width, 6000);
    app.browse.rating = RatingFilter::Exactly(2);
    settle(&mut app);
    assert_eq!(app.filtered, vec![0]);
    assert_eq!(app.selected_rating(id), 2);
    app.start_metadata_index();
    assert!(app.metadata_run.is_none());
    assert_eq!(app.stats.metadata_reads, 65);
    app.set_step(Step::Organize);
    settle(&mut app);
    assert_eq!(
        app.filtered.len(),
        65,
        "Cull filters must not leak into guided workflows"
    );
}
#[test]
fn cancelled_metadata_batches_do_not_change_a_different_workflow() {
    let (_temp, mut app) = fixture();
    app.set_step(Step::Cull);
    assert!(app.metadata_run.is_some());
    app.set_step(Step::Organize);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let deadline = Instant::now() + Duration::from_millis(150);
    while Instant::now() < deadline {
        app.poll(&ctx);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.metadata.is_empty());
    assert!(app.metadata_run.is_none());
    assert_eq!(app.stats.decodes, 0);
    app.set_step(Step::Cull);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.metadata_run.is_some() && Instant::now() < deadline {
        app.poll(&ctx);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert_eq!(app.metadata.len(), 65);
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn browse_controls_wrap_at_compact_width_and_filters_keep_visible_focus() {
    let (_temp, mut app) = fixture();
    app.step = Step::Cull;
    for m in &app.media {
        app.metadata.insert(
            m.id,
            Metadata {
                rating: 3,
                ..Default::default()
            },
        );
    }
    let active = app.media[20].id;
    app.active = Some(active);
    app.browse.rating = RatingFilter::Exactly(3);
    app.browse.sort = Sort::Newest;
    settle(&mut app);
    assert_eq!(app.active, Some(active));
    let ctx = egui::Context::default();
    theme_install(&ctx);
    for width in [360.0, 576.0, 960.0] {
        for _ in 0..3 {
            let _ = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        app.browse_controls(ui);
                        assert!(
                            ui.min_rect().right() <= width,
                            "Browse controls overflow at {width}: {:?}",
                            ui.min_rect()
                        );
                    });
                },
            );
        }
    }
    app.browse.rating = RatingFilter::Exactly(5);
    app.filter_dirty = true;
    settle(&mut app);
    assert!(app.filtered.is_empty());
    assert_eq!(app.active, None);
}
#[test]
fn cull_group_filter_and_comparison_do_not_use_other_workflows_cluster_ids() {
    let (_temp, mut app) = fixture();
    app.step = Step::Cull;
    let a = app.media[0].id;
    let b = app.media[1].id;
    let c = app.media[2].id;
    app.cull_group_run = Some(1);
    app.record_analysis_group(1, a, 7);
    app.record_analysis_group(1, b, 7);
    app.groups.clear();
    app.record_analysis_group(2, a, 7);
    app.record_analysis_group(2, c, 7);
    app.active = Some(a);
    app.browse.group = Some(7);
    settle(&mut app);
    assert_eq!(app.filtered, vec![0, 1]);
    assert_eq!(app.browse_groups, vec![7]);
    app.compare_similar();
    assert!(app.comparison.contains(&b));
    assert!(!app.comparison.contains(&c));
    app.set_step(Step::Organize);
    settle(&mut app);
    assert_eq!(app.filtered.len(), 65);
    assert_eq!(app.cull_groups.len(), 2);
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn group_sort_reuses_application_embeddings_and_preserves_selection_and_bookmarks() {
    let (_temp, mut app) = fixture();
    app.step = Step::Cull;
    for (i, m) in app.media.iter().enumerate() {
        app.metadata.insert(
            m.id,
            Metadata {
                captured_ms: Some(i as i64),
                ..Default::default()
            },
        );
        if i < 3 {
            Arc::make_mut(&mut app.cull_groups).insert(m.id, i + 1);
            app.embeddings
                .insert(m.id, Arc::new(vec![3.0 - i as f32, 0.0]));
        }
    }
    let focus = app.media[1].id;
    app.active = Some(focus);
    app.selection.single(focus);
    let original = app.embeddings[&focus].clone();
    assert!(Arc::ptr_eq(
        &original,
        app.browse_item(focus).embedding.as_ref().unwrap()
    ));
    app.browse.group_sort = GroupSort::Similarity;
    settle(&mut app);
    assert_eq!(&app.filtered[..3], &[2, 1, 0]);
    assert_eq!(app.active, Some(focus));
    assert!(app.selection.contains(focus));
    app.checkpoint_bookmarks();
    let saved = &app.saved_views.bookmarks[&Step::Cull];
    assert_eq!(saved.browse.group_sort, GroupSort::Similarity);
    app.browse.group_sort = GroupSort::Time;
    app.filter_dirty = true;
    settle(&mut app);
    assert_eq!(&app.filtered[..3], &[0, 1, 2]);
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    assert!(app.requested.is_empty() && app.analysis_run.is_none() && app.metadata_run.is_none());
}
