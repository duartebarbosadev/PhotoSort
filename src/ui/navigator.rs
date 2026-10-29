//! Presentation only: tree construction/projection belongs to the browse worker.
use super::*;
use crate::core::browsing::tree::{Key, Mode};
impl PhotoSortApp {
    pub(super) fn set_navigator(&mut self, mode: Mode, enabled: bool) {
        self.navigator.mode = mode;
        self.navigator.enabled = enabled;
        if enabled {
            self.review = false;
        }
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
            Key::Folder(path) => self.browse.folder.show_only(Some(path)),
            Key::Date(date) => self.browse.date = Some(date),
            Key::Group(group) => self.browse.group = Some(group),
        }
        self.navigator_changed();
    }
    /// A click on a folder in the navigator, by the Organize sidebar's rules:
    /// a plain click shows just that folder; Ctrl/Cmd adds it to the folders
    /// shown, or takes it away again; Shift shows the folders listed from the
    /// last one clicked through this one, added to those shown with Ctrl/Cmd.
    fn choose_navigator_folder(
        &mut self,
        view: &crate::core::browsing::tree::View,
        path: std::path::PathBuf,
        modifiers: egui::Modifiers,
    ) {
        let toggle = modifiers.command || modifiers.ctrl;
        let row = |target: &std::path::Path| {
            view.visible.iter().position(
                |i| matches!(&view.tree.nodes[*i].key, Key::Folder(folder) if folder == target),
            )
        };
        let span = self
            .browse
            .folder
            .path
            .as_deref()
            .filter(|_| modifiers.shift)
            .and_then(row)
            .zip(row(&path));
        let folder = &mut self.browse.folder;
        if let Some((from, to)) = span {
            let rows = view.visible[from.min(to)..=from.max(to)].iter();
            let span = rows.filter_map(|i| match &view.tree.nodes[*i].key {
                Key::Folder(folder) => Some(folder.clone()),
                _ => None,
            });
            folder.show_span(span.collect(), toggle);
        } else if toggle {
            folder.toggle(path);
        } else {
            folder.show_only(Some(path));
        }
        self.navigator_changed();
    }
    pub(super) fn navigator_controls(&mut self, ui: &mut egui::Ui) {
        // Review temporarily hides the tree but keeps its presentation bookmark.
        // Show actual visibility here so one click always opens the navigator.
        let mut visible = self.navigator.enabled && !self.review;
        if icon_text_button(
            ui,
            true,
            visible,
            egui_phosphor::regular::TREE_VIEW,
            "Library navigator",
        )
        .on_hover_text(
            "Browse folders, capture dates and similarity groups without moving files · F/S",
        )
        .clicked()
        {
            visible = !visible;
            self.set_navigator(self.navigator.mode, visible);
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
            egui::Panel::left("library_tree_panel")
                .resizable(true)
                .default_size(220.)
                .size_range(180.0..=300.0)
                .show(ui, |ui| self.navigator_contents(ui, ui.available_height()));
        } else {
            // A stacked tree can consume the entire photo viewport in a short
            // window. Reuse the same virtualized navigator in a bounded overlay.
            let selected = self.navigator_key().and_then(|key| {
                let view = self.browse_tree.as_ref()?;
                let index = view.tree.lookup.get(&key)?;
                Some(view.tree.nodes[*index].label.clone())
            });
            let several = self.browse.folder.folders().count();
            let label = match selected {
                _ if self.navigator.mode == Mode::Folders && several > 1 => {
                    format!("Folders: {several} folders")
                }
                Some(label) => format!("{}: {label}", self.navigator.mode.title()),
                None => format!("Browse {}", self.navigator.mode.title().to_lowercase()),
            };
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
            egui::UiBuilder::new().id(egui::Id::new(("library_navigator", &self.root))),
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
                    Mode::Folders => self.browse.folder.show_only(None),
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
                                    match &node.key {
                                        Key::Folder(path) => self.browse.folder.shows(path),
                                        key => selected.as_ref() == Some(key),
                                    },
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
                            if let Key::Group(group) = &node.key {
                                let group = *group;
                                if response
                                    .dnd_hover_payload::<super::grouping::PhotoDrag>()
                                    .is_some()
                                {
                                    ui.painter().rect_stroke(
                                        response.rect,
                                        R_CONTROL,
                                        Stroke::new(2.0, ACCENT),
                                        StrokeKind::Inside,
                                    );
                                }
                                if let Some(drag) =
                                    response.dnd_release_payload::<super::grouping::PhotoDrag>()
                                {
                                    self.drop_on_group(&drag, group);
                                }
                            }
                            response.context_menu(|ui| {
                                if tree_controls::menu(
                                    ui,
                                    &view.tree,
                                    i,
                                    &mut self.navigator.collapsed,
                                ) {
                                    self.navigator_changed();
                                }
                                if let Key::Group(group) = node.key {
                                    let count = self.group_members(group).len();
                                    ui.separator();
                                    if ui
                                        .add_enabled(
                                            (2..=9).contains(&count),
                                            egui::Button::new(format!("Compare these {count}")),
                                        )
                                        .on_disabled_hover_text("Comparison shows 2 to 9 photos")
                                        .clicked()
                                    {
                                        self.compare_group(group);
                                        ui.close();
                                    }
                                }
                            });
                            if response
                                .on_hover_text(match &node.key {
                                    Key::Folder(p) => format!(
                                        "{} · Ctrl/Cmd-click to show it with other folders",
                                        p.display()
                                    ),
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
                                let modifiers = ui.input(|i| i.modifiers);
                                match &node.key {
                                    Key::Folder(path)
                                        if modifiers.shift
                                            || modifiers.command
                                            || modifiers.ctrl =>
                                    {
                                        self.choose_navigator_folder(
                                            &view,
                                            path.clone(),
                                            modifiers,
                                        );
                                    }
                                    key => self.select_navigation_node(key.clone()),
                                }
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
#[path = "navigator/tests.rs"]
mod tests;
