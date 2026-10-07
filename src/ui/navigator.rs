//! Presentation only: tree construction/projection belongs to the browse worker.
use super::*;
use crate::core::browsing::tree::{Key, Mode};
impl PhotoSortApp {
    pub(super) fn set_navigator(&mut self, mode: Mode, enabled: bool) {
        self.navigator.mode = mode;
        self.navigator.enabled = enabled;
        self.navigator_changed();
    }
    fn navigator_changed(&mut self) {
        self.pending_bookmark = None;
        self.filter_dirty = true;
        self.library_reveal = true;
    }
    fn navigator_key(&self) -> Option<Key> {
        match self.navigator.mode {
            Mode::Folders => self.browse.folder.path.clone().map(Key::Folder),
            Mode::Dates => self.browse.date.clone().map(Key::Date),
            Mode::Groups => self.browse.group.map(Key::Group),
        }
    }
    fn select_navigation_node(&mut self, key: Key) {
        match key {
            Key::Folder(path) => self.browse.folder.path = Some(path),
            Key::Date(date) => self.browse.date = Some(date),
            Key::Group(group) => self.browse.group = Some(group),
        }
        self.navigator_changed();
    }
    pub(super) fn navigator_controls(&mut self, ui: &mut egui::Ui) {
        if ui
            .checkbox(&mut self.navigator.enabled, "Library navigator")
            .on_hover_text(
                "Browse folders, capture dates and similarity groups without moving files.",
            )
            .changed()
        {
            self.set_navigator(self.navigator.mode, self.navigator.enabled);
        }
        if let Some(date) = self.browse.date.clone() {
            let label = if date.is_empty() {
                "Unknown date"
            } else {
                &date
            };
            if ui.button(format!("Date: {label} ×")).clicked() {
                self.browse.date = None;
                self.navigator_changed();
            }
        }
    }
    fn navigator_popup_id(&self) -> egui::Id {
        egui::Id::new(("navigator_popup_open", &self.root))
    }
    pub(super) fn navigator_popup_open(&self, ctx: &egui::Context) -> bool {
        ctx.data(|data| data.get_temp::<(bool, u64)>(self.navigator_popup_id()))
            .is_some_and(|(open, frame)| {
                open && frame.saturating_add(1) >= ctx.cumulative_frame_nr()
            })
    }
    pub(super) fn navigator_panel(&mut self, ui: &mut egui::Ui) {
        let popup_state = self.navigator_popup_id();
        if self.step != Step::Cull || !self.navigator.enabled || self.review {
            ui.ctx()
                .data_mut(|data| data.remove::<(bool, u64)>(popup_state));
            return;
        }
        if ui.available_width() >= 780. {
            ui.ctx()
                .data_mut(|data| data.remove::<(bool, u64)>(popup_state));
            egui::SidePanel::left("library_tree_panel")
                .resizable(true)
                .default_width(220.)
                .width_range(180.0..=300.0)
                .show_inside(ui, |ui| self.navigator_contents(ui, ui.available_height()));
        } else {
            // A stacked tree can consume the entire photo viewport in a short
            // window. Reuse the same virtualized navigator in a bounded overlay.
            let selected = self.navigator_key().and_then(|key| {
                let view = self.browse_tree.as_ref()?;
                let index = view.tree.lookup.get(&key)?;
                Some(view.tree.nodes[*index].label.clone())
            });
            let label = selected.map_or_else(
                || format!("Browse {}", self.navigator.mode.title().to_lowercase()),
                |label| format!("{}: {label}", self.navigator.mode.title()),
            );
            let width = ui.available_width().clamp(240., 420.);
            let height = (ui.ctx().content_rect().height() - 180.).clamp(160., 420.);
            let response = ui
                .add(egui::Button::new(label).truncate())
                .on_hover_text("Browse folders, dates and groups. Counts cover the whole library.");
            #[cfg(test)]
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("test_navigator_toggle"), response.rect));
            // Context menus use egui's single memory-owned popup slot. Keep
            // this enclosing browser independently open so they cannot evict it.
            let mut open = self.navigator_popup_open(ui.ctx());
            if response.clicked() {
                open = !open;
            }
            egui::Popup::from_response(&response)
                .open_bool(&mut open)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(width)
                .show(|ui| {
                    ui.set_width(width);
                    self.navigator_contents(ui, height);
                });
            let frame = ui.ctx().cumulative_frame_nr();
            ui.ctx()
                .data_mut(|data| data.insert_temp(popup_state, (open, frame)));
            ui.add_space(4.);
        }
    }
    fn navigator_contents(&mut self, ui: &mut egui::Ui, height: f32) {
        // Only one presentation is rendered: inline panel or compact popup.
        ui.scope_builder(
            egui::UiBuilder::new().id(("library_navigator", &self.root)),
            |ui| self.navigator_body(ui, height),
        );
    }
    fn navigator_body(&mut self, ui: &mut egui::Ui, height: f32) {
        ui.horizontal(|ui| {
            for mode in Mode::ALL {
                if ui
                    .selectable_label(self.navigator.mode == mode, mode.title())
                    .clicked()
                    && self.navigator.mode != mode
                {
                    self.set_navigator(mode, true);
                }
            }
        });
        ui.label(
            RichText::new("Counts cover the whole library")
                .small()
                .color(MUTED),
        );
        let mut selected = self.navigator_key();
        let Some(view) = self
            .browse_tree
            .clone()
            .filter(|v| v.mode == self.navigator.mode)
        else {
            ui.spinner();
            return;
        };
        let waiting = self.browse_run.is_some() || self.filter_dirty;
        ui.horizontal(|ui| {
            if ui
                .selectable_label(
                    selected.is_none(),
                    format!("All {}", self.navigator.mode.title().to_lowercase()),
                )
                .clicked()
            {
                match self.navigator.mode {
                    Mode::Folders => self.browse.folder.path = None,
                    Mode::Dates => self.browse.date = None,
                    Mode::Groups => self.browse.group = None,
                }
                self.navigator_changed();
            }
            let node = selected
                .as_ref()
                .and_then(|key| view.tree.lookup.get(key))
                .map(|i| &view.tree.nodes[*i]);
            for (name, index) in [
                ("Previous", node.and_then(|n| n.previous)),
                ("Next", node.and_then(|n| n.next)),
            ] {
                if ui
                    .add_enabled(!waiting && index.is_some(), egui::Button::new(name).small())
                    .on_hover_text("Select the adjacent folder, date or group at the same level")
                    .clicked()
                {
                    self.select_navigation_node(view.tree.nodes[index.unwrap()].key.clone());
                }
            }
        });
        if view.tree.nodes.is_empty() {
            ui.label(if self.navigator.mode == Mode::Groups {
                "Analyze the library to find similarity groups."
            } else {
                "No entries in this library."
            });
            return;
        }
        // Navigation only changes the query against this cached tree. Keep it
        // responsive while the worker replaces an older projection/filter result.
        ui.scope(|ui| {
            let row_height = button_row_height(ui);
            let max_height = (height - 78.).max(24.);
            let salt = ("library_tree", self.navigator.mode);
            let keyboard = tree_controls::Keyboard::new(ui, salt, max_height, row_height);
            if let Some(result) = keyboard.navigate(
                ui,
                &view.tree,
                selected.as_ref(),
                &mut self.navigator.collapsed,
            ) {
                if selected.as_ref() != Some(&result.key) {
                    self.select_navigation_node(result.key.clone());
                    selected = Some(result.key);
                } else if result.expansion_changed {
                    self.navigator_changed();
                }
            }
            let scroll = keyboard.scroll(
                ui,
                egui::ScrollArea::vertical().id_salt(salt),
                &view.tree,
                &view.visible,
                !self.filter_dirty && self.browse_run.is_none(),
            );
            let output = scroll
                .max_height(max_height)
                .auto_shrink([false, false])
                .show_rows(ui, row_height, view.visible.len(), |ui, range| {
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| {
                        d.insert_temp(egui::Id::new("test_navigator_first_row"), range.start)
                    });
                    for row in range {
                        let i = view.visible[row];
                        let node = &view.tree.nodes[i];
                        ui.horizontal(|ui| {
                            ui.add_space((node.depth.min(12) * 12) as f32);
                            if tree_controls::expander(
                                ui,
                                &view.tree,
                                i,
                                &mut self.navigator.collapsed,
                                &keyboard,
                            ) {
                                self.navigator_changed();
                            }
                            let label = format!("{}  ·  {}", node.label, node.count);
                            let response = ui.add_sized(
                                [ui.available_width(), row_height],
                                egui::Button::selectable(
                                    selected.as_ref() == Some(&node.key),
                                    label,
                                )
                                .truncate()
                                .sense(tree_controls::pointer_sense(false)),
                            );
                            keyboard.focus_on_click(&response);
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| {
                                d.insert_temp(
                                    egui::Id::new(("test_navigator_node", &node.key)),
                                    response.rect,
                                )
                            });
                            response.context_menu(|ui| {
                                if tree_controls::menu(
                                    ui,
                                    &view.tree,
                                    i,
                                    &mut self.navigator.collapsed,
                                ) {
                                    self.navigator_changed();
                                }
                            });
                            if response
                                .on_hover_text(match &node.key {
                                    Key::Folder(p) => p.display().to_string(),
                                    Key::Date(d) => {
                                        if d.is_empty() {
                                            "Unknown capture date".into()
                                        } else {
                                            d.clone()
                                        }
                                    }
                                    Key::Group(g) => format!("Similarity group {g}"),
                                })
                                .clicked()
                            {
                                self.select_navigation_node(node.key.clone());
                            }
                        });
                    }
                });
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(
                    egui::Id::new("test_navigator_scroll"),
                    (
                        output.id,
                        output.inner_rect,
                        output.state.offset.y,
                        row_height,
                    ),
                )
            });
            keyboard.finish(ui, output.inner_rect);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::browsing::{Query, Seed, tree};
    fn settle(app: &mut PhotoSortApp) {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            app.refilter_browse();
            if app.browse_run.is_none() {
                break;
            }
            assert!(Instant::now() < deadline);
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
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        app.keyboard(ctx);
                        egui::CentralPanel::default().show(ctx, |ui| app.navigator_panel(ui));
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
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        app.keyboard(ctx);
                        egui::CentralPanel::default().show(ctx, |ui| app.navigator_panel(ui));
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
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| app.navigator_panel(ui));
                },
            );
        };
        let scroll = || {
            ctx.data(|d| {
                d.get_temp::<(egui::Id, egui::Rect, f32, f32)>(egui::Id::new(
                    "test_navigator_scroll",
                ))
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
            let output = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 500.),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
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
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(result) = app.runtime.browsing.take_result() {
                assert_eq!(result.run, run);
                app.browse_tree = result.tree.map(Arc::new);
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(1));
        }
        app.filter_dirty = false;
        app.select_navigation_node(Key::Folder("/absent/empty".into()));
        assert!(app.filtered.is_empty());
        let ctx = egui::Context::default();
        let mut shown = false;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let before = ui.cursor();
                app.navigator_panel(ui);
                shown = ui.cursor() != before;
            });
        });
        assert!(shown);
        app.step = Step::Organize;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let before = ui.cursor();
                app.navigator_panel(ui);
                assert_eq!(ui.cursor(), before);
            });
        });
    }
}
