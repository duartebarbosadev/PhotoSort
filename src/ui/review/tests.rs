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
        let mut draw = |ui: &mut egui::Ui| {
            egui::CentralPanel::default().show(ui, |ui| app.review_context_menu(ui, MediaId(1)));
        };
        let output = ctx.run_test(input(), &mut draw);
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
        let _ = ctx.run_test(raw, &mut draw);
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
        // Zooming out stops at Fit, the slider's lowest position.
        for (factor, expected) in [(0.075, 1.0), (20.0, 20.0), (1000.0, 32.0), (0.00001, 1.0)] {
            app.fit_view();
            app.zoom_view(factor);
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.enable_accesskit();
            let output = ctx.run_test(egui::RawInput::default(), |ui| {
                egui::CentralPanel::default().show(ui, |ui| app.review_ratings(ui, MediaId(1)));
            });
            let tree = output.platform_output.accesskit_update.unwrap();
            let slider = tree
                .nodes
                .iter()
                .map(|(_, node)| node)
                .find(|node| node.role() == egui::accesskit::Role::Slider)
                .unwrap();
            assert_eq!(slider.numeric_value(), Some(f64::from(expected)));
            assert_eq!(slider.min_numeric_value(), Some(1.0));
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
            let _ = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 600.),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
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
                let _ = ctx.run_test(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800.0, 600.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
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
    let _ = ctx.run_test(egui::RawInput::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| app.review_canvas(ui));
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
    app.set_step(Step::Rotation);
    // A staged turn: the photo as it is beside it turned.
    crate::core::rotation::set_turns(&mut app.decisions, MediaId(1), 1);
    app.active = Some(MediaId(1));
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let output = ctx.run_test(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 180.0),
            )),
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| app.review_canvas(ui));
        },
    );
    let panels: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| {
            // Compared photos sit on CHROME cards.
            if let egui::Shape::Rect(rect) = &shape.shape
                && rect.fill == CHROME
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
    let texts = |app: &mut PhotoSortApp, width: f32| -> Vec<String> {
        let mut output = None;
        for _ in 0..2 {
            output = Some(ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
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
    let count = |texts: &[String], text: &str| texts.iter().filter(|t| *t == text).count();
    // Wide: one column per photo, so the two photos sharing 12.0 MP each show
    // it. Narrow: stacked, and photos that share a value are listed once.
    // Counted against JPG, which every photo shares and is always said once.
    for (width, megapixels) in [(1200.0, 2), (300.0, 1)] {
        let compared = texts(&mut app, width);
        for text in [
            "Compare details",
            "0.jpg",
            "1.jpg",
            "2.jpg",
            "1",
            "2",
            "3",
            "Fired (Auto)",
            "Off",
            "-",
        ] {
            assert!(
                compared.iter().any(|t| t == text),
                "missing {text} at {width}: {compared:?}"
            );
        }
        assert!(count(&compared, "JPG") > 0);
        assert_eq!(
            count(&compared, "12.0 MP"),
            megapixels * count(&compared, "JPG"),
            "{width}: {compared:?}"
        );
    }
    app.comparison.clear();
    for width in [1200.0, 300.0] {
        let single = texts(&mut app, width);
        assert!(single.iter().all(|t| t != "Compare details"));
        assert!(single.iter().any(|t| t == "Fired (Auto)"));
        assert!(single.iter().any(|t| t == "Flash"));
        assert!(single.iter().all(|t| t != "Off"));
    }
    // Organize keeps Show in Finder but no longer repeats where the photo
    // goes or offers renaming here; F2 and the board still do.
    app.step = Step::Organize;
    let organize = texts(&mut app, 300.0);
    assert!(
        organize
            .iter()
            .any(|t| t == crate::core::desktop::Action::Reveal.label())
    );
    for gone in [
        "Stays in",
        "Will move to",
        "Renamed to",
        "Rename",
        "Move to",
    ] {
        assert!(
            organize.iter().all(|t| !t.contains(gone)),
            "{gone}: {organize:?}"
        );
    }
    app.runtime.shutdown();
}

/// An app showing photos of the given pixel sizes, none of them decodable,
/// so the canvas lays each out from its metadata alone.
fn sized_photos(sizes: &[[u32; 2]]) -> (tempfile::TempDir, PhotoSortApp, Vec<MediaId>) {
    let cache = tempfile::tempdir().unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, cache.path().into()), None);
    let ids: Vec<MediaId> = (0..sizes.len() as u64).map(|n| MediaId(n + 1)).collect();
    for (n, (&id, &[width, height])) in ids.iter().zip(sizes).enumerate() {
        app.media.push(Media {
            id,
            path: format!("/missing-photo-{n}.jpg").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        });
        app.index.insert(id, n);
        app.metadata.insert(
            id,
            Metadata {
                width,
                height,
                orientation: 1,
                ..Default::default()
            },
        );
    }
    app.active = Some(ids[0]);
    (cache, app, ids)
}
/// One frame of `ids` side by side on canvases of `size`. The first canvas
/// sits near the top left, so its centre is close to `size / 2`.
fn canvases(
    app: &mut PhotoSortApp,
    ctx: &egui::Context,
    ids: &[MediaId],
    size: Vec2,
    events: Vec<egui::Event>,
) {
    let _ = ctx.run_test(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(size.x * ids.len() as f32 + 100.0, size.y + 100.0),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (slot, id) in ids.iter().enumerate() {
                        app.photo_canvas(ui, *id, slot as u64, size, false);
                    }
                });
            });
        },
    );
}
fn wheel(y: f32) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: egui::vec2(0.0, y),
        phase: egui::TouchPhase::Move,
        modifiers: Default::default(),
    }
}
#[test]
fn sync_is_offered_only_when_there_is_more_than_one_photo() {
    let labels = |app: &mut PhotoSortApp| {
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let output = ctx.run_test(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| app.review_view_controls(ui));
        });
        output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .into_iter()
            .filter_map(|(_, node)| node.label().map(str::to_owned))
            .collect::<Vec<_>>()
    };
    let (_temp, mut app) = comparison_fixture(Step::Cull);
    assert!(labels(&mut app).iter().any(|l| l == "Sync"));
    app.comparison = vec![MediaId(0)];
    let single = labels(&mut app);
    assert!(single.iter().any(|l| l == "Fit") && single.iter().all(|l| l != "Sync"));
    // Fix Rotation compares the original with its correction.
    app.step = Step::Rotation;
    assert!(labels(&mut app).iter().any(|l| l == "Sync"));
    app.runtime.shutdown();
}
#[test]
fn zooming_out_stops_at_fit_on_every_input_path() {
    for step in Step::ALL {
        let (_cache, mut app, ids) = sized_photos(&[[6000, 4000]]);
        app.step = step;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let size = egui::vec2(640.0, 400.0);
        let pointer = egui::pos2(size.x / 2.0 + 40.0, size.y / 2.0 + 30.0);
        canvases(&mut app, &ctx, &ids, size, vec![]);
        // Keyboard, as Ctrl/Cmd+- does.
        for _ in 0..10 {
            app.zoom_view(0.8);
        }
        assert_eq!(app.view_for(0), InspectionView::default(), "{step:?}");
        // Trackpad pinch.
        for _ in 0..3 {
            canvases(
                &mut app,
                &ctx,
                &ids,
                size,
                vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(0.5)],
            );
            assert_eq!(app.view_for(0), InspectionView::default(), "{step:?}");
        }
        // The scroll wheel zooms in, then out no further than Fit.
        for _ in 0..5 {
            canvases(
                &mut app,
                &ctx,
                &ids,
                size,
                vec![egui::Event::PointerMoved(pointer), wheel(200.0)],
            );
        }
        assert!(
            app.view_for(0).zoom > 1.0,
            "{step:?}: the wheel reached the canvas"
        );
        for _ in 0..40 {
            canvases(
                &mut app,
                &ctx,
                &ids,
                size,
                vec![egui::Event::PointerMoved(pointer), wheel(-400.0)],
            );
        }
        assert_eq!(app.view_for(0), InspectionView::default(), "{step:?}");
        // A drag at Fit leaves the photo where it is and decides nothing.
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Default::default(),
        };
        let end = pointer + egui::vec2(120.0, 60.0);
        canvases(
            &mut app,
            &ctx,
            &ids,
            size,
            vec![
                egui::Event::PointerMoved(pointer),
                button(pointer, true),
                egui::Event::PointerMoved(pointer + egui::vec2(30.0, 15.0)),
            ],
        );
        canvases(
            &mut app,
            &ctx,
            &ids,
            size,
            vec![egui::Event::PointerMoved(end), button(end, false)],
        );
        canvases(&mut app, &ctx, &ids, size, vec![]);
        assert_eq!(app.view_for(0), InspectionView::default(), "{step:?}");
        assert!(app.decisions.is_empty(), "{step:?}: the drag clicked");
        app.runtime.shutdown();
    }
}
#[test]
fn one_to_one_zooms_out_to_fit_but_a_small_photo_keeps_one_to_one() {
    let size = egui::vec2(640.0, 400.0);
    let pointer = egui::pos2(size.x / 2.0, size.y / 2.0);
    let pinch_out = || vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(0.5)];
    let slider_min = |app: &mut PhotoSortApp, id| {
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let output = ctx.run_test(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| app.review_ratings(ui, id));
        });
        output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .find(|(_, node)| node.role() == egui::accesskit::Role::Slider)
            .and_then(|(_, node)| node.min_numeric_value())
            .unwrap()
    };
    // A large photo: 1:1 zooms out through its Fit and then is Fit.
    let (_cache, mut app, ids) = sized_photos(&[[6000, 4000]]);
    app.step = Step::Cull;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    app.actual_size();
    canvases(&mut app, &ctx, &ids, size, vec![]);
    let fit = app.fit_zooms[&0];
    assert!(fit < 0.2, "{fit}");
    // The slider's lowest position is this photo's Fit.
    assert!((slider_min(&mut app, ids[0]) - f64::from(fit)).abs() < 1e-6);
    canvases(&mut app, &ctx, &ids, size, pinch_out());
    let view = app.view_for(0);
    assert!(view.actual_size && view.zoom == 0.5, "{view:?}");
    for _ in 0..4 {
        canvases(&mut app, &ctx, &ids, size, pinch_out());
    }
    assert_eq!(app.view_for(0), InspectionView::default());
    // Once Fit, the slider is fit-relative again.
    assert_eq!(slider_min(&mut app, ids[0]), 1.0);
    app.actual_size();
    for _ in 0..6 {
        app.zoom_view(0.5);
    }
    assert_eq!(app.view_for(0), InspectionView::default());
    app.runtime.shutdown();

    // A photo smaller than the view: Fit enlarges it, so zooming out of 1:1
    // stays at 1:1 rather than growing to Fit.
    let (_cache, mut app, ids) = sized_photos(&[[200, 100]]);
    app.step = Step::Cull;
    app.actual_size();
    canvases(&mut app, &ctx, &ids, size, vec![]);
    assert!(app.fit_zooms[&0] > 1.0);
    assert_eq!(slider_min(&mut app, ids[0]), 1.0);
    for _ in 0..3 {
        canvases(&mut app, &ctx, &ids, size, pinch_out());
        app.zoom_view(0.8);
    }
    let one = InspectionView {
        actual_size: true,
        ..InspectionView::default()
    };
    assert_eq!(app.view_for(0), one);
    // The whole photo is on screen, so a drag does not move it.
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    };
    let end = pointer + egui::vec2(100.0, 50.0);
    canvases(
        &mut app,
        &ctx,
        &ids,
        size,
        vec![
            egui::Event::PointerMoved(pointer),
            button(pointer, true),
            egui::Event::PointerMoved(end),
        ],
    );
    canvases(&mut app, &ctx, &ids, size, vec![button(end, false)]);
    assert_eq!(app.view_for(0), one);
    app.runtime.shutdown();
}
#[test]
fn resizing_keeps_fit_and_snaps_a_view_below_the_new_fit() {
    let small = egui::vec2(640.0, 400.0);
    let large = egui::vec2(1600.0, 1000.0);
    for synced in [true, false] {
        let (_cache, mut app, ids) = sized_photos(&[[6000, 4000]]);
        app.step = Step::Cull;
        app.sync_views = synced;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        // Fit stays Fit, and a fit-relative zoom stays relative to the new Fit.
        for zoom in [1.0, 2.0] {
            app.fit_view();
            app.zoom_view(zoom);
            let before = app.view_for(0);
            for size in [small, large, small] {
                canvases(&mut app, &ctx, &ids, size, vec![]);
                assert_eq!(app.view_for(0), before, "synced {synced}");
            }
        }
        // 1:1 zoomed out to just above the small view's Fit is below the
        // large view's Fit, so growing the window snaps it to Fit.
        app.actual_size();
        canvases(&mut app, &ctx, &ids, small, vec![]);
        app.zoom_view(0.15);
        let zoomed = app.view_for(0);
        assert!(zoomed.actual_size && zoomed.zoom == 0.15, "{zoomed:?}");
        canvases(&mut app, &ctx, &ids, small, vec![]);
        assert_eq!(app.view_for(0), zoomed, "synced {synced}");
        canvases(&mut app, &ctx, &ids, large, vec![]);
        assert_eq!(
            app.view_for(0),
            InspectionView::default(),
            "synced {synced}"
        );
        // And it keeps fitting when the window shrinks again.
        canvases(&mut app, &ctx, &ids, small, vec![]);
        assert_eq!(
            app.view_for(0),
            InspectionView::default(),
            "synced {synced}"
        );
        app.runtime.shutdown();
    }
}
#[test]
fn synced_views_follow_the_focused_photo_and_settle() {
    // Landscape and portrait: at 1:1 zoom 0.08 the landscape photo is below
    // its Fit and the portrait one above.
    let (_cache, mut app, ids) = sized_photos(&[[6000, 4000], [4000, 6000]]);
    app.step = Step::Cull;
    app.comparison = ids.clone();
    app.sync_views = true;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let size = egui::vec2(640.0, 400.0);
    let zoomed = InspectionView {
        zoom: 0.08,
        actual_size: true,
        ..InspectionView::default()
    };
    app.inspection_view = zoomed;
    app.focused_slot = 1;
    for _ in 0..3 {
        canvases(&mut app, &ctx, &ids, size, vec![]);
        assert_eq!(app.view_for(0), zoomed);
        assert_eq!(app.view_for(1), zoomed);
    }
    app.focused_slot = 0;
    for _ in 0..3 {
        canvases(&mut app, &ctx, &ids, size, vec![]);
        assert_eq!(app.view_for(0), InspectionView::default());
        assert_eq!(app.view_for(1), InspectionView::default());
    }
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

#[test]
fn each_compared_photo_can_be_rated_from_its_own_stars() {
    let cache = tempfile::tempdir().unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, cache.path().into()), None);
    let id = MediaId(5);
    app.media = vec![Media {
        id,
        path: cache.path().join("photo.jpg"),
        kind: MediaKind::Photo,
        fingerprint: Fingerprint::default(),
    }];
    app.index.insert(id, 0);
    app.filtered = vec![0];
    app.step = Step::Cull;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let run = |app: &mut PhotoSortApp, events: Vec<egui::Event>| {
        ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let slot = ui.max_rect();
                    app.slot_stars(ui, slot, id);
                });
            },
        )
    };
    let output = run(&mut app, vec![]);
    let tree = output.platform_output.accesskit_update.unwrap();
    let bounds = tree
        .nodes
        .iter()
        .find(|(_, n)| n.label() == Some("Rate 3"))
        .and_then(|(_, n)| n.bounds())
        .expect("Rate 3 star");
    let at = egui::pos2(
        ((bounds.x0 + bounds.x1) / 2.0) as f32,
        ((bounds.y0 + bounds.y1) / 2.0) as f32,
    );
    for pressed in [true, false] {
        run(
            &mut app,
            vec![
                egui::Event::PointerMoved(at),
                egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
    }
    assert_eq!(app.selected_rating(id), 3);
    app.runtime.shutdown();
}

#[test]
fn stepping_to_an_unloaded_photo_keeps_the_previous_one_instead_of_flashing() {
    let (_temp, mut app) = comparison_fixture(Step::Cull);
    app.comparison.clear();
    app.selection.clear();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let first = ctx.load_texture(
        "first",
        egui::ColorImage::filled([40, 30], Color32::WHITE),
        egui::TextureOptions::LINEAR,
    );
    let first_id = first.id();
    app.textures.insert(
        (MediaId(0), true),
        Texture {
            handle: first,
            bytes: 4800,
            used: 0,
        },
    );
    let draw = |app: &mut PhotoSortApp| {
        ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let id = app.active.unwrap();
                    app.photo_canvas(ui, id, 0, egui::vec2(700.0, 500.0), true);
                });
            },
        )
    };
    let _ = draw(&mut app);
    // The next photo has no pixels yet: the first stays on screen and no
    // loading text appears.
    app.set_active(MediaId(1));
    let output = draw(&mut app);
    let painted = |shape: &egui::Shape, texture: egui::TextureId| match shape {
        egui::Shape::Mesh(mesh) => mesh.texture_id == texture,
        _ => false,
    };
    assert!(
        output
            .shapes
            .iter()
            .any(|clipped| painted(&clipped.shape, first_id)),
        "The previous photo must stay on screen while the next loads"
    );
    let text = |shape: &egui::Shape| match shape {
        egui::Shape::Text(text) => text.galley.text().contains("Loading preview"),
        _ => false,
    };
    assert!(!output.shapes.iter().any(|clipped| text(&clipped.shape)));
    // The previous photo's neighbours were asked for ahead of time.
    assert!(app.requested.contains(&(MediaId(1), true)));
}

#[test]
fn fast_scrolling_keeps_only_a_few_thumbnails_queued() {
    let (_temp, mut app) = comparison_fixture(Step::Cull);
    for n in 0..12 {
        let _ = app.texture(MediaId(n), false);
    }
    let queued = app.requested.iter().filter(|(_, preview)| !preview).count();
    const { assert!(THUMBNAILS_IN_FLIGHT < 12) };
    assert_eq!(queued, THUMBNAILS_IN_FLIGHT);
    // Previews for the photo being viewed are never held back.
    let _ = app.texture(MediaId(0), true);
    assert!(app.requested.contains(&(MediaId(0), true)));
}

#[test]
fn guided_comparisons_never_show_the_previous_photo_under_the_new_ones_pill() {
    for step in [Step::EasyDelete, Step::PickBest, Step::Rotation] {
        let (_temp, mut app) = comparison_fixture(step);
        app.comparison.clear();
        app.selection.clear();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let first = ctx.load_texture(
            "first",
            egui::ColorImage::filled([40, 30], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        let first_id = first.id();
        for preview in [true, false] {
            app.textures.insert(
                (MediaId(0), preview),
                Texture {
                    handle: first.clone(),
                    bytes: 4800,
                    used: 0,
                },
            );
        }
        let draw = |app: &mut PhotoSortApp| {
            ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let id = app.active.unwrap();
                        app.photo_canvas(ui, id, 0, egui::vec2(700.0, 500.0), true);
                    });
                },
            )
        };
        let shows_pill = |output: &egui::FullOutput| {
            output.shapes.iter().any(|clipped| match &clipped.shape {
                egui::Shape::Text(text) => {
                    let text = text.galley.text();
                    text.contains("Keep") || text.contains("Trash") || text.contains("Suggested")
                }
                _ => false,
            })
        };
        let loaded = draw(&mut app);
        // With its pixels, a guided candidate carries its Keep/Trash pill.
        assert_eq!(shows_pill(&loaded), step != Step::Rotation, "{step:?}");
        app.set_active(MediaId(1));
        let output = draw(&mut app);
        let old_photo = output.shapes.iter().any(|clipped| {
            matches!(&clipped.shape, egui::Shape::Mesh(mesh) if mesh.texture_id == first_id)
        });
        assert!(!old_photo, "{step:?} kept the previous photo on screen");
        assert!(
            !shows_pill(&output),
            "{step:?} showed a decision before the photo loaded"
        );
    }
}

#[test]
fn compared_photos_marked_for_trash_stay_bright() {
    for step in [Step::Cull, Step::EasyDelete, Step::PickBest] {
        let (_temp, mut app) = comparison_fixture(step);
        let id = MediaId(1);
        app.decisions.entry(id).or_default().rejected = true;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.global_style_mut(|style| style.animation_time = 0.0);
        let texture = ctx.load_texture(
            "photo",
            egui::ColorImage::filled([40, 30], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        for preview in [true, false] {
            app.textures.insert(
                (id, preview),
                Texture {
                    handle: texture.clone(),
                    bytes: 4800,
                    used: 0,
                },
            );
        }
        let output = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800.0, 600.0),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    app.photo_canvas(ui, id, 1, egui::vec2(700.0, 500.0), true);
                });
            },
        );
        super::review_card::assert_trash_drawn_bright(&output, texture.id());
        app.runtime.shutdown();
    }
}
