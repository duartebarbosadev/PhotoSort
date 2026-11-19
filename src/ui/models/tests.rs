use super::*;
use crate::test_support::worker_patience;
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
    let until = Instant::now() + worker_patience();
    while app.runtime.model_assets.state().running {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(1));
    }
    app.model_issue = Some(model_assets::Issue {
        missing: model_assets::ALL.iter().map(|a| a.id).collect(),
        runtime_missing: true,
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
            package: None,
            bytes: 10,
            total: 100,
            step: Some((2, 7)),
        });
        state.error =
            Some("A transfer could not complete. Existing files remain available. ".repeat(8));
        for (width, height) in [(480., 640.), (800., 600.), (1200., 900.)] {
            for pass in 0..2 {
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
                // What went wrong is in sight, above the footer, however
                // many cards there are.
                // The first frame only measures the panel.
                assert!(
                    pass == 0
                        || super::super::review_card::painted_text(&out)
                            .contains("A transfer could not complete")
                            != running,
                    "{width}x{height}"
                );
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
    // The runtime is the first card, over the models, installed or not, and
    // the two sections never change, so the list does not jump.
    let mut state = app.runtime.model_assets.state();
    for runtime in [None, Some(std::path::PathBuf::from("runtime"))] {
        Arc::make_mut(state.snapshot.as_mut().unwrap()).runtime = runtime;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let mut nodes = Vec::new();
        for _ in 0..3 {
            let out = ctx.run_test(egui::RawInput::default(), |ui| {
                let ctx = &ui.ctx().clone();
                app.render_model_setup(ctx, &state)
            });
            nodes = out.platform_output.accesskit_update.unwrap().nodes;
        }
        // Where each label sits, top to bottom.
        let at = |what: &str| {
            nodes
                .iter()
                .find(|(_, n)| n.label().or_else(|| n.value()) == Some(what))
                .and_then(|(_, n)| n.bounds())
                .unwrap_or_else(|| panic!("{what}"))
                .y0
        };
        let order = [
            "RUNTIME AND SHARED MODELS",
            model_assets::runtime::info(model_assets::runtime::Package::Cpu)
                .map_or("ONNX Runtime", |info| info.title),
            model_assets::Id::Scene.asset().title,
            "FOR PEOPLE FOLDERS AND FIX ROTATION",
            model_assets::Id::Rotation.asset().title,
        ];
        for pair in order.windows(2) {
            assert!(at(pair[0]) < at(pair[1]), "{pair:?}");
        }
    }
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
#[test]
fn a_long_path_in_a_model_s_details_wraps_inside_the_panel() {
    let (dir, mut app) = app();
    let root = dir
        .path()
        .join("a".repeat(60))
        .join("b".repeat(60))
        .join("c".repeat(40));
    let id = model_assets::Id::Detector;
    std::fs::create_dir_all(root.join("models")).unwrap();
    std::fs::write(root.join("models").join(id.asset().name), b"copy").unwrap();
    let copy = model_assets::review_managed(&root, id).unwrap().unwrap();
    let row = model_assets::Row {
        id,
        health: Health::Verified(copy.path().into()),
        bundled: None,
        managed: Some(copy),
        partial: 0,
    };
    let state = model_assets::State::default();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let bounds = egui::Rect::from_min_size(egui::pos2(120.0, 20.0), egui::vec2(360.0, 600.0));
    let out = ctx.run_test(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(800.0, 700.0),
            )),
            ..Default::default()
        },
        |ui| {
            ui.scope_builder(egui::UiBuilder::new().max_rect(bounds), |ui| {
                ui.indent("details", |ui| app.model_details(ui, &state, &row, true));
            });
        },
    );
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, f32, f32)>) {
        match shape {
            egui::Shape::Text(text) => out.push((
                text.galley.text().to_owned(),
                text.pos.x,
                text.pos.x + text.galley.size().x,
            )),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, out)),
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut texts);
    }
    assert!(texts.iter().any(|(text, ..)| text.contains("ccccc")));
    for (text, left, right) in texts {
        assert!(
            left >= bounds.left() - 0.5 && right <= bounds.right() + 0.5,
            "{left}..{right} outside {bounds:?}: {text}"
        );
    }
}
#[test]
fn download_all_takes_the_runtime_and_what_is_missing_but_never_gpu_support() {
    use model_assets::{Id, Row, runtime};
    let row = |id, health, bundled: bool, partial| Row {
        id,
        health,
        bundled: bundled.then(|| "bundled".into()),
        managed: None,
        partial,
    };
    let mut snapshot = model_assets::Snapshot {
        rows: vec![
            row(Id::Scene, Health::Missing, false, 1_000_000),
            row(Id::Aesthetic, Health::Verified("model".into()), false, 0),
            row(Id::Detector, Health::Missing, true, 0),
            row(
                Id::People,
                Health::Invalid("model".into(), "Damaged".into()),
                false,
                0,
            ),
        ],
        runtime: None,
        orientation_override: None,
        cuda: None,
        hash_reads: 0,
    };
    let named = |snapshot: &model_assets::Snapshot| {
        let (works, bytes) = missing_downloads(snapshot);
        let names: Vec<String> = works
            .iter()
            .map(|work| match work {
                Work::InstallRuntime(package) => format!("runtime {package:?}"),
                Work::Download(id) => format!("download {id:?}"),
                Work::Restore(id) => format!("restore {id:?}"),
                _ => "other".to_owned(),
            })
            .collect();
        (names, bytes)
    };
    // Installed models are skipped, a bundled copy is restored, and a
    // download that was interrupted counts only what is left of it.
    let models = ["download Scene", "restore Detector", "download People"];
    let left = Id::Scene.asset().bytes - 1_000_000 + Id::People.asset().bytes;
    let (names, bytes) = named(&snapshot);
    match runtime::info(runtime::Package::Cpu) {
        Some(info) => {
            assert_eq!(names[0], "runtime Cpu");
            assert_eq!(names[1..], models);
            assert_eq!(bytes, left + info.download_bytes());
        }
        None => assert_eq!((names, bytes), (models.map(String::from).to_vec(), left)),
    }
    snapshot.runtime = Some("runtime".into());
    assert_eq!(named(&snapshot), (models.map(String::from).to_vec(), left));
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
        let deadline = Instant::now() + worker_patience();
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
    });
    let mut state = model_assets::State {
        snapshot: Some(Arc::new(model_assets::Snapshot {
            rows: vec![model_assets::Row {
                id: model_assets::Id::Scene,
                health: Health::Missing,
                bundled: None,
                managed: None,
                partial: 0,
            }],
            runtime: None,
            orientation_override: None,
            cuda: None,
            hash_reads: 0,
        })),
        ..Default::default()
    };
    assert!(!app.model_retry_ready(&state));
    let snapshot = Arc::make_mut(state.snapshot.as_mut().unwrap());
    snapshot.runtime = Some("runtime".into());
    snapshot.rows[0].health = Health::Verified("model".into());
    assert!(app.model_retry_ready(&state));
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
    });
    app.status = "AI models needed · open AI models".into();
}
fn verified_setup() -> model_assets::State {
    model_assets::State {
        snapshot: Some(Arc::new(model_assets::Snapshot {
            rows: vec![model_assets::Row {
                id: model_assets::Id::Scene,
                health: Health::Verified("verified-model".into()),
                bundled: None,
                managed: None,
                partial: 0,
            }],
            runtime: Some("verified-runtime".into()),
            orientation_override: None,
            cuda: None,
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
            .contains("AI models needed")
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
    assert!(message.contains("AI models ready"));
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
            .any(|label| label.starts_with("The AI runtime is missing"))
    );
    assert!(
        labels
            .iter()
            .any(|label| label.starts_with("The models are ready."))
    );
    assert!(
        !labels
            .iter()
            .any(|label| label.starts_with("Needed for this analysis"))
    );
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
                || label.starts_with("The models are ready."))
    );
    assert!(app.analysis_run.is_none());
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
#[test]
fn model_panel_groups_models_and_links_every_source() {
    let (_dir, mut app) = app();
    app.step = Step::Cull;
    pending_recovery(&mut app);
    app.open_model_setup();
    let until = Instant::now() + worker_patience();
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
    // The same two sections whatever the paused analysis needs; what it
    // needs (here the runtime and the scene model) says so on its card.
    for group in [
        "RUNTIME AND SHARED MODELS",
        "FOR PEOPLE FOLDERS AND FIX ROTATION",
    ] {
        assert!(labels.iter().any(|l| l == group), "{group}: {labels:?}");
    }
    assert_eq!(labels.iter().filter(|l| *l == "Needed now").count(), 2);
    assert_eq!(labels.iter().filter(|l| *l == "Missing").count(), 6);
    let runtime = model_assets::runtime::info(model_assets::runtime::Package::Cpu);
    // Every model downloads from its pinned file; Import stays in Details.
    let count = |pred: &dyn Fn(&str) -> bool| labels.iter().filter(|l| pred(l)).count();
    let total = model_assets::ALL.len() + usize::from(runtime.is_some());
    assert_eq!(
        count(&|l| l.starts_with("Download ") && l.ends_with(" MB") && !l.contains(" all ")),
        total
    );
    // One action, in the footer, downloads them all.
    assert_eq!(count(&|l| l.starts_with("Download all · ")), 1);
    assert_eq!(count(&|l| l == "Import prepared model…"), 0);
    assert_eq!(count(&|l| l.contains("no public download")), 0);
    // The aesthetic model's AGPL-3.0 licence is stated beside its download.
    let agpl = model_assets::Id::Aesthetic.asset().download_note.unwrap();
    assert_eq!(count(&|l| l.ends_with(agpl)), 1, "Said once, on its card");
    // Every model links its upstream page; links open only when clicked.
    let links = nodes
        .iter()
        .filter(|(role, _)| *role == egui::accesskit::Role::Link)
        .count();
    assert_eq!(links, model_assets::ALL.len() + 1, "and the runtime's");
    for asset in model_assets::ALL {
        assert!(asset.page.starts_with("https://"));
        let name = format!("Source page for {}", asset.title);
        assert!(labels.contains(&name), "{name}");
        assert!(labels.iter().any(|l| l == asset.about));
    }
    assert!(app.analysis_run.is_none());
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
/// Every label the model panel shows for `state`.
fn model_panel_labels(app: &mut PhotoSortApp, state: &model_assets::State) -> Vec<String> {
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut labels = Vec::new();
    for _ in 0..3 {
        let out = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 1400.),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.render_model_setup(ctx, state)
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
    labels
}
#[test]
fn a_missing_runtime_is_offered_for_download_and_gpu_support_only_with_an_nvidia_driver() {
    use model_assets::runtime::{self, Package};
    let (_dir, mut app) = app();
    let mut state = verified_setup();
    Arc::make_mut(state.snapshot.as_mut().unwrap()).runtime = None;
    let labels = model_panel_labels(&mut app, &state);
    let offered = |labels: &[String], prefix: &str| labels.iter().any(|l| l.starts_with(prefix));
    // The runtime is a card like the models': its title, Source, status and
    // its own Download.
    let download = runtime::info(Package::Cpu)
        .map(|cpu| format!("Download {}", download_size(cpu.download_bytes())));
    if let (Some(cpu), Some(download)) = (runtime::info(Package::Cpu), &download) {
        assert!(labels.iter().any(|l| l == cpu.title), "{labels:?}");
        assert!(labels.contains(download), "{labels:?}");
        assert!(
            labels.contains(&format!("Source page for {}", cpu.title)),
            "{labels:?}"
        );
    }
    let gpu_offered = labels.iter().any(|l| l == "NVIDIA GPU support");
    assert_eq!(
        gpu_offered,
        runtime::info(Package::Cuda).is_some() && runtime::nvidia_driver_present(),
        "{labels:?}"
    );
    // Installed packages are reported, with a way to remove GPU support.
    {
        let snapshot = Arc::make_mut(state.snapshot.as_mut().unwrap());
        snapshot.runtime = Some("runtime".into());
        snapshot.cuda = Some("runtime-cuda".into());
    }
    let labels = model_panel_labels(&mut app, &state);
    if let Some(download) = &download {
        assert!(!labels.contains(download), "{labels:?}");
    }
    if runtime::info(Package::Cuda).is_some() {
        assert!(
            labels.iter().any(|l| l == "NVIDIA GPU support"),
            "{labels:?}"
        );
        assert!(!offered(&labels, "Download "), "{labels:?}");
    }
}
/// A model download in progress, with its panel closed.
fn downloading() -> model_assets::State {
    model_assets::State {
        running: true,
        mutating: true,
        progress: Some(model_assets::Progress {
            id: Some(model_assets::Id::Scene),
            package: None,
            bytes: 45,
            total: 100,
            step: None,
        }),
        message: "Downloading and verifying model…".into(),
        ..verified_setup()
    }
}
fn footer_labels(app: &mut PhotoSortApp, state: &model_assets::State) -> Vec<String> {
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut labels = Vec::new();
    for _ in 0..2 {
        let out = ctx.run_test(egui::RawInput::default(), |ui| {
            ui.horizontal(|ui| app.model_work_chip(ui, state));
        });
        labels = out
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(_, n)| n.label().map(str::to_owned))
            .collect();
    }
    labels
}
#[test]
fn analyze_during_a_model_download_waits_for_it_and_then_starts() {
    for step in [Step::PickBest, Step::EasyDelete, Step::Cull] {
        let (dir, mut app) = app();
        app.media.push(Media {
            id: MediaId(8),
            path: dir.path().join("photo.jpg"),
            kind: MediaKind::Photo,
            fingerprint: Default::default(),
        });
        app.step = step;
        let state = downloading();
        // Pressed during the download: it waits, says so, and shows the download.
        assert!(app.wait_for_model_work(&state), "{step:?}");
        assert!(app.analysis_waits_for_models() && app.analysis_run.is_none());
        assert!(
            app.workflow_status()
                .contains("Waiting for the AI model download")
        );
        assert!(footer_labels(&mut app, &state).contains(&"AI model download".to_owned()));
        app.request_analysis();
        assert!(app.model_dialog, "pressed again, it shows the download");
        app.model_dialog = false;
        app.follow_model_work(&state);
        assert!(app.analysis_run.is_none());
        // The download ends: the analysis starts on its own.
        app.follow_model_work(&verified_setup());
        assert!(app.analysis_run.is_some(), "{step:?}");
        assert!(!app.analysis_waits_for_models() && app.error.is_none());
        assert!(footer_labels(&mut app, &verified_setup()).is_empty());
        app.cancel_analysis();
    }
}
#[test]
fn a_failed_model_download_is_reported_and_its_waiting_analysis_does_not_start() {
    let (dir, mut app) = app();
    app.media.push(Media {
        id: MediaId(8),
        path: dir.path().join("photo.jpg"),
        kind: MediaKind::Photo,
        fingerprint: Default::default(),
    });
    app.step = Step::Rotation;
    assert!(app.wait_for_model_work(&downloading()));
    app.follow_model_work(&downloading());
    let failed = model_assets::State {
        error: Some("Connection reset".into()),
        message: "Model operation did not complete".into(),
        ..verified_setup()
    };
    app.follow_model_work(&failed);
    let error = app.error.clone().unwrap();
    assert!(error.contains("did not complete") && error.contains("Connection reset"));
    assert!(app.analysis_run.is_none() && !app.analysis_waits_for_models());
    // A cancelled download starts nothing and reports no error.
    app.error = None;
    assert!(app.wait_for_model_work(&downloading()));
    let cancelled = model_assets::State {
        cancelled: true,
        message: "Download cancelled".into(),
        ..verified_setup()
    };
    app.follow_model_work(&cancelled);
    assert!(app.error.is_none() && app.analysis_run.is_none());
    assert!(!app.analysis_waits_for_models());
}
