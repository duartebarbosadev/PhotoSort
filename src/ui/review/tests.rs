use super::*;
fn comparison_fixture(step: Step) -> (tempfile::TempDir, PhotoSortApp) {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.media = (0..12)
        .map(|n| Media {
            id: MediaId(n),
            path: temp.path().join(format!("{n}.jpg")),
            kind: if n == 2 {
                MediaKind::Video
            } else {
                MediaKind::Photo
            },
            fingerprint: Default::default(),
        })
        .collect();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.filtered = (0..12).collect();
    app.selection.replace(app.media.iter().map(|m| m.id));
    app.comparison = (0..3).map(MediaId).collect();
    app.active = Some(MediaId(0));
    app.review = true;
    app.step = step;
    (temp, app)
}
#[test]
fn comparison_others_uses_clicked_pane_not_active_selection_or_library() {
    for step in [Step::Organize, Step::Cull] {
        let (_temp, mut app) = comparison_fixture(step);
        app.decisions.entry(MediaId(1)).or_default().rating = Some(5);
        app.decisions.entry(MediaId(9)).or_default().rejected = true;
        app.comparison.extend([MediaId(2), MediaId(99)]);
        assert_eq!(
            app.comparison_others(MediaId(1)),
            vec![MediaId(0), MediaId(2)]
        );
        app.mark_comparison_others(MediaId(1), true);
        assert!(app.decision(MediaId(0)).rejected && app.decision(MediaId(2)).rejected);
        assert!(!app.decision(MediaId(1)).rejected);
        assert_eq!(app.decision(MediaId(1)).rating, Some(5));
        assert!(app.decision(MediaId(9)).rejected);
        assert_eq!(app.decisions.len(), 4);
        assert_eq!(app.active, Some(MediaId(0)));
        assert_eq!(app.selection.len(), 12);
        app.mark_comparison_others(MediaId(1), false);
        assert!(!app.decision(MediaId(0)).rejected && !app.decision(MediaId(2)).rejected);
        assert!(app.decision(MediaId(9)).rejected);
        assert!(app.requested.is_empty() && app.inspection_wanted.is_empty());
        assert!(app.analysis_run.is_none() && app.rotation_run.is_none() && !app.busy);
        assert_eq!(app.stats.decodes, 0);
        assert!(app.plan.is_none() && app.applying_plan.is_none());
        app.runtime.shutdown();
    }
}
#[test]
fn comparison_others_rejects_stale_busy_single_and_guided_contexts() {
    for step in Step::ALL {
        let (_temp, mut app) = comparison_fixture(step);
        if matches!(step, Step::Organize | Step::Cull) {
            for state in 0..7 {
                app.busy = state == 0;
                app.scanning = state == 1;
                app.review = state != 2;
                app.plan_open = state == 3;
                app.error = (state == 4).then(|| "error".into());
                app.comparison = if state == 5 {
                    vec![MediaId(0)]
                } else if state == 6 {
                    vec![MediaId(1), MediaId(2)]
                } else {
                    vec![MediaId(0), MediaId(1), MediaId(2)]
                };
                app.mark_comparison_others(MediaId(0), true);
                assert!(app.decisions.is_empty());
            }
        } else {
            app.mark_comparison_others(MediaId(0), true);
            assert!(app.decisions.is_empty());
        }
        assert!(app.requested.is_empty());
        app.runtime.shutdown();
    }
}
#[test]
fn comparison_menu_pointer_action_is_bounded_and_compact() {
    for step in [Step::Organize, Step::Cull] {
        let (_temp, mut app) = comparison_fixture(step);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(340.0, 430.0),
            )),
            ..Default::default()
        };
        let mut draw = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| app.review_context_menu(ui, MediaId(1)));
        };
        let output = ctx.run(input(), &mut draw);
        let tree = output.platform_output.accesskit_update.unwrap();
        let (_, node) = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Mark other 2 for Trash"))
            .unwrap();
        let b = node.bounds().unwrap();
        assert!(b.x0 >= 0.0 && b.x1 <= 340.0 && b.y0 >= 0.0 && b.y1 <= 430.0);
        let pos = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
        let mut raw = input();
        raw.events = vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            },
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            },
        ];
        let _ = ctx.run(raw, &mut draw);
        assert!(app.decision(MediaId(0)).rejected && app.decision(MediaId(2)).rejected);
        assert!(!app.decision(MediaId(1)).rejected && !app.decision(MediaId(9)).rejected);
        assert!(app.requested.is_empty() && app.inspection_wanted.is_empty());
        assert_eq!(app.stats.decodes, 0);
        app.runtime.shutdown();
    }
}
#[test]
fn zoom_controls_report_the_same_limits_and_values_as_keyboard_and_wheel() {
    for step in Step::ALL {
        let cache = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, cache.path().into()), None);
        app.step = step;
        for (factor, expected) in [
            (0.075, 0.075),
            (20.0, 20.0),
            (1000.0, 32.0),
            (0.00001, 0.05),
        ] {
            app.fit_view();
            app.zoom_view(factor);
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.enable_accesskit();
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.review_ratings(ui, MediaId(1)));
            });
            let tree = output.platform_output.accesskit_update.unwrap();
            let slider = tree
                .nodes
                .iter()
                .map(|(_, node)| node)
                .find(|node| node.role() == egui::accesskit::Role::Slider)
                .unwrap();
            assert_eq!(slider.numeric_value(), Some(f64::from(expected)));
            assert_eq!(
                slider.min_numeric_value(),
                Some(f64::from(InspectionView::MIN_ZOOM))
            );
            assert_eq!(
                slider.max_numeric_value(),
                Some(f64::from(InspectionView::MAX_ZOOM))
            );
            assert_eq!(app.inspection_view.zoom, expected);
        }
    }
}
#[test]
fn adequate_previews_do_not_request_idle_originals_in_any_workflow() {
    for step in Step::ALL {
        let cache = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, cache.path().into()), None);
        let id = MediaId(1);
        app.media.push(Media {
            id,
            path: "/missing.ARW".into(),
            kind: MediaKind::Raw,
            fingerprint: Fingerprint::default(),
        });
        app.index.insert(id, 0);
        app.metadata.insert(
            id,
            Metadata {
                width: 6000,
                height: 4000,
                orientation: 1,
                ..Default::default()
            },
        );
        app.active = Some(id);
        app.step = step;
        app.view_started = Instant::now() - Duration::from_secs(2);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let texture = ctx.load_texture(
            "preview",
            egui::ColorImage::filled([1200, 800], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        app.textures.insert(
            (id, true),
            Texture {
                handle: texture,
                bytes: 1200 * 800 * 4,
                used: 0,
            },
        );
        let draw = |app: &mut PhotoSortApp| {
            app.inspection_wanted.clear();
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        app.photo_canvas(ui, id, 0, egui::vec2(640., 400.), false);
                    });
                },
            );
        };
        draw(&mut app);
        assert!(
            app.inspection_wanted.is_empty(),
            "{step:?} scheduled unnecessary full RAW rendering"
        );
        app.actual_size();
        draw(&mut app);
        assert_eq!(
            app.inspection_wanted.len(),
            1,
            "{step:?} did not request explicit original detail"
        );
        assert!(app.decisions.is_empty());
    }
}
#[test]
fn shared_canvas_pans_each_pointer_gesture_once_across_frame_boundaries() {
    for step in Step::ALL {
        for (timing, synced) in
            (0..4).flat_map(|timing| [true, false].map(|synced| (timing, synced)))
        {
            let cache = tempfile::tempdir().unwrap();
            let mut app =
                PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, cache.path().into()), None);
            let id = MediaId(1);
            app.media.push(Media {
                id,
                path: "/missing-photo.jpg".into(),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint::default(),
            });
            app.index.insert(id, 0);
            app.metadata.insert(
                id,
                Metadata {
                    width: 6000,
                    height: 4000,
                    orientation: 1,
                    ..Default::default()
                },
            );
            app.active = Some(id);
            app.step = step;
            app.sync_views = synced;
            app.actual_size();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let frame = |app: &mut PhotoSortApp, events| {
                app.inspection_wanted.clear();
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            app.photo_canvas(ui, id, 0, egui::vec2(640.0, 400.0), false);
                        });
                    },
                );
            };
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            let original = app.inspection_wanted[0].region;
            let start = egui::pos2(100.0, 100.0);
            let middle = egui::pos2(130.0, 115.0);
            let end = egui::pos2(220.0, 160.0);
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            };
            use egui::Event::PointerMoved;
            match timing {
                0 => {
                    frame(&mut app, vec![PointerMoved(start), button(start, true)]);
                    frame(&mut app, vec![PointerMoved(middle)]);
                    frame(&mut app, vec![PointerMoved(end)]);
                    frame(&mut app, vec![button(end, false)]);
                }
                1 => frame(
                    &mut app,
                    vec![
                        PointerMoved(start),
                        button(start, true),
                        PointerMoved(end),
                        button(end, false),
                    ],
                ),
                2 => {
                    frame(&mut app, vec![PointerMoved(start), button(start, true)]);
                    frame(&mut app, vec![PointerMoved(end), button(end, false)]);
                }
                _ => {
                    frame(
                        &mut app,
                        vec![PointerMoved(start), button(start, true), PointerMoved(end)],
                    );
                    frame(&mut app, vec![button(end, false)]);
                }
            }
            let moved_now = app.inspection_wanted[0].region;
            assert_eq!(
                (original.x - moved_now.x, original.y - moved_now.y),
                (120, 60),
                "The completed gesture must request the visible region in the same frame"
            );
            for _ in 0..2 {
                frame(&mut app, vec![]);
            }
            assert_eq!(
                app.view_for(0).pan,
                [120.0, 60.0],
                "{step:?}, timing {timing}, synced {synced}"
            );
            assert_eq!(
                app.view_for(1).pan,
                if synced { [120.0, 60.0] } else { [0.0, 0.0] }
            );
            let moved = app.inspection_wanted[0].region;
            assert_eq!((original.x - moved.x, original.y - moved.y), (120, 60));
            assert!(
                app.decisions.is_empty(),
                "Panning changed a review decision"
            );
        }
    }
}
#[test]
fn entering_easy_delete_before_analysis_renders_an_empty_review_safely() {
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.active = Some(MediaId(1));
    app.set_step(Step::EasyDelete);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let _ = ctx.run(egui::RawInput::default(), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| app.review_canvas(ui));
    });
}
#[test]
fn a_single_comparison_row_fits_the_short_canvas_without_clipping() {
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.media.push(Media {
        id: MediaId(1),
        path: "/missing-photo.jpg".into(),
        kind: MediaKind::Photo,
        fingerprint: Fingerprint::default(),
    });
    app.index.insert(MediaId(1), 0);
    app.active = Some(MediaId(1));
    app.set_step(Step::Rotation);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let output = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 180.0),
            )),
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.review_canvas(ui));
        },
    );
    let panels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            if let egui::Shape::Rect(rect) = &shape.shape
                && rect.fill == BG
                && rect.rect.width() > 200.0
                && rect.rect.width() < 400.0
            {
                Some((shape.clip_rect, rect.rect))
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        panels.len(),
        2,
        "Exercise both panels in the shared comparison canvas"
    );
    for (clip, panel) in panels {
        assert!(
            clip.contains_rect(panel.shrink(1.0)),
            "Photo panel {panel:?} exceeds viewport {clip:?}"
        );
    }
}

#[test]
fn details_panel_compares_every_open_photo_and_returns_to_one_photo() {
    let (_temp, mut app) = comparison_fixture(Step::Cull);
    for (id, flash) in [(0, 25), (1, 0)] {
        app.metadata.insert(
            MediaId(id),
            Metadata {
                width: 4000,
                height: 3000,
                orientation: 1,
                details: crate::core::model::Details {
                    flash: Some(flash),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let texts = |app: &mut PhotoSortApp| -> Vec<String> {
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1200.0, 900.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        egui::ScrollArea::vertical().show(ui, |ui| app.inspector(ui))
                    });
                },
            ));
        }
        let tree = output.unwrap().platform_output.accesskit_update.unwrap();
        tree.nodes
            .iter()
            .filter_map(|(_, n)| n.value().or(n.label()).map(str::to_owned))
            .collect()
    };
    let compared = texts(&mut app);
    for text in [
        "Compare details",
        "0.jpg",
        "1.jpg",
        "2.jpg",
        "Fired (Auto)",
        "Off",
        "12.0 MP",
    ] {
        assert!(
            compared.iter().any(|t| t == text),
            "missing {text}: {compared:?}"
        );
    }
    app.comparison.clear();
    let single = texts(&mut app);
    assert!(single.iter().all(|t| t != "Compare details"));
    assert!(single.iter().any(|t| t == "Fired (Auto)"));
    assert!(single.iter().all(|t| t != "Off"));
    app.runtime.shutdown();
}

#[test]
fn wheel_zoom_keeps_the_point_under_the_pointer_and_fit_recentres() {
    let cache = tempfile::tempdir().unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, cache.path().into()), None);
    app.fit_view();
    app.zoom_view(2.0);
    let mut view = app.view_for(app.focused_slot);
    view.pan = [10.0, -5.0];
    app.set_view(app.focused_slot, view);
    let anchor = egui::vec2(100.0, 50.0);
    // Screen offsets scale with zoom, so (anchor - pan) / zoom is the image
    // point under the pointer; it must not move.
    let under = |v: InspectionView| {
        [
            (anchor.x - v.pan[0]) / v.zoom,
            (anchor.y - v.pan[1]) / v.zoom,
        ]
    };
    let before = under(app.view_for(app.focused_slot));
    app.zoom_view_at(1.5, anchor);
    let after = app.view_for(app.focused_slot);
    assert!((after.zoom - 3.0).abs() < 1e-5);
    for (now, then) in under(after).into_iter().zip(before) {
        assert!((now - then).abs() < 1e-3);
    }
    app.zoom_view_at(0.1, anchor);
    assert_eq!(app.view_for(app.focused_slot).pan, [0.0, 0.0]);
    app.runtime.shutdown();
}
