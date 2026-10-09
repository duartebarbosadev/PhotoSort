use super::*;
#[test]
fn settings_errors_are_bounded_and_dismissible_without_hiding_footer_or_losing_draft() {
    for size in [egui::vec2(600., 500.), egui::vec2(1440., 990.)] {
        let (_dir, mut app) = fixture();
        app.open_settings(Section::Caches);
        app.preferences_draft.as_mut().unwrap().navigation.rating = Some(5);
        let error = "Open folder failed: /unavailable/long cache folder café/".repeat(20);
        app.error = Some(error.clone());
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        ctx.enable_accesskit();
        let mut time = 0.;
        let mut frame = |app: &mut PhotoSortApp, events| {
            time += 0.1;
            ctx.run(
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |ctx| app.dialogs(ctx),
            )
        };
        let mut output = frame(&mut app, vec![]);
        for _ in 0..7 {
            output = frame(&mut app, vec![]);
        }
        let tree = output.platform_output.accesskit_update.unwrap();
        assert!(
            !tree
                .nodes
                .iter()
                .any(|(_, n)| n.label() == Some("Something needs attention")),
            "Settings owns this error"
        );
        let mut dismiss = None;
        for label in [
            "Settings",
            "Dismiss error",
            "General",
            "Caches",
            "Cancel",
            "Save settings",
        ] {
            let node = &tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label) || n.value() == Some(label))
                .unwrap()
                .1;
            let bounds = node.bounds().unwrap();
            assert!(
                bounds.x0 >= 0.
                    && bounds.x1 <= f64::from(size.x)
                    && bounds.y0 >= 0.
                    && bounds.y1 <= f64::from(size.y),
                "{label}: {bounds:?}, {size:?}"
            );
            if label == "Dismiss error" {
                dismiss = Some(egui::pos2(
                    ((bounds.x0 + bounds.x1) / 2.) as f32,
                    ((bounds.y0 + bounds.y1) / 2.) as f32,
                ));
            }
        }
        let pos = dismiss.unwrap();
        for pressed in [true, false] {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: egui::Modifiers::NONE,
                    },
                ],
            );
        }
        assert!(app.error.is_none());
        assert_eq!(
            app.preferences_draft.as_ref().unwrap().navigation.rating,
            Some(5)
        );
        assert_eq!(app.preferences.navigation.rating, None);
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
}
#[test]
fn cache_folder_settings_uses_injected_location_in_every_workflow_without_extra_work() {
    use crate::core::desktop::{Action, Worker};
    for size in [egui::vec2(640., 600.), egui::vec2(1440., 900.)] {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache ' café #1");
        std::fs::create_dir(&cache).unwrap();
        std::fs::write(cache.join("retained.txt"), b"retained").unwrap();
        let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, cache.clone()), None);
        app.runtime.shutdown();
        app.runtime.cache.take();
        app.runtime.cache.finish(Ok(Default::default()), false);
        let (sent, received) = std::sync::mpsc::channel();
        app.runtime.desktop = Worker::with_launcher(
            Arc::new(std::sync::atomic::AtomicU64::new(app.runtime.epoch())),
            Arc::new(|| {}),
            Arc::new(move |request| {
                sent.send(request.clone()).unwrap();
                Ok(())
            }),
        );
        app.open_settings(Section::Caches);
        assert!(!app.runtime.cache.take().unwrap().clear);
        app.runtime.cache.finish(Ok(Default::default()), false);
        app.preferences_draft.as_mut().unwrap().navigation.rating = Some(5);
        app.decisions.insert(
            MediaId(7),
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        for step in Step::ALL {
            app.step = step;
            app.open_settings(Section::Caches);
            let ctx = egui::Context::default();
            crate::ui::theme::install(&ctx);
            ctx.enable_accesskit();
            let mut time = 0.;
            let mut frame = |app: &mut PhotoSortApp, events| {
                time += 0.1;
                ctx.run(
                    egui::RawInput {
                        time: Some(time),
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.preferences_dialog(ctx),
                )
            };
            let mut output = frame(&mut app, vec![]);
            for _ in 0..7 {
                output = frame(&mut app, vec![]);
            }
            let tree = output.platform_output.accesskit_update.unwrap();
            let node = &tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Open cache folder"))
                .unwrap()
                .1;
            assert!(!node.is_disabled());
            let bounds = node.bounds().unwrap();
            assert!(
                bounds.x0 >= 0.
                    && bounds.x1 <= f64::from(size.x)
                    && bounds.y0 >= 0.
                    && bounds.y1 <= f64::from(size.y)
            );
            let pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.) as f32,
                ((bounds.y0 + bounds.y1) / 2.) as f32,
            );
            for pressed in [true, false] {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button: egui::PointerButton::Primary,
                            pressed,
                            modifiers: egui::Modifiers::NONE,
                        },
                    ],
                );
            }
            let request = received.recv_timeout(Duration::from_secs(5)).unwrap();
            assert_eq!(request.path, cache);
            assert_eq!(request.action, Action::OpenFolder);
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.runtime.desktop.running() {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(2));
            }
            app.poll_desktop();
            assert!(app.error.is_none() && received.try_recv().is_err());
            assert!(
                app.runtime.cache.take().is_none(),
                "opening repeated cache work"
            );
            assert_eq!(
                app.preferences_draft.as_ref().unwrap().navigation.rating,
                Some(5)
            );
            assert_eq!(app.preferences.navigation.rating, None);
            assert_eq!(app.decisions[&MediaId(7)].rating, Some(4));
            assert!(
                app.root.is_none()
                    && app.requested.is_empty()
                    && app.runtime.processing.status().is_empty()
            );
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
            assert_eq!(
                std::fs::read(cache.join("retained.txt")).unwrap(),
                b"retained"
            );
        }
        app.runtime.cache = Default::default();
        assert!(app.cache_folder_request().is_none());
        assert!(
            crate::core::cache::Control::with_directory("relative".into())
                .directory()
                .is_none()
        );
    }
}
fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
    let dir = tempfile::tempdir().unwrap();
    let app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
    (dir, app)
}
#[test]
fn hiding_current_step_uses_departure_guard_and_keeps_it_reachable_until_resolved() {
    let (_dir, mut app) = fixture();
    app.step = Step::Rotation;
    app.decisions.insert(
        MediaId(1),
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let mut p = Preferences::default();
    p.hidden_steps.extend([Step::Rotation, Step::PickBest]);
    app.apply_preferences(p);
    assert_eq!(app.step, Step::Rotation);
    assert!(matches!(app.departure, Some(Destination::Step(Step::Cull))));
    assert!(app.navigation_steps().contains(&Step::Rotation));
    assert!(app.decisions.contains_key(&MediaId(1)));
    let destination = app.departure.take().unwrap();
    app.discard_staged_choices();
    app.continue_departure(destination);
    assert_eq!(app.step, Step::Cull);
    assert!(!app.navigation_steps().contains(&Step::Rotation));
}
#[test]
fn hidden_saved_workflow_falls_forward_without_losing_its_bookmark_or_running_models() {
    let (_dir, mut app) = fixture();
    app.preferences
        .hidden_steps
        .extend([Step::PickBest, Step::EasyDelete, Step::Rotation]);
    let saved = crate::core::views::Saved {
        step: Some(Step::PickBest),
        bookmarks: [(Step::PickBest, Default::default())].into_iter().collect(),
        ..Default::default()
    };
    app.load_bookmarks(saved);
    assert_eq!(app.step, Step::Cull);
    assert!(app.saved_views.bookmarks.contains_key(&Step::PickBest));
    assert!(app.analysis_run.is_none() && app.requested.is_empty() && app.textures.is_empty());
    assert!(app.workflow_shortcut(
        egui::Key::Num4,
        egui::Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        }
    ));
    assert_eq!(app.step, Step::Cull);
    assert!(!app.workflow_shortcut(
        egui::Key::Num4,
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        }
    ));
    assert!(app.workflow_shortcut(
        egui::Key::Num1,
        egui::Modifiers {
            command: true,
            alt: true,
            ..Default::default()
        }
    ));
    assert_eq!(app.step, Step::Organize);
    // A new folder has no bookmark, but must not reopen a hidden step retained
    // by a previously cancelled departure.
    app.step = Step::Rotation;
    app.load_bookmarks(Default::default());
    assert_eq!(app.step, Step::Cull);
}
#[test]
fn changing_sensitivity_preserves_current_reviews_decisions_and_cached_results() {
    let (_dir, mut app) = fixture();
    app.step = Step::EasyDelete;
    app.analysis.insert(
        MediaId(1),
        Analysis {
            issues: vec!["Possibly soft".into()],
            ..Default::default()
        },
    );
    app.decisions.insert(
        MediaId(1),
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let mut next = app.preferences.clone();
    next.easy_delete.blur = 10.0;
    app.apply_preferences(next);
    assert_eq!(app.preferences.easy_delete.blur, 10.0);
    assert_eq!(app.analysis[&MediaId(1)].issues, vec!["Possibly soft"]);
    assert_eq!(app.decisions[&MediaId(1)].rating, Some(4));
    assert!(app.analysis_run.is_none() && app.requested.is_empty());
}
#[test]
fn saving_cpu_allocation_updates_shared_runtime_without_restarting_current_review() {
    use crate::core::processing::{Mode, Preferences as Cpu};
    let (_dir, mut app) = fixture();
    app.analysis_run = Some(77);
    app.step = Step::PickBest;
    app.decisions.insert(
        MediaId(1),
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let mut next = app.preferences.clone();
    next.processing = Cpu {
        backend: crate::core::processing::Backend::CoreMl,
        mode: Mode::Custom,
        custom_threads: 1,
    };
    app.apply_preferences(next);
    assert_eq!(app.runtime.processing.snapshot(), 1);
    assert_eq!(
        app.runtime.processing.snapshot_run().backend,
        crate::core::processing::Backend::CoreMl
    );
    assert_eq!(app.analysis_run, Some(77));
    assert_eq!(app.step, Step::PickBest);
    assert_eq!(app.decisions[&MediaId(1)].rating, Some(4));
    assert!(app.departure.is_none());
}
#[test]
fn settings_preserve_live_history_and_render_compactly_without_image_or_analysis_work() {
    let (dir, mut app) = fixture();
    let draft = app.preferences.clone();
    app.preferences.remember(dir.path());
    app.apply_preferences(draft);
    assert_eq!(
        app.preferences.recent_folders,
        vec![dir.path().to_path_buf()]
    );
    app.preferences_draft = Some(app.preferences.clone());
    let ctx = egui::Context::default();
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 600.0),
            )),
            ..Default::default()
        },
        |ctx| app.preferences_dialog(ctx),
    );
    assert!(!output.shapes.is_empty());
    assert!(app.analysis_run.is_none() && app.metadata_run.is_none() && app.requested.is_empty());
    assert!(app.preferences_draft.is_some());
}
#[test]
fn clearing_shared_caches_preserves_every_workflows_review_and_bookmark_state() {
    let (_dir, mut app) = fixture();
    for step in Step::ALL {
        app.step = step;
        app.saved_views.bookmarks.insert(step, Default::default());
        app.decisions.insert(
            MediaId(1),
            Decision {
                rating: Some(4),
                rejected: true,
                ..Default::default()
            },
        );
        app.analysis.insert(
            MediaId(1),
            Analysis {
                issues: vec!["Possibly soft".into()],
                ..Default::default()
            },
        );
        let saved = serde_json::to_vec(&app.saved_views).unwrap();
        app.stats.resident_bytes = 123;
        app.stats.detail_bytes = 456;
        assert!(app.runtime.cache.request(true));
        let deadline = Instant::now() + std::time::Duration::from_secs(5);
        while app.runtime.cache.status().busy {
            assert!(Instant::now() < deadline);
            app.poll(&egui::Context::default());
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        app.poll(&egui::Context::default());
        assert_eq!((app.stats.resident_bytes, app.stats.detail_bytes), (0, 0));
        assert_eq!(app.step, step);
        assert_eq!(serde_json::to_vec(&app.saved_views).unwrap(), saved);
        assert_eq!(app.decisions[&MediaId(1)].rating, Some(4));
        assert!(app.decisions[&MediaId(1)].rejected);
        assert_eq!(app.analysis[&MediaId(1)].issues, vec!["Possibly soft"]);
        assert!(app.analysis_run.is_none() && app.metadata_run.is_none() && !app.scanning);
    }
}
#[test]
fn escape_cancels_settings_without_applying_draft_navigation() {
    let (_dir, mut app) = fixture();
    let mut draft = app.preferences.clone();
    draft.navigation.rating = Some(5);
    app.preferences_draft = Some(draft);
    let ctx = egui::Context::default();
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(640.0, 600.0),
        )),
        ..Default::default()
    };
    for _ in 0..2 {
        let _ = ctx.run(raw(), |ctx| app.preferences_dialog(ctx));
    }
    let mut input = raw();
    input.events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    let _ = ctx.run(input, |ctx| app.preferences_dialog(ctx));
    assert!(app.preferences_draft.is_none());
    assert_eq!(app.preferences.navigation.rating, None);
    assert!(app.analysis_run.is_none() && app.metadata_run.is_none());
}
#[test]
fn settings_access_reuses_draft_and_requests_usage_once_without_clearing() {
    let (_dir, mut app) = fixture();
    app.runtime.shutdown();
    app.runtime.cache.take();
    app.runtime.cache.finish(Ok(Default::default()), false);
    app.open_settings(Section::Caches);
    let request = app.runtime.cache.take().expect("one usage request");
    assert!(!request.clear);
    app.runtime.cache.finish(Ok(Default::default()), false);
    app.preferences_draft.as_mut().unwrap().navigation.rating = Some(5);
    app.open_settings(Section::General);
    app.open_settings(Section::Caches);
    assert!(
        app.runtime.cache.take().is_none(),
        "section navigation repeated cache work"
    );
    assert_eq!(
        app.preferences_draft.as_ref().unwrap().navigation.rating,
        Some(5)
    );
    assert_eq!(app.preferences.navigation.rating, None);
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
#[test]
fn settings_access_cache_and_general_anchors_keep_footer_visible_at_small_sizes() {
    for size in [egui::vec2(600., 500.), egui::vec2(1440., 900.)] {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        ctx.enable_accesskit();
        let mut time = 0.;
        for (section, label) in [
            (Section::Caches, "Shared caches"),
            (Section::General, "Application updates"),
            (Section::Caches, "Shared caches"),
        ] {
            app.open_settings(section);
            let mut output = None;
            for _ in 0..8 {
                time += 0.1;
                output = Some(ctx.run(
                    egui::RawInput {
                        time: Some(time),
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| app.preferences_dialog(ctx),
                ));
            }
            let tree = output.unwrap().platform_output.accesskit_update.unwrap();
            for label in [label, "General", "Caches", "Cancel", "Save settings"] {
                let node = tree
                    .nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label) || n.value() == Some(label))
                    .unwrap_or_else(|| {
                        panic!(
                            "missing {label} at {size:?}: {:?}",
                            tree.nodes
                                .iter()
                                .filter_map(|(_, n)| n.label())
                                .collect::<Vec<_>>()
                        )
                    });
                let bounds = node.1.bounds().expect("visible control bounds");
                assert!(
                    bounds.x0 >= 0.
                        && bounds.x1 <= f64::from(size.x)
                        && bounds.y0 >= 0.
                        && bounds.y1 <= f64::from(size.y),
                    "{label}: {bounds:?}, {size:?}"
                );
            }
        }
        assert!(
            app.analysis_run.is_none() && app.metadata_run.is_none() && app.requested.is_empty()
        );
        app.runtime.shutdown();
    }
}
