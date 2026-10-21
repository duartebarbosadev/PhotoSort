use super::*;
fn app() -> (tempfile::TempDir, PhotoSortApp) {
    let dir = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
    app.runtime.model_assets =
        crate::core::model_assets::Worker::new(vec![dir.path().join("assets")], Arc::new(|| {}));
    app.root = Some(dir.path().into());
    (dir, app)
}
#[test]
fn missing_models_pause_all_ai_workflows_preserve_decisions_and_ignore_stale_results() {
    for step in [
        Step::Organize,
        Step::EasyDelete,
        Step::Rotation,
        Step::PickBest,
        Step::Cull,
    ] {
        let (_dir, mut app) = app();
        app.step = step;
        app.analysis_run = Some(5);
        app.pick_run = Some(5);
        app.decisions.insert(
            MediaId(8),
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        );
        let issue = model_assets::Issue {
            missing: vec![model_assets::Id::Scene],
            ..Default::default()
        };
        app.models_needed(4, issue.clone());
        assert!(!app.model_dialog);
        assert_eq!(app.analysis_run, Some(5));
        app.models_needed(5, issue);
        assert!(app.model_dialog && app.resume_suspended);
        assert!(app.analysis_run.is_none());
        assert!(app.pick_run.is_none());
        assert_eq!(app.decisions[&MediaId(8)].rating, Some(5));
        assert!(app.model_retry_valid());
        app.root = Some("/different".into());
        assert!(!app.model_retry_valid());
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
}
#[test]
fn model_recovery_retains_preview_intent_only_for_its_original_context() {
    for mode in [GroupMode::Visual, GroupMode::Faces, GroupMode::Mixed] {
        let (_dir, mut app) = app();
        app.step = Step::Organize;
        app.group_mode = mode;
        app.analysis_run = Some(5);
        app.organize_run = Some((5, mode));
        app.organize_preview_requested = true;
        app.models_needed(
            5,
            model_assets::Issue {
                missing: vec![model_assets::Id::Scene],
                ..Default::default()
            },
        );
        assert!(app.model_retry.as_ref().unwrap().preview);
        assert!(!app.organize_preview_requested && app.organize_run.is_none());
        app.set_group_mode(GroupMode::Current);
        assert!(app.model_retry.is_none());
    }
    let (_dir, mut app) = app();
    app.step = Step::Organize;
    app.analysis_run = Some(5);
    app.models_needed(5, model_assets::Issue::default());
    assert!(
        !app.model_retry.as_ref().unwrap().preview,
        "Analyze alone must not acquire preview intent"
    );
}
#[test]
fn model_dialog_fits_compact_and_wide_windows_without_loading_models_or_images() {
    let (dir, mut app) = app();
    std::fs::create_dir_all(dir.path().join("assets/models")).unwrap();
    for asset in model_assets::ALL {
        std::fs::write(
            dir.path().join("assets/models").join(asset.name),
            b"test copy",
        )
        .unwrap();
    }
    app.open_model_setup();
    let until = Instant::now() + Duration::from_secs(5);
    while app.runtime.model_assets.state().running {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    app.model_issue = Some(model_assets::Issue {
        missing: model_assets::ALL.iter().map(|a| a.id).collect(),
        runtime_missing: true,
        unsupported: !cfg!(feature = "onnx"),
    });
    app.model_removal = app
        .runtime
        .model_assets
        .state()
        .snapshot
        .as_ref()
        .unwrap()
        .rows[0]
        .managed
        .clone();
    assert!(app.model_removal.is_some());
    let ctx = egui::Context::default();
    theme_install(&ctx);
    for running in [false, true] {
        let mut state = app.runtime.model_assets.state();
        state.running = running;
        state.progress = Some(model_assets::Progress {
            id: Some(model_assets::Id::Scene),
            bytes: 10,
            total: 100,
        });
        state.error =
            Some("A transfer could not complete. Existing files remain available. ".repeat(8));
        for (width, height) in [(480., 640.), (800., 600.), (1200., 900.)] {
            for _ in 0..2 {
                let out = ctx.run_test(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, height),
                        )),
                        ..Default::default()
                    },
                    |ui| {
                        let ctx = &ui.ctx().clone();
                        app.render_model_setup(ctx, &state)
                    },
                );
                assert!(out.shapes.len() < 500);
                let rect = ctx
                    .memory(|memory| memory.area_rect(egui::Id::new("model_setup")))
                    .unwrap();
                assert!(
                    rect.width() <= width && rect.height() <= height,
                    "Model panel {rect:?} must fit {width}x{height}"
                );
                assert!(app.analysis_run.is_none());
            }
        }
    }
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
#[test]
fn model_removal_uses_shared_worker_and_preserves_all_workflow_reviews() {
    for step in Step::ALL {
        let (dir, mut app) = app();
        app.step = step;
        let root = dir.path().join("assets");
        std::fs::create_dir_all(root.join("models")).unwrap();
        let path = root
            .join("models")
            .join(model_assets::Id::Detector.asset().name);
        std::fs::write(&path, b"disposable").unwrap();
        let copy = model_assets::review_managed(&root, model_assets::Id::Detector)
            .unwrap()
            .unwrap();
        app.decisions.insert(
            MediaId(8),
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        app.analysis_run = Some(3);
        app.start_model_operation(Work::Remove(copy.clone()));
        assert!(path.exists());
        assert!(app.runtime.model_assets.state().snapshot.is_none());
        app.analysis_run = None;
        app.busy = true;
        app.start_model_operation(Work::Remove(copy.clone()));
        assert!(path.exists());
        app.busy = false;
        app.start_model_operation(Work::Remove(copy));
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.runtime.model_assets.state().running {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        assert!(app.runtime.model_assets.state().error.is_none());
        assert!(!path.exists());
        assert_eq!(app.decisions[&MediaId(8)].rating, Some(4));
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
}
#[test]
fn retry_requires_verified_prerequisites_and_the_original_workflow_context() {
    let (_dir, mut app) = app();
    app.step = Step::Cull;
    app.remember_model_retry();
    app.model_issue = Some(model_assets::Issue {
        missing: vec![model_assets::Id::Scene],
        runtime_missing: true,
        unsupported: false,
    });
    let mut state = model_assets::State {
        snapshot: Some(Arc::new(model_assets::Snapshot {
            rows: vec![model_assets::Row {
                id: model_assets::Id::Scene,
                health: Health::Missing,
                bundled: None,
                managed: None,
            }],
            runtime: None,
            orientation_override: None,
            hash_reads: 0,
        })),
        ..Default::default()
    };
    assert!(!app.model_retry_ready(&state));
    let snapshot = Arc::make_mut(state.snapshot.as_mut().unwrap());
    snapshot.runtime = Some("runtime".into());
    snapshot.rows[0].health = Health::Verified("model".into());
    assert_eq!(app.model_retry_ready(&state), cfg!(feature = "onnx"));
    state.running = true;
    assert!(!app.model_retry_ready(&state));
    state.running = false;
    app.step = Step::Organize;
    assert!(!app.model_retry_ready(&state));
}
fn pending_recovery(app: &mut PhotoSortApp) {
    app.remember_model_retry();
    app.model_issue = Some(model_assets::Issue {
        missing: vec![model_assets::Id::Scene],
        runtime_missing: true,
        unsupported: false,
    });
    app.status = "Local AI needs model setup · your current reviews are preserved".into();
}
fn verified_setup() -> model_assets::State {
    model_assets::State {
        snapshot: Some(Arc::new(model_assets::Snapshot {
            rows: vec![model_assets::Row {
                id: model_assets::Id::Scene,
                health: Health::Verified("verified-model".into()),
                bundled: None,
                managed: None,
            }],
            runtime: Some("verified-runtime".into()),
            orientation_override: None,
            hash_reads: 0,
        })),
        ..Default::default()
    }
}
#[test]
fn workflow_and_group_changes_expire_recovery_without_changing_reviews() {
    for previous in Step::ALL {
        for next in Step::ALL {
            if previous == next {
                continue;
            }
            let (_dir, mut app) = app();
            app.step = previous;
            pending_recovery(&mut app);
            app.decisions.insert(
                MediaId(8),
                Decision {
                    rating: Some(4),
                    ..Default::default()
                },
            );
            app.set_step(next);
            assert!(app.model_issue.is_none() && app.model_retry.is_none());
            assert!(!app.status.contains("model setup"));
            assert_eq!(app.decisions[&MediaId(8)].rating, Some(4));
            assert!(app.model_recovery_status(&verified_setup()).is_none());
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        }
    }
    let (_dir, mut app) = app();
    app.step = Step::Organize;
    app.set_group_mode(GroupMode::Visual);
    pending_recovery(&mut app);
    app.set_group_mode(GroupMode::Visual);
    assert!(
        app.model_retry_valid(),
        "An unchanged choice preserves its retry"
    );
    app.set_group_mode(GroupMode::Current);
    assert!(app.model_issue.is_none() && app.model_retry.is_none());
    assert!(!app.status.contains("model setup"));
    app.step = Step::Cull;
    pending_recovery(&mut app);
    app.set_group_mode(GroupMode::Faces);
    assert!(
        app.model_retry_valid(),
        "Organize options do not change Cull prerequisites"
    );
}
#[test]
fn new_folder_and_fresh_analysis_drop_previous_recovery() {
    let (dir, mut app) = app();
    app.media.push(Media {
        id: MediaId(8),
        path: dir.path().join("photo.jpg"),
        kind: MediaKind::Photo,
        fingerprint: Default::default(),
    });
    app.step = Step::Organize;
    pending_recovery(&mut app);
    app.analyze();
    assert!(app.analysis_run.is_some());
    assert!(app.model_issue.is_none() && app.model_retry.is_none());
    app.cancel_analysis();
    pending_recovery(&mut app);
    let next = dir.path().join("next");
    std::fs::create_dir(&next).unwrap();
    app.scan_folder(next);
    assert!(app.model_issue.is_none() && app.model_retry.is_none());
    assert!(app.scanning);
    assert!(!app.status.contains("model setup"));
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
#[test]
fn repaired_models_update_footer_and_dialog_without_restarting_analysis() {
    let (_dir, mut app) = app();
    app.step = Step::Cull;
    pending_recovery(&mut app);
    assert!(
        app.model_recovery_status(&model_assets::State::default())
            .unwrap()
            .contains("needs model setup")
    );
    let mut state = verified_setup();
    state.running = true;
    assert!(
        app.model_recovery_status(&state)
            .unwrap()
            .contains("Checking")
    );
    state.running = false;
    let message = app.model_recovery_status(&state).unwrap();
    assert_eq!(message.contains("Models ready"), cfg!(feature = "onnx"));
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut labels = Vec::new();
    for _ in 0..3 {
        let out = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 700.),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.render_model_setup(ctx, &state)
            },
        );
        labels = out
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
            .collect();
    }
    assert!(
        !labels
            .iter()
            .any(|label| label.starts_with("The native AI runtime is missing"))
    );
    if cfg!(feature = "onnx") {
        assert!(
            labels
                .iter()
                .any(|label| label.starts_with("The models this analysis needs are ready."))
        );
        assert!(
            !labels
                .iter()
                .any(|label| label.starts_with("Needed for this analysis"))
        );
    }
    app.set_step(Step::Organize);
    for _ in 0..3 {
        let out = ctx.run_test(egui::RawInput::default(), |ui| {
            let ctx = &ui.ctx().clone();
            app.render_model_setup(ctx, &state)
        });
        labels = out
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
            .collect();
    }
    assert!(
        !labels
            .iter()
            .any(|label| label.starts_with("Needed for this analysis")
                || label.starts_with("The models this analysis needs are ready."))
    );
    assert!(app.analysis_run.is_none());
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
#[test]
fn model_panel_summarises_groups_and_links_every_model_source() {
    let (_dir, mut app) = app();
    app.step = Step::Cull;
    pending_recovery(&mut app);
    app.open_model_setup();
    let until = Instant::now() + Duration::from_secs(5);
    while app.runtime.model_assets.state().running {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    let state = app.runtime.model_assets.state();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut nodes = Vec::new();
    for _ in 0..3 {
        let out = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900., 4000.),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.render_model_setup(ctx, &state)
            },
        );
        nodes = out
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .map(|(_, n)| (n.role(), n.label().or_else(|| n.value()).map(str::to_owned)))
            .collect();
    }
    // Wrapped text also appears as text runs; count each widget once.
    let labels: Vec<String> = nodes
        .iter()
        .filter(|(role, _)| *role != egui::accesskit::Role::TextRun)
        .filter_map(|(_, l)| l.clone())
        .collect();
    assert!(
        labels
            .iter()
            .any(|l| l == "0 of 7 models installed · 0.0 MB downloaded or imported")
    );
    // What the paused analysis needs comes first, then the shared and
    // optional groups; nothing is said twice.
    for group in ["NEEDED FOR THE PAUSED ANALYSIS", "OPTIONAL FEATURES"] {
        assert!(labels.iter().any(|l| l == group), "{group}: {labels:?}");
    }
    assert_eq!(
        labels
            .iter()
            .filter(|l| l.contains("ship only inside the full app bundle"))
            .count(),
        1
    );
    assert_eq!(labels.iter().filter(|l| *l == "Missing").count(), 7);
    // Every model links its upstream page; links open only when clicked.
    let links = nodes
        .iter()
        .filter(|(role, _)| *role == egui::accesskit::Role::Link)
        .count();
    assert_eq!(links, model_assets::ALL.len());
    for asset in model_assets::ALL {
        assert!(asset.page.starts_with("https://"));
        let name = format!("Source page for {}", asset.title);
        assert!(labels.contains(&name), "{name}");
        assert!(labels.iter().any(|l| l == asset.about));
    }
    assert!(app.analysis_run.is_none());
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
