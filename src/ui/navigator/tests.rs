use super::*;
use crate::core::browsing::{Query, Seed, tree};
use crate::test_support::worker_patience;
fn settle(app: &mut PhotoSortApp) {
    let deadline = Instant::now() + worker_patience();
    loop {
        let expired = Instant::now() >= deadline;
        app.refilter_browse();
        if app.browse_run.is_none() {
            break;
        }
        assert!(!expired);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn folder_expansion_navigator_context_menu_survives_compact_popup() {
    for width in [640.0, 1200.0] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("A/B/C")).unwrap();
        std::fs::create_dir(root.path().join("A-1")).unwrap();
        std::fs::write(root.path().join("photo.jpg"), b"not decoded").unwrap();
        let catalog = Arc::new(Catalog::scan(root.path(), || false, |_| {}).unwrap());
        let library = catalog.root.clone();
        let key = Key::Folder(library.join("A"));
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.root = Some(library);
        app.step = Step::Cull;
        app.media = catalog.media.clone();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.active = Some(app.media[0].id);
        app.catalog = Some(catalog);
        app.set_navigator(Mode::Folders, true);
        settle(&mut app);
        let tree = app.browse_tree.as_ref().unwrap().tree.clone();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let frame = |app: &mut PhotoSortApp, events| {
            ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = &ui.ctx().clone();
                    {
                        app.keyboard(ctx);
                        egui::CentralPanel::default().show(ui, |ui| app.navigator_panel(ui));
                    }
                },
            )
        };
        let click = |pos, button| {
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ]
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        if width < 780.0 {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("test_navigator_toggle")))
                .unwrap();
            frame(&mut app, click(rect.center(), egui::PointerButton::Primary));
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            assert!(app.navigator_popup_open(&ctx));
            frame(
                &mut app,
                vec![egui::Event::Key {
                    key: egui::Key::D,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                }],
            );
            assert!(
                app.decisions.is_empty(),
                "Photo shortcuts must not run under the navigator"
            );
        }
        let rect = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_navigator_node", &key))))
            .unwrap();
        frame(
            &mut app,
            click(rect.center(), egui::PointerButton::Secondary),
        );
        // A context menu must not evict its containing tree popup from egui memory.
        let output = frame(&mut app, vec![]);
        let update = output.platform_output.accesskit_update.unwrap();
        let node = update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Collapse subtree"))
            .unwrap_or_else(|| panic!("Context menu disappeared at width {width}"));
        let bounds = node.1.bounds().unwrap();
        frame(
            &mut app,
            click(
                egui::pos2(
                    ((bounds.x0 + bounds.x1) / 2.0) as f32,
                    ((bounds.y0 + bounds.y1) / 2.0) as f32,
                ),
                egui::PointerButton::Primary,
            ),
        );
        settle(&mut app);
        assert!(app.navigator.collapsed.contains(&key));
        assert_eq!(app.filtered, vec![0]);
        assert!(Arc::ptr_eq(&tree, &app.browse_tree.as_ref().unwrap().tree));
        assert!(app.decisions.is_empty() && app.plan.is_none() && app.requested.is_empty());
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        if width < 780.0 {
            app.review = true;
            frame(&mut app, vec![]);
            assert!(
                !app.navigator_popup_open(&ctx),
                "Leaving the navigator must dismiss its popup"
            );
        }
    }
}
#[test]
fn tree_keyboard_navigator_reuses_worker_tree_and_accepts_input_during_refilter() {
    for width in [640., 1200.] {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(root.path().join("A/B/C")).unwrap();
        std::fs::create_dir(root.path().join("A-1")).unwrap();
        std::fs::write(root.path().join("photo.jpg"), b"not decoded").unwrap();
        let catalog = Arc::new(Catalog::scan(root.path(), || false, |_| {}).unwrap());
        let library = catalog.root.clone();
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.root = Some(library.clone());
        app.step = Step::Cull;
        app.media = catalog.media.clone();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.active = Some(app.media[0].id);
        app.catalog = Some(catalog);
        app.set_navigator(Mode::Folders, true);
        settle(&mut app);
        let tree = app.browse_tree.as_ref().unwrap().tree.clone();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let frame = |app: &mut PhotoSortApp, events| {
            let _ = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.),
                    )),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = &ui.ctx().clone();
                    {
                        app.keyboard(ctx);
                        egui::CentralPanel::default().show(ui, |ui| app.navigator_panel(ui));
                    }
                },
            );
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        if width < 780. {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("test_navigator_toggle")))
                .unwrap();
            frame(&mut app, tree_controls::tests::click(rect.center()));
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
        }
        let key = Key::Folder(library.join("A"));
        let rect = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_navigator_node", &key))))
            .unwrap();
        frame(&mut app, tree_controls::tests::click(rect.center()));
        // Do not settle between inputs: old projection remains on screen.
        for (key, expected) in [
            (egui::Key::ArrowLeft, "A"),
            (egui::Key::ArrowDown, "A-1"),
            (egui::Key::ArrowUp, "A"),
            (egui::Key::ArrowRight, "A"),
            (egui::Key::ArrowRight, "A/B"),
            (egui::Key::ArrowDown, "A/B/C"),
        ] {
            frame(&mut app, vec![tree_controls::tests::key(key)]);
            assert_eq!(
                app.navigator_key(),
                Some(Key::Folder(library.join(expected))),
                "width {width}, {key:?}"
            );
        }
        frame(&mut app, vec![tree_controls::tests::key(egui::Key::D)]);
        assert!(
            app.decisions.is_empty(),
            "focused tree must shield photo shortcuts"
        );
        settle(&mut app);
        assert!(Arc::ptr_eq(&tree, &app.browse_tree.as_ref().unwrap().tree));
        assert!(app.filtered.is_empty());
        assert!(app.decisions.is_empty() && app.plan.is_none() && app.requested.is_empty());
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
}
#[test]
fn navigator_selection_and_expansion_resume_without_creating_file_decisions() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("child")).unwrap();
    std::fs::write(root.path().join("child/a.jpg"), b"not decoded").unwrap();
    let catalog = Arc::new(Catalog::scan(root.path(), || false, |_| {}).unwrap());
    let library = catalog.root.clone();
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.root = Some(library.clone());
    app.step = Step::Cull;
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.catalog = Some(catalog);
    app.navigator.enabled = true;
    app.navigator.collapsed.insert(Key::Folder(library.clone()));
    app.select_navigation_node(Key::Folder(library.join("child")));
    settle(&mut app);
    assert_eq!(app.filtered, vec![0]);
    assert_eq!(app.browse_tree.as_ref().unwrap().visible, vec![0]);
    app.checkpoint_bookmarks();
    let saved = app.saved_views.bookmarks[&Step::Cull].clone();
    app.saved_views.bookmarks.insert(Step::Cull, saved.clone());
    app.navigator = Default::default();
    app.browse = Default::default();
    app.prepare_bookmark();
    assert_eq!(app.navigator, saved.navigator);
    assert_eq!(app.browse.folder, saved.browse.folder);
    assert!(app.decisions.is_empty() && app.plan.is_none());
    assert_eq!(app.stats.decodes, 0);
    assert!(app.requested.is_empty());
}
#[test]
fn responsive_navigator_keeps_scroll_across_panel_and_popup_without_image_work() {
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.root = Some("/absent/one".into());
    app.step = Step::Cull;
    app.navigator = tree::State {
        enabled: true,
        mode: Mode::Groups,
        ..Default::default()
    };
    let tree = Arc::new(
        tree::build(
            Mode::Groups,
            &[],
            (0..1000).map(|g| tree::Entry {
                path: std::path::Path::new("/absent/photo.jpg"),
                date: None,
                group: Some(g),
            }),
            || false,
        )
        .unwrap(),
    );
    app.browse_tree = Some(Arc::new(tree.project(&app.navigator, || false).unwrap()));
    app.filter_dirty = false;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let frame = |app: &mut PhotoSortApp, width, events| {
        let _ = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 900.0),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| app.navigator_panel(ui));
            },
        );
    };
    let scroll = || {
        ctx.data(|d| {
            d.get_temp::<(egui::Id, egui::Rect, f32, f32)>(egui::Id::new("test_navigator_scroll"))
        })
        .unwrap()
    };
    for _ in 0..3 {
        frame(&mut app, 1200.0, vec![]);
    }
    let id = scroll().0;
    let mut state = egui::scroll_area::State::load(&ctx, id).unwrap();
    state.offset.y = 4000.0;
    state.store(&ctx, id);
    for _ in 0..3 {
        frame(&mut app, 1200.0, vec![]);
    }
    let first = ctx
        .data(|d| d.get_temp::<usize>(egui::Id::new("test_navigator_first_row")))
        .unwrap();
    let row_height = scroll().3;
    for _ in 0..3 {
        frame(&mut app, 480.0, vec![]);
    }
    let toggle = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("test_navigator_toggle")))
        .unwrap()
        .center();
    frame(
        &mut app,
        480.0,
        vec![
            egui::Event::PointerMoved(toggle),
            egui::Event::PointerButton {
                pos: toggle,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: Default::default(),
            },
            egui::Event::PointerButton {
                pos: toggle,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: Default::default(),
            },
        ],
    );
    for _ in 0..3 {
        frame(&mut app, 480.0, vec![]);
    }
    assert_eq!(scroll().0, id);
    assert!((scroll().2 - 4000.0).abs() < 1.0);
    assert_eq!(
        scroll().3,
        row_height,
        "Responsive layout must keep the same row metrics"
    );
    assert_eq!(
        ctx.data(|d| d.get_temp::<usize>(egui::Id::new("test_navigator_first_row")))
            .unwrap(),
        first
    );
    let viewport = scroll().1;
    frame(
        &mut app,
        480.0,
        vec![
            egui::Event::PointerMoved(viewport.center()),
            egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0.0, -100.0),
                modifiers: Default::default(),
            },
        ],
    );
    for _ in 0..20 {
        frame(&mut app, 480.0, vec![]);
    }
    let moved = scroll().2;
    assert!(moved > 4020.0);
    for _ in 0..3 {
        frame(&mut app, 1200.0, vec![]);
    }
    assert_eq!(scroll().0, id);
    assert!((scroll().2 - moved).abs() < 1.0);
    app.root = Some("/absent/two".into());
    for _ in 0..3 {
        frame(&mut app, 1200.0, vec![]);
    }
    assert_ne!(scroll().0, id);
    assert_eq!(scroll().2, 0.0);
    assert!(app.requested.is_empty() && app.textures.is_empty() && app.browse_run.is_none());
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}

#[test]
fn large_navigator_renders_only_visible_rows_at_compact_and_wide_sizes() {
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.step = Step::Cull;
    app.navigator = tree::State {
        enabled: true,
        mode: Mode::Groups,
        ..Default::default()
    };
    let path = std::path::Path::new("/absent/photo.jpg");
    let tree = Arc::new(
        tree::build(
            Mode::Groups,
            &[],
            (0..100_000).map(|g| tree::Entry {
                path,
                date: None,
                group: Some(g),
            }),
            || false,
        )
        .unwrap(),
    );
    app.browse_tree = Some(Arc::new(tree.project(&app.navigator, || false).unwrap()));
    app.filter_dirty = false;
    for width in [480., 800., 1200.] {
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let output = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 500.),
                )),
                ..Default::default()
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    app.navigator_panel(ui);
                    assert!(ui.min_rect().right() <= width + 1.);
                });
            },
        );
        assert!(
            output.shapes.len() < 1000,
            "Navigator rendered offscreen nodes: {}",
            output.shapes.len()
        );
    }
    assert!(app.requested.is_empty() && app.textures.is_empty());
}
#[test]
fn empty_filtered_scope_keeps_navigation_available_and_other_workflows_stay_isolated() {
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.step = Step::Cull;
    let catalog = Arc::new(Catalog {
        root: "/absent".into(),
        directories: vec!["/absent/empty".into()],
        ..Default::default()
    });
    app.runtime.browsing.reset(Some(Seed {
        catalog,
        updates: Default::default(),
        decisions: Default::default(),
    }));
    app.navigator.enabled = true;
    let run = app.runtime.browsing.request(Query {
        navigator: app.navigator.clone(),
        ..Default::default()
    });
    let deadline = Instant::now() + worker_patience();
    loop {
        let expired = Instant::now() >= deadline;
        if let Some(result) = app.runtime.browsing.take_result() {
            assert_eq!(result.run, run);
            app.browse_tree = result.tree.map(Arc::new);
            break;
        }
        assert!(!expired);
        std::thread::sleep(Duration::from_millis(1));
    }
    app.filter_dirty = false;
    app.select_navigation_node(Key::Folder("/absent/empty".into()));
    assert!(app.filtered.is_empty());
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let mut shown = false;
    let _ = ctx.run_test(Default::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            let before = ui.cursor();
            app.navigator_panel(ui);
            shown = ui.cursor() != before;
        });
    });
    assert!(shown);
    app.step = Step::Organize;
    let _ = ctx.run_test(Default::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            let before = ui.cursor();
            app.navigator_panel(ui);
            assert_eq!(ui.cursor(), before);
        });
    });
}

#[test]
fn cull_navigator_shows_several_folders_chosen_with_ctrl_cmd_or_shift() {
    use super::super::compact_cull_tests::{fixture, frame, rendered};
    let (_temp, mut app) = fixture();
    let root = app.root.clone().unwrap();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    let click = |app: &mut PhotoSortApp, folder: &str, modifiers: egui::Modifiers| {
        rendered(app, &ctx, size);
        let key = Key::Folder(root.join(folder));
        let pos = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_navigator_node", &key))))
            .unwrap_or_else(|| panic!("{folder} is not in the navigator"))
            .center();
        for pressed in [true, false] {
            frame(
                app,
                &ctx,
                size,
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ],
            );
        }
        frame(
            app,
            &ctx,
            size,
            vec![egui::Event::ModifiersChanged(Default::default())],
        );
        settle(app);
    };
    let shown = |app: &PhotoSortApp| {
        let folders = app.browse.folder.folders();
        let mut names: Vec<String> = folders
            // Windows lists paths with backslashes.
            .map(|f| {
                f.strip_prefix(&root)
                    .unwrap()
                    .display()
                    .to_string()
                    .replace('\\', "/")
            })
            .collect();
        names.sort_unstable();
        names
    };
    let none = egui::Modifiers::default();
    click(&mut app, "B", none);
    assert_eq!(shown(&app), ["B"]);
    assert_eq!(app.filtered.len(), 5);
    // Cmd (macOS) or Ctrl adds a folder; the photos of both are shown.
    click(&mut app, "A/Day1", egui::Modifiers::COMMAND);
    assert_eq!(shown(&app), ["A/Day1", "B"]);
    assert_eq!(app.filtered.len(), 9);
    assert!(
        app.active_cull_filters().contains(&"2 folders".to_owned()),
        "{:?}",
        app.active_cull_filters()
    );
    // And takes one away again.
    click(&mut app, "B", egui::Modifiers::CTRL);
    assert_eq!(shown(&app), ["A/Day1"]);
    assert_eq!(app.filtered.len(), 4);
    // Shift shows the folders listed from the last one clicked through
    // this one; Cmd/Ctrl with Shift adds them.
    click(&mut app, "A", none);
    click(&mut app, "B", egui::Modifiers::SHIFT);
    assert_eq!(shown(&app), ["A", "A/Day1", "B"]);
    click(&mut app, "Empty", none);
    click(&mut app, "B", egui::Modifiers::SHIFT);
    assert_eq!(shown(&app), ["B", "Empty"]);
    click(&mut app, "A/Day1", egui::Modifiers::COMMAND);
    click(
        &mut app,
        "Empty",
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
    );
    assert_eq!(shown(&app), ["A/Day1", "B", "Empty"]);
    // A plain click shows one folder again.
    click(&mut app, "A/Day1", none);
    assert_eq!(shown(&app), ["A/Day1"]);
    assert!(app.browse.folder.also.is_empty());
    app.runtime.shutdown();
}
