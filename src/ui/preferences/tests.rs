use super::*;
use crate::test_support::worker_patience;
#[test]
fn settings_errors_are_bounded_and_dismissible_without_hiding_footer_or_losing_draft() {
    for size in [egui::vec2(600., 500.), egui::vec2(1440., 990.)] {
        let (_dir, mut app) = fixture();
        app.open_settings(Section::Storage);
        app.preferences_draft.as_mut().unwrap().navigation.rating = Some(5);
        let error = "Open folder failed: /unavailable/long cache folder café/".repeat(20);
        app.error = Some(error.clone());
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let mut time = 0.;
        let mut frame = |app: &mut PhotoSortApp, events| {
            time += 0.1;
            ctx.run_test(
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = &ui.ctx().clone();
                    app.dialogs(ctx)
                },
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
            "Storage",
            "Close without saving",
            "Apply",
            "Save",
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
        app.open_settings(Section::Storage);
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
            app.open_settings(Section::Storage);
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.enable_accesskit();
            let mut time = 0.;
            let mut frame = |app: &mut PhotoSortApp, events| {
                time += 0.1;
                ctx.run_test(
                    egui::RawInput {
                        time: Some(time),
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let ctx = &ui.ctx().clone();
                        app.preferences_dialog(ctx)
                    },
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
            let request = received.recv_timeout(worker_patience()).unwrap();
            assert_eq!(request.path, cache);
            assert_eq!(request.action, Action::OpenFolder);
            let deadline = Instant::now() + worker_patience();
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
fn an_unreadable_preferences_file_is_kept_until_settings_are_saved() {
    let (dir, mut app) = fixture();
    let path = dir.path().join("preferences.json");
    // A newer PhotoSort's file, say.
    let unreadable = b"[2, {\"show_shortcuts\": false}]";
    std::fs::write(&path, unreadable).unwrap();
    app.load_preferences(path.clone());
    assert!(app.preferences_unreadable && app.error.is_some());
    // Opening a folder remembers it and the startup check records its
    // attempt; neither writes defaults over the file.
    app.preferences.remember(dir.path());
    app.save_preferences();
    app.check_updates_at_startup();
    // Shutdown finishes every queued write.
    app.runtime.shutdown();
    assert_eq!(std::fs::read(&path).unwrap(), unreadable);

    let (_other, mut app) = fixture();
    app.load_preferences(path.clone());
    let mut draft = app.preferences.clone();
    draft.show_shortcuts = false;
    app.save_settings(draft);
    assert!(!app.preferences_unreadable);
    app.runtime.shutdown();
    let saved = Preferences::load(&path).unwrap();
    assert!(!saved.show_shortcuts);
    assert_eq!(saved.recent_folders, app.preferences.recent_folders);
}
#[test]
fn hiding_current_step_moves_on_and_keeps_its_staged_choices() {
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
    app.save_settings(p);
    // Nothing is asked: the staged rating waits for the Review step.
    assert!(app.departure.is_none());
    assert_eq!(app.step, Step::Cull);
    assert!(app.decisions.contains_key(&MediaId(1)));
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
    app.save_settings(next);
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
    app.save_settings(next);
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
    app.save_settings(draft);
    assert_eq!(
        app.preferences.recent_folders,
        vec![dir.path().to_path_buf()]
    );
    app.preferences_draft = Some(app.preferences.clone());
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let output = ctx.run_test(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 600.0),
            )),
            ..Default::default()
        },
        |ui| {
            let ctx = &ui.ctx().clone();
            app.preferences_dialog(ctx)
        },
    );
    assert!(!output.shapes.is_empty());
    assert!(app.analysis_run.is_none() && app.metadata_run.is_none() && app.requested.is_empty());
    assert!(app.preferences_draft.is_some());
    // Clearing recent folders acts at once and is not an unsaved change; a
    // draft edit marks only the page that owns it.
    app.clear_recent_folders();
    let mut draft = app.preferences_draft.take().unwrap();
    draft.recent_folders = vec![dir.path().to_path_buf()];
    assert!(!pages::has_pending(&draft, &app.preferences));
    draft.cache.disk_mib += 1;
    let pending: Vec<_> = Section::ALL
        .into_iter()
        .filter(|s| s.pending(&draft, &app.preferences))
        .collect();
    assert_eq!(pending, [Section::Storage]);
    // "Show again" for prompts turned off is a change to save on its own.
    app.preferences.limit_prompts.wanted_raw = Some(crate::core::preferences::WantedRaw::Keep);
    let mut draft = app.preferences.clone();
    assert!(draft.limit_prompts.any() && !pages::has_pending(&draft, &app.preferences));
    draft.limit_prompts = Default::default();
    let pending: Vec<_> = Section::ALL
        .into_iter()
        .filter(|s| s.pending(&draft, &app.preferences))
        .collect();
    assert_eq!(pending, [Section::Performance]);
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
        let deadline = Instant::now() + worker_patience();
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
    theme_install(&ctx);
    let raw = || egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(640.0, 600.0),
        )),
        ..Default::default()
    };
    for _ in 0..2 {
        let _ = ctx.run_test(raw(), |ui| {
            let ctx = &ui.ctx().clone();
            app.preferences_dialog(ctx)
        });
    }
    let mut input = raw();
    input.events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    let _ = ctx.run_test(input, |ui| {
        let ctx = &ui.ctx().clone();
        app.preferences_dialog(ctx)
    });
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
    app.open_settings(Section::Storage);
    let request = app.runtime.cache.take().expect("one usage request");
    assert!(!request.clear);
    app.runtime.cache.finish(Ok(Default::default()), false);
    app.preferences_draft.as_mut().unwrap().navigation.rating = Some(5);
    app.open_settings(Section::General);
    app.open_settings(Section::Storage);
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
        theme_install(&ctx);
        ctx.enable_accesskit();
        let mut time = 0.;
        for (section, label) in [
            (Section::Storage, "Disk cache limit"),
            (Section::General, "Save changes to"),
            (Section::Storage, "Disk cache limit"),
        ] {
            app.open_settings(section);
            let mut output = None;
            for _ in 0..8 {
                time += 0.1;
                output = Some(ctx.run_test(
                    egui::RawInput {
                        time: Some(time),
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ui| {
                        let ctx = &ui.ctx().clone();
                        app.preferences_dialog(ctx)
                    },
                ));
            }
            let tree = output.unwrap().platform_output.accesskit_update.unwrap();
            for label in [label, "General", "Storage", "Close", "Apply", "Save"] {
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
#[test]
fn switching_the_raw_look_reloads_only_raw_previews() {
    let (dir, mut app) = fixture();
    app.media = [(1, MediaKind::Raw), (2, MediaKind::Photo)]
        .into_iter()
        .map(|(n, kind)| Media {
            id: MediaId(n),
            path: dir.path().join(format!("{n}.file")),
            kind,
            fingerprint: Fingerprint::default(),
        })
        .collect();
    let ctx = egui::Context::default();
    for n in [1, 2] {
        let handle = ctx.load_texture(
            format!("t{n}"),
            egui::ColorImage::filled([2, 2], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        app.textures.insert(
            (MediaId(n), false),
            Texture {
                handle,
                bytes: 16,
                used: 0,
            },
        );
        app.requested.insert((MediaId(n), true));
    }
    let mut next = app.preferences.clone();
    next.raw_look = crate::core::raw_look::RawLook::Enhanced;
    app.save_settings(next);
    assert_eq!(
        app.runtime.raw_look.get(),
        crate::core::raw_look::RawLook::Enhanced
    );
    assert!(!app.textures.contains_key(&(MediaId(1), false)));
    assert!(app.textures.contains_key(&(MediaId(2), false)));
    assert_eq!(app.requested, [(MediaId(2), true)].into_iter().collect());
    app.runtime.shutdown();
}
#[test]
fn first_visit_to_each_settings_page_draws_the_dialog_in_place() {
    for size in [egui::vec2(600., 500.), egui::vec2(1440., 900.)] {
        let (_dir, mut app) = fixture();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let mut time = 0.;
        let mut save = |app: &mut PhotoSortApp| {
            time += 0.1;
            let output = ctx.run_test(
                egui::RawInput {
                    time: Some(time),
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    let ctx = &ui.ctx().clone();
                    app.preferences_dialog(ctx)
                },
            );
            let tree = output.platform_output.accesskit_update.unwrap();
            let node = &tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Save"))
                .unwrap()
                .1;
            node.bounds().unwrap()
        };
        app.open_settings(Section::General);
        let mut settled = save(&mut app);
        for _ in 0..8 {
            settled = save(&mut app);
        }
        // The frame shown on the first visit is already the settled layout.
        for section in Section::ALL {
            app.open_settings(section);
            assert_eq!(save(&mut app), settled, "{section:?} at {size:?}");
        }
        app.runtime.shutdown();
    }
}
#[test]
fn apply_saves_and_keeps_settings_open_cancel_drops_only_what_is_unsaved_and_save_closes() {
    use crate::core::processing::Backend;
    let (dir, mut app) = fixture();
    let path = dir.path().join("preferences.json");
    app.preferences_path = Some(path.clone());
    app.open_settings(Section::Performance);
    app.preferences_draft.as_mut().unwrap().processing.backend = Backend::Cpu;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut time = 0.;
    let mut frame = |app: &mut PhotoSortApp, events: Vec<egui::Event>| {
        time += 0.1;
        ctx.run_test(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1000.0, 700.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.preferences_dialog(ctx)
            },
        )
    };
    let find = |output: &egui::FullOutput, label: &str| {
        output
            .platform_output
            .accesskit_update
            .clone()
            .unwrap()
            .nodes
            .into_iter()
            .find(|(_, n)| n.label() == Some(label) || n.value() == Some(label))
            .map(|(_, n)| n)
    };
    let mut click = |app: &mut PhotoSortApp, label: &str| {
        let mut output = frame(app, vec![]);
        for _ in 0..4 {
            output = frame(app, vec![]);
        }
        let node = find(&output, label).unwrap_or_else(|| panic!("no {label}"));
        assert!(!node.is_disabled(), "{label} is disabled");
        let rect = node.bounds().unwrap();
        let pos = egui::pos2(
            ((rect.x0 + rect.x1) / 2.0) as f32,
            ((rect.y0 + rect.y1) / 2.0) as f32,
        );
        for pressed in [true, false] {
            frame(
                app,
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
        frame(app, vec![])
    };
    // The file worker writes in the background.
    let saved = |check: &dyn Fn(&Preferences) -> bool| {
        let deadline = Instant::now() + worker_patience();
        loop {
            if Preferences::load(&path).is_ok_and(|p| check(&p)) {
                return;
            }
            assert!(Instant::now() < deadline, "preferences were not written");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    };
    // Apply: in effect and written, for the next analysis, with Settings open.
    let output = click(&mut app, "Apply");
    assert!(app.preferences_draft.is_some(), "Apply keeps Settings open");
    assert_eq!(app.runtime.processing.snapshot_run().backend, Backend::Cpu);
    saved(&|p| p.processing.backend == Backend::Cpu);
    assert!(find(&output, "Unsaved changes").is_none());
    assert!(find(&output, "Apply").unwrap().is_disabled());
    assert!(find(&output, "Save").unwrap().is_disabled());
    // Nothing unsaved: the × reads Close.
    assert!(find(&output, "Close").is_some() && find(&output, "Close without saving").is_none());
    // Closing drops the edit made since Apply and keeps what Apply saved.
    app.preferences_draft.as_mut().unwrap().show_shortcuts = false;
    click(&mut app, "Close without saving");
    assert!(app.preferences_draft.is_none());
    assert!(app.preferences.show_shortcuts);
    assert_eq!(app.runtime.processing.snapshot_run().backend, Backend::Cpu);
    saved(&|p| p.show_shortcuts && p.processing.backend == Backend::Cpu);
    // Save writes and closes.
    app.open_settings(Section::Performance);
    app.preferences_draft.as_mut().unwrap().processing.backend = Backend::Auto;
    click(&mut app, "Save");
    assert!(app.preferences_draft.is_none(), "Save closes Settings");
    assert_eq!(app.runtime.processing.snapshot_run().backend, Backend::Auto);
    app.runtime.shutdown();
    assert_eq!(
        Preferences::load(&path).unwrap().processing.backend,
        Backend::Auto
    );
}
#[test]
fn run_ai_on_says_where_ai_will_run_then_where_it_runs() {
    use crate::core::processing::{Accelerators, Backend, Probe};
    let (_dir, mut app) = fixture();
    assert_eq!(app.preferences.processing.backend, Backend::Auto);
    app.runtime.processing.configure(app.preferences.processing);
    assert_eq!(
        app.running_on_note(Backend::Auto),
        "Checking this computer's accelerators…"
    );
    // Before any model loads, the check names the backend for any choice
    // in the dialog, applied or not.
    app.runtime.processing.record_accelerators(Ok(Accelerators {
        probes: vec![
            (Backend::Cuda, Probe::Works),
            (Backend::DirectMl, Probe::NotInRuntime),
        ],
        nvidia_driver: true,
    }));
    if cfg!(any(target_os = "windows", target_os = "linux")) {
        assert_eq!(
            app.running_on_note(Backend::Auto),
            "Will run on NVIDIA CUDA"
        );
    }
    assert_eq!(app.running_on_note(Backend::Cpu), "Will run on CPU");
    assert_eq!(
        app.running_on_note(Backend::DirectMl),
        "Will run on CPU · Windows DirectML unavailable"
    );
    // Once models have loaded, the line says where they actually run.
    app.runtime
        .processing
        .report("dinov2", Backend::Cuda, "Provider enabled".into());
    app.runtime.processing.set_prepared_for(Backend::Auto);
    assert_eq!(app.running_on_note(Backend::Auto), "Running on NVIDIA CUDA");
    // Not part of a running analysis: the progress line stays hidden.
    assert_eq!(app.running_on(), None);
    app.analysis_run = Some(3);
    assert_eq!(app.running_on().as_deref(), Some("Running on NVIDIA CUDA"));
    // Choosing CPU, saved or not, tells what the next analysis uses.
    assert_eq!(
        app.running_on_note(Backend::Cpu),
        "Running on NVIDIA CUDA · next analysis on CPU"
    );
    let mut draft = app.preferences.clone();
    draft.processing.backend = Backend::Cpu;
    app.save_settings(draft);
    assert_eq!(
        app.running_on_note(Backend::Cpu),
        "Running on NVIDIA CUDA · next analysis on CPU"
    );
    assert_eq!(app.running_on().as_deref(), Some("Running on NVIDIA CUDA"));
    // ONNX Runtime missing: the line says the check could not run.
    app.runtime
        .processing
        .record_accelerators(Err("ONNX Runtime library is missing".into()));
    app.runtime.processing.clear_status();
    assert_eq!(
        app.running_on_note(Backend::Cpu),
        "Could not check this computer: ONNX Runtime library is missing"
    );
    app.runtime.shutdown();
}
