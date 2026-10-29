use super::*;
use crate::test_support::worker_patience;
fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        std::fs::write(dir.path().join(name), name).unwrap();
    }
    let catalog = Arc::new(Catalog::scan(dir.path(), || false, |_| {}).unwrap());
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
    app.root = Some(catalog.root.clone());
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.filtered = (0..3).collect();
    for (i, m) in app.media.iter().enumerate() {
        app.metadata.insert(
            m.id,
            Metadata {
                rating: i as u8,
                ..Default::default()
            },
        );
    }
    app.catalog = Some(catalog);
    app.active = Some(app.media[0].id);
    (dir, app)
}
fn settle(app: &mut PhotoSortApp) {
    let until = Instant::now() + worker_patience();
    loop {
        let expired = Instant::now() >= until;
        app.refilter();
        app.restore_bookmark();
        if !app.filter_dirty && app.browse_run.is_none() && app.pending_bookmark.is_none() {
            break;
        }
        assert!(!expired, "view failed to settle");
        std::thread::sleep(Duration::from_millis(2));
    }
}
#[test]
fn step_views_preserve_filters_selection_scroll_and_zoom_without_pipeline_work() {
    let (_dir, mut app) = fixture();
    let a = app.media[0].id;
    let b = app.media[1].id;
    app.query = "a.jpg".into();
    app.library_offset = 80.0;
    app.set_step(Step::Cull);
    app.query.clear();
    app.browse.rating = crate::core::browsing::RatingFilter::Exactly(1);
    app.browse.sort = crate::core::browsing::Sort::Newest;
    app.library_layout = crate::core::views::Layout::List;
    app.review = false;
    settle(&mut app);
    app.selection.single(b);
    app.set_active(b);
    app.library_offset = 20.0;
    app.inspection_view.zoom = 2.0;
    app.set_step(Step::Organize);
    settle(&mut app);
    assert_eq!(app.active, Some(a));
    assert_eq!(app.query, "a.jpg");
    assert_eq!(app.library_restore_offset, Some(80.0));
    app.set_step(Step::Cull);
    settle(&mut app);
    assert_eq!(app.active, Some(b));
    assert!(app.selection.contains(b));
    assert_eq!(app.selection.len(), 1);
    assert_eq!(app.filtered.len(), 1);
    assert_eq!(app.library_layout, crate::core::views::Layout::List);
    assert!(!app.review);
    assert_eq!(app.inspection_view.zoom, 2.0);
    assert_eq!(app.library_restore_offset, Some(20.0));
    assert!(app.requested.is_empty() && app.textures.is_empty());
    assert!(app.analysis_run.is_none() && app.metadata_run.is_none());
}
#[test]
fn delayed_restoration_preserves_saved_focus_and_explicit_navigation_takes_over() {
    let (_dir, mut app) = fixture();
    let b = app.media[1].id;
    let c = app.media[2].id;
    app.step = Step::Cull;
    app.saved_views.bookmarks.insert(
        Step::Cull,
        Bookmark {
            active: Some(b),
            selected: vec![b],
            ..Default::default()
        },
    );
    app.prepare_bookmark();
    app.metadata_run = Some(100);
    app.refilter();
    let until = Instant::now() + worker_patience();
    while app.browse_run.is_some() {
        app.refilter();
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(2));
    }
    app.restore_bookmark();
    assert!(app.pending_bookmark.is_some());
    assert_eq!(app.active, Some(b));
    app.checkpoint_bookmarks();
    assert_eq!(app.saved_views.bookmarks[&Step::Cull].active, Some(b));
    app.select_library(c, false, false);
    assert!(app.pending_bookmark.is_none());
    app.metadata_run = None;
    app.restore_bookmark();
    assert_eq!(app.active, Some(c));
    assert!(app.requested.is_empty());
    app.prepare_bookmark();
    app.scan_folder(app.root.clone().unwrap());
    assert!(app.pending_bookmark.is_none() && app.saved_views.bookmarks.is_empty());
}
#[test]
fn failed_restore_preserves_checkpoint_until_explicit_navigation_or_retry() {
    let (_dir, mut app) = fixture();
    let b = app.media[1].id;
    let c = app.media[2].id;
    app.step = Step::Cull;
    let saved = Bookmark {
        active: Some(b),
        selected: vec![b],
        scroll: 123.0,
        ..Default::default()
    };
    app.saved_views.bookmarks.insert(Step::Cull, saved.clone());
    app.prepare_bookmark();
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.runtime.events = rx;
    tx.send(crate::core::jobs::Envelope {
        epoch: app.runtime.epoch(),
        event: Event::Error("Temporary analysis failure".into()),
    })
    .unwrap();
    app.poll(&egui::Context::default());
    app.library_offset = 0.0;
    app.restore_bookmark();
    app.checkpoint_bookmarks();
    assert!(app.resume_suspended && app.pending_bookmark.is_some());
    assert_eq!(app.saved_views.bookmarks[&Step::Cull], saved);
    app.prepare_bookmark();
    assert!(!app.resume_suspended);
    settle(&mut app);
    assert_eq!(app.active, Some(b));
    app.select_library(c, false, false);
    app.checkpoint_bookmarks();
    assert_eq!(app.saved_views.bookmarks[&Step::Cull].active, Some(c));
}
#[test]
fn empty_selection_survives_restoration_and_missing_focus_has_a_fallback() {
    let (_dir, mut app) = fixture();
    app.step = Step::Cull;
    app.saved_views
        .bookmarks
        .insert(Step::Cull, Bookmark::default());
    app.prepare_bookmark();
    settle(&mut app);
    assert!(app.active.is_none() && app.selection.is_empty());
    app.saved_views
        .bookmarks
        .get_mut(&Step::Cull)
        .unwrap()
        .missing_focus = true;
    app.prepare_bookmark();
    settle(&mut app);
    assert_eq!(app.active, Some(app.media[0].id));
}
#[test]
fn empty_guided_bookmark_focuses_new_results_without_restarting_analysis() {
    for step in [Step::EasyDelete, Step::PickBest, Step::Rotation] {
        for has_results in [false, true] {
            let (_dir, mut app) = fixture();
            let ids: Vec<_> = app.media.iter().map(|m| m.id).collect();
            app.step = step;
            app.saved_views.bookmarks.insert(
                step,
                Bookmark {
                    review: true,
                    rotation_queue_only: step == Step::Rotation,
                    ..Default::default()
                },
            );
            app.prepare_bookmark();
            // The saved library previously had no reviews. A later completed
            // analysis may produce a pair or suggestion after the scan.
            app.resume_started = true;
            app.view_data_ready.insert(step);
            if has_results {
                match step {
                    Step::EasyDelete => {
                        app.delete_review
                            .replace(vec![crate::core::delete_review::Entry {
                                subject: ids[1],
                                companion: Some(ids[2]),
                                category: crate::core::delete_review::Category::Exact,
                                tag: "Tag".into(),
                                keep_why: String::new(),
                                eyes: Vec::new(),
                                extra: Vec::new(),
                                reason: "identical".into(),
                            }])
                    }
                    Step::PickBest => app.pick_review.replace(
                        vec![crate::core::pick_review::Cluster {
                            key: 1,
                            ordered: ids.clone(),
                            scores: ids.iter().map(|id| (*id, 1.0)).collect(),
                        }],
                        &mut app.decisions,
                    ),
                    Step::Rotation => app.rotation_review.replace_suggestions(vec![(ids[1], 1)]),
                    _ => unreachable!(),
                }
            }
            app.refilter();
            app.restore_bookmark();
            // Easy Delete and Fix Rotation wait for the user to pick a card.
            assert_eq!(
                app.active.is_some(),
                has_results && step == Step::PickBest,
                "{step:?}"
            );
            if let Some(active) = app.active {
                assert!(app.comparison_ids().contains(&active));
            }
            assert!(app.pending_bookmark.is_none());
            assert!(app.decisions.is_empty());
            assert!(app.analysis_run.is_none() && app.metadata_run.is_none());
            assert!(app.requested.is_empty() && app.textures.is_empty());
        }
    }
}
#[test]
fn cancelling_deferred_analysis_stays_cancelled_and_preserves_the_saved_view() {
    let (_dir, mut app) = fixture();
    app.step = Step::PickBest;
    app.saved_views
        .bookmarks
        .insert(Step::PickBest, Bookmark::default());
    app.prepare_bookmark();
    app.analysis_run = Some(100);
    app.pick_run = Some(100);
    app.cancel_analysis();
    for _ in 0..3 {
        app.restore_bookmark();
        app.checkpoint_bookmarks();
        assert!(app.analysis_run.is_none() && app.pick_run.is_none());
        assert!(app.resume_suspended && app.pending_bookmark.is_some());
    }
    app.analyze(); // Explicit retry, using the same application analysis owner.
    assert!(!app.resume_suspended && app.analysis_run.is_some());
    app.cancel_analysis();
}
#[test]
fn discard_then_exit_preserves_the_later_pair_without_replaying_confirmations() {
    let (_dir, mut app) = fixture();
    let ids: Vec<_> = app.media.iter().map(|m| m.id).collect();
    let clusters = vec![crate::core::pick_review::Cluster {
        key: 1,
        ordered: ids.clone(),
        scores: ids
            .iter()
            .enumerate()
            .map(|(i, id)| (*id, i as f64))
            .collect(),
    }];
    app.pick_review
        .replace(clusters.clone(), &mut app.decisions);
    app.step = Step::PickBest;
    app.view_data_ready.insert(Step::PickBest);
    app.sync_pick_view();
    app.confirm_pick(false);
    app.pick_focus = true;
    let pair = app.pick_review.comparison().unwrap().paths.clone();
    assert!(pair.contains(&ids[2]));
    let active = app.active;
    app.discard_staged_choices();
    app.checkpoint_bookmarks(); // The actual exit and step-departure paths checkpoint again.
    assert!(app.decisions.is_empty());
    assert_eq!(app.saved_views.bookmarks[&Step::PickBest].comparison, pair);
    app.pick_review.replace(clusters, &mut app.decisions);
    app.prepare_bookmark();
    settle(&mut app);
    assert_eq!(app.pick_review.comparison().unwrap().paths, pair);
    assert_eq!(app.active, active);
    assert!(app.pick_focus && app.decisions.is_empty());
    assert!(!app.pick_review.comparison().unwrap().confirmed);
}
#[test]
fn guided_views_restore_position_and_pair_without_confirmations_or_decisions() {
    let (_dir, mut app) = fixture();
    let ids: Vec<_> = app.media.iter().map(|m| m.id).collect();
    app.pick_review.replace(
        vec![crate::core::pick_review::Cluster {
            key: 1,
            ordered: ids.clone(),
            scores: ids
                .iter()
                .enumerate()
                .map(|(i, id)| (*id, i as f64))
                .collect(),
        }],
        &mut app.decisions,
    );
    app.view_data_ready.insert(Step::PickBest);
    app.step = Step::PickBest;
    app.saved_views.bookmarks.insert(
        Step::PickBest,
        Bookmark {
            active: Some(ids[2]),
            comparison: vec![ids[1], ids[2]],
            pick_focus: true,
            review: true,
            ..Default::default()
        },
    );
    app.prepare_bookmark();
    settle(&mut app);
    assert_eq!(app.active, Some(ids[2]));
    assert!(app.pick_focus);
    assert_eq!(app.comparison_ids(), vec![ids[2]]);
    assert!(!app.pick_review.comparison().unwrap().confirmed && app.decisions.is_empty());
    app.delete_review
        .replace(vec![crate::core::delete_review::Entry {
            subject: ids[1],
            companion: Some(ids[2]),
            category: crate::core::delete_review::Category::Near,
            tag: "Tag".into(),
            keep_why: String::new(),
            eyes: Vec::new(),
            extra: Vec::new(),
            reason: String::new(),
        }]);
    app.view_data_ready.insert(Step::EasyDelete);
    app.saved_views.bookmarks.insert(
        Step::EasyDelete,
        Bookmark {
            active: Some(ids[2]),
            delete_current: Some(ids[1]),
            review: true,
            ..Default::default()
        },
    );
    app.set_step(Step::EasyDelete);
    settle(&mut app);
    assert_eq!(app.active, Some(ids[2]));
    assert_eq!(app.delete_current, Some(ids[1]));
    // Restoring the view changes no decision: opening Easy Delete only
    // stages its suggestion, the copy to Trash and its keeper kept.
    assert!(app.decision(ids[1]).rejected && !app.decision(ids[2]).rejected);
    assert_eq!(app.decisions.values().filter(|d| !d.is_empty()).count(), 1);
    let staged = app.decisions.clone();
    app.saved_views.bookmarks.insert(
        Step::Rotation,
        Bookmark {
            active: Some(ids[2]),
            review: true,
            ..Default::default()
        },
    );
    app.set_step(Step::Rotation);
    settle(&mut app);
    assert_eq!(app.active, Some(ids[2]));
    assert!(app.decisions == staged && app.requested.is_empty());
}
