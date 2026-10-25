use super::*;
use crate::core::{
    operations::{Action, Report},
    organize::Draft,
};

fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
    fixture_in("Before")
}
fn fixture_in(folder: &str) -> (tempfile::TempDir, PhotoSortApp) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join(folder)).unwrap();
    // Deliberately invalid images: departure and file planning must not decode.
    std::fs::write(root.join(folder).join("a.jpg"), b"original bytes").unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.join("cache")), None);
    app.root = Some(root.clone());
    app.output = root.display().to_string();
    app.media = catalog.media.clone();
    app.catalog = Some(Arc::new(catalog));
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.active = app.media.first().map(|m| m.id);
    (dir, app)
}
fn edit(app: &mut PhotoSortApp) {
    let mut draft = Draft::from_catalog(app.catalog.as_ref().unwrap(), app.organize_labels());
    draft.rename_folder("Before", "After").unwrap();
    app.organize_drafts.insert(app.group_mode, draft);
}
/// How long a test waits for the file worker before calling it hung. Waits end
/// as soon as the worker reports, so this only bounds a failure. On Windows each
/// file replacement may retry for up to 5 s (`core::replace`), and shared CI
/// runners can pause every thread for seconds, so CI gets a much longer bound.
fn worker_patience() -> Duration {
    Duration::from_secs(if std::env::var_os("CI").is_some() {
        120
    } else {
        15
    })
}
fn settle(app: &mut PhotoSortApp) {
    let until = Instant::now() + worker_patience();
    loop {
        // Read the clock before polling: a pause after the deadline still gets
        // one more look at the worker's results before the test fails.
        let expired = Instant::now() >= until;
        app.poll(&egui::Context::default());
        if !app.busy && !app.scanning && app.metadata_run.is_none() {
            break;
        }
        assert!(!expired, "worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
}
#[test]
fn folder_trash_discard_preserves_unrelated_organization_intent() {
    let (_dir, mut app) = fixture();
    let id = app.media[0].id;
    app.file_names.insert(id, "Keep".into());
    app.retry_plan = Some(Plan::default());
    app.discard_remaining_operations();
    assert_eq!(app.file_names[&id], "Keep");
    assert!(app.retry_plan.is_none() && app.dirty.is_some());
    app.retry_plan = Some(Plan::default());
    assert!(
        !app.has_unsaved_plan_extras(),
        "Trash recovery checkpoints include kept Organize intent"
    );
}
#[cfg(feature = "metadata-native")]
#[test]
fn embedded_rotation_plan_keeps_its_destination_and_unapplied_rating_after_reload() {
    let (dir, mut app) = fixture();
    let path = dir.path().join("Before/a.jpg");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata/mixed.jpg"),
        &path,
    )
    .unwrap();
    let catalog = Catalog::scan(app.root.as_ref().unwrap(), || false, |_| {}).unwrap();
    app.media = catalog.media.clone();
    app.catalog = Some(Arc::new(catalog));
    let id = app.media[0].id;
    app.step = Step::Rotation;
    app.preferences.metadata_writes = crate::core::metadata::WritePolicy::EmbedWhenSupported;
    app.decisions.insert(
        id,
        Decision {
            rating: Some(2),
            ..Default::default()
        },
    );
    app.rotation_review.ensure(id, 1);
    app.rotation_review.confirm(id);
    app.review_rotations();
    settle(&mut app);
    assert!(app.plan_open && app.rotation_plan, "{:?}", app.error);
    assert!(matches!(
        app.plan.as_ref().unwrap().actions[0],
        Action::Metadata {
            target: crate::core::metadata::WriteTarget::Embedded,
            ..
        }
    ));
    app.preferences.metadata_writes = crate::core::metadata::WritePolicy::SidecarOnly;
    app.apply_reviewed_plan();
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.decisions[&id].rating, Some(2));
    let meta = crate::core::metadata::read(&path);
    assert_eq!((meta.rating, meta.orientation), (4, 3));
    assert!(!crate::core::metadata::sidecar(&path).exists());
    assert_eq!(app.rotation_review.pending_count(), 0);
}
fn smoke_frame(app: &mut PhotoSortApp, ctx: &egui::Context, now: Instant) -> egui::FullOutput {
    ctx.run_test(egui::RawInput::default(), |ui| {
        let ctx = &ui.ctx().clone();
        {
            let requested = app.smoke_close_requested_at(ctx, now);
            app.handle_close_request(ctx, requested);
        }
    })
}
#[test]
fn smoke_exit_starts_on_first_frame_and_requests_normal_close_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None)
            .with_smoke_test(Duration::from_millis(200));
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let first = ctx.run_test(egui::RawInput::default(), |ui| {
        app.update_workspace(ui, false)
    });
    assert!(!app.close_authorized);
    assert!(
        first.viewport_output[&egui::ViewportId::ROOT].repaint_delay <= Duration::from_millis(200)
    );
    let deadline = app.smoke_exit.as_ref().and_then(|s| s.deadline).unwrap();
    assert!(!app.smoke_close_requested_at(&ctx, deadline - Duration::from_millis(100)));
    assert!(!app.close_authorized);
    std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
    let last = ctx.run_test(egui::RawInput::default(), |ui| {
        app.update_workspace(ui, false)
    });
    assert!(app.close_authorized && app.smoke_exit.is_none());
    assert!(
        last.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
    );
    assert!(app.media.is_empty() && app.analysis_run.is_none());
    assert_eq!(app.stats.decodes, 0);
    assert_eq!(app.stats.metadata_reads, 0);
}
#[test]
fn smoke_exit_preserves_staged_changes_in_every_workflow_and_cancel_is_final() {
    for step in Step::ALL {
        let (dir, app) = fixture();
        let mut app = app.with_smoke_test(Duration::from_millis(200));
        app.step = step;
        let id = app.media[0].id;
        app.decisions.insert(
            id,
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let start = Instant::now();
        smoke_frame(&mut app, &ctx, start);
        smoke_frame(&mut app, &ctx, start + Duration::from_millis(300));
        assert!(!app.close_authorized);
        assert!(matches!(app.departure, Some(Destination::Close)));
        assert_eq!(app.decisions[&id].rating, Some(4));
        app.departure = None; // The existing dialog's Cancel action.
        smoke_frame(&mut app, &ctx, start + Duration::from_secs(10));
        assert!(!app.close_authorized && app.departure.is_none());
        assert_eq!(
            std::fs::read(dir.path().join("Before/a.jpg")).unwrap(),
            b"original bytes"
        );
    }
}
#[test]
fn smoke_exit_does_not_interrupt_active_file_work_or_apply_a_retry_plan() {
    for busy in [true, false] {
        let (_dir, app) = fixture();
        let mut app = app.with_smoke_test(Duration::from_millis(200));
        app.busy = busy;
        if !busy {
            app.retry_plan = Some(Plan::default());
        }
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let start = Instant::now();
        smoke_frame(&mut app, &ctx, start);
        smoke_frame(&mut app, &ctx, start + Duration::from_millis(300));
        assert!(!app.close_authorized);
        assert!(app.applying_plan.is_none());
        assert_eq!(app.busy, busy);
        if !busy {
            assert!(app.plan_open && app.retry_plan.is_some());
        }
    }
}
fn close_request(app: &mut PhotoSortApp, native: bool) -> egui::FullOutput {
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let mut input = egui::RawInput::default();
    if !native {
        input
            .viewports
            .entry(egui::ViewportId::ROOT)
            .or_default()
            .events
            .push(egui::ViewportEvent::Close);
    }
    ctx.run_test(input, |ui| {
        let ctx = &ui.ctx().clone();
        app.handle_close_request(ctx, native)
    })
}
#[test]
fn native_quit_and_window_close_share_staged_change_guards_in_every_workflow() {
    for step in Step::ALL {
        for native in [false, true] {
            let (_dir, mut app) = fixture();
            app.step = step;
            let id = app.media[0].id;
            app.decisions.insert(
                id,
                Decision {
                    rating: Some(4),
                    ..Default::default()
                },
            );
            let output = close_request(&mut app, native);
            assert!(matches!(app.departure, Some(Destination::Close)));
            assert!(!app.close_authorized);
            assert_eq!(app.decisions[&id].rating, Some(4));
            let commands = &output.viewport_output[&egui::ViewportId::ROOT].commands;
            assert!(
                !commands
                    .iter()
                    .any(|c| matches!(c, egui::ViewportCommand::Close))
            );
            assert_eq!(
                commands
                    .iter()
                    .any(|c| matches!(c, egui::ViewportCommand::CancelClose)),
                !native
            );
            // Cancelling the prompt retains the decision; a later native request
            // opens it again rather than leaving termination authorized.
            app.departure = None;
            close_request(&mut app, true);
            assert!(matches!(app.departure, Some(Destination::Close)));
            assert!(!app.close_authorized);
        }
    }
}
#[test]
fn native_quit_cannot_interrupt_file_operations_or_bypass_open_plans() {
    let (_dir, mut app) = fixture();
    app.busy = true;
    close_request(&mut app, true);
    assert!(!app.close_authorized && app.departure.is_none());
    app.busy = false;
    app.plan_open = true;
    close_request(&mut app, true);
    assert!(!app.close_authorized && app.departure.is_none());
    app.plan_open = false;
    app.retry_plan = Some(Plan::default());
    close_request(&mut app, true);
    assert!(!app.close_authorized && app.plan_open);
    assert!(matches!(app.after_apply, Some(Destination::Close)));
    app.discard_remaining_operations();
    assert!(app.close_authorized);
}
#[test]
fn native_quit_preserves_unfinished_rotation_and_routes_organize_drafts() {
    let (dir, mut app) = fixture();
    let id = app.media[0].id;
    app.step = Step::Rotation;
    app.rotation_review.ensure(id, 1);
    app.rotation_review.rotate(id, 1);
    close_request(&mut app, true);
    assert!(app.review_notice == Some(Step::Rotation) && !app.close_authorized);
    app.review_notice = None;
    app.rotation_review = Default::default();
    app.step = Step::Organize;
    edit(&mut app);
    close_request(&mut app, true);
    settle(&mut app);
    assert!(matches!(app.organize_departure, Some(Destination::Close)));
    assert!(!app.close_authorized && app.plan.is_some());
    app.resolve_organize_departure(OrganizeResolution::Cancel);
    assert!(!app.close_authorized && app.organize_changes_pending());
    close_request(&mut app, true);
    settle(&mut app);
    app.resolve_organize_departure(OrganizeResolution::Discard);
    assert!(app.close_authorized);
    assert_eq!(
        std::fs::read(dir.path().join("Before/a.jpg")).unwrap(),
        b"original bytes"
    );
    assert!(!dir.path().join("After").exists());
}
#[test]
fn clean_native_quit_authorizes_the_normal_window_shutdown_path() {
    let (_dir, mut app) = fixture();
    let output = close_request(&mut app, true);
    assert!(app.close_authorized);
    assert!(
        output.viewport_output[&egui::ViewportId::ROOT]
            .commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
    );
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn unchanged_library_leaves_without_planning_or_analysis() {
    let (_dir, mut app) = fixture();
    app.request_departure(Destination::Step(Step::Cull));
    assert!(app.step == Step::Cull);
    assert!(app.plan.is_none() && !app.busy && app.analysis_run.is_none());
}
// Windows cannot create these names.
#[cfg(unix)]
#[test]
fn existing_folder_names_with_reserved_characters_are_not_changes() {
    for folder in [
        "Archive/toEdit/23:Dec/DCIM/10051129",
        "?*\"<>|",
        "trailing. ",
    ] {
        let (_dir, mut app) = fixture_in(folder);
        assert_eq!(app.group_mode, GroupMode::Current);
        assert_eq!(app.organize_labels()[&app.media[0].id], folder);
        assert!(!app.organize_changes_pending(), "{folder}");
        app.request_departure(Destination::Step(Step::Cull));
        assert!(app.step == Step::Cull && app.plan.is_none(), "{folder}");
    }
}
#[test]
fn cancel_preserves_edits_and_discard_leaves_without_file_mutation() {
    let (dir, mut app) = fixture();
    edit(&mut app);
    let before = app.organize_drafts.clone();
    app.request_departure(Destination::Step(Step::Cull));
    settle(&mut app);
    assert!(app.step == Step::Organize && !app.plan_open);
    assert!(
        app.plan
            .as_ref()
            .unwrap()
            .actions
            .iter()
            .any(|a| matches!(a, Action::Move { .. }))
    );
    app.resolve_organize_departure(OrganizeResolution::Cancel);
    assert!(app.organize_drafts == before);
    assert!(app.organize_departure.is_none() && app.after_apply.is_none());
    app.request_departure(Destination::Step(Step::Cull));
    settle(&mut app);
    app.resolve_organize_departure(OrganizeResolution::Discard);
    assert!(app.step == Step::Cull && app.group_mode == GroupMode::Current);
    assert!(app.organize_drafts.is_empty() && app.analysis_run.is_none());
    app.set_step(Step::Organize);
    assert!(
        !app.organize_changes_pending(),
        "Discarded edits must not return"
    );
    assert_eq!(
        std::fs::read(dir.path().join("Before/a.jpg")).unwrap(),
        b"original bytes"
    );
    assert!(!dir.path().join("After").exists());
}
#[test]
fn generated_plan_requires_resolution_and_discard_preserves_other_review_marks() {
    let (_dir, mut app) = fixture();
    app.set_group_mode(GroupMode::Date);
    settle(&mut app);
    assert!(app.organize_grouping_ready());
    let id = app.media[0].id;
    app.decisions.insert(
        id,
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    app.request_departure(Destination::Step(Step::Cull));
    settle(&mut app);
    assert!(app.organize_departure.is_some());
    app.resolve_organize_departure(OrganizeResolution::Discard);
    assert!(app.step == Step::Organize && app.departure.is_some());
    assert_eq!(app.decisions[&id].rating, Some(4));
    assert!(app.group_mode == GroupMode::Current);
}
#[test]
fn failed_plan_can_be_discarded_without_trapping_close() {
    let (dir, mut app) = fixture();
    edit(&mut app);
    let output = dir.path().join("not-a-directory");
    std::fs::write(&output, b"obstruction").unwrap();
    app.output = output.display().to_string();
    app.request_departure(Destination::Close);
    settle(&mut app);
    assert!(app.error.is_some() && app.organize_departure.is_some());
    app.resolve_organize_departure(OrganizeResolution::Review);
    assert!(!app.plan_open && !app.close_authorized);
    app.resolve_organize_departure(OrganizeResolution::Discard);
    assert!(app.close_authorized && app.error.is_none());
    assert!(dir.path().join("Before/a.jpg").exists());
}
#[test]
fn case_only_file_edit_uses_shared_review_and_preserves_staged_rating_after_reload() {
    let (_dir, mut app) = fixture();
    let root = app.root.clone().unwrap();
    let old_id = app.media[0].id;
    app.decisions.insert(
        old_id,
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    app.file_names.insert(old_id, "A".into());
    assert!(app.organize_changes_pending());
    app.organize();
    settle(&mut app);
    assert!(app.plan_open && app.error.is_none(), "{:?}", app.error);
    assert!(matches!(
        app.plan.as_ref().unwrap().actions.as_slice(),
        [Action::RenameFile { .. }]
    ));
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut output = None;
    for _ in 0..3 {
        output = Some(ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.plan_review_dialog(ctx)
            },
        ));
    }
    let tree = output.unwrap().platform_output.accesskit_update.unwrap();
    // The plain view names the folder once and the photo by its names.
    for label in [
        "1 photo in Before gets a new name".to_string(),
        "a.jpg → A.jpg".to_string(),
    ] {
        assert!(
            tree.nodes
                .iter()
                .any(|(_, n)| n.value().or(n.label()) == Some(label.as_str())),
            "{label}"
        );
    }
    app.apply_reviewed_plan();
    settle(&mut app);
    assert!(
        app.error.is_none() && app.retry_plan.is_none(),
        "{:?}",
        app.error
    );
    let new_id = MediaId::for_path(&root.join("Before/A.jpg"));
    assert_ne!(old_id, new_id);
    assert_eq!(app.media[0].id, new_id);
    assert_eq!(app.decisions[&new_id].rating, Some(4));
    assert_eq!(app.stats.decodes, 0);
    assert_eq!(app.stats.metadata_reads, 0);
    assert_eq!(
        std::fs::read(root.join("Before/A.jpg")).unwrap(),
        b"original bytes"
    );
    let mut saved = Session::load(&root).unwrap();
    saved.reconcile(&app.media);
    assert_eq!(saved.decisions[&new_id].rating, Some(4));
    assert!(!app.organize_changes_pending());
}
#[test]
fn case_only_folder_edit_uses_shared_plan_and_preserves_staged_rating_after_reload() {
    let (_dir, mut app) = fixture();
    let root = app.root.clone().unwrap();
    let old_id = app.media[0].id;
    app.decisions.insert(
        old_id,
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let mut draft = Draft::from_catalog(app.catalog.as_ref().unwrap(), app.organize_labels());
    draft.rename_folder("Before", "BEFORE").unwrap();
    app.organize_drafts.insert(app.group_mode, draft);
    assert!(app.organize_changes_pending());
    app.organize();
    settle(&mut app);
    assert!(app.plan_open && app.error.is_none(), "{:?}", app.error);
    assert!(matches!(
        app.plan.as_ref().unwrap().actions.as_slice(),
        [Action::RenameDirectory(_)]
    ));
    app.apply_reviewed_plan();
    settle(&mut app);
    assert!(
        app.error.is_none() && app.retry_plan.is_none(),
        "{:?}",
        app.error
    );
    let new_id = MediaId::for_path(&root.join("BEFORE/a.jpg"));
    assert_ne!(old_id, new_id);
    assert_eq!(app.media[0].id, new_id);
    assert_eq!(app.decisions[&new_id].rating, Some(4));
    assert_eq!(app.stats.decodes, 0);
    assert_eq!(
        std::fs::read(root.join("BEFORE/a.jpg")).unwrap(),
        b"original bytes"
    );
    let mut session = Session::load(&root).unwrap();
    session.reconcile(&app.media);
    assert_eq!(session.decisions[&new_id].rating, Some(4));
    assert!(!app.organize_changes_pending());
}
#[test]
fn reviewed_apply_uses_worker_and_navigates_only_after_catalog_reload() {
    let (dir, mut app) = fixture();
    edit(&mut app);
    app.request_departure(Destination::Step(Step::Cull));
    settle(&mut app);
    app.resolve_organize_departure(OrganizeResolution::Review);
    assert!(app.plan_open && app.after_apply.is_some() && app.step == Step::Organize);
    assert!(dir.path().join("Before/a.jpg").exists());
    app.runtime
        .io(IoWork::Apply(app.plan.take().unwrap()))
        .unwrap();
    app.plan_open = false;
    app.busy = true;
    settle(&mut app);
    assert!(app.step == Step::Cull && app.after_apply.is_none());
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(
        std::fs::read(dir.path().join("After/a.jpg")).unwrap(),
        b"original bytes"
    );
    assert!(app.media[0].path.ends_with("After/a.jpg"));
}
#[test]
fn failed_or_cancelled_apply_does_not_continue_departure() {
    for cancelled in [false, true] {
        let (_dir, mut app) = fixture();
        app.after_apply = Some(Destination::Step(Step::Cull));
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let original = std::mem::replace(&mut app.runtime.events, rx);
        tx.send(crate::core::jobs::Envelope {
            epoch: app.runtime.epoch(),
            event: Event::Applied(Report {
                cancelled,
                errors: if cancelled {
                    vec![]
                } else {
                    vec!["source changed".into()]
                },
                ..Default::default()
            }),
        })
        .unwrap();
        app.poll(&egui::Context::default());
        app.runtime.events = original;
        settle(&mut app);
        assert!(app.step == Step::Organize && app.after_apply.is_none());
    }
}
#[test]
fn departure_modal_remains_usable_during_analysis_and_discard_rejects_late_results() {
    let (_dir, mut app) = fixture();
    edit(&mut app);
    app.analysis_run = Some(41);
    app.organize_run = Some((41, GroupMode::Mixed));
    app.request_departure(Destination::Step(Step::Cull));
    settle(&mut app);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(640.0, 600.0),
        )),
        ..Default::default()
    };
    let output = ctx.run_test(input, |ui| {
        let ctx = &ui.ctx().clone();
        app.organize_departure_dialog(ctx)
    });
    assert!(
        !output.shapes.is_empty(),
        "Background analysis must not hide resolution choices"
    );
    app.resolve_organize_departure(OrganizeResolution::Discard);
    assert!(app.analysis_run.is_none() && app.organize_run.is_none());
    let (tx, rx) = std::sync::mpsc::sync_channel(2);
    app.runtime.events = rx;
    for event in [
        Event::MixedGroups {
            run: 41,
            labels: HashMap::new(),
        },
        Event::AnalysisDone {
            run: 41,
            stats: Default::default(),
        },
    ] {
        tx.send(crate::core::jobs::Envelope {
            epoch: app.runtime.epoch(),
            event,
        })
        .unwrap();
    }
    app.poll(&ctx);
    assert!(app.step == Step::Cull && !app.plan_open && !app.busy);
    assert!(app.mixed_labels.is_none());
}
#[test]
fn exact_empty_plan_continues_without_a_resolution_prompt() {
    let (_dir, mut app) = fixture();
    // A lexically different spelling triggers conservative prediction, but
    // the worker resolves it to the same actual folder and finds no moves.
    app.output = app.root.as_ref().unwrap().join(".").display().to_string();
    app.organize_departure = Some(Destination::Step(Step::Cull));
    app.organize();
    settle(&mut app);
    assert!(app.step == Step::Cull && !app.plan_open && app.organize_departure.is_none());
}
fn add_second_photo(app: &mut PhotoSortApp) {
    let root = app.root.clone().unwrap();
    std::fs::write(root.join("Before/b.jpg"), b"second original").unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    app.media = catalog.media.clone();
    app.catalog = Some(Arc::new(catalog));
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
}
#[test]
fn partial_move_reload_retains_retry_and_staged_ratings_on_both_photos() {
    let (_dir, mut app) = fixture();
    add_second_photo(&mut app);
    let root = app.root.clone().unwrap();
    for media in &app.media {
        app.decisions.insert(
            media.id,
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
    }
    edit(&mut app);
    app.organize();
    settle(&mut app);
    std::fs::create_dir(root.join("After")).unwrap();
    std::fs::write(root.join("After/b.jpg"), b"late collision").unwrap();
    app.apply_reviewed_plan();
    settle(&mut app);
    assert!(app.error.is_some() && app.retry_plan.is_some());
    assert_eq!(app.decisions.len(), 2);
    let moved = MediaId::for_path(&root.join("After/a.jpg"));
    let failed = MediaId::for_path(&root.join("Before/b.jpg"));
    assert_eq!(app.decisions[&moved].rating, Some(4));
    assert_eq!(app.decisions[&failed].rating, Some(4));
    assert!(
        !app.can_edit_folders(),
        "Resolve exact failed operations before making a competing plan"
    );
    std::fs::remove_file(root.join("After/b.jpg")).unwrap();
    app.request_departure(Destination::Step(Step::Cull));
    assert!(app.plan_open && app.after_apply.is_some());
    app.apply_reviewed_plan();
    settle(&mut app);
    assert!(app.retry_plan.is_none() && app.error.is_none());
    assert_eq!(app.decisions.len(), 2);
    assert!(
        app.departure.is_some(),
        "Moving files must not implicitly apply or discard their ratings"
    );
    assert!(!root.join("Before").exists());
    assert_eq!(std::fs::read_dir(root.join("After")).unwrap().count(), 2);
}
#[test]
fn external_output_retry_preserves_earlier_moves_and_existing_destination_marks() {
    let (_dir, mut app) = fixture();
    add_second_photo(&mut app);
    let original_root = app.root.clone().unwrap();
    let target = tempfile::tempdir().unwrap();
    let output = target.path().canonicalize().unwrap();
    std::fs::write(output.join("existing.jpg"), b"existing photo").unwrap();
    let existing = Catalog::scan(&output, || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    Session {
        organize: None,
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: HashMap::from([(
            existing.id,
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        )]),
        fingerprints: HashMap::from([(existing.id, existing.fingerprint)]),
    }
    .save(&output)
    .unwrap();
    for media in &app.media {
        app.decisions.insert(
            media.id,
            Decision {
                rating: Some(3),
                ..Default::default()
            },
        );
    }
    let focus = app.media.iter().find(|m| m.name() == "a.jpg").unwrap().id;
    app.active = Some(focus);
    app.saved_views.bookmarks.insert(
        Step::Cull,
        crate::core::views::Bookmark {
            active: Some(focus),
            selected: vec![focus],
            ..Default::default()
        },
    );
    app.saved_views.capture_fingerprints(&app.media, &app.index);
    app.output = output.display().to_string();
    app.organize();
    settle(&mut app);
    std::fs::create_dir(output.join("Before")).unwrap();
    std::fs::write(output.join("Before/b.jpg"), b"conflict").unwrap();
    app.apply_reviewed_plan();
    settle(&mut app);
    assert_eq!(
        app.root.as_ref(),
        Some(&original_root),
        "Stay with failed source work until resolved"
    );
    assert!(app.retry_plan.is_some());
    std::fs::remove_file(output.join("Before/b.jpg")).unwrap();
    app.review_changes();
    app.apply_reviewed_plan();
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert_eq!(app.root.as_ref(), Some(&output));
    assert_eq!(
        app.saved_views.bookmarks[&Step::Cull].active,
        Some(MediaId::for_path(&output.join("Before/a.jpg")))
    );
    assert_eq!(app.media.len(), 3);
    assert_eq!(
        app.decisions.len(),
        3,
        "Loaded decisions: {:?}; media: {:?}; saved: {:?}",
        app.decisions,
        app.media,
        Session::load(&output).unwrap().fingerprints
    );
    assert_eq!(app.decisions[&existing.id].rating, Some(5));
    for name in ["a.jpg", "b.jpg"] {
        let id = MediaId::for_path(&output.join("Before").join(name));
        assert_eq!(app.decisions[&id].rating, Some(3));
    }
    let mut saved = Session::load(&output).unwrap();
    saved.reconcile(&app.media);
    assert_eq!(saved.decisions.len(), 3);
}
#[test]
fn abandoning_retry_preserves_completed_files_and_does_not_replay_the_failed_plan() {
    let (_dir, mut app) = fixture();
    edit(&mut app);
    app.organize();
    settle(&mut app);
    let root = app.root.clone().unwrap();
    std::fs::create_dir(root.join("After")).unwrap();
    std::fs::write(root.join("After/a.jpg"), b"collision").unwrap();
    app.apply_reviewed_plan();
    settle(&mut app);
    app.request_departure(Destination::Step(Step::Cull));
    assert!(app.plan_open && app.retry_plan.is_some());
    app.discard_remaining_operations();
    assert!(app.busy && app.retry_plan.is_some());
    settle(&mut app);
    assert!(app.step == Step::Cull && app.retry_plan.is_none() && !app.plan_open);
    assert!(root.join("Before/a.jpg").exists());
    assert_eq!(
        std::fs::read(root.join("After/a.jpg")).unwrap(),
        b"collision"
    );
    app.set_step(Step::Organize);
    assert!(!app.organize_changes_pending());
}
#[test]
fn worker_error_retains_the_inflight_plan_and_explicit_discard_clears_its_draft() {
    let (_dir, mut app) = fixture();
    edit(&mut app);
    app.organize();
    settle(&mut app);
    app.applying_plan = app.plan.take();
    app.plan_open = false;
    app.busy = true;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.runtime.events = rx;
    tx.send(crate::core::jobs::Envelope {
        epoch: app.runtime.epoch(),
        event: Event::Error("Unable to open operation journal".into()),
    })
    .unwrap();
    app.poll(&egui::Context::default());
    assert!(!app.busy && app.retry_plan.is_some() && app.applying_plan.is_none());
    assert!(!app.organize_drafts.is_empty());
    app.request_departure(Destination::Step(Step::Cull));
    app.discard_remaining_operations();
    assert!(app.step == Step::Cull && app.organize_drafts.is_empty());
    app.set_step(Step::Organize);
    assert!(!app.organize_changes_pending());
    assert!(app.root.as_ref().unwrap().join("Before/a.jpg").exists());
}
#[test]
fn folder_departure_is_deferred_until_discard() {
    let (_dir, mut app) = fixture();
    let next = tempfile::tempdir().unwrap();
    let next_root = next.path().canonicalize().unwrap();
    let old_root = app.root.clone();
    edit(&mut app);
    app.request_departure(Destination::Folder(next_root.clone()));
    settle(&mut app);
    assert_eq!(app.root, old_root);
    app.resolve_organize_departure(OrganizeResolution::Discard);
    settle(&mut app);
    assert_eq!(app.root.as_ref(), Some(&next_root));
    assert!(app.organize_drafts.is_empty());
}
fn failed_saved_plan(app: &mut PhotoSortApp) -> PathBuf {
    let root = app.root.clone().unwrap();
    let destination = root.join("Sorted/a.jpg");
    std::fs::create_dir(root.join("Sorted")).unwrap();
    std::fs::write(&destination, b"unrelated destination").unwrap();
    app.plan = Some(Plan {
        actions: vec![
            Action::CreateDirectory {
                path: root.join("Sorted"),
            },
            Action::Move {
                files: vec![(
                    crate::core::operations::Source::capture(root.join("Before/a.jpg")).unwrap(),
                    destination.clone(),
                )],
            },
        ],
        output_root: Some(root),
        ..Default::default()
    });
    app.apply_reviewed_plan();
    settle(app);
    assert!(
        app.retry_plan.is_some() && app.recovery_ticket.is_some(),
        "{:?}",
        app.error
    );
    assert_eq!(app.retry_plan.as_ref().unwrap().actions.len(), 1);
    destination
}
#[test]
fn unfinished_plan_survives_application_restart_and_reuses_reviewed_destinations() {
    let (_dir, mut app) = fixture();
    let root = app.root.clone().unwrap();
    let old = app.media[0].id;
    app.decisions.insert(
        old,
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let destination = failed_saved_plan(&mut app);
    let ticket = app.recovery_ticket.clone();
    let original_error = app.error.clone();
    app.runtime.shutdown();
    drop(app);
    let mut reopened =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.join("cache")), None);
    reopened.scan_folder(root.clone());
    settle(&mut reopened);
    assert_eq!(reopened.recovery_ticket, ticket);
    assert_eq!(reopened.error, original_error);
    assert_eq!(reopened.decisions[&old].rating, Some(4));
    assert!(
        reopened.retry_plan.is_some() && !reopened.plan_open,
        "Restart must not apply automatically"
    );
    assert_eq!(
        std::fs::read(&destination).unwrap(),
        b"unrelated destination"
    );
    std::fs::remove_file(&destination).unwrap();
    assert!(reopened.review_remaining_operations());
    reopened.apply_reviewed_plan();
    settle(&mut reopened);
    assert!(
        reopened.retry_plan.is_none() && reopened.recovery_ticket.is_none(),
        "{:?}",
        reopened.error
    );
    assert_eq!(std::fs::read(&destination).unwrap(), b"original bytes");
    assert_eq!(
        reopened.decisions[&MediaId::for_path(&destination)].rating,
        Some(4)
    );
    assert_eq!(reopened.stats.decodes, 0);
}
#[test]
fn recovery_details_show_interruption_and_every_failure_in_all_workflows() {
    for step in Step::ALL {
        let (_dir, mut app) = fixture();
        app.step = step;
        let root = app.root.clone().unwrap();
        let destination = failed_saved_plan(&mut app);
        let original = app.error.clone().unwrap();
        let ticket = app.recovery_ticket.clone().unwrap();
        app.restore_operations(crate::core::operations::recovery::Restored {
            ticket: ticket.clone(),
            plan: app.retry_plan.clone().unwrap(),
            rotation_only: step == Step::Rotation,
            uncertain: 1,
            errors: vec![
                original.clone(),
                "Second failure: café\nPermission denied".into(),
            ],
        });
        let error = app.error.as_ref().unwrap();
        assert!(error.starts_with("1 operation(s) were interrupted"));
        assert!(error.contains(&original));
        assert!(error.ends_with("Second failure: café\nPermission denied"));
        let recovered_error = error.clone();
        app.report_scan_warnings(&["Cannot read café/locked".into()]);
        assert_eq!(
            app.error.as_deref(),
            Some(format!("{recovered_error}\n1 scan warning(s). Cannot read café/locked").as_str())
        );
        let with_warning = app.error.clone();
        app.report_scan_warnings(&[]);
        assert_eq!(app.error, with_warning);
        assert_eq!(app.recovery_ticket, Some(ticket));
        assert_eq!(app.retry_plan.as_ref().unwrap().actions.len(), 1);
        assert_eq!(app.rotation_plan, step == Step::Rotation);
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(
            std::fs::read(destination).unwrap(),
            b"unrelated destination"
        );
        assert_eq!(
            std::fs::read(root.join("Before/a.jpg")).unwrap(),
            b"original bytes"
        );
        app.discard_remaining_operations();
        settle(&mut app);
    }
}

#[test]
fn saved_plan_discard_is_worker_owned_and_all_workflows_use_the_same_path() {
    for step in Step::ALL {
        let (_dir, mut app) = fixture();
        app.step = step;
        let root = app.root.clone().unwrap();
        let destination = failed_saved_plan(&mut app);
        app.discard_remaining_operations();
        assert!(app.busy && app.retry_plan.is_some());
        settle(&mut app);
        assert!(app.retry_plan.is_none() && app.recovery_ticket.is_none());
        assert!(!crate::core::operations::recovery::Store::standard().has_pending(&root));
        assert_eq!(
            std::fs::read(destination).unwrap(),
            b"unrelated destination"
        );
        assert_eq!(
            std::fs::read(root.join("Before/a.jpg")).unwrap(),
            b"original bytes"
        );
    }
}
#[test]
fn stale_recovery_discard_reloads_instead_of_trapping_departure() {
    let (_dir, mut app) = fixture();
    let root = app.root.clone().unwrap();
    failed_saved_plan(&mut app);
    crate::core::operations::recovery::Store::standard()
        .discard(&root, app.recovery_ticket.as_ref().unwrap())
        .unwrap();
    app.discard_remaining_operations();
    settle(&mut app);
    assert!(app.retry_plan.is_none() && app.recovery_ticket.is_none());
    app.request_departure(Destination::Step(Step::Cull));
    assert_eq!(app.step, Step::Cull);
}
#[test]
fn pre_apply_ui_cannot_autosave_over_worker_persisted_moves_before_receipt_delivery() {
    let (_dir, mut app) = fixture();
    let root = app.root.clone().unwrap();
    let old = app.media[0].id;
    let destination = root.join("After/a.jpg");
    let new = MediaId::for_path(&destination);
    app.decisions.insert(
        old,
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    app.plan = Some(Plan {
        actions: vec![Action::Move {
            files: vec![(
                crate::core::operations::Source::capture(root.join("Before/a.jpg")).unwrap(),
                destination,
            )],
        }],
        output_root: Some(root.clone()),
        ..Default::default()
    });
    app.apply_reviewed_plan();
    loop {
        let event = app
            .runtime
            .events
            .recv_timeout(worker_patience())
            .unwrap()
            .event;
        match event {
            Event::Applied(_) => break,
            Event::Progress { .. } => {}
            _ => panic!("Unexpected file work"),
        }
    }
    // Intentionally leave the UI with its original catalog and pending receipt.
    assert_eq!(app.active, Some(old));
    assert!(app.applying_plan.is_some());
    app.save_session();
    app.checkpoint_bookmarks();
    let barrier = tempfile::tempdir().unwrap();
    app.runtime
        .io(IoWork::Save {
            root: barrier.path().canonicalize().unwrap(),
            session: Default::default(),
        })
        .unwrap();
    loop {
        if matches!(
            app.runtime
                .events
                .recv_timeout(worker_patience())
                .unwrap()
                .event,
            Event::Saved
        ) {
            break;
        }
    }
    let saved = Session::load(&root).unwrap();
    assert_eq!(saved.decisions[&new].rating, Some(4));
    assert!(!saved.decisions.contains_key(&old));
    let views = crate::core::views::Saved::load(&root).unwrap();
    assert_eq!(views.bookmarks[&Step::Organize].active, Some(new));
    app.runtime.shutdown();
}
#[test]
fn shared_application_move_and_reload_preserve_scene_membership() {
    let (_dir, mut app) = fixture();
    let old = app.media[0].id;
    app.scene_groups = crate::core::manual_groups::edit(
        1,
        Default::default(),
        Default::default(),
        &app.media,
        crate::core::manual_groups::Edit::Move {
            selected: vec![old],
            target: None,
        },
        || false,
    )
    .unwrap()
    .manual;
    edit(&mut app);
    app.organize();
    settle(&mut app);
    assert_eq!(app.plan.as_ref().unwrap().actions.len(), 2);
    app.apply_reviewed_plan();
    settle(&mut app);
    let new = MediaId::for_path(&app.root.as_ref().unwrap().join("After/a.jpg"));
    assert!(app.scene_groups.contains(new));
    assert!(!app.scene_groups.contains(old));
    assert!(app.manual_groups.is_empty());
    assert_eq!(
        Session::load(app.root.as_ref().unwrap())
            .unwrap()
            .scene_groups,
        app.scene_groups
    );
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn every_workflow_edits_saved_move_destinations_and_names_without_image_work() {
    for step in Step::ALL {
        let (_dir, mut app) = fixture();
        app.step = step;
        let collision = failed_saved_plan(&mut app);
        let root = app.root.clone().unwrap();
        let ticket = app.recovery_ticket.clone();
        assert!(app.review_remaining_operations());
        app.begin_destination_edit();
        app.retry_edit.as_mut().unwrap().folder = "Corrected".into();
        app.retry_edit.as_mut().unwrap().stem = "Renamed".into();
        app.save_destination_edit();
        assert!(app.busy && app.retry_edit.as_ref().unwrap().saving);
        settle(&mut app);
        assert!(
            app.retry_edit.is_none() && app.plan_open && !app.busy,
            "{:?}",
            app.error
        );
        assert_ne!(app.recovery_ticket, ticket);
        assert!(
            !root.join("Corrected").exists(),
            "Saving intent must not apply files"
        );
        assert_eq!(std::fs::read(&collision).unwrap(), b"unrelated destination");
        assert_eq!(app.stats.decodes, 0);
        let restored = crate::core::operations::recovery::Store::standard()
            .restore(&root, &|| false)
            .unwrap()
            .unwrap();
        assert_eq!(restored.ticket, app.recovery_ticket.clone().unwrap());
        app.apply_reviewed_plan();
        settle(&mut app);
        assert!(app.retry_plan.is_none(), "{:?}", app.error);
        assert_eq!(
            std::fs::read(root.join("Corrected/Renamed.jpg")).unwrap(),
            b"original bytes"
        );
        assert_eq!(std::fs::read(collision).unwrap(), b"unrelated destination");
        assert_eq!(app.stats.decodes, 0);
    }
}
#[test]
fn durable_quit_checkpoints_every_workflow_and_reopens_without_replaying_operations() {
    for step in Step::ALL {
        for already_open in [false, true] {
            let (_dir, mut app) = fixture();
            app.step = step;
            let collision = failed_saved_plan(&mut app);
            let root = app.root.clone().unwrap();
            let id = MediaId::for_path(&root.join("Before/a.jpg"));
            let ticket = app.recovery_ticket.clone();
            app.decisions.insert(
                id,
                Decision {
                    rating: Some(5),
                    ..Default::default()
                },
            );
            app.scene_groups = crate::core::manual_groups::edit(
                1,
                Default::default(),
                Default::default(),
                &app.media,
                crate::core::manual_groups::Edit::Move {
                    selected: vec![id],
                    target: None,
                },
                || false,
            )
            .unwrap()
            .manual;
            app.query = "latest view".into();
            app.pending_bookmark = None;
            app.view_data_ready.insert(step);
            if already_open {
                assert!(app.review_remaining_operations());
            }
            close_request(&mut app, already_open);
            assert!(matches!(app.after_apply, Some(Destination::Close)));
            assert!(!app.close_authorized);
            app.save_plan_and_quit();
            assert!(app.close_save_pending && app.busy && !app.close_authorized);
            // Repeated requests cannot requeue or bypass the acknowledgement.
            close_request(&mut app, true);
            assert!(!app.close_authorized);
            settle(&mut app);
            assert!(
                app.close_authorized && app.close_checkpointed,
                "{:?}",
                app.error
            );
            assert_eq!(app.recovery_ticket, ticket);
            assert_eq!(Session::load(&root).unwrap().decisions[&id].rating, Some(5));
            let views = crate::core::views::Saved::load(&root).unwrap();
            assert_eq!(views.step, Some(step));
            assert_eq!(views.bookmarks[&step].query, "latest view");
            eframe::App::on_exit(&mut app);
            drop(app);
            let mut reopened =
                PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.join("cache")), None);
            reopened.scan_folder(root.clone());
            settle(&mut reopened);
            assert_eq!(reopened.recovery_ticket, ticket);
            assert_eq!(reopened.decisions[&id].rating, Some(5));
            assert!(reopened.scene_groups.contains(id));
            assert!(reopened.retry_plan.is_some() && !reopened.plan_open);
            assert_eq!(std::fs::read(collision).unwrap(), b"unrelated destination");
            assert_eq!(
                std::fs::read(root.join("Before/a.jpg")).unwrap(),
                b"original bytes"
            );
            assert_eq!(reopened.stats.decodes, 0);
            reopened.discard_remaining_operations();
            settle(&mut reopened);
        }
    }
}
#[test]
fn failed_quit_checkpoint_stays_open_and_can_retry_without_losing_the_saved_plan() {
    let (_dir, mut app) = fixture();
    failed_saved_plan(&mut app);
    let root = app.root.clone().unwrap();
    let ticket = app.recovery_ticket.clone();
    let path = crate::core::session::data_dir()
        .join("sessions")
        .join(format!(
            "{}.json",
            blake3::hash(root.as_os_str().as_encoded_bytes()).to_hex()
        ));
    let original = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    close_request(&mut app, true);
    app.save_plan_and_quit();
    settle(&mut app);
    // Restore the disposable session before asserting, including on failure.
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, original).unwrap();
    assert!(!app.close_authorized && !app.close_checkpointed && !app.close_save_pending);
    assert!(app.plan_open && !app.busy);
    assert!(
        app.error
            .as_ref()
            .unwrap()
            .contains("Could not save before quitting")
    );
    assert_eq!(app.recovery_ticket, ticket);
    app.save_plan_and_quit();
    settle(&mut app);
    assert!(app.close_authorized && app.close_checkpointed);
    assert_eq!(app.stats.decodes, 0);
    app.runtime.shutdown();
}
#[test]
fn save_plan_quit_requires_durable_review_and_keeps_destination_editor_guard() {
    let (_dir, mut app) = fixture();
    app.after_apply = Some(Destination::Close);
    app.retry_plan = Some(Plan::default());
    app.save_plan_and_quit();
    assert!(!app.busy && !app.close_authorized);
    app.after_apply = None;
    failed_saved_plan(&mut app);
    app.review_remaining_operations();
    app.begin_destination_edit();
    close_request(&mut app, true);
    app.save_plan_and_quit();
    assert!(app.retry_edit.is_some());
    assert!(!app.busy && !app.close_authorized);
    app.retry_edit = None;
    app.request_departure(Destination::Step(Step::Cull));
    app.save_plan_and_quit();
    assert!(!app.busy && !app.close_authorized);
    app.discard_remaining_operations();
    settle(&mut app);
}
#[test]
fn durable_quit_never_drops_unsaved_organization_or_rotation_choices() {
    let (_dir, mut app) = fixture();
    failed_saved_plan(&mut app);
    close_request(&mut app, true);
    edit(&mut app);
    assert!(app.has_unsaved_plan_extras());
    app.save_plan_and_quit();
    assert!(!app.busy && !app.close_authorized);
    app.organize_drafts.clear();
    let id = app.media[0].id;
    app.rotation_review.ensure(id, 1);
    app.rotation_review.confirm(id);
    assert!(app.has_unsaved_plan_extras());
    app.save_plan_and_quit();
    assert!(!app.busy && !app.close_authorized);
    app.rotation_review.discard();
    assert!(!app.has_unsaved_plan_extras());
    app.save_plan_and_quit();
    settle(&mut app);
    assert!(app.close_authorized);
}
#[test]
#[cfg(unix)]
fn every_workflow_finishes_staged_moves_on_the_shared_worker_without_decoding() {
    for step in Step::ALL {
        let (_dir, mut app) = fixture();
        app.step = step;
        let root = app.root.clone().unwrap();
        let source = root.join("Before/a.jpg");
        let to = root.join("After/a.jpg");
        std::fs::create_dir(to.parent().unwrap()).unwrap();
        std::fs::hard_link(&source, &to).unwrap();
        let old = app.media[0].id;
        app.decisions.insert(
            old,
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        );
        app.set_review_plan(Some(Plan {
            actions: vec![Action::FinishMove {
                files: vec![(
                    crate::core::operations::Source::capture(source.clone()).unwrap(),
                    to.clone(),
                )],
            }],
            ..Default::default()
        }));
        app.apply_reviewed_plan();
        settle(&mut app);
        assert!(app.retry_plan.is_none(), "{:?}", app.error);
        assert!(!source.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"original bytes");
        assert_eq!(app.decisions[&MediaId::for_path(&to)].rating, Some(5));
        assert_eq!(app.stats.decodes, 0);
    }
}
/// Window state for a frame in which eframe runs only `App::logic`.
fn hidden_input(minimized: bool) -> egui::RawInput {
    let mut input = egui::RawInput::default();
    let viewport = input.viewports.entry(egui::ViewportId::ROOT).or_default();
    viewport.minimized = Some(minimized);
    viewport.occluded = Some(!minimized);
    input
}
fn logic_frame(
    ctx: &egui::Context,
    input: &egui::RawInput,
    logic: impl FnOnce(&egui::Context),
) -> Vec<egui::ViewportCommand> {
    let mut output = ctx.run_logic(input, logic);
    output
        .viewport_commands
        .remove(&egui::ViewportId::ROOT)
        .unwrap_or_default()
}
fn app_logic(
    app: &mut PhotoSortApp,
    ctx: &egui::Context,
    input: &egui::RawInput,
) -> Vec<egui::ViewportCommand> {
    let mut frame = eframe::Frame::_new_kittest();
    logic_frame(ctx, input, |ctx| eframe::App::logic(app, ctx, &mut frame))
}
#[test]
fn smoke_exit_closes_an_occluded_or_minimized_window() {
    // eframe skips `ui` for a hidden window, and egui's input time stays at
    // the last shown frame. A smoke run started behind another app used to
    // wait until its window was shown again.
    for minimized in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None)
                .with_smoke_test(Duration::from_millis(50));
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let input = hidden_input(minimized);
        let commands = app_logic(&mut app, &ctx, &input);
        assert!(!app.close_authorized && commands.is_empty());
        std::thread::sleep(Duration::from_millis(60));
        let commands = app_logic(&mut app, &ctx, &input);
        assert!(app.close_authorized && app.smoke_exit.is_none());
        assert!(commands.contains(&egui::ViewportCommand::Close));
        // eframe ends the app on the next logic-only pass unless it is cancelled.
        let mut closing = input.clone();
        closing
            .viewports
            .get_mut(&egui::ViewportId::ROOT)
            .unwrap()
            .events
            .push(egui::ViewportEvent::Close);
        let commands = app_logic(&mut app, &ctx, &closing);
        assert!(!commands.contains(&egui::ViewportCommand::CancelClose));
    }
}
#[test]
fn shown_window_leaves_quit_to_the_frame_after_its_input() {
    let dir = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None)
            .with_smoke_test(Duration::ZERO);
    let ctx = egui::Context::default();
    let mut input = hidden_input(false);
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .occluded = Some(false);
    let commands = app_logic(&mut app, &ctx, &input);
    assert!(commands.is_empty() && !app.close_authorized && app.smoke_exit.is_some());
}
#[test]
fn native_quit_in_a_hidden_window_closes_or_brings_the_prompt_forward() {
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let input = hidden_input(true);
    let (_dir, mut app) = fixture();
    let commands = logic_frame(&ctx, &input, |ctx| app.update_hidden(ctx, true));
    assert!(app.close_authorized);
    assert!(commands.contains(&egui::ViewportCommand::Close));

    let (_dir, mut app) = fixture();
    let id = app.media[0].id;
    app.decisions.insert(
        id,
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let commands = logic_frame(&ctx, &input, |ctx| app.update_hidden(ctx, true));
    assert!(matches!(app.departure, Some(Destination::Close)) && !app.close_authorized);
    assert!(!commands.contains(&egui::ViewportCommand::Close));
    assert!(commands.contains(&egui::ViewportCommand::Minimized(false)));
    assert!(commands.contains(&egui::ViewportCommand::Focus));
    assert_eq!(app.decisions[&id].rating, Some(4));
    // Without a new request, a hidden window with an open prompt stays put.
    let commands = logic_frame(&ctx, &input, |ctx| app.update_hidden(ctx, false));
    assert!(!commands.contains(&egui::ViewportCommand::Focus));
}
#[test]
fn window_close_of_a_minimized_window_with_staged_changes_restores_the_prompt() {
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let (_dir, mut app) = fixture();
    let id = app.media[0].id;
    app.decisions.insert(
        id,
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let mut input = hidden_input(true);
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let commands = logic_frame(&ctx, &input, |ctx| app.update_hidden(ctx, false));
    assert!(matches!(app.departure, Some(Destination::Close)) && !app.close_authorized);
    assert!(commands.contains(&egui::ViewportCommand::CancelClose));
    assert!(!commands.contains(&egui::ViewportCommand::Close));
    assert!(commands.contains(&egui::ViewportCommand::Minimized(false)));
    assert!(commands.contains(&egui::ViewportCommand::Focus));
    assert_eq!(app.decisions[&id].rating, Some(4));
}
#[test]
fn hidden_window_keeps_receiving_worker_results() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap().join("Library");
    std::fs::create_dir(&root).unwrap();
    std::fs::write(root.join("a.jpg"), b"original bytes").unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    app.open(root);
    assert!(app.scanning);
    let input = hidden_input(false);
    let until = Instant::now() + worker_patience();
    while app.scanning || app.catalog.is_none() {
        let expired = Instant::now() >= until;
        app_logic(&mut app, &ctx, &input);
        assert!(
            !expired || !(app.scanning || app.catalog.is_none()),
            "results waited for the window"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(app.media.len(), 1);
}
#[test]
fn one_review_shows_organize_changes_with_the_staged_trash_marks() {
    let (dir, mut app) = fixture();
    let root = dir.path().canonicalize().unwrap();
    app.step = Step::Organize;
    edit(&mut app);
    let old_id = app.media[0].id;
    app.decisions.entry(old_id).or_default().rejected = true;
    app.review_changes();
    settle(&mut app);
    assert!(app.plan_open && app.error.is_none(), "{:?}", app.error);
    assert!(app.review_after_moves);
    let plan = app.plan.clone().unwrap();
    assert!(!plan.actions.is_empty());
    // The moves are planned first; the Trash mark is planned for the moved
    // file once they are applied, by the same Apply, with no second review.
    assert!(
        plan.actions
            .iter()
            .all(|a| !matches!(a, Action::Trash { .. } | Action::TrashDirectory(_)))
    );
    let sentence = app.review_sentence(&plan, &app.plan_rows);
    assert!(
        sentence.ends_with("send 1 photo to the Trash."),
        "{sentence}"
    );
    // Nothing has changed on disk while the plan is only reviewed.
    assert!(root.join("Before/a.jpg").is_file());
}
#[test]
fn review_changes_without_organize_changes_reviews_the_staged_marks_directly() {
    let (_dir, mut app) = fixture();
    app.step = Step::Organize;
    let id = app.media[0].id;
    app.decisions.entry(id).or_default().rejected = true;
    app.review_changes();
    settle(&mut app);
    assert!(app.plan_open && !app.review_after_moves);
    assert!(matches!(
        app.plan.as_ref().unwrap().actions.as_slice(),
        [Action::Trash { .. }]
    ));
    // Keeping on reviewing forgets nothing and moves nothing.
    app.plan_open = false;
    assert!(app.decisions[&id].rejected);
}
#[test]
fn moving_an_existing_folder_to_trash_shows_the_whole_folder_for_review_first() {
    use super::super::organize_board::FolderAction;
    let (dir, mut app) = fixture();
    let root = dir.path().canonicalize().unwrap();
    app.step = Step::Organize;
    let folder = app
        .board_library_folder("Before")
        .expect("Before is on disk");
    app.board_folder_action(FolderAction::TrashFolder(folder));
    settle(&mut app);
    assert!(app.plan_open && app.error.is_none(), "{:?}", app.error);
    assert!(matches!(
        app.plan.as_ref().unwrap().actions.as_slice(),
        [Action::TrashDirectory(_)]
    ));
    assert!(root.join("Before/a.jpg").is_file());
}
