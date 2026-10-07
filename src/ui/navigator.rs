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
    pub(super) fn navigator_panel(&mut self, ui: &mut egui::Ui) {
        if self.step != Step::Cull || !self.navigator.enabled || self.review {
            return;
        }
        if ui.available_width() >= 780. {
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
            egui::Popup::from_toggle_button_response(&response)
                .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                .width(width)
                .show(|ui| {
                    ui.set_width(width);
                    self.navigator_contents(ui, height);
                });
            ui.add_space(4.);
        }
    }
    fn navigator_contents(&mut self, ui: &mut egui::Ui, height: f32) {
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
        let selected = self.navigator_key();
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
        ui.add_enabled_ui(!waiting, |ui| {
            let row_height = button_row_height(ui);
            egui::ScrollArea::vertical()
                .id_salt(("library_tree", self.navigator.mode))
                .max_height((height - 78.).max(24.))
                .auto_shrink([false, false])
                .show_rows(ui, row_height, view.visible.len(), |ui, range| {
                    for row in range {
                        let i = view.visible[row];
                        let node = &view.tree.nodes[i];
                        ui.horizontal(|ui| {
                            ui.add_space((node.depth.min(12) * 12) as f32);
                            if node.expandable(i) {
                                let collapsed = self.navigator.collapsed.contains(&node.key);
                                if ui
                                    .add_sized(
                                        [18., 22.],
                                        egui::Button::new(if collapsed { ">" } else { "⌄" })
                                            .frame(false),
                                    )
                                    .on_hover_text(if collapsed { "Expand" } else { "Collapse" })
                                    .clicked()
                                {
                                    if collapsed {
                                        self.navigator.collapsed.remove(&node.key);
                                    } else {
                                        self.navigator.collapsed.insert(node.key.clone());
                                    }
                                    self.navigator_changed();
                                }
                            } else {
                                ui.add_space(22.);
                            }
                            let label = format!("{}  ·  {}", node.label, node.count);
                            let response = ui.add_sized(
                                [ui.available_width(), row_height],
                                egui::Button::selectable(
                                    selected.as_ref() == Some(&node.key),
                                    label,
                                )
                                .truncate(),
                            );
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
