use super::*;
#[test]
fn organize_trash_menus_share_state_for_mixed_files_and_folder_descendants() {
    for width in [640.0, 1440.0] {
        for folder in [false, true] {
            let mut app = app();
            app.index = app
                .media
                .iter()
                .enumerate()
                .map(|(i, m)| (m.id, i))
                .collect();
            let document = PathBuf::from("/missing/trip/child/notes.txt");
            let doc = MediaId::for_path(&document);
            let catalog = Arc::get_mut(app.catalog.as_mut().unwrap()).unwrap();
            catalog.inventory.push(document);
            catalog
                .supporting_fingerprints
                .insert(doc, Fingerprint::default());
            catalog.directories = vec!["/missing/trip".into(), "/missing/trip/child".into()];
            app.stage_manual([MediaId(0)], crate::core::decisions::Change::Trash(true));
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.selected = HashSet::from([MediaId(0), doc]);
            editor.compact_folders = folder;
            editor.draft.create_folder("", "trip-1").unwrap();
            editor.draft.move_photos(&[MediaId(2)], "trip-1").unwrap();
            editor.refresh();
            let labels = editor.draft.labels().clone();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.enable_accesskit();
            let frame = |app: &mut PhotoSortApp, events| {
                ctx.run_test(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 850.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let ctx = &ui.ctx().clone();
                        app.organize_dialog(ctx)
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
            for desired in [true, false] {
                for _ in 0..3 {
                    frame(&mut app, vec![]);
                }
                let key = if folder {
                    egui::Id::new(("test_folder_rect", "trip"))
                } else {
                    egui::Id::new(("test_file_rect", 0_u64))
                };
                let pos = ctx
                    .data(|d| d.get_temp::<egui::Rect>(key))
                    .unwrap()
                    .center();
                frame(&mut app, click(pos, egui::PointerButton::Secondary));
                let output = frame(&mut app, vec![]);
                let update = output.platform_output.accesskit_update.unwrap();
                let label = if desired {
                    "Mark for Trash"
                } else {
                    "Unmark from Trash"
                };
                let node = update
                    .nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label))
                    .expect(label);
                let bounds = node.1.bounds().unwrap();
                assert!(bounds.x0 >= 0.0 && bounds.x1 <= f64::from(width) && bounds.y1 <= 850.0);
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
                assert_eq!(app.decisions[&MediaId(0)].rejected, desired);
                assert_eq!(app.decisions[&doc].rejected, desired);
                assert!(
                    !app.decision(MediaId(2)).rejected,
                    "prefix-like sibling stays untouched"
                );
                assert_eq!(app.decision(MediaId(1)).rejected, folder && desired);
                assert_eq!(
                    app.organize_editor.as_ref().unwrap().draft.labels(),
                    &labels
                );
            }
            app.stage_manual([doc], crate::core::decisions::Change::Trash(true));
            let editor = app.organize_editor.take().unwrap();
            app.finish_organize_edit(editor, 1);
            assert!(
                app.decisions[&doc].rejected,
                "layout cancel preserves shared marks"
            );
            assert!(app.organize_drafts.is_empty());
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
            assert!(
                app.requested.is_empty()
                    && app.analysis_run.is_none()
                    && app.metadata_run.is_none()
            );
        }
    }
}
#[test]
fn folder_expansion_editor_controls_persist_without_repeating_image_work() {
    for width in [640.0, 1440.0] {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.draft.create_folder("", "A").unwrap();
        editor.draft.create_folder("A", "B").unwrap();
        editor.draft.create_folder("A/B", "C").unwrap();
        editor.draft.create_folder("", "A-1").unwrap();
        editor.refresh();
        editor.compact_folders = true;
        editor.folder = "A/B/C".into();
        editor.selected_folders.insert("A/B/C".into());
        editor.selected.insert(MediaId(1));
        let labels = editor.draft.labels().clone();
        let tree_address = editor.tree.nodes.as_ptr();
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
                    app.organize_dialog(ctx)
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
        let rect = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", "A"))))
            .unwrap();
        frame(
            &mut app,
            click(rect.center(), egui::PointerButton::Secondary),
        );
        let output = frame(&mut app, vec![]);
        let update = output.platform_output.accesskit_update.unwrap();
        let bounds = update
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Collapse subtree"))
            .unwrap()
            .1
            .bounds()
            .unwrap();
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
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!(
            editor.collapsed,
            [Key::Folder("A".into()), Key::Folder("A/B".into())].into()
        );
        assert!(
            editor
                .visible_folders
                .iter()
                .any(|&i| editor.tree.nodes[i].key == Key::Folder("A-1".into()))
        );
        assert!(
            !editor
                .visible_folders
                .iter()
                .any(|&i| editor.tree.nodes[i].key == Key::Folder("A/B".into()))
        );
        assert_eq!(editor.folder, "A/B/C");
        assert!(editor.selected_folders.contains("A/B/C") && editor.selected.contains(&MediaId(1)));
        assert_eq!(editor.draft.labels(), &labels);
        assert_eq!(editor.tree.nodes.as_ptr(), tree_address);
        for _ in 0..2 {
            frame(&mut app, vec![]);
        }
        let rect = ctx
            .data(|d| {
                d.get_temp::<egui::Rect>(egui::Id::new((
                    "test_tree_expander",
                    Key::Folder("A".into()),
                )))
            })
            .unwrap();
        frame(&mut app, click(rect.center(), egui::PointerButton::Primary));
        let mut editor = app.organize_editor.take().unwrap();
        assert_eq!(editor.collapsed, [Key::Folder("A/B".into())].into());
        let bookmark = editor.bookmark();
        let encoded = serde_json::to_string(&bookmark).unwrap();
        assert_eq!(
            serde_json::from_str::<crate::core::views::EditorBookmark>(&encoded).unwrap(),
            bookmark
        );
        // Rename is structural: reveal the destination, drop obsolete keys.
        editor.folder = "A".into();
        editor.name = "Renamed".into();
        editor.apply(Command::Rename, &app.media, false);
        assert!(editor.collapsed.is_empty());
        assert!(editor.draft.folders().contains("Renamed/B/C"));
        editor.collapsed.insert(Key::Folder("Renamed".into()));
        editor.project_folders();
        app.finish_organize_edit(editor, 2);
        app.open_organize_editor();
        let editor = app.organize_editor.take().unwrap();
        assert_eq!(editor.collapsed, [Key::Folder("Renamed".into())].into());
        assert!(editor.selected.contains(&MediaId(1)));
        app.finish_organize_edit(editor, 1);
        assert!(
            app.requested.is_empty()
                && app.metadata_run.is_none()
                && app.analysis_run.is_none()
                && !app.scanning
                && !app.busy
        );
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(app.stats.metadata_reads, 0);
    }
}
#[test]
fn folder_ranges_pointer_keyboard_and_resume_use_visible_order_without_image_work() {
    for width in [640., 1200.] {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        for path in ["A", "A/B", "A/B/C", "A-1", "B", "C"] {
            let (parent, leaf) = path.rsplit_once('/').unwrap_or(("", path));
            editor.draft.create_folder(parent, leaf).unwrap();
        }
        editor.refresh();
        editor.collapsed.insert(Key::Folder("A".into()));
        editor.project_folders();
        editor.compact_folders = true;
        editor.selected.insert(MediaId(1));
        let labels = editor.draft.labels().clone();
        let tree = editor.tree.nodes.as_ptr();
        let projection = editor.visible_folders.as_ptr();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let frame = |app: &mut PhotoSortApp, events: Vec<egui::Event>, modifiers| {
            // egui 0.36 carries modifier state as an event, not a RawInput field.
            let events = std::iter::once(egui::Event::ModifiersChanged(modifiers))
                .chain(events)
                .collect();
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
                    app.organize_dialog(ctx)
                },
            );
        };
        let click = |app: &mut PhotoSortApp, path: &str, modifiers: egui::Modifiers| {
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", path))))
                .unwrap();
            let mut events = tree_controls::tests::click(rect.center());
            for event in &mut events {
                if let egui::Event::PointerButton { modifiers: m, .. } = event {
                    *m = modifiers;
                }
            }
            frame(app, events, modifiers);
        };
        let key = |key, modifiers| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
        let shift = egui::Modifiers::SHIFT;
        let command = egui::Modifiers::COMMAND;
        let additive = egui::Modifiers {
            shift: true,
            command: true,
            ..Default::default()
        };
        for _ in 0..3 {
            frame(&mut app, vec![], egui::Modifiers::NONE);
        }
        click(&mut app, "A", egui::Modifiers::NONE);
        click(&mut app, "C", shift);
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected_folders,
            ["A", "A-1", "B", "C"].map(String::from).into()
        );
        assert_eq!(
            app.organize_editor
                .as_ref()
                .unwrap()
                .folder_anchor
                .as_deref(),
            Some("A")
        );
        click(&mut app, "A", egui::Modifiers::NONE);
        frame(
            &mut app,
            vec![
                key(egui::Key::ArrowDown, shift),
                key(egui::Key::ArrowDown, shift),
            ],
            shift,
        );
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected_folders,
            ["A", "A-1", "B"].map(String::from).into()
        );
        frame(&mut app, vec![key(egui::Key::ArrowUp, shift)], shift);
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected_folders,
            ["A", "A-1"].map(String::from).into()
        );
        click(&mut app, "C", command);
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected_folders,
            ["A", "A-1", "C"].map(String::from).into()
        );
        frame(&mut app, vec![key(egui::Key::ArrowUp, additive)], additive);
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected_folders,
            ["A", "A-1", "B", "C"].map(String::from).into()
        );
        frame(&mut app, vec![key(egui::Key::ArrowUp, shift)], shift);
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!(
            editor.selected_folders,
            ["A-1", "B", "C"].map(String::from).into()
        );
        assert_eq!(editor.folder_anchor.as_deref(), Some("C"));
        assert_eq!(editor.tree.nodes.as_ptr(), tree);
        assert_eq!(editor.visible_folders.as_ptr(), projection);
        assert_eq!(editor.draft.labels(), &labels);
        assert_eq!(editor.selected, [MediaId(1)].into());
        let bookmark = editor.bookmark();
        assert_eq!(
            serde_json::from_str::<crate::core::views::EditorBookmark>(
                &serde_json::to_string(&bookmark).unwrap()
            )
            .unwrap(),
            bookmark
        );
        let editor = app.organize_editor.take().unwrap();
        app.finish_organize_edit(editor, 2);
        app.open_organize_editor();
        assert_eq!(
            app.organize_editor
                .as_ref()
                .unwrap()
                .folder_anchor
                .as_deref(),
            Some("C")
        );
        for _ in 0..3 {
            frame(&mut app, vec![], egui::Modifiers::NONE);
        }
        frame(&mut app, vec![key(egui::Key::Home, shift)], shift);
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!(
            editor.selected_folders,
            ["A", "A-1", "B", "C"].map(String::from).into()
        );
        assert!(!editor.selected_folders.contains(""));
        assert!(!editor.selected_folders.contains("A/B"));
        assert!(
            app.requested.is_empty()
                && app.metadata_run.is_none()
                && app.analysis_run.is_none()
                && !app.scanning
        );
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        let editor = app.organize_editor.take().unwrap();
        app.finish_organize_edit(editor, 4);
        assert!(
            app.organize_editor
                .as_ref()
                .unwrap()
                .folder_anchor
                .is_none()
        );
    }
}

#[test]
fn folder_ranges_reconcile_hidden_and_renamed_anchors_and_reuse_large_tree() {
    let mut app = app();
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.draft = Draft::new(
        (0..100_000)
            .map(|i| (MediaId(i), format!("folder-{i:06}")))
            .collect(),
    );
    editor.refresh();
    let tree = editor.tree.nodes.as_ptr();
    let projection = editor.visible_folders.as_ptr();
    editor.select_folder("folder-099998", false, false);
    editor.select_folder("folder-099999", false, true);
    assert_eq!(
        editor.selected_folders,
        ["folder-099998", "folder-099999"].map(String::from).into()
    );
    assert_eq!(editor.tree.nodes.as_ptr(), tree);
    assert_eq!(editor.visible_folders.as_ptr(), projection);
    // Restore drops an obsolete path; the next range starts from current focus.
    let mut bookmark = editor.bookmark();
    bookmark.folder_anchor = Some("missing".into());
    editor.restore_bookmark(&bookmark);
    assert!(editor.folder_anchor.is_none());
    editor.select_folder("folder-099998", false, true);
    assert_eq!(editor.selected_folders.len(), 2);
    // A hidden anchor is not a range through collapsed descendants.
    editor.collapsed.insert(Key::Folder("".into()));
    editor.project_folders();
    editor.select_folder("", false, true);
    assert!(editor.selected_folders.is_empty());
    assert_eq!(editor.folder_anchor.as_deref(), Some(""));
    editor.draft = Draft::new([(MediaId(1), "A/B".into())].into());
    editor.refresh();
    editor.collapsed.clear();
    editor.project_folders();
    editor.select_folder("A", false, false);
    editor.name = "Renamed".into();
    editor.apply(Command::Rename, &app.media, false);
    assert!(editor.folder_anchor.is_none());
    assert_eq!(editor.folder, "Renamed");
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}

#[test]
fn tree_keyboard_organize_keeps_drafts_and_file_selection_and_reveals_virtual_rows() {
    for width in [640.0, 1200.0] {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        for path in ["A", "A/B", "A/B/C", "A-1"] {
            let (parent, leaf) = path.rsplit_once('/').unwrap_or(("", path));
            editor.draft.create_folder(parent, leaf).unwrap();
        }
        for i in 0..2000 {
            editor.draft.create_folder("", &format!("z{i:04}")).unwrap();
        }
        editor.refresh();
        editor.compact_folders = true;
        editor.selected.insert(MediaId(1));
        let labels = editor.draft.labels().clone();
        let address = editor.tree.nodes.as_ptr();
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
                    app.organize_dialog(ctx)
                },
            );
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let rect = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", "A"))))
            .unwrap();
        frame(&mut app, tree_controls::tests::click(rect.center()));
        for (key, expected) in [
            (egui::Key::ArrowLeft, "A"),
            (egui::Key::ArrowDown, "A-1"),
            (egui::Key::ArrowUp, "A"),
            (egui::Key::ArrowRight, "A"),
            (egui::Key::ArrowRight, "A/B"),
            (egui::Key::End, "z1999"),
        ] {
            frame(&mut app, vec![tree_controls::tests::key(key)]);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().folder,
                expected,
                "width {width}, {key:?}"
            );
        }
        let editor = app.organize_editor.as_ref().unwrap();
        assert!(editor.folder_scroll > 10_000.);
        assert_eq!(editor.name, "z1999");
        assert_eq!(editor.selected_folders, ["z1999".into()].into());
        assert!(editor.selected.contains(&MediaId(1)));
        assert_eq!(editor.tree.nodes.as_ptr(), address);
        assert_eq!(editor.draft.labels(), &labels);
        frame(&mut app, vec![tree_controls::tests::key(egui::Key::Home)]);
        assert_eq!(app.organize_editor.as_ref().unwrap().folder, "");
        assert_eq!(app.organize_editor.as_ref().unwrap().folder_scroll, 0.);
        assert!(
            app.requested.is_empty()
                && app.metadata_run.is_none()
                && app.analysis_run.is_none()
                && !app.scanning
        );
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
}
#[test]
fn location_layout_reuses_metadata_for_editor_and_inspector_without_image_work() {
    use crate::core::geography::{Layout, Location};
    let mut app = app();
    app.group_mode = GroupMode::Location;
    for media in &app.media {
        app.metadata.insert(
            media.id,
            Metadata {
                location: Some(Location {
                    country: "Japan".into(),
                    region: "Shizuoka".into(),
                    city: "Fuji".into(),
                }),
                ..Default::default()
            },
        );
    }
    for (layout, expected) in [
        (Layout::Country, "Japan"),
        (Layout::CountryCity, "Japan/Fuji"),
        (Layout::CountryRegionCity, "Japan/Shizuoka/Fuji"),
    ] {
        app.change_location_layout(layout);
        assert_eq!(app.label_for(&app.media[0]), expected);
        assert!(
            app.organize_labels()
                .values()
                .all(|label| label == expected)
        );
        app.open_organize_editor();
        let editor = app.organize_editor.take().unwrap();
        assert!(editor.rows.iter().all(|row| row.after == expected));
        assert!(editor.draft.folders().contains(expected));
        assert!(
            app.requested.is_empty()
                && app.metadata_run.is_none()
                && app.analysis_run.is_none()
                && !app.scanning
                && !app.busy
        );
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(app.metadata.len(), 3);
    }
}
#[test]
fn location_layout_requires_explicit_replacement_and_preserves_other_intent() {
    use crate::core::geography::Layout;
    let mut app = app();
    app.group_mode = GroupMode::Location;
    let draft = Draft::new([(MediaId(0), "Custom location".into())].into());
    app.organize_drafts
        .insert(GroupMode::Location, draft.clone());
    app.organize_drafts.insert(GroupMode::Camera, draft.clone());
    app.file_names.insert(MediaId(0), "Keeper".into());
    app.decisions.insert(
        MediaId(0),
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    app.change_location_layout(Layout::Country);
    assert_eq!(app.preferences.location_layout, Layout::CountryRegionCity);
    assert!(app.modal_active());
    app.resolve_location_layout(false);
    assert_eq!(app.organize_drafts[&GroupMode::Location], draft);
    assert_eq!(app.preferences.location_layout, Layout::CountryRegionCity);
    app.change_location_layout(Layout::CountryCity);
    app.resolve_location_layout(true);
    assert_eq!(app.preferences.location_layout, Layout::CountryCity);
    assert!(!app.organize_drafts.contains_key(&GroupMode::Location));
    assert_eq!(app.organize_drafts[&GroupMode::Camera], draft);
    assert_eq!(app.file_names[&MediaId(0)], "Keeper");
    assert_eq!(app.decisions[&MediaId(0)].rating, Some(4));
    assert!(app.requested.is_empty() && app.analysis_run.is_none() && app.plan.is_none());
    for state in 0..5 {
        app.step = if state == 0 {
            Step::Cull
        } else {
            Step::Organize
        };
        app.busy = state == 1;
        app.scanning = state == 2;
        app.analysis_run = (state == 3).then_some(1);
        app.plan_open = state == 4;
        app.change_location_layout(Layout::Country);
        assert_eq!(app.preferences.location_layout, Layout::CountryCity);
    }
}
fn metadata_fixture(count: usize) -> (PhotoSortApp, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("photos");
    std::fs::create_dir(&root).unwrap();
    for i in 0..count {
        image::RgbImage::new(4, 4)
            .save(root.join(format!("photo-{i:04}.png")))
            .unwrap();
    }
    let catalog = Arc::new(Catalog::scan(&root, || false, |_| {}).unwrap());
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some(catalog.root.clone());
    app.output = catalog.root.display().to_string();
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.catalog = Some(catalog);
    (app, temp)
}
fn wait_metadata(app: &mut PhotoSortApp) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.metadata_run.is_some() || app.busy {
        app.poll(&egui::Context::default());
        assert!(
            Instant::now() < deadline,
            "metadata or planning did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn metadata_organize_modes_coalesce_one_read_and_wait_for_offscreen_photos() {
    let (mut app, _temp) = metadata_fixture(129);
    let catalog = app.catalog.clone().unwrap();
    app.set_group_mode(GroupMode::Date);
    let run = app.metadata_run.unwrap();
    assert!(!app.organize_grouping_ready() && !app.can_edit_folders());
    app.open_organize_editor();
    app.organize();
    assert!(app.organize_editor.is_none() && app.plan.is_none() && !app.busy);
    for mode in [GroupMode::Location, GroupMode::Camera] {
        app.set_group_mode(mode);
        assert_eq!(app.metadata_run, Some(run));
    }
    wait_metadata(&mut app);
    assert!(app.organize_grouping_ready() && app.can_edit_folders());
    assert_eq!(app.metadata.len(), 129);
    assert_eq!(app.stats.metadata_reads, 129);
    assert_eq!(app.stats.decodes, 0);
    assert!(app.analysis_run.is_none() && app.analysis.is_empty() && app.requested.is_empty());
    assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
    for mode in [GroupMode::Date, GroupMode::Location, GroupMode::Camera] {
        app.set_group_mode(mode);
        assert!(app.metadata_run.is_none());
    }
    app.open_organize_editor();
    let editor = app.organize_editor.take().unwrap();
    assert_eq!(editor.rows.len(), 129);
    assert!(editor.rows.iter().all(|row| row.after == "Unknown camera"));
    app.organize();
    wait_metadata(&mut app);
    assert_eq!(app.plan.as_ref().unwrap().actions.len(), 129);
    assert_eq!(app.stats.metadata_reads, 129);
    assert!(app.media.iter().all(|m| m.path.is_file()));
}
#[test]
fn metadata_organize_cancel_rejects_late_batches_and_resume_reuses_completed_data() {
    let (mut app, _temp) = metadata_fixture(129);
    app.set_group_mode(GroupMode::Date);
    let old = app.metadata_run.unwrap();
    app.set_group_mode(GroupMode::Current);
    assert!(app.metadata_run.is_none());
    // Drain any already-published old batch. It must not revive the cancelled run.
    for _ in 0..10 {
        app.poll(&egui::Context::default());
    }
    assert!(app.metadata.is_empty());
    app.set_group_mode(GroupMode::Camera);
    assert_ne!(app.metadata_run, Some(old));
    wait_metadata(&mut app);
    let reads = app.stats.metadata_reads;
    app.cancel_metadata_index();
    app.start_metadata_index();
    assert!(app.metadata_run.is_none());
    assert_eq!(app.metadata.len(), 129);
    assert_eq!(app.stats.metadata_reads, reads);
    assert_eq!(app.stats.decodes, 0);
    assert!(app.plan.is_none() && app.analysis.is_empty());
}
#[test]
fn metadata_organize_missing_file_blocks_proposals_until_shared_retry_succeeds() {
    let (mut app, temp) = metadata_fixture(2);
    let path = app.media[0].path.clone();
    let backup = temp.path().join("temporarily-unavailable.png");
    std::fs::rename(&path, &backup).unwrap();
    app.set_group_mode(GroupMode::Location);
    wait_metadata(&mut app);
    assert_eq!(app.metadata.len(), 1);
    assert_eq!(app.metadata_failures.len(), 1);
    assert!(!app.organize_grouping_ready() && !app.can_edit_folders());
    app.open_organize_editor();
    assert!(app.organize_editor.is_none() && app.plan.is_none());
    std::fs::rename(backup, path).unwrap();
    app.start_metadata_index();
    wait_metadata(&mut app);
    assert_eq!(app.metadata.len(), 2);
    assert_eq!(app.stats.metadata_reads, 2);
    assert!(app.metadata_failures.is_empty() && app.can_edit_folders());
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn metadata_organize_departure_waits_for_complete_metadata_before_reviewing_names() {
    let (mut app, _temp) = metadata_fixture(2);
    app.group_mode = GroupMode::Camera;
    app.file_names.insert(app.media[0].id, "Keeper".into());
    app.request_departure(Destination::Step(Step::Cull));
    assert!(app.organize_departure.is_some() && app.metadata_run.is_some());
    assert!(app.plan.is_none() && !app.busy && app.step == Step::Organize);
    wait_metadata(&mut app);
    assert!(app.plan.is_some() && app.organize_departure.is_some());
    assert_eq!(app.stats.metadata_reads, 2);
    assert_eq!(app.stats.decodes, 0);
    assert!(app.media.iter().all(|m| m.path.is_file()));
    app.resolve_organize_departure(super::super::transitions::OrganizeResolution::Cancel);
    assert!(app.plan.is_none() && app.file_names.len() == 1);
}
#[test]
fn selective_reset_menu_preserves_other_edits_names_and_selection_at_all_sizes() {
    for size in [egui::vec2(600.0, 650.0), egui::vec2(1200.0, 850.0)] {
        for (selected_click, companions) in [(false, false), (true, false), (false, true)] {
            let mut app = app();
            app.companions = companions;
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.draft.create_folder("", "Custom").unwrap();
            editor
                .draft
                .move_photos(&[MediaId(0), MediaId(1), MediaId(2)], "Custom")
                .unwrap();
            editor.names.insert(MediaId(1), "Keeper".into());
            editor.selected = if selected_click {
                HashSet::from([MediaId(1), MediaId(2)])
            } else {
                HashSet::from([MediaId(2)])
            };
            let selected = editor.selected.clone();
            editor.refresh();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.enable_accesskit();
            let frame = |app: &mut PhotoSortApp, events| {
                ctx.run_test(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ui| {
                        let ctx = &ui.ctx().clone();
                        app.organize_dialog(ctx)
                    },
                )
            };
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            let pos = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 1_u64))))
                .unwrap()
                .center();
            let button = |pos, button, pressed| egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Default::default(),
            };
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    button(pos, egui::PointerButton::Secondary, true),
                    button(pos, egui::PointerButton::Secondary, false),
                ],
            );
            let tree = frame(&mut app, vec![])
                .platform_output
                .accesskit_update
                .unwrap();
            let node = tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Reset file destinations"))
                .unwrap();
            let bounds = node.1.bounds().unwrap();
            assert!(
                bounds.x0 >= 0.0
                    && bounds.y0 >= 0.0
                    && bounds.x1 <= f64::from(size.x)
                    && bounds.y1 <= f64::from(size.y)
            );
            let pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    button(pos, egui::PointerButton::Primary, true),
                    button(pos, egui::PointerButton::Primary, false),
                ],
            );
            let editor = app.organize_editor.take().unwrap();
            assert_eq!(editor.draft.labels()[&MediaId(1)], "trip");
            assert_eq!(
                editor.draft.labels()[&MediaId(0)],
                if companions { "trip" } else { "Custom" }
            );
            assert_eq!(
                editor.draft.labels()[&MediaId(2)],
                if selected_click { "trip" } else { "Custom" }
            );
            assert_eq!(editor.names[&MediaId(1)], "Keeper");
            assert_eq!(editor.selected, selected);
            assert!(editor.draft.folders().contains("Custom"));
            assert!(
                app.requested.is_empty()
                    && app.analysis_run.is_none()
                    && app.metadata_run.is_none()
            );
            assert_eq!(app.stats.decodes, 0);
            assert_eq!(app.stats.metadata_reads, 0);
            app.finish_organize_edit(editor, 1);
            assert!(app.organize_drafts.is_empty() && app.file_names.is_empty());
        }
    }
}
#[test]
fn selective_reset_waits_for_generated_metadata_and_survives_keep_reopen() {
    let mut app = app();
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.draft.create_folder("", "Custom").unwrap();
    editor.draft.move_photos(&[MediaId(0)], "Custom").unwrap();
    app.finish_organize_edit(editor, 2);
    app.open_organize_editor();
    app.organize_editor.as_mut().unwrap().reset_request = Some(Selection {
        files: HashSet::from([MediaId(0)]),
        ..Default::default()
    });
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let _ = ctx.run_test(Default::default(), |ui| {
        let ctx = &ui.ctx().clone();
        app.organize_dialog(ctx)
    });
    let editor = app.organize_editor.take().unwrap();
    assert_eq!(editor.draft.labels()[&MediaId(0)], "trip");
    app.finish_organize_edit(editor, 2);
    app.open_organize_editor();
    assert_eq!(
        app.organize_editor.as_ref().unwrap().draft.labels()[&MediaId(0)],
        "trip"
    );
    app.group_mode = GroupMode::Location;
    app.metadata.clear();
    let editor = app.organize_editor.as_mut().unwrap();
    editor.draft.move_photos(&[MediaId(0)], "Custom").unwrap();
    editor.reset_request = Some(Selection {
        files: HashSet::from([MediaId(0)]),
        ..Default::default()
    });
    let _ = ctx.run_test(Default::default(), |ui| {
        let ctx = &ui.ctx().clone();
        app.organize_dialog(ctx)
    });
    assert_eq!(
        app.organize_editor.as_ref().unwrap().draft.labels()[&MediaId(0)],
        "Custom"
    );
    assert!(!app.organize_editor.as_ref().unwrap().reset_enabled);
}
#[test]
fn responsive_editor_keeps_each_scroll_and_selection_without_repeating_work() {
    let mut app = app();
    let seed = |app: &mut PhotoSortApp| {
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.rows = (0..1000)
            .map(|i| PhotoRow {
                id: MediaId(i),
                source: format!("/missing/photo-{i:04}.jpg").into(),
                after: String::new(),
            })
            .collect();
        editor.draft = Draft::new(
            (0..1000)
                .map(|i| (MediaId(i), format!("folder-{i:04}")))
                .collect(),
        );
        editor.refresh();
    };
    seed(&mut app);
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
                let ctx = &ui.ctx().clone();
                app.organize_dialog(ctx)
            },
        );
    };
    let editor = app.organize_editor.as_mut().unwrap();
    editor.restore_scroll = [Some(6500.0), Some(9000.0)];
    editor.selected.insert(MediaId(200));
    editor.selected_folders.insert("folder-0400".into());
    editor.folder = "folder-0400".into();
    let mut expected = [6500.0, 9000.0];
    for (width, folders) in [
        (1440.0, false),
        (640.0, true),
        (640.0, false),
        (1440.0, false),
        (640.0, true),
    ] {
        app.organize_editor.as_mut().unwrap().compact_folders = folders;
        for _ in 0..3 {
            frame(&mut app, width, vec![]);
        }
        let editor = app.organize_editor.as_ref().unwrap();
        for (actual, wanted) in [editor.photo_scroll, editor.folder_scroll]
            .into_iter()
            .zip(expected)
        {
            assert!(
                (actual - wanted).abs() < 1.0,
                "width {width}, folders {folders}: {actual} != {wanted}"
            );
        }
        assert!(
            editor.selected.contains(&MediaId(200))
                && editor.selected_folders.contains("folder-0400")
        );
        assert_eq!(editor.folder, "folder-0400");
        let key = if folders {
            "test_organize_folder_scroll"
        } else {
            "test_organize_photo_scroll"
        };
        let viewport = ctx
            .data(|d| d.get_temp::<(egui::Rect, Vec2, f32, f32)>(egui::Id::new(key)))
            .unwrap()
            .0;
        frame(
            &mut app,
            width,
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
            frame(&mut app, width, vec![]);
        }
        let editor = app.organize_editor.as_ref().unwrap();
        let actual = if folders {
            editor.folder_scroll
        } else {
            editor.photo_scroll
        };
        assert!(
            actual > expected[usize::from(folders)] + 20.0,
            "Scrolling must still work after a layout change"
        );
        expected[usize::from(folders)] = actual;
    }
    // A new editor with no saved bookmark must not inherit the previous
    // editor's egui state, even when it opens the same library and layout.
    app.organize_editor = None;
    app.organize_view = None;
    seed(&mut app);
    for _ in 0..3 {
        frame(&mut app, 1440.0, vec![]);
    }
    let editor = app.organize_editor.as_ref().unwrap();
    assert_eq!([editor.photo_scroll, editor.folder_scroll], [0.0, 0.0]);
    assert!(app.requested.is_empty() && app.metadata_run.is_none() && app.analysis_run.is_none());
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}

#[test]
fn virtual_editor_rows_keep_exact_pitch_and_last_entries_visible_without_image_work() {
    for width in [360.0, 800.0] {
        let mut app = app();
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        let label = |i| format!("folder-{i:04}-{}", "long-name".repeat(25));
        editor.rows = (0..1000)
            .map(|i| PhotoRow {
                id: MediaId(i),
                source: format!("/missing/photo-{i:04}.jpg").into(),
                after: label(i),
            })
            .collect();
        editor.draft = Draft::new((0..1000).map(|i| (MediaId(i), label(i))).collect());
        editor.refresh();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        for folders in [false, true] {
            for end in [false, true] {
                editor.restore_scroll[usize::from(folders)] =
                    Some(if end { 1_000_000.0 } else { 0.0 });
                for _ in 0..3 {
                    let _ = ctx.run_test(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 650.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                if folders {
                                    editor.structure(ui, 380.0, &HashMap::new());
                                } else {
                                    editor.photos(ui, 380.0, &HashMap::new());
                                }
                            });
                        },
                    );
                }
                let key = if folders {
                    "test_organize_folder_scroll"
                } else {
                    "test_organize_photo_scroll"
                };
                let (viewport, content, offset, height) = ctx
                    .data(|d| d.get_temp::<(egui::Rect, Vec2, f32, f32)>(egui::Id::new(key)))
                    .unwrap();
                let count = if folders {
                    editor.tree.nodes.len()
                } else {
                    editor.rows.len()
                };
                let gap = ctx.global_style().spacing.item_spacing.y;
                assert!(
                    (content.y - (count as f32 * (height + gap) - gap)).abs() <= height + gap,
                    "{key}: content {content:?} height {height}"
                );
                assert!(
                    viewport.width() >= width - 30.0,
                    "Scroll target must cover the available width"
                );
                let indices = if end { [998, 999] } else { [0, 1] };
                let rects: Vec<_> = indices
                    .into_iter()
                    .map(|i| {
                        let key = if folders {
                            egui::Id::new(("test_folder_rect", label(i)))
                        } else {
                            egui::Id::new(("test_file_rect", i))
                        };
                        ctx.data(|d| d.get_temp::<egui::Rect>(key)).unwrap()
                    })
                    .collect();
                assert!(
                    (rects[1].top() - rects[0].top() - height - gap).abs() < 0.6,
                    "{key}: {rects:?}, height {height}, offset {offset}"
                );
                if end {
                    assert!(
                        rects[1].intersects(viewport),
                        "Last entry is reachable: {key}, rects {rects:?}, viewport {viewport:?}, offset {offset}"
                    );
                }
                assert!(
                    rects.iter().all(|r| r.right() <= viewport.right() + 1.0),
                    "Long labels must truncate"
                );
            }
        }
        assert!(
            app.requested.is_empty() && app.metadata_run.is_none() && app.analysis_run.is_none()
        );
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(app.stats.metadata_reads, 0);
    }
}
fn app() -> PhotoSortApp {
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    // Nonexistent paths prove folder editing needs no rescan or image load.
    let media: Vec<_> = [
        "/missing/trip/a.jpg",
        "/missing/trip/a.ARW",
        "/missing/trip/b.jpg",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, path)| Media {
        id: MediaId(i as u64),
        path: path.into(),
        kind: MediaKind::Photo,
        fingerprint: Default::default(),
    })
    .collect();
    app.catalog = Some(Arc::new(Catalog {
        root: "/missing".into(),
        media: media.clone(),
        ..Default::default()
    }));
    app.metadata = media.iter().map(|m| (m.id, Metadata::default())).collect();
    app.media = media;
    app
}
#[test]
fn path_menus_keep_originals_separate_from_renamed_companions_and_new_folders() {
    let mut app = app();
    let root = PathBuf::from("/missing");
    let document = root.join("trip/notes and café.txt");
    let catalog = Arc::get_mut(app.catalog.as_mut().unwrap()).unwrap();
    catalog.inventory.push(document.clone());
    catalog.directories = vec![root.clone(), root.join("trip")];
    app.output = "/output".into();
    app.companions = true;
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.apply(
        Command::RenameFile(MediaId(0), "Keeper".into()),
        &app.media,
        true,
    );
    editor.folder = "trip".into();
    editor.name = "Selected".into();
    editor.apply(Command::Rename, &app.media, true);
    for (id, original, destination) in [
        (
            MediaId(0),
            root.join("trip/a.jpg"),
            PathBuf::from("/output/Selected/Keeper.jpg"),
        ),
        (
            MediaId(1),
            root.join("trip/a.ARW"),
            PathBuf::from("/output/Selected/Keeper.ARW"),
        ),
        (
            MediaId::for_path(&document),
            document.clone(),
            PathBuf::from("/output/Selected/notes and café.txt"),
        ),
    ] {
        let row = editor.rows.iter().find(|row| row.id == id).unwrap();
        let paths = editor.file_locations(row);
        assert_eq!(paths.source.as_deref(), Some(original.as_path()));
        assert_eq!(paths.relative.as_deref(), original.strip_prefix(&root).ok());
        assert_eq!(paths.proposed.as_deref(), Some(destination.as_path()));
    }
    let renamed = editor.folder_locations("Selected");
    assert_eq!(renamed.source, Some(root.join("trip")));
    assert_eq!(renamed.relative, Some(PathBuf::from("Selected")));
    assert_eq!(renamed.proposed, Some(PathBuf::from("/output/Selected")));
    editor.draft.create_folder("", "New folder").unwrap();
    let created = editor.folder_locations("New folder");
    assert!(created.source.is_none());
    assert_eq!(created.relative, Some(PathBuf::from("New folder")));
    assert_eq!(created.proposed, Some(PathBuf::from("/output/New folder")));
    assert_eq!(editor.folder_locations("").source, Some(root));
    assert_eq!(
        editor.folder_locations("").relative,
        Some(PathBuf::from("."))
    );
    assert_eq!(app.stats.decodes, 0);
    assert!(app.requested.is_empty());
    assert!(app.decisions.is_empty());
    app.runtime.shutdown();
}

#[test]
fn editor_context_menu_copies_clicked_source_at_compact_and_wide_sizes() {
    for size in [egui::vec2(600.0, 650.0), egui::vec2(1200.0, 850.0)] {
        let mut app = app();
        app.active = Some(MediaId(0));
        app.output = "/output".into();
        app.open_organize_editor();
        app.organize_editor
            .as_mut()
            .unwrap()
            .selected
            .insert(MediaId(0));
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let frame = |app: &mut PhotoSortApp, events| {
            ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    events,
                    ..Default::default()
                },
                |ui| {
                    let ctx = &ui.ctx().clone();
                    app.organize_dialog(ctx)
                },
            )
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let pos = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 1_u64))))
            .unwrap()
            .center();
        let button = |pos, button, pressed| egui::Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Default::default(),
        };
        frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                button(pos, egui::PointerButton::Secondary, true),
                button(pos, egui::PointerButton::Secondary, false),
            ],
        );
        let output = frame(&mut app, vec![]);
        let tree = output.platform_output.accesskit_update.unwrap();
        let node = tree
            .nodes
            .iter()
            .find(|(_, n)| n.label() == Some("Copy full path"))
            .expect("Organize row context menu");
        let bounds = node.1.bounds().unwrap();
        assert!(
            bounds.x0 >= 0.0
                && bounds.y0 >= 0.0
                && bounds.x1 <= f64::from(size.x)
                && bounds.y1 <= f64::from(size.y)
        );
        let pos = egui::pos2(
            ((bounds.x0 + bounds.x1) / 2.0) as f32,
            ((bounds.y0 + bounds.y1) / 2.0) as f32,
        );
        let output = frame(
            &mut app,
            vec![
                egui::Event::PointerMoved(pos),
                button(pos, egui::PointerButton::Primary, true),
                button(pos, egui::PointerButton::Primary, false),
            ],
        );
        assert!(output.platform_output.commands.iter().any(
            |c| matches!(c, egui::OutputCommand::CopyText(text) if text == "/missing/trip/a.ARW")
        ));
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected,
            HashSet::from([MediaId(0)])
        );
        assert_eq!(app.active, Some(MediaId(0)));
        assert!(app.decisions.is_empty());
        assert!(app.requested.is_empty());
        assert_eq!(app.stats.decodes, 0);
        app.runtime.shutdown();
    }
}

#[test]
fn file_rename_drafts_cancel_accept_restore_and_preserve_companions_and_decisions() {
    let mut app = app();
    app.companions = true;
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.apply(
        Command::RenameFile(MediaId(1), "Keeper".into()),
        &app.media,
        true,
    );
    assert_eq!(editor.names.len(), 2);
    assert_eq!(editor.names[&MediaId(0)], "Keeper");
    assert!(app.file_names.is_empty());
    app.finish_organize_edit(editor, 1);
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    assert!(editor.names.is_empty());
    editor.start_rename(MediaId(1));
    editor.renaming.as_mut().unwrap().1 = "Kept".into();
    app.finish_organize_edit(editor, 2);
    assert_eq!(app.file_names.len(), 2);
    assert_eq!(app.file_names[&MediaId(0)], "Kept");
    assert_eq!(app.file_names[&MediaId(1)], "Kept");
    assert!(app.organize_editor.is_none());
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.start_rename(MediaId(0));
    editor.renaming.as_mut().unwrap().1 = "bad/name".into();
    app.finish_organize_edit(editor, 2);
    let mut editor = app.organize_editor.take().unwrap();
    assert!(editor.error.is_some());
    assert_eq!(app.file_names[&MediaId(0)], "Kept");
    editor.apply(
        Command::RenameFile(MediaId(0), String::new()),
        &app.media,
        true,
    );
    app.finish_organize_edit(editor, 2);
    assert!(app.file_names.is_empty());
    assert!(app.decisions.is_empty() && app.requested.is_empty());
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
#[test]
fn enter_commits_a_focused_file_name_after_the_widget_releases_focus() {
    let mut app = app();
    app.companions = true;
    app.open_organize_editor();
    app.organize_editor
        .as_mut()
        .unwrap()
        .start_rename(MediaId(0));
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let mut frame = |events| {
        let _ = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(800., 650.),
                )),
                events,
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.organize_dialog(ctx)
            },
        );
    };
    for _ in 0..3 {
        frame(vec![]);
    }
    frame(vec![egui::Event::Text("Keyboard name".into())]);
    frame(vec![egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }]);
    let editor = app.organize_editor.as_ref().unwrap();
    assert!(editor.renaming.is_none());
    assert_eq!(editor.names[&MediaId(0)], "Keyboard name");
    assert_eq!(editor.names[&MediaId(1)], "Keyboard name");
    assert!(
        app.file_names.is_empty(),
        "Enter stages inside the disposable editor draft"
    );
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
}
#[test]
fn file_keyboard_ranges_marks_rename_and_resume_share_cached_editor_state() {
    for width in [640., 1200.] {
        let mut app = app();
        Arc::get_mut(app.catalog.as_mut().unwrap())
            .unwrap()
            .inventory
            .push("/missing/trip/notes.txt".into());
        app.open_organize_editor();
        let editor = app.organize_editor.as_ref().unwrap();
        let ids: Vec<_> = editor.rows.iter().map(|row| row.id).collect();
        assert_eq!(ids.len(), 4);
        let rows = editor.rows.as_ptr();
        let labels = editor.draft.labels().clone();
        app.decisions.insert(
            ids[1],
            Decision {
                rejected: true,
                ..Default::default()
            },
        );
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
                    app.organize_dialog(ctx)
                },
            );
        };
        let key = |key, shift, command, repeat| egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers: egui::Modifiers {
                shift,
                command,
                ..Default::default()
            },
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let rect = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", ids[0].0))))
            .unwrap();
        frame(&mut app, tree_controls::tests::click(rect.center()));
        frame(
            &mut app,
            vec![key(egui::Key::ArrowDown, false, false, false)],
        );
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!(editor.file_cursor, Some(2), "plain Down skips a Trash mark");
        assert_eq!(
            editor.selected,
            HashSet::from([ids[0]]),
            "focus movement retains checked files"
        );
        frame(&mut app, vec![key(egui::Key::ArrowUp, true, false, false)]);
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected,
            ids[1..=2].iter().copied().collect()
        );
        frame(&mut app, vec![key(egui::Key::ArrowUp, true, false, false)]);
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected,
            ids[..=2].iter().copied().collect()
        );
        frame(
            &mut app,
            vec![key(egui::Key::ArrowDown, true, false, false)],
        );
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected,
            ids[1..=2].iter().copied().collect()
        );
        frame(
            &mut app,
            vec![
                key(egui::Key::Space, false, false, false),
                key(egui::Key::Space, false, false, true),
            ],
        );
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected,
            HashSet::from([ids[2]])
        );
        frame(&mut app, vec![key(egui::Key::A, false, true, false)]);
        assert_eq!(app.organize_editor.as_ref().unwrap().selected.len(), 4);
        frame(&mut app, vec![key(egui::Key::A, true, true, false)]);
        assert!(app.organize_editor.as_ref().unwrap().selected.is_empty());
        frame(
            &mut app,
            vec![
                key(egui::Key::Home, false, false, false),
                key(egui::Key::End, true, false, false),
            ],
        );
        assert_eq!(app.organize_editor.as_ref().unwrap().selected.len(), 4);
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!(editor.rows.as_ptr(), rows);
        assert_eq!(editor.draft.labels(), &labels);
        let bookmark = editor.bookmark();
        assert_eq!(bookmark.focused, Some(ids[3]));
        assert_eq!(bookmark.anchor, Some(ids[0]));
        let editor = app.organize_editor.take().unwrap();
        app.finish_organize_edit(editor, 1);
        app.open_organize_editor();
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!(editor.bookmark(), bookmark);
        for _ in 0..2 {
            frame(&mut app, vec![]);
        }
        let rect = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", ids[3].0))))
            .unwrap();
        frame(&mut app, tree_controls::tests::click(rect.center()));
        frame(&mut app, vec![key(egui::Key::F2, false, false, false)]);
        assert_eq!(
            app.organize_editor
                .as_ref()
                .unwrap()
                .renaming
                .as_ref()
                .unwrap()
                .0,
            ids[3]
        );
        frame(&mut app, vec![]);
        frame(&mut app, vec![key(egui::Key::Home, false, false, false)]);
        assert_eq!(
            app.organize_editor.as_ref().unwrap().file_cursor,
            Some(3),
            "name field owns Home"
        );
        assert!(
            app.requested.is_empty()
                && app.analysis_run.is_none()
                && app.metadata_run.is_none()
                && !app.scanning
        );
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
}
#[test]
fn file_keyboard_large_list_reveals_end_home_and_preserves_pointer_toggle_behavior() {
    for width in [640., 1200.] {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.rows = (0..100_000)
            .map(|i| PhotoRow {
                id: MediaId(i),
                source: format!("/missing/{i:06}.txt").into(),
                after: String::new(),
            })
            .collect();
        let rows = editor.rows.as_ptr();
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
                    app.organize_dialog(ctx)
                },
            );
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let rect = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 0u64))))
            .unwrap();
        frame(&mut app, tree_controls::tests::click(rect.center()));
        frame(&mut app, vec![tree_controls::tests::key(egui::Key::End)]);
        assert_eq!(
            app.organize_editor.as_ref().unwrap().file_cursor,
            Some(99_999)
        );
        assert!(app.organize_editor.as_ref().unwrap().photo_scroll > 1_000_000.);
        frame(&mut app, vec![tree_controls::tests::key(egui::Key::Space)]);
        assert_eq!(
            app.organize_editor.as_ref().unwrap().selected,
            HashSet::from([MediaId(0), MediaId(99_999)])
        );
        frame(&mut app, vec![tree_controls::tests::key(egui::Key::PageUp)]);
        assert!(app.organize_editor.as_ref().unwrap().file_cursor.unwrap() < 99_999);
        frame(&mut app, vec![tree_controls::tests::key(egui::Key::Home)]);
        assert_eq!(app.organize_editor.as_ref().unwrap().photo_scroll, 0.);
        assert_eq!(app.organize_editor.as_ref().unwrap().rows.as_ptr(), rows);
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
}
#[test]
fn file_name_entry_and_errors_fit_compact_and_wide_editor_windows() {
    for size in [
        egui::vec2(640., 600.),
        egui::vec2(800., 650.),
        egui::vec2(1280., 800.),
    ] {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.start_rename(MediaId(0));
        editor.apply(
            Command::RenameFile(MediaId(0), "bad/name".into()),
            &app.media,
            app.companions,
        );
        let ctx = egui::Context::default();
        theme_install(&ctx);
        for _ in 0..3 {
            let _ = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    let ctx = &ui.ctx().clone();
                    app.organize_dialog(ctx)
                },
            );
        }
        let rect = ctx
            .memory(|m| m.area_rect(egui::Id::new("organize_editor")))
            .unwrap();
        assert!(
            rect.width() <= size.x && rect.height() <= size.y,
            "{size:?}: {rect:?}"
        );
        assert!(app.organize_editor.as_ref().unwrap().renaming.is_some());
        assert_eq!(app.stats.decodes, 0);
    }
}
#[test]
fn organize_trash_supporting_selection_and_marks_survive_worker_reload_and_replacement() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.jpg"), b"photo").unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"document").unwrap();
    let root = dir.path().canonicalize().unwrap();
    fn settle(app: &mut PhotoSortApp) {
        let end = Instant::now() + Duration::from_secs(10);
        while app.scanning
            || app.pending_bookmark.is_some()
            || app.filter_dirty
            || app.browse_run.is_some()
        {
            app.poll(&egui::Context::default());
            app.refilter();
            app.restore_bookmark();
            assert!(Instant::now() < end, "{:?}", app.error);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
    app.scan_folder(root.clone());
    settle(&mut app);
    let id = MediaId::for_path(&root.join("notes.txt"));
    app.stage_manual([id], crate::core::decisions::Change::Trash(true));
    assert!(app.decisions[&id].rejected);
    app.save_session();
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.selected.insert(id);
    app.organize_view = Some(editor.bookmark());
    app.view_data_ready.insert(Step::Organize);
    app.checkpoint_bookmarks();
    app.runtime.shutdown();
    drop(app);
    let mut reopened =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
    reopened.scan_folder(root.clone());
    settle(&mut reopened);
    reopened.open_organize_editor();
    assert!(reopened.decisions[&id].rejected);
    assert_eq!(reopened.stats.metadata_reads, 0);
    assert!(
        reopened
            .organize_editor
            .as_ref()
            .unwrap()
            .selected
            .contains(&id)
    );
    assert_eq!(reopened.stats.decodes, 0);
    reopened.organize_editor = None;
    std::fs::remove_file(root.join("notes.txt")).unwrap();
    reopened.scan_folder(root);
    settle(&mut reopened);
    assert!(!reopened.decisions.contains_key(&id));
    reopened.open_organize_editor();
    assert!(
        !reopened
            .organize_editor
            .as_ref()
            .unwrap()
            .selected
            .contains(&id)
    );
}

#[test]
fn large_editor_selection_restores_from_cached_rows() {
    let mut app = app();
    app.open_organize_editor();
    let editor = app.organize_editor.as_mut().unwrap();
    editor.rows = (0..100_000)
        .map(|i| PhotoRow {
            id: MediaId(i),
            source: PathBuf::new(),
            after: String::new(),
        })
        .collect();
    editor.draft = Draft::new(
        (0..10_000)
            .map(|i| (MediaId(i), format!("folder-{i}")))
            .collect(),
    );
    editor.refresh();
    let view = crate::core::views::EditorBookmark {
        selected: (0..100_001).map(MediaId).collect(),
        folders: (0..10_001).map(|i| format!("folder-{i}")).collect(),
        ..Default::default()
    };
    editor.restore_bookmark(&view);
    assert_eq!(editor.selected.len(), 100_000);
    assert_eq!(editor.selected_folders.len(), 10_000);
    assert_eq!(app.stats.decodes, 0);
}

#[test]
fn editor_restores_presentation_without_restoring_draft_edits() {
    let mut app = app();
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.selected.insert(MediaId(1));
    editor.folder = "trip".into();
    editor.selected_folders.insert("trip".into());
    editor.photo_scroll = 120.0;
    editor.folder_scroll = 40.0;
    editor.compact_folders = true;
    let before = editor.draft.labels().clone();
    editor.name = "Changed".into();
    editor.apply(Command::Rename, &app.media, app.companions);
    assert_ne!(editor.draft.labels(), &before);
    app.organize_view = Some(editor.bookmark());
    // Closing without storing the draft is Cancel; only presentation survives.
    app.open_organize_editor();
    let reopened = app.organize_editor.as_ref().unwrap();
    assert_eq!(reopened.draft.labels(), &before);
    assert!(reopened.selected.contains(&MediaId(1)));
    assert_eq!(reopened.restore_scroll, [Some(120.0), Some(40.0)]);
    assert!(reopened.compact_folders);
    assert!(!reopened.draft.folders().contains("Changed"));
    assert!(app.decisions.is_empty() && app.requested.is_empty());
}
#[test]
fn editor_moves_companions_together_without_touching_review_decisions_or_images() {
    let mut app = app();
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.folder = editor.draft.create_folder("", "keepers").unwrap();
    editor.selected.insert(MediaId(0));
    editor.apply(Command::Move, &app.media, true);
    assert_eq!(editor.draft.labels()[&MediaId(0)], "keepers");
    assert_eq!(editor.draft.labels()[&MediaId(1)], "keepers");
    assert_eq!(editor.draft.labels()[&MediaId(2)], "trip");
    assert!(app.decisions.is_empty());
    assert_eq!(app.stats.decodes, 0);
    assert!(!app.scanning && app.analysis_run.is_none());
}
#[test]
fn editor_isolated_draft_can_cancel_and_modes_retain_separate_edits() {
    let mut app = app();
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.draft.rename_folder("trip", "holiday").unwrap();
    app.organize_drafts.insert(GroupMode::Current, editor.draft);
    app.open_organize_editor();
    app.organize_editor
        .as_mut()
        .unwrap()
        .draft
        .rename_folder("holiday", "discarded")
        .unwrap();
    app.organize_editor = None; // Cancel the working copy.
    app.group_mode = GroupMode::Camera;
    app.open_organize_editor();
    assert!(
        app.organize_editor
            .as_ref()
            .unwrap()
            .draft
            .folders()
            .contains("Unknown camera")
    );
    app.organize_editor = None;
    app.group_mode = GroupMode::Current;
    app.open_organize_editor();
    let editor = app.organize_editor.as_ref().unwrap();
    assert!(editor.draft.folders().contains("holiday"));
    assert!(!editor.draft.folders().contains("discarded"));
}
#[test]
fn pending_analysis_blocks_editor_and_modal_blocks_workflow_keyboard() {
    let mut app = app();
    app.analysis_run = Some(8);
    app.open_organize_editor();
    assert!(app.organize_editor.is_none());
    app.analysis_run = None;
    app.open_organize_editor();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key {
        key: egui::Key::I,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    let _ = ctx.run_test(input, |ui| {
        let ctx = &ui.ctx().clone();
        app.keyboard(ctx)
    });
    assert!(
        app.inspector,
        "Editor keystrokes must not toggle the underlying inspector"
    );
    app.request_departure(Destination::Step(Step::Cull));
    assert_eq!(app.step, Step::Organize);
}
#[test]
fn editor_and_footer_fit_wide_and_compact_windows() {
    for size in [
        egui::vec2(640.0, 600.0),
        egui::vec2(800.0, 640.0),
        egui::vec2(1440.0, 960.0),
    ] {
        for folders in [false, true] {
            let mut app = app();
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.compact_folders = folders;
            editor.error = Some("A folder cannot contain itself".into());
            for n in 0..100 {
                editor
                    .draft
                    .create_folder("", &format!("folder-{n}"))
                    .unwrap();
            }
            editor.refresh();
            // Enough rows to exercise scroll limits, not just empty content.
            for n in 3..100 {
                editor.rows.push(PhotoRow {
                    id: MediaId(n),
                    source: PathBuf::from(format!("/missing/trip/day1/IMG_{n:04}.jpg")),

                    after: "trip/day1".into(),
                });
            }
            let ctx = egui::Context::default();
            theme_install(&ctx);
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                };
                let _ = ctx.run_test(input, |ui| {
                    let ctx = &ui.ctx().clone();
                    app.organize_dialog(ctx)
                });
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new("organize_editor")))
                .unwrap();
            assert!(
                rect.height() <= size.y && rect.width() <= size.x,
                "{size:?} folders={folders}: {rect:?}"
            );
        }
    }
}
#[test]
fn editor_includes_documents_from_shared_inventory_in_subtree_edits() {
    let mut app = app();
    Arc::get_mut(app.catalog.as_mut().unwrap())
        .unwrap()
        .inventory
        .push("/missing/trip/notes.json".into());
    app.open_organize_editor();
    let editor = app.organize_editor.as_mut().unwrap();
    assert_eq!(editor.rows.len(), 4);
    editor.draft.rename_folder("trip", "holiday").unwrap();
    editor.refresh();
    assert!(editor.rows.iter().all(|row| row.after == "holiday"));
    assert!(editor.rows.iter().any(|row| {
        row.source
            .file_name()
            .is_some_and(|name| name == "notes.json")
    }));
    assert_eq!(app.stats.decodes, 0);
    assert!(app.analysis_run.is_none());
}
#[test]
fn hidden_inventory_reaches_editor_and_shared_planner_without_decoding() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join(".before/.empty")).unwrap();
    for name in [".photo.jpg", ".photo.xmp", ".notes.json"] {
        std::fs::write(temp.path().join(".before").join(name), b"unchanged").unwrap();
    }
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.output = catalog.root.display().to_string();
    app.media = catalog.media.clone();
    app.catalog = Some(Arc::new(catalog));
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    assert_eq!(editor.rows.len(), 2);
    assert!(editor.rows.iter().any(|row| {
        row.source
            .file_name()
            .is_some_and(|name| name == ".photo.jpg")
    }));
    assert!(editor.rows.iter().any(|row| {
        row.source
            .file_name()
            .is_some_and(|name| name == ".notes.json")
    }));
    assert!(editor.rows.iter().all(|row| {
        row.source
            .parent()
            .is_some_and(|parent| parent == editor.catalog.root.join(".before"))
    }));
    assert!(editor.draft.folders().contains(".before/.empty"));
    editor.draft.rename_folder(".before", ".after").unwrap();
    editor.refresh();
    assert!(editor.rows.iter().all(|row| row.after == ".after"));
    app.organize_drafts.insert(GroupMode::Current, editor.draft);
    app.organize();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.poll(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.plan_open && app.error.is_none(), "{:?}", app.error);
    let plan = app.plan.as_ref().unwrap();
    use crate::core::operations::Action;
    assert_eq!(
        plan.actions
            .iter()
            .filter(|action| matches!(action, Action::Move { .. }))
            .count(),
        2
    );
    assert!(plan.actions.iter().any(|action| matches!(action, Action::CreateDirectory { path } if path.ends_with(".after/.empty"))));
    for name in [".photo.jpg", ".photo.xmp", ".notes.json"] {
        assert_eq!(
            std::fs::read(temp.path().join(".before").join(name)).unwrap(),
            b"unchanged"
        );
    }
    assert!(!temp.path().join(".after").exists());
    assert_eq!(app.stats.decodes, 0);
    assert!(app.analysis_run.is_none());
}
#[test]
fn empty_folder_editor_intent_reaches_shared_io_worker_without_mutation() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("existing/empty")).unwrap();
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.output = catalog.root.display().to_string();
    app.catalog = Some(Arc::new(catalog));
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    assert!(editor.draft.folders().contains("existing/empty"));
    editor.draft.create_folder("", "new-empty").unwrap();
    app.organize_drafts.insert(GroupMode::Current, editor.draft);
    app.organize();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let deadline = Instant::now() + Duration::from_secs(5);
    while app.busy && Instant::now() < deadline {
        app.poll(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.plan_open && app.error.is_none());
    let plan = app.plan.as_ref().unwrap();
    assert_eq!(plan.actions.len(), 1);
    assert!(
        matches!(&plan.actions[0], crate::core::operations::Action::CreateDirectory {path} if path.ends_with("new-empty"))
    );
    assert!(!temp.path().join("new-empty").exists());
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn drag_sources_preserve_multi_selection_and_replace_unselected_sources() {
    let mut app = app();
    app.open_organize_editor();
    let editor = app.organize_editor.as_mut().unwrap();
    editor.selected.extend([MediaId(0), MediaId(1)]);
    assert_eq!(editor.file_drag(MediaId(0)).selection.files.len(), 2);
    assert_eq!(
        editor.file_drag(MediaId(2)).selection.files,
        HashSet::from([MediaId(2)])
    );
    editor.draft.create_folder("", "target").unwrap();
    editor
        .selected_folders
        .extend(["trip".into(), "target".into()]);
    assert_eq!(editor.folder_drag("trip").selection.folders.len(), 2);
    editor.draft.create_folder("", "third").unwrap();
    assert_eq!(editor.folder_drag("third").selection.folders.len(), 1);
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn drop_and_button_share_companion_expansion_and_failed_drop_is_unchanged() {
    let mut app = app();
    app.open_organize_editor();
    let mut button = app.organize_editor.take().unwrap();
    app.open_organize_editor();
    let mut dragged = app.organize_editor.take().unwrap();
    for editor in [&mut button, &mut dragged] {
        editor.draft.create_folder("", "target").unwrap();
        editor.selected.insert(MediaId(0));
    }
    button.folder = "target".into();
    button.apply(Command::Move, &app.media, true);
    let payload = dragged.file_drag(MediaId(0));
    dragged.apply(
        Command::Drop(payload.selection, "target".into()),
        &app.media,
        true,
    );
    assert_eq!(dragged.draft, button.draft);
    assert_eq!(dragged.draft.labels()[&MediaId(1)], "target");
    assert_eq!(dragged.rows[1].after, "target");
    let before = dragged.draft.clone();
    let payload = dragged.folder_drag("trip");
    dragged.apply(
        Command::Drop(payload.selection, "trip".into()),
        &app.media,
        true,
    );
    assert!(dragged.error.is_some());
    assert_eq!(dragged.draft, before);
    assert!(app.decisions.is_empty());
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn drop_target_scopes_payload_to_editor_and_escape_cancels_drag() {
    let mut app = app();
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    editor.draft.create_folder("", "target").unwrap();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let mut rect = egui::Rect::NOTHING;
    let mut dropped = None;
    let _ = ctx.run_test(Default::default(), |ui| {
        egui::CentralPanel::default().show(ui, |ui| {
            rect = ui.button("target").rect;
        });
    });
    let position = rect.center();
    let payload = editor.file_drag(MediaId(0));
    for own in [false, true] {
        let payload = FolderDrag {
            owner: if own {
                payload.owner.clone()
            } else {
                Arc::new(())
            },
            selection: payload.selection.clone(),
        };
        egui::DragAndDrop::set_payload(&ctx, payload);
        let input = egui::RawInput {
            events: vec![
                egui::Event::PointerMoved(position),
                egui::Event::PointerButton {
                    pos: position,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
            ..Default::default()
        };
        let _ = ctx.run_test(input, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let response = ui.button("target");
                dropped = editor.drop_target(ui, &response, "target");
            });
        });
        assert_eq!(dropped.is_some(), own);
    }
    egui::DragAndDrop::set_payload(&ctx, payload);
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: Default::default(),
    });
    let _ = ctx.run_test(input, |_| {});
    assert!(!egui::DragAndDrop::has_any_payload(&ctx));
    assert_eq!(editor.draft.labels()[&MediaId(0)], "trip");
    for cancellation in 0..4 {
        let _ = ctx.run_test(
            egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                editor.begin_drag_frame(ctx)
            },
        );
        assert!(editor.drag.start().is_some());
        let mut input = egui::RawInput {
            focused: cancellation != 1,
            ..Default::default()
        };
        if cancellation == 0 {
            input.events.push(egui::Event::Key {
                key: egui::Key::Escape,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            });
        }
        if cancellation == 2 {
            input.events.push(egui::Event::PointerGone);
        }
        egui::DragAndDrop::set_payload(&ctx, editor.file_drag(MediaId(0)));
        if cancellation == 3 {
            let _ = ctx.run_test(Default::default(), |_| {});
        }
        let _ = ctx.run_test(input, |ui| {
            let ctx = &ui.ctx().clone();
            editor.begin_drag_frame(ctx)
        });
        assert!(editor.drag.start().is_none());
        assert!(editor.completed_drag.is_none());
        assert!(
            !egui::DragAndDrop::has_any_payload(&ctx),
            "cancellation {cancellation}"
        );
    }
}
fn hover_frame(
    ctx: &egui::Context,
    app: &mut PhotoSortApp,
    time: f64,
    width: f32,
    events: Vec<egui::Event>,
) {
    let _ = ctx.run_test(
        egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(width, 960.),
            )),
            events,
            ..Default::default()
        },
        |ui| {
            let ctx = &ui.ctx().clone();
            app.organize_dialog(ctx)
        },
    );
}
fn hover_rect(ctx: &egui::Context, path: &str) -> egui::Pos2 {
    ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", path))))
        .expect("visible folder row")
        .center()
}
fn hover_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    }
}
#[test]
fn drag_hover_expansion_reaches_nested_destinations_in_wide_and_compact_layouts() {
    for (width, folder_drag) in [640., 1440.]
        .into_iter()
        .flat_map(|w| [false, true].map(|folder| (w, folder)))
    {
        let mut app = app();
        app.companions = true;
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.draft.create_folder("", "target").unwrap();
        editor.draft.create_folder("target", "nested").unwrap();
        editor.draft.create_folder("target/nested", "deep").unwrap();
        editor.refresh();
        editor.collapsed.extend([
            Key::Folder("target".into()),
            Key::Folder("target/nested".into()),
        ]);
        editor.project_folders();
        editor.compact_folders = folder_drag;
        let original = editor.draft.clone();
        let nodes = editor.tree.nodes.as_ptr();
        let catalog = Arc::as_ptr(&editor.catalog);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        for t in [0., 0.05, 0.1] {
            hover_frame(&ctx, &mut app, t, width, vec![]);
        }
        let source = if folder_drag {
            hover_rect(&ctx, "trip")
        } else {
            ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 0_u64))))
                .unwrap()
                .center()
        };
        hover_frame(
            &ctx,
            &mut app,
            0.2,
            width,
            vec![
                egui::Event::PointerMoved(source),
                hover_button(source, true),
            ],
        );
        hover_frame(
            &ctx,
            &mut app,
            0.3,
            width,
            vec![egui::Event::PointerMoved(source + egui::vec2(20., 0.))],
        );
        if width < 680. && !folder_drag {
            let tab = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("test_folders_tab")))
                .unwrap()
                .center();
            hover_frame(
                &ctx,
                &mut app,
                0.35,
                width,
                vec![egui::Event::PointerMoved(tab)],
            );
            hover_frame(&ctx, &mut app, 0.36, width, vec![]);
            // The newly shown folder list settles on its second frame.
            hover_frame(&ctx, &mut app, 0.37, width, vec![]);
        }
        let selected = app.organize_editor.as_ref().unwrap().selected.clone();
        let folders = app
            .organize_editor
            .as_ref()
            .unwrap()
            .selected_folders
            .clone();
        let target = hover_rect(&ctx, "target");
        hover_frame(
            &ctx,
            &mut app,
            0.4,
            width,
            vec![egui::Event::PointerMoved(target)],
        );
        hover_frame(&ctx, &mut app, 0.9, width, vec![]);
        assert!(
            app.organize_editor
                .as_ref()
                .unwrap()
                .collapsed
                .contains(&Key::Folder("target".into()))
        );
        hover_frame(&ctx, &mut app, 1.05, width, vec![]);
        hover_frame(&ctx, &mut app, 1.1, width, vec![]);
        assert!(
            !app.organize_editor
                .as_ref()
                .unwrap()
                .collapsed
                .contains(&Key::Folder("target".into()))
        );
        assert!(hover_rect(&ctx, "target").distance(target) <= 1.0);
        let nested = hover_rect(&ctx, "target/nested");
        hover_frame(
            &ctx,
            &mut app,
            1.2,
            width,
            vec![egui::Event::PointerMoved(nested)],
        );
        hover_frame(&ctx, &mut app, 1.9, width, vec![]);
        hover_frame(&ctx, &mut app, 1.95, width, vec![]);
        assert!(hover_rect(&ctx, "target").distance(target) <= 1.0);
        assert!(hover_rect(&ctx, "target/nested").distance(nested) <= 1.0);
        let editor = app.organize_editor.as_ref().unwrap();
        assert!(
            !editor
                .collapsed
                .contains(&Key::Folder("target/nested".into())),
            "width={width} folder_drag={folder_drag} initial={nested:?} current={:?} pointer={:?}",
            hover_rect(&ctx, "target/nested"),
            ctx.pointer_interact_pos()
        );
        assert_eq!(editor.draft, original);
        assert_eq!(editor.selected, selected);
        assert_eq!(editor.selected_folders, folders);
        assert_eq!(editor.tree.nodes.as_ptr(), nodes);
        assert_eq!(Arc::as_ptr(&editor.catalog), catalog);
        let destination = hover_rect(&ctx, "target/nested/deep");
        hover_frame(
            &ctx,
            &mut app,
            2.,
            width,
            vec![
                egui::Event::PointerMoved(destination),
                hover_button(destination, false),
            ],
        );
        let editor = app.organize_editor.as_ref().unwrap();
        assert!(editor.error.is_none(), "{:?}", editor.error);
        let expected = if folder_drag {
            "target/nested/deep/trip"
        } else {
            "target/nested/deep"
        };
        assert_eq!(editor.draft.labels()[&MediaId(0)], expected);
        assert_eq!(editor.draft.labels()[&MediaId(1)], expected);
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(app.stats.metadata_reads, 0);
    }
}
#[test]
fn drag_hover_expansion_ignores_foreign_and_cancelled_gestures() {
    for interruption in 0..4 {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.draft.create_folder("", "target").unwrap();
        editor.draft.create_folder("target", "nested").unwrap();
        editor.refresh();
        editor.collapsed.insert(Key::Folder("target".into()));
        editor.project_folders();
        let original = editor.draft.clone();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        for t in [0., 0.05, 0.1] {
            hover_frame(&ctx, &mut app, t, 1440., vec![]);
        }
        let source = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 0_u64))))
            .unwrap()
            .center();
        hover_frame(
            &ctx,
            &mut app,
            0.2,
            1440.,
            vec![
                egui::Event::PointerMoved(source),
                hover_button(source, true),
            ],
        );
        hover_frame(
            &ctx,
            &mut app,
            0.3,
            1440.,
            vec![egui::Event::PointerMoved(source + egui::vec2(20., 0.))],
        );
        let target = hover_rect(&ctx, "target");
        hover_frame(
            &ctx,
            &mut app,
            0.4,
            1440.,
            vec![egui::Event::PointerMoved(target)],
        );
        let events = match interruption {
            0 => vec![tree_controls::tests::key(egui::Key::Escape)],
            1 => vec![
                egui::Event::WindowFocused(false),
                egui::Event::WindowFocused(true),
            ],
            2 => vec![egui::Event::PointerGone],
            _ => {
                let mut payload = app.organize_editor.as_mut().unwrap().file_drag(MediaId(0));
                payload.owner = Arc::new(());
                egui::DragAndDrop::set_payload(&ctx, payload);
                vec![]
            }
        };
        hover_frame(&ctx, &mut app, 0.5, 1440., events);
        hover_frame(
            &ctx,
            &mut app,
            1.5,
            1440.,
            vec![egui::Event::PointerMoved(target)],
        );
        let editor = app.organize_editor.as_ref().unwrap();
        assert!(
            editor.collapsed.contains(&Key::Folder("target".into())),
            "interruption {interruption}"
        );
        assert_eq!(editor.draft, original);
        assert_eq!(app.stats.decodes, 0);
    }
}
#[test]
fn pointer_drags_in_scrollable_modal_reach_shared_draft() {
    for (folder, timing) in [false, true]
        .into_iter()
        .flat_map(|folder| (0..4).map(move |timing| (folder, timing)))
    {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.draft.create_folder("", "target").unwrap();
        editor.refresh();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let frame = |app: &mut PhotoSortApp, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 960.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run_test(input, |ui| {
                let ctx = &ui.ctx().clone();
                app.organize_dialog(ctx)
            });
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let source = if folder {
            egui::Id::new(("test_folder_rect", "trip"))
        } else {
            egui::Id::new(("test_file_rect", 0_u64))
        };
        let source = ctx
            .data(|d| d.get_temp::<egui::Rect>(source))
            .unwrap()
            .center();
        let target = ctx
            .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", "target"))))
            .unwrap()
            .center();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Default::default(),
        };
        if timing == 1 {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(source),
                    button(source, true),
                    egui::Event::PointerMoved(target),
                    button(target, false),
                ],
            );
        } else if timing == 2 {
            frame(
                &mut app,
                vec![egui::Event::PointerMoved(source), button(source, true)],
            );
            frame(
                &mut app,
                vec![egui::Event::PointerMoved(target), button(target, false)],
            );
        } else if timing == 3 {
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(source),
                    button(source, true),
                    egui::Event::PointerMoved(target),
                ],
            );
            frame(&mut app, vec![button(target, false)]);
        } else {
            frame(
                &mut app,
                vec![egui::Event::PointerMoved(source), button(source, true)],
            );
            frame(
                &mut app,
                vec![egui::Event::PointerMoved(source + egui::vec2(20.0, 0.0))],
            );
            assert!(
                egui::DragAndDrop::has_payload_of_type::<FolderDrag>(&ctx),
                "Drag did not start: folder={folder}, source={source:?}, current={:?}, scroll={:?}, gesture={:?}",
                ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 0_u64)))),
                ctx.data(
                    |d| d.get_temp::<(egui::Rect, Vec2, f32, f32)>(egui::Id::new(
                        "test_organize_photo_scroll"
                    ))
                ),
                app.organize_editor.as_ref().unwrap().drag.start()
            );
            frame(&mut app, vec![egui::Event::PointerMoved(target)]);
            frame(&mut app, vec![button(target, false)]);
        }
        let editor = app.organize_editor.as_ref().unwrap();
        assert!(editor.error.is_none(), "{:?}", editor.error);
        assert_eq!(
            editor.draft.labels()[&MediaId(0)],
            if folder { "target/trip" } else { "target" }
        );
        assert_eq!(
            editor.draft.labels()[&MediaId(1)],
            if folder { "target/trip" } else { "target" }
        );
        assert_eq!(app.stats.decodes, 0);
    }
}
#[test]
fn compact_drag_can_switch_tabs_without_losing_its_selection() {
    let mut app = app();
    app.open_organize_editor();
    let editor = app.organize_editor.as_mut().unwrap();
    editor.draft.create_folder("", "target").unwrap();
    editor.refresh();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let frame = |app: &mut PhotoSortApp, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 600.0),
            )),
            events,
            ..Default::default()
        };
        let _ = ctx.run_test(input, |ui| {
            let ctx = &ui.ctx().clone();
            app.organize_dialog(ctx)
        });
    };
    for _ in 0..3 {
        frame(&mut app, vec![]);
    }
    let rect = |id| ctx.data(|d| d.get_temp::<egui::Rect>(id)).unwrap();
    let source = rect(egui::Id::new(("test_file_rect", 0_u64))).center();
    let tab = rect(egui::Id::new("test_folders_tab")).center();
    let button = |pos, pressed| egui::Event::PointerButton {
        pos,
        pressed,
        button: egui::PointerButton::Primary,
        modifiers: Default::default(),
    };
    frame(
        &mut app,
        vec![egui::Event::PointerMoved(source), button(source, true)],
    );
    frame(
        &mut app,
        vec![egui::Event::PointerMoved(source + egui::vec2(20.0, 0.0))],
    );
    frame(&mut app, vec![egui::Event::PointerMoved(tab)]);
    assert!(app.organize_editor.as_ref().unwrap().compact_folders);
    frame(&mut app, vec![]);
    let target = rect(egui::Id::new(("test_folder_rect", "target"))).center();
    frame(&mut app, vec![egui::Event::PointerMoved(target)]);
    frame(&mut app, vec![button(target, false)]);
    let editor = app.organize_editor.as_ref().unwrap();
    assert_eq!(editor.draft.labels()[&MediaId(0)], "target");
    assert_eq!(editor.draft.labels()[&MediaId(1)], "target");
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn organize_selected_media_preview_is_available_without_accepting_edits() {
    let mut app = app();
    app.step = Step::Organize;
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.open_organize_editor();
    let editor = app.organize_editor.as_mut().unwrap();
    editor.selected.extend([MediaId(0), MediaId(2)]);
    editor.draft.create_folder("", "Pending").unwrap();
    editor.refresh();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut labels = Vec::new();
    for _ in 0..3 {
        let output = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640., 600.),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.organize_dialog(ctx)
            },
        );
        labels = output
            .platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .filter_map(|(_, n)| n.label().map(str::to_owned))
            .collect();
    }
    assert!(
        labels.iter().any(|s| s == "Preview selection"),
        "No photo preview from the folder editor: {labels:?}"
    );
    assert!(app.organize_drafts.is_empty());
    assert!(app.plan.is_none());
    app.runtime.shutdown();
}
#[test]
fn organize_preview_preserves_draft_and_view_and_reuses_textures_at_both_sizes() {
    use super::super::compact_cull_tests::{click_label, fixture, rendered};
    for size in [egui::vec2(640., 600.), egui::vec2(1440., 960.)] {
        let (_temp, mut app) = fixture();
        app.step = Step::Organize;
        app.review = true;
        let ids: Vec<_> = app.media.iter().map(|m| m.id).collect();
        app.active = Some(ids[8]);
        app.comparison = vec![ids[7], ids[8]];
        app.focused_slot = 1;
        app.sync_views = false;
        app.inspection_view.zoom = 2.0;
        app.independent_views.insert(
            1,
            InspectionView {
                zoom: 3.0,
                ..Default::default()
            },
        );
        app.decisions.insert(
            ids[0],
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        let before = app.capture_bookmark();
        let catalog = app.catalog.clone().unwrap();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        for id in &ids {
            for preview in [false, true] {
                let handle = ctx.load_texture(
                    format!("existing-{id:?}-{preview}"),
                    egui::ColorImage::filled([16, 16], Color32::WHITE),
                    egui::TextureOptions::LINEAR,
                );
                app.textures.insert(
                    (*id, preview),
                    Texture {
                        handle,
                        bytes: 16 * 16 * 4,
                        used: 0,
                    },
                );
            }
        }
        let texture_ids: HashMap<_, _> = app
            .textures
            .iter()
            .map(|(key, value)| (*key, value.handle.id()))
            .collect();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.selected.extend([ids[2], ids[0], MediaId(u64::MAX)]);
        editor.draft.create_folder("", "Pending").unwrap();
        editor.renaming = Some((ids[0], "unfinished name".into()));
        editor.refresh();
        let selected = editor.selected.clone();
        let labels = editor.draft.labels().clone();
        click_label(&mut app, &ctx, size, "Preview selection");
        assert_eq!(app.comparison_ids(), vec![ids[0], ids[2]]);
        let nodes = rendered(&mut app, &ctx, size);
        for label in ["< Back to arranging", "Fit", "1:1", "Sync"] {
            let rect = nodes.iter().find(|(name, _)| name == label).expect(label).1;
            assert!(
                rect.min.x >= 0.
                    && rect.max.x <= size.x
                    && rect.min.y >= 0.
                    && rect.max.y <= size.y,
                "{label} clipped at {size:?}: {rect:?}"
            );
        }
        assert!(!nodes.iter().any(|(label, _)| label == "Clear rating"));
        let saved = app.capture_bookmark();
        assert_eq!(saved.active, before.active);
        assert_eq!(saved.comparison, before.comparison);
        assert_eq!(saved.view, before.view);
        assert_eq!(saved.independent_views, before.independent_views);
        assert_eq!(saved.sync_views, before.sync_views);
        click_label(&mut app, &ctx, size, "Sync");
        click_label(&mut app, &ctx, size, "Fit");
        click_label(&mut app, &ctx, size, "< Back to arranging");
        assert!(app.organize_preview().is_none());
        assert_eq!(app.active, before.active);
        assert_eq!(app.focused_slot, 1);
        assert_eq!(app.inspection_view, before.view);
        assert_eq!(app.independent_views, before.independent_views);
        assert_eq!(app.sync_views, before.sync_views);
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!(editor.selected, selected);
        assert_eq!(editor.draft.labels(), &labels);
        assert!(editor.draft.folders().contains("Pending"));
        assert_eq!(editor.renaming, Some((ids[0], "unfinished name".into())));
        assert!(app.organize_drafts.is_empty() && app.file_names.is_empty() && app.plan.is_none());
        assert_eq!(app.decisions[&ids[0]].rating, Some(4));
        assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        assert!(
            app.requested.is_empty() && app.analysis_run.is_none() && app.metadata_run.is_none()
        );
        for (key, handle) in texture_ids {
            assert_eq!(app.textures[&key].handle.id(), handle);
        }
        click_label(&mut app, &ctx, size, "Cancel edits");
        assert!(app.organize_editor.is_none() && app.organize_drafts.is_empty());
        app.runtime.shutdown();
    }
}

#[test]
fn organize_preview_bounds_selection_and_does_not_accept_pending_names() {
    let mut app = app();
    app.step = Step::Organize;
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.open_organize_editor();
    for selection in [
        HashSet::new(),
        HashSet::from([MediaId(u64::MAX)]),
        (0..100_000).map(MediaId).collect(),
    ] {
        app.organize_editor.as_mut().unwrap().selected = selection;
        app.begin_organize_preview();
        assert!(app.organize_preview().is_none());
    }
    app.organize_editor.as_mut().unwrap().selected = HashSet::from([MediaId(2)]);
    app.begin_organize_preview();
    assert_eq!(app.comparison_ids(), vec![MediaId(2)]);
    app.end_organize_preview();
    assert!(app.plan.is_none() && app.organize_drafts.is_empty());
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    app.runtime.shutdown();
}

#[test]
fn organize_preview_submits_detail_after_modal_render_and_escape_cancels_it() {
    use super::super::compact_cull_tests::{fixture, frame};
    let (_temp, mut app) = fixture();
    app.step = Step::Organize;
    let id = app.media[0].id;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let handle = ctx.load_texture(
        "preview",
        egui::ColorImage::filled([16, 16], Color32::WHITE),
        egui::TextureOptions::LINEAR,
    );
    app.textures.insert(
        (id, true),
        Texture {
            handle,
            bytes: 16 * 16 * 4,
            used: 0,
        },
    );
    app.open_organize_editor();
    app.organize_editor.as_mut().unwrap().selected.insert(id);
    app.begin_organize_preview();
    app.actual_size();
    let size = egui::vec2(640., 600.);
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.inspection_frames.is_empty() {
        frame(&mut app, &ctx, size, vec![]);
        assert!(
            Instant::now() < deadline,
            "modal detail never reached the shared worker: {:?}",
            app.error
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(!app.inspection_wanted.is_empty());
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert!(app.organize_preview().is_none());
    assert!(app.inspection_wanted.is_empty() && app.inspection_frames.is_empty());
    assert!(app.organize_editor.is_some() && app.plan.is_none());
    app.runtime.shutdown();
}
#[cfg(any(target_os = "macos", feature = "portable-video"))]
#[test]
fn organize_preview_video_uses_shared_player_and_pauses_for_other_dialogs() {
    use super::super::compact_cull_tests::fixture;
    let (_temp, mut app) = fixture();
    app.step = Step::Organize;
    app.review = false;
    app.media[0].kind = MediaKind::Video;
    let id = app.media[0].id;
    app.runtime.playback = crate::core::playback::test_player();
    app.open_organize_editor();
    app.organize_editor.as_mut().unwrap().selected.insert(id);
    app.begin_organize_preview();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let sync = |app: &mut PhotoSortApp, focused| {
        let _ = ctx.run_test(
            egui::RawInput {
                focused,
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.sync_playback(ctx)
            },
        );
    };
    sync(&mut app, true);
    assert_eq!(app.runtime.playback.snapshot().id, Some(id));
    app.runtime.playback.toggle();
    sync(&mut app, true);
    assert!(app.runtime.playback.snapshot().play_requested);
    app.help = true;
    sync(&mut app, true);
    assert!(!app.runtime.playback.snapshot().play_requested);
    app.help = false;
    app.runtime.playback.toggle();
    sync(&mut app, false);
    assert!(!app.runtime.playback.snapshot().play_requested);
    app.end_organize_preview();
    sync(&mut app, true);
    assert!(app.runtime.playback.snapshot().id.is_none());
    app.runtime.shutdown();
}
#[test]
fn organize_preview_defers_saved_view_restoration_until_editor_closes() {
    let mut app = app();
    app.step = Step::Organize;
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.filtered = vec![0, 1, 2];
    app.active = Some(MediaId(1));
    app.open_organize_editor();
    app.organize_editor
        .as_mut()
        .unwrap()
        .selected
        .insert(MediaId(2));
    app.pending_bookmark = Some(crate::core::views::Bookmark {
        active: Some(MediaId(0)),
        comparison: vec![MediaId(0), MediaId(1)],
        view: InspectionView {
            zoom: 2.0,
            ..Default::default()
        },
        ..Default::default()
    });
    app.filter_dirty = false;
    app.view_data_ready.insert(Step::Organize);
    app.begin_organize_preview();
    app.restore_bookmark();
    app.fit_view();
    assert_eq!(app.active, Some(MediaId(2)));
    assert!(app.pending_bookmark.is_some());
    app.end_organize_preview();
    assert_eq!(app.active, Some(MediaId(1)));
    let editor = app.organize_editor.take().unwrap();
    app.finish_organize_edit(editor, 1);
    app.restore_bookmark();
    assert!(app.pending_bookmark.is_none());
    assert_eq!(app.active, Some(MediaId(0)));
    assert_eq!(app.comparison, vec![MediaId(0), MediaId(1)]);
    assert_eq!(app.inspection_view.zoom, 2.0);
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    app.runtime.shutdown();
}
#[test]
fn clear_all_trash_editor_menu_preserves_pending_names_layout_and_selection() {
    use super::super::compact_cull_tests::{click_label, fixture};
    for size in [egui::vec2(640., 600.), egui::vec2(1440., 960.)] {
        let (_temp, mut app) = fixture();
        app.step = Step::Organize;
        let ids = [app.media[0].id, app.media[8].id];
        for id in ids {
            app.decisions.insert(
                id,
                Decision {
                    rejected: true,
                    rating: Some(4),
                    turns: 1,
                },
            );
        }
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.selected.insert(ids[0]);
        editor.draft.create_folder("", "Pending").unwrap();
        editor.renaming = Some((ids[0], "unfinished name".into()));
        editor.refresh();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        click_label(&mut app, &ctx, size, "Actions");
        click_label(&mut app, &ctx, size, "Clear all Trash marks · Alt+D");
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!(editor.renaming, Some((ids[0], "unfinished name".into())));
        assert_eq!(editor.selected, HashSet::from([ids[0]]));
        assert!(editor.draft.folders().contains("Pending"));
        assert!(app.plan.is_none() && app.file_names.is_empty() && app.organize_drafts.is_empty());
        for id in ids {
            assert!(!app.decision(id).rejected);
            assert_eq!(app.decision(id).rating, Some(4));
            assert_eq!(app.decision(id).turns, 1);
        }
        app.runtime.shutdown();
    }
}
#[test]
fn clear_all_trash_editor_keyboard_uses_owned_file_and_folder_focus() {
    use super::super::compact_cull_tests::{fixture, frame, rendered};
    let size = egui::vec2(640., 600.);
    for folders in [false, true] {
        let (_temp, mut app) = fixture();
        app.step = Step::Organize;
        let id = app.media[0].id;
        app.decisions.insert(
            id,
            Decision {
                rejected: true,
                ..Default::default()
            },
        );
        app.open_organize_editor();
        app.organize_editor.as_mut().unwrap().compact_folders = folders;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        rendered(&mut app, &ctx, size);
        let key = if folders {
            egui::Id::new(("test_folder_rect", ""))
        } else {
            egui::Id::new(("test_file_rect", id.0))
        };
        let pos = ctx
            .data(|d| d.get_temp::<egui::Rect>(key))
            .expect("visible row")
            .center();
        frame(
            &mut app,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Key {
                key: egui::Key::D,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::ALT,
            }],
        );
        assert!(!app.decision(id).rejected, "folders={folders}");
        assert!(app.organize_editor.is_some() && app.plan.is_none());
        app.runtime.shutdown();
    }
}
fn action_frame(app: &mut PhotoSortApp, ctx: &egui::Context, width: f32, events: Vec<egui::Event>) {
    let events = events
        .into_iter()
        .flat_map(|event| {
            let release = match &event {
                egui::Event::Key {
                    key,
                    physical_key,
                    modifiers,
                    pressed: true,
                    ..
                } => Some(egui::Event::Key {
                    key: *key,
                    physical_key: *physical_key,
                    modifiers: *modifiers,
                    pressed: false,
                    repeat: false,
                }),
                _ => None,
            };
            std::iter::once(event).chain(release)
        })
        .collect();
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
            app.organize_dialog(ctx)
        },
    );
}
fn action_key(key: egui::Key, modifiers: egui::Modifiers, repeat: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat,
        modifiers,
    }
}
fn action_focus(app: &mut PhotoSortApp, ctx: &egui::Context, width: f32, folders: bool) {
    app.organize_editor.as_mut().unwrap().compact_folders = folders;
    for _ in 0..3 {
        action_frame(app, ctx, width, vec![]);
    }
    let id = if folders {
        egui::Id::new(("test_folder_rect", "trip"))
    } else {
        egui::Id::new(("test_file_rect", 0_u64))
    };
    let pos = ctx
        .data(|d| d.get_temp::<egui::Rect>(id))
        .expect("visible fixture row")
        .center();
    action_frame(app, ctx, width, tree_controls::tests::click(pos));
    action_frame(app, ctx, width, vec![]);
}
#[test]
fn organize_action_mark_shortcuts_share_selection_and_preserve_draft() {
    for width in [640., 1440.] {
        for folders in [false, true] {
            let mut app = app();
            app.root = Some("/missing".into());
            app.index = app
                .media
                .iter()
                .enumerate()
                .map(|(i, m)| (m.id, i))
                .collect();
            let doc = PathBuf::from("/missing/trip/child/notes.txt");
            let doc_id = MediaId::for_path(&doc);
            let catalog = Arc::get_mut(app.catalog.as_mut().unwrap()).unwrap();
            catalog.inventory.push(doc);
            catalog
                .supporting_fingerprints
                .insert(doc_id, Fingerprint::default());
            app.open_organize_editor();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            action_focus(&mut app, &ctx, width, folders);
            let editor = app.organize_editor.as_mut().unwrap();
            editor.selected = HashSet::from([MediaId(0), doc_id]);
            editor.draft.create_folder("", "Pending").unwrap();
            editor.renaming = Some((MediaId(0), "unfinished".into()));
            editor.rename_focus = false;
            let labels = editor.draft.labels().clone();
            let rows = editor.rows.as_ptr();
            app.decisions.insert(
                MediaId(0),
                Decision {
                    rejected: true,
                    rating: Some(5),
                    turns: 1,
                },
            );
            let expected = if folders {
                vec![MediaId(0), MediaId(1), MediaId(2), doc_id]
            } else {
                vec![MediaId(0), doc_id]
            };
            for marked in [true, false] {
                action_frame(
                    &mut app,
                    &ctx,
                    width,
                    vec![action_key(egui::Key::D, egui::Modifiers::NONE, false)],
                );
                for id in &expected {
                    assert_eq!(
                        app.decision(*id).rejected,
                        marked,
                        "width={width} folders={folders} id={id:?}"
                    );
                }
            }
            let editor = app.organize_editor.as_ref().unwrap();
            assert_eq!(editor.draft.labels(), &labels);
            assert_eq!(editor.rows.as_ptr(), rows);
            assert!(editor.draft.folders().contains("Pending"));
            assert_eq!(editor.renaming, Some((MediaId(0), "unfinished".into())));
            assert_eq!(editor.selected, HashSet::from([MediaId(0), doc_id]));
            assert_eq!(app.decision(MediaId(0)).rating, Some(5));
            assert_eq!(app.decision(MediaId(0)).turns, 1);
            assert!(
                !app.busy
                    && app.plan.is_none()
                    && app.analysis_run.is_none()
                    && app.requested.is_empty()
            );
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
            app.runtime.shutdown();
        }
    }
}
#[test]
fn organize_action_order_fallback_and_repeat_guards_share_navigation() {
    let mut app = app();
    app.root = Some("/missing".into());
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.open_organize_editor();
    // action_focus clicks MediaId(0); ArrowDown moves to the next displayed row,
    // which follows folder and capture order rather than fixture insertion order.
    let rows: Vec<_> = app
        .organize_editor
        .as_ref()
        .unwrap()
        .rows
        .iter()
        .map(|r| r.id)
        .collect();
    let first = MediaId(0);
    let second = rows[rows.iter().position(|id| *id == first).unwrap() + 1];
    let ctx = egui::Context::default();
    theme_install(&ctx);
    action_focus(&mut app, &ctx, 640., false);
    app.organize_editor.as_mut().unwrap().selected.clear();
    let key = |key| action_key(key, egui::Modifiers::NONE, false);
    action_frame(
        &mut app,
        &ctx,
        640.,
        vec![key(egui::Key::ArrowDown), key(egui::Key::D)],
    );
    assert!(!app.decision(first).rejected && app.decision(second).rejected);
    action_frame(
        &mut app,
        &ctx,
        640.,
        vec![
            action_key(egui::Key::D, egui::Modifiers::ALT, false),
            key(egui::Key::D),
        ],
    );
    assert!(
        app.decision(second).rejected,
        "clear then mark lost event ordering"
    );
    action_frame(
        &mut app,
        &ctx,
        640.,
        vec![
            key(egui::Key::D),
            action_key(egui::Key::D, egui::Modifiers::ALT, false),
        ],
    );
    assert!(app.decisions.values().all(|d| !d.rejected));
    action_frame(
        &mut app,
        &ctx,
        640.,
        vec![key(egui::Key::D), key(egui::Key::D)],
    );
    assert!(app.decisions.values().all(|d| !d.rejected));
    ctx.input_mut(|input| {
        input.keys_down.insert(egui::Key::D);
    });
    action_frame(
        &mut app,
        &ctx,
        640.,
        vec![
            action_key(egui::Key::D, egui::Modifiers::NONE, true),
            action_key(egui::Key::D, egui::Modifiers::CTRL, false),
        ],
    );
    assert!(app.decisions.values().all(|d| !d.rejected));
    action_focus(&mut app, &ctx, 640., true);
    action_frame(
        &mut app,
        &ctx,
        640.,
        vec![key(egui::Key::ArrowUp), key(egui::Key::D)],
    );
    assert!(app.decisions.values().all(|d| d.rejected));
    assert_eq!(app.organize_editor.as_ref().unwrap().folder, "");
    assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    app.runtime.shutdown();
}
#[test]
fn organize_action_create_and_rename_folder_use_the_existing_draft_commands() {
    for width in [640., 1440.] {
        for folders in [false, true] {
            let mut app = app();
            app.open_organize_editor();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            action_focus(&mut app, &ctx, width, folders);
            let selected = app.organize_editor.as_ref().unwrap().selected.clone();
            action_frame(
                &mut app,
                &ctx,
                width,
                vec![action_key(
                    egui::Key::N,
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    false,
                )],
            );
            action_frame(&mut app, &ctx, width, vec![]);
            assert!(app.organize_editor.as_ref().unwrap().compact_folders);
            assert!(ctx.egui_wants_keyboard_input());
            action_frame(
                &mut app,
                &ctx,
                width,
                vec![egui::Event::Text("Child".into())],
            );
            action_frame(
                &mut app,
                &ctx,
                width,
                vec![action_key(egui::Key::Enter, egui::Modifiers::NONE, false)],
            );
            assert!(
                app.organize_editor
                    .as_ref()
                    .unwrap()
                    .draft
                    .folders()
                    .contains("trip/Child")
            );
            action_focus(&mut app, &ctx, width, true);
            action_frame(
                &mut app,
                &ctx,
                width,
                vec![action_key(egui::Key::F2, egui::Modifiers::NONE, false)],
            );
            action_frame(&mut app, &ctx, width, vec![]);
            action_frame(
                &mut app,
                &ctx,
                width,
                vec![egui::Event::Text("Journey".into())],
            );
            action_frame(
                &mut app,
                &ctx,
                width,
                vec![action_key(egui::Key::Enter, egui::Modifiers::NONE, false)],
            );
            let editor = app.organize_editor.as_ref().unwrap();
            assert!(editor.draft.folders().contains("Journey/Child"));
            // Selected files (with their companions) moved into the folder
            // created for them; the rest followed the renamed parent.
            let labels = editor.draft.labels();
            assert!(selected.iter().all(|id| labels[id] == "Journey/Child"));
            assert!(
                labels
                    .values()
                    .all(|l| l == "Journey" || l == "Journey/Child")
            );
            assert!(selected.is_empty() || labels.values().any(|l| l == "Journey"));
            assert_eq!(editor.selected, selected);
            assert!(editor.folder_edit.is_none());
            assert!(
                app.organize_drafts.is_empty() && app.file_names.is_empty() && app.plan.is_none()
            );
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
            app.runtime.shutdown();
        }
    }
}
#[test]
fn organize_action_text_popup_drag_and_modified_keys_keep_input_ownership() {
    for guard in 0..3 {
        let mut app = app();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.open_organize_editor();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        action_focus(&mut app, &ctx, 640., false);
        match guard {
            0 => {
                app.organize_editor
                    .as_mut()
                    .unwrap()
                    .start_rename(MediaId(0));
                action_frame(&mut app, &ctx, 640., vec![]);
            }
            1 => egui::Popup::open_id(&ctx, egui::Id::new("action-guard-popup")),
            _ => egui::DragAndDrop::set_payload(&ctx, 42_u8),
        }
        action_frame(
            &mut app,
            &ctx,
            640.,
            vec![
                action_key(egui::Key::D, egui::Modifiers::NONE, false),
                action_key(egui::Key::Delete, egui::Modifiers::NONE, false),
                action_key(
                    egui::Key::N,
                    egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
                    false,
                ),
            ],
        );
        assert!(
            app.decisions.is_empty() && !app.busy && app.plan.is_none(),
            "guard={guard}"
        );
        assert!(app.organize_editor.as_ref().unwrap().folder_edit.is_none());
        assert!(app.organize_drafts.is_empty());
        app.runtime.shutdown();
    }
    for (key, modifiers) in [
        (egui::Key::D, egui::Modifiers::SHIFT),
        (egui::Key::Delete, egui::Modifiers::COMMAND),
        (egui::Key::F2, egui::Modifiers::ALT),
        (egui::Key::N, egui::Modifiers::COMMAND),
        (
            egui::Key::Enter,
            egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
        ),
    ] {
        assert!(EditAction::from_key(key, modifiers).is_none());
    }
}
fn action_settle(app: &mut PhotoSortApp, ctx: &egui::Context) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.busy {
        app.poll(ctx);
        assert!(Instant::now() < deadline, "{:?}", app.error);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
}
fn real_action_focus(
    app: &mut PhotoSortApp,
    ctx: &egui::Context,
    width: f32,
    folders: bool,
) -> MediaId {
    let id = app
        .media
        .iter()
        .find(|m| m.path.file_name().unwrap() == "photo-0.png")
        .unwrap()
        .id;
    app.organize_editor.as_mut().unwrap().compact_folders = folders;
    for _ in 0..3 {
        action_frame(app, ctx, width, vec![]);
    }
    let key = if folders {
        egui::Id::new(("test_folder_rect", "A"))
    } else {
        egui::Id::new(("test_file_rect", id.0))
    };
    let pos = ctx
        .data(|d| d.get_temp::<egui::Rect>(key))
        .unwrap()
        .center();
    action_frame(app, ctx, width, tree_controls::tests::click(pos));
    action_frame(app, ctx, width, vec![]);
    id
}
#[test]
fn organize_action_delete_reviews_only_selected_sources_and_keeps_pending_edits() {
    for (width, folders, key) in [
        (640., false, egui::Key::Delete),
        (1440., true, egui::Key::Backspace),
    ] {
        let (_temp, mut app) = super::super::compact_cull_tests::fixture();
        app.step = Step::Organize;
        app.output = app.root.as_ref().unwrap().display().to_string();
        app.companions = true;
        app.open_organize_editor();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let id = real_action_focus(&mut app, &ctx, width, folders);
        let source = app.media[app.index[&id]].path.clone();
        let original = std::fs::read(&source).unwrap();
        let unrelated = app
            .media
            .iter()
            .find(|m| m.path.file_name().unwrap() == "photo-8.png")
            .unwrap()
            .id;
        app.stage_manual([unrelated], crate::core::decisions::Change::Trash(true));
        let editor = app.organize_editor.as_mut().unwrap();
        editor.draft.create_folder("", "Pending").unwrap();
        editor.renaming = Some((id, "pending-name".into()));
        editor.rename_focus = false;
        action_frame(
            &mut app,
            &ctx,
            width,
            vec![action_key(key, egui::Modifiers::NONE, false)],
        );
        assert!(app.organize_editor.is_none());
        action_settle(&mut app, &ctx);
        assert!(app.plan_open);
        let plan = app.plan.as_ref().unwrap();
        let mut paths = Vec::new();
        for action in &plan.actions {
            if let operations::Action::Trash { files } = action {
                paths.extend(files.iter().map(|s| s.path.clone()));
            } else {
                panic!("Unexpected non-Trash action: {}", action.label());
            }
        }
        assert_eq!(paths.len(), if folders { 8 } else { 2 });
        assert!(
            paths.contains(&source) && paths.contains(&source.with_extension("xmp")),
            "folders={folders} source={source:?} paths={paths:?}"
        );
        assert!(!paths.contains(&app.media[app.index[&unrelated]].path));
        assert!(app.decision(unrelated).rejected);
        assert_eq!(
            app.file_names.get(&id).map(String::as_str),
            Some("pending-name")
        );
        assert!(
            app.organize_drafts[&GroupMode::Current]
                .folders()
                .contains("Pending")
        );
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert!(!app.root.as_ref().unwrap().join("Pending").exists());
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        app.runtime.shutdown();
    }
}
#[test]
fn organize_action_preview_shortcuts_accept_names_then_review_without_mutation() {
    for modifiers in [
        egui::Modifiers::SHIFT,
        egui::Modifiers::COMMAND,
        egui::Modifiers::CTRL,
    ] {
        let (_temp, mut app) = super::super::compact_cull_tests::fixture();
        app.step = Step::Organize;
        app.output = app.root.as_ref().unwrap().display().to_string();
        app.open_organize_editor();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let id = real_action_focus(&mut app, &ctx, 640., false);
        let source = app.media[app.index[&id]].path.clone();
        let original = std::fs::read(&source).unwrap();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.draft.create_folder("", "Pending").unwrap();
        editor.renaming = Some((id, "renamed".into()));
        editor.rename_focus = false;
        action_frame(
            &mut app,
            &ctx,
            640.,
            vec![action_key(egui::Key::Enter, modifiers, false)],
        );
        assert!(app.organize_editor.is_none());
        action_settle(&mut app, &ctx);
        assert!(app.plan_open && !app.plan.as_ref().unwrap().actions.is_empty());
        assert!(app.plan.as_ref().unwrap().actions.iter().all(|a| !matches!(
            a,
            operations::Action::Trash { .. } | operations::Action::TrashDirectory(_)
        )));
        assert_eq!(app.file_names.get(&id).map(String::as_str), Some("renamed"));
        assert_eq!(std::fs::read(&source).unwrap(), original);
        assert!(!app.root.as_ref().unwrap().join("Pending").exists());
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        app.runtime.shutdown();
    }
}

#[test]
fn f2_and_new_folder_keys_open_the_editor_from_the_organize_grid() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["a.jpg", "b.jpg"] {
        std::fs::write(temp.path().join(name), b"unchanged").unwrap();
    }
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.output = catalog.root.display().to_string();
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.catalog = Some(Arc::new(catalog));
    app.step = Step::Organize;
    let b = app.media.iter().find(|m| m.name() == "b.jpg").unwrap().id;
    app.active = Some(b);
    // Other steps and plain keys are not taken over.
    assert!(!app.organize_shortcut(egui::Key::F3, egui::Modifiers::NONE));
    assert!(app.organize_editor.is_none());
    assert!(app.organize_shortcut(egui::Key::F2, egui::Modifiers::NONE));
    let editor = app.organize_editor.as_ref().unwrap();
    assert_eq!(editor.file_cursor.map(|i| editor.rows[i].id), Some(b));
    assert!(
        editor.renaming.is_some(),
        "F2 starts renaming the active photo"
    );
    app.organize_editor = None;
    let new_folder = egui::Modifiers {
        command: true,
        shift: true,
        ..Default::default()
    };
    assert!(app.organize_shortcut(egui::Key::N, new_folder));
    assert!(app.organize_editor.as_ref().unwrap().folder_name_focus);
    // Through the real dispatch: pressed-key filtering and handler order.
    app.organize_editor = None;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let _ = ctx.run_test(
        egui::RawInput {
            events: vec![egui::Event::Key {
                key: egui::Key::F2,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
            ..Default::default()
        },
        |ui| {
            let ctx = &ui.ctx().clone();
            app.keyboard(ctx)
        },
    );
    assert!(
        app.organize_editor
            .as_ref()
            .is_some_and(|e| e.renaming.is_some()),
        "F2 from the grid reaches the rename shortcut"
    );
    app.runtime.shutdown();
}

#[test]
fn create_folder_moves_the_selected_files_into_it() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        std::fs::write(temp.path().join(name), b"unchanged").unwrap();
    }
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.output = catalog.root.display().to_string();
    app.media = catalog.media.clone();
    app.catalog = Some(Arc::new(catalog));
    app.open_organize_editor();
    let mut editor = app.organize_editor.take().unwrap();
    let id = |name: &str| app.media.iter().find(|m| m.name() == name).unwrap().id;
    editor.selected = [id("a.jpg"), id("c.jpg")].into();
    editor.name = "Keepers".into();
    editor.apply(Command::Create, &app.media, false);
    assert!(editor.error.is_none(), "{:?}", editor.error);
    assert_eq!(editor.folder, "Keepers");
    let labels = editor.draft.labels();
    assert_eq!(labels[&id("a.jpg")], "Keepers");
    assert_eq!(labels[&id("c.jpg")], "Keepers");
    assert_ne!(
        labels.get(&id("b.jpg")).map(String::as_str),
        Some("Keepers")
    );
    // With nothing selected, the folder is created empty.
    editor.selected.clear();
    editor.folder.clear();
    editor.name = "Later".into();
    editor.apply(Command::Create, &app.media, false);
    assert!(editor.draft.folders().contains("Later"));
    assert!(!editor.draft.labels().values().any(|l| l == "Later"));
    app.runtime.shutdown();
}

#[test]
fn hovering_a_destination_folder_previews_only_its_photos() {
    let temp = tempfile::tempdir().unwrap();
    for (folder, names) in [("Trip", ["a.png", "b.png"]), ("Home", ["c.png", "d.png"])] {
        std::fs::create_dir_all(temp.path().join(folder)).unwrap();
        for name in names {
            image::RgbaImage::new(8, 8)
                .save(temp.path().join(folder).join(name))
                .unwrap();
        }
    }
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.output = catalog.root.display().to_string();
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.catalog = Some(Arc::new(catalog));
    app.step = Step::Organize;
    app.open_organize_editor();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut time = 0.0;
    let mut frame = |app: &mut PhotoSortApp, events: Vec<egui::Event>, dt: f64| {
        time += dt;
        ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1440.0, 900.0),
                )),
                time: Some(time),
                events,
                ..Default::default()
            },
            |ui| app.organize_dialog(ui.ctx()),
        )
    };
    for _ in 0..3 {
        frame(&mut app, vec![], 0.05);
    }
    let row = ctx
        .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", "Trip"))))
        .expect("Trip row");
    frame(
        &mut app,
        vec![egui::Event::PointerMoved(row.center())],
        0.05,
    );
    // Not yet: a passing pointer must not load thumbnails.
    let early = frame(&mut app, vec![], 0.1);
    assert!(app.requested.is_empty());
    assert!(!labels(&early).iter().any(|l| l.starts_with("Trip ·")));
    frame(&mut app, vec![], 0.5);
    let shown = frame(&mut app, vec![], 0.05);
    assert!(
        labels(&shown).iter().any(|l| l == "Trip · 2 files"),
        "{:?}",
        labels(&shown)
    );
    let trip: HashSet<MediaId> = app
        .media
        .iter()
        .filter(|m| m.path.parent().unwrap().ends_with("Trip"))
        .map(|m| m.id)
        .collect();
    assert!(!app.requested.is_empty());
    assert!(
        app.requested
            .iter()
            .all(|(id, preview)| trip.contains(id) && !preview)
    );
    app.runtime.shutdown();
}

fn labels(output: &egui::FullOutput) -> Vec<String> {
    output
        .platform_output
        .accesskit_update
        .as_ref()
        .unwrap()
        .nodes
        .iter()
        .filter_map(|(_, n)| n.value().or(n.label()).map(str::to_owned))
        .collect()
}
