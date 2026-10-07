//! Shared controls for cached actual and proposed folder trees.
use super::*;
use crate::core::browsing::tree::{Expansion, Key, Navigation, NavigationResult, Tree};
use std::collections::BTreeSet;

pub(super) fn expander(
    ui: &mut egui::Ui,
    tree: &Tree,
    index: usize,
    collapsed: &mut BTreeSet<Key>,
    keyboard: &Keyboard,
) -> bool {
    let node = &tree.nodes[index];
    if !node.expandable(index) {
        ui.add_space(22.);
        return false;
    }
    let closed = collapsed.contains(&node.key);
    let response = ui
        .add_sized(
            [18., button_row_height(ui)],
            egui::Button::new(if closed { ">" } else { "⌄" })
                .frame(false)
                .sense(pointer_sense(false)),
        )
        .on_hover_text(if closed { "Expand" } else { "Collapse" });
    #[cfg(test)]
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            egui::Id::new(("test_tree_expander", &node.key)),
            response.rect,
        )
    });
    keyboard.focus_on_click(&response);
    response.clicked() && tree.set_expanded(&node.key, Expansion::Node, closed, collapsed)
}
pub(super) fn menu(
    ui: &mut egui::Ui,
    tree: &Tree,
    index: usize,
    collapsed: &mut BTreeSet<Key>,
) -> bool {
    let node = &tree.nodes[index];
    if !node.expandable(index) {
        return false;
    }
    let mut changed = false;
    for (label, scope, expanded) in [
        ("Expand subtree", Expansion::Subtree, true),
        ("Collapse subtree", Expansion::Subtree, false),
        ("Expand all children", Expansion::Children, true),
        ("Collapse all children", Expansion::Children, false),
    ] {
        let enabled = !matches!(scope, Expansion::Children)
            || (index + 1..node.end).any(|i| tree.nodes[i].expandable(i));
        if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
            changed |= tree.set_expanded(&node.key, scope, expanded, collapsed);
            ui.close();
        }
    }
    ui.separator();
    changed
}

/// Tree policy layered on the same focus/scroll owner as file lists.
pub(super) struct Keyboard {
    focus: collection_controls::Keyboard,
    id: egui::Id,
}
pub(super) use collection_controls::pointer_sense;
impl Keyboard {
    pub(super) fn new(
        ui: &egui::Ui,
        salt: impl std::hash::Hash,
        height: f32,
        row_height: f32,
    ) -> Self {
        let focus = collection_controls::Keyboard::new(ui, salt, height, row_height);
        let id = focus.id;
        Self { focus, id }
    }
    fn reveal_id(&self) -> egui::Id {
        self.id.with("tree_reveal")
    }
    pub(super) fn navigate(
        &self,
        ui: &egui::Ui,
        tree: &Tree,
        current: Option<&Key>,
        collapsed: &mut BTreeSet<Key>,
    ) -> Option<NavigationResult> {
        let mut selected = current.cloned();
        let mut changed = false;
        let mut moved = false;
        for (command, _) in self.navigation_events(ui, false) {
            if let Some(result) =
                self.navigate_once(ui, tree, selected.as_ref(), command, collapsed)
            {
                selected = Some(result.key);
                changed |= result.expansion_changed;
                moved = true;
            }
        }
        moved.then(|| NavigationResult {
            key: selected.unwrap(),
            expansion_changed: changed,
        })
    }
    pub(super) fn navigation_events(
        &self,
        ui: &egui::Ui,
        ranges: bool,
    ) -> Vec<(Navigation, egui::Modifiers)> {
        let page = self.focus.page_rows(ui) as isize;
        let command = |key| match key {
            egui::Key::ArrowUp => Some(Navigation::Advance(-1)),
            egui::Key::ArrowDown => Some(Navigation::Advance(1)),
            egui::Key::ArrowLeft => Some(Navigation::Parent),
            egui::Key::ArrowRight => Some(Navigation::Child),
            egui::Key::Home => Some(Navigation::First),
            egui::Key::End => Some(Navigation::Last),
            egui::Key::PageUp => Some(Navigation::Advance(-page)),
            egui::Key::PageDown => Some(Navigation::Advance(page)),
            _ => None,
        };
        self.focus
            .events(ui, |k, m| {
                command(k).is_some()
                    && (m.is_none()
                        || (ranges
                            && m.shift
                            && !m.alt
                            && !matches!(k, egui::Key::ArrowLeft | egui::Key::ArrowRight)))
            })
            .into_iter()
            .map(|(key, modifiers, _)| (command(key).unwrap(), modifiers))
            .collect()
    }
    pub(super) fn navigate_once(
        &self,
        ui: &egui::Ui,
        tree: &Tree,
        current: Option<&Key>,
        command: Navigation,
        collapsed: &mut BTreeSet<Key>,
    ) -> Option<NavigationResult> {
        let result = tree.navigate(current, command, collapsed)?;
        ui.ctx()
            .data_mut(|d| d.insert_temp(self.reveal_id(), result.key.clone()));
        Some(result)
    }
    pub(super) fn scroll(
        &self,
        ui: &egui::Ui,
        scroll: egui::ScrollArea,
        tree: &Tree,
        visible: &[usize],
        projection_ready: bool,
    ) -> egui::ScrollArea {
        let key = ui.ctx().data(|d| d.get_temp::<Key>(self.reveal_id()));
        let Some(row) = key
            .as_ref()
            .and_then(|k| tree.lookup.get(k))
            .and_then(|i| visible.binary_search(i).ok())
        else {
            return scroll;
        };
        // Retain the identity until Cull publishes its new projection.
        if projection_ready {
            ui.ctx().data_mut(|d| d.remove::<Key>(self.reveal_id()));
        }
        self.focus.scroll(ui, scroll, row)
    }
    pub(super) fn focus_on_click(&self, response: &egui::Response) {
        self.focus.focus_on_click(response);
        if response.clicked() {
            response.ctx.data_mut(|d| d.remove::<Key>(self.reveal_id()));
        }
    }
    pub(super) fn finish(&self, ui: &egui::Ui, rect: egui::Rect) {
        self.focus.finish(ui, rect, "Navigation tree. Arrow keys navigate and expand; Home, End and Page keys move through the tree.");
        #[cfg(test)]
        ui.ctx()
            .data_mut(|d| d.insert_temp(egui::Id::new("test_tree_keyboard"), self.id));
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;
    pub(in crate::ui) fn key(key: egui::Key) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }
    }
    pub(in crate::ui) fn click(pos: egui::Pos2) -> Vec<egui::Event> {
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
        ]
    }
    #[test]
    fn tree_keyboard_focus_text_menu_and_page_keys_share_one_container() {
        let tree = Tree::folders(
            (0..100).map(|i| (format!("{i:03}").into(), i.to_string(), 0)),
            || false,
        )
        .unwrap();
        let ctx = egui::Context::default();
        let mut selected = Some(tree.nodes[0].key.clone());
        let mut collapsed = BTreeSet::new();
        let mut text = String::from("sample name");
        let mut text_rect = egui::Rect::NOTHING;
        let mut tree_rect = egui::Rect::NOTHING;
        let mut focus = None;
        let frame = |events,
                     selected: &mut Option<Key>,
                     text: &mut String,
                     text_rect: &mut egui::Rect,
                     tree_rect: &mut egui::Rect,
                     focus: &mut Option<egui::Id>,
                     collapsed: &mut BTreeSet<Key>| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(450., 400.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let keyboard = Keyboard::new(ui, "rows", 200., 24.);
                        *focus = Some(keyboard.id);
                        if let Some(result) =
                            keyboard.navigate(ui, &tree, selected.as_ref(), collapsed)
                        {
                            *selected = Some(result.key);
                        }
                        let visible = tree.visible(collapsed, || false).unwrap();
                        let output = keyboard
                            .scroll(
                                ui,
                                egui::ScrollArea::vertical().id_salt("rows"),
                                &tree,
                                &visible,
                                true,
                            )
                            .max_height(200.)
                            .show_rows(ui, 24., visible.len(), |ui, range| {
                                for row in range {
                                    let response = ui.add_sized(
                                        [300., 24.],
                                        egui::Button::new(&tree.nodes[visible[row]].label)
                                            .sense(pointer_sense(false)),
                                    );
                                    keyboard.focus_on_click(&response);
                                }
                            });
                        *tree_rect = output.inner_rect;
                        keyboard.finish(ui, output.inner_rect);
                        *text_rect = ui.text_edit_singleline(text).rect;
                    });
                },
            );
        };
        macro_rules! run {
            ($events:expr) => {
                frame(
                    $events,
                    &mut selected,
                    &mut text,
                    &mut text_rect,
                    &mut tree_rect,
                    &mut focus,
                    &mut collapsed,
                )
            };
        }
        run!(vec![]);
        run!(vec![]);
        let pos = tree_rect.min + egui::vec2(25., 12.);
        run!(click(pos));
        run!(vec![key(egui::Key::PageDown)]);
        assert_ne!(selected, Some(tree.nodes[0].key.clone()));
        assert!(ctx.memory(|m| m.has_focus(focus.unwrap())));
        run!(vec![key(egui::Key::PageUp)]);
        assert_eq!(selected, Some(tree.nodes[0].key.clone()));
        let mut modified = key(egui::Key::End);
        if let egui::Event::Key { modifiers, .. } = &mut modified {
            modifiers.shift = true;
        }
        run!(vec![modified]);
        assert_eq!(selected, Some(tree.nodes[0].key.clone()));
        run!(vec![key(egui::Key::ArrowDown), key(egui::Key::ArrowDown)]);
        assert_eq!(selected, Some(tree.nodes[2].key.clone()));
        run!(vec![key(egui::Key::Home)]);
        let popup = egui::Id::new("context");
        egui::Popup::open_id(&ctx, popup);
        run!(vec![key(egui::Key::End)]);
        assert_eq!(selected, Some(tree.nodes[0].key.clone()));
        egui::Popup::close_all(&ctx);
        // Same-frame outside click must let the text field own Home/End.
        let mut events = click(text_rect.center());
        events.push(key(egui::Key::End));
        run!(events);
        assert_eq!(selected, Some(tree.nodes[0].key.clone()));
        run!(vec![key(egui::Key::Home), egui::Event::Text("X".into())]);
        assert_eq!(text, "Xsample name");
        assert_eq!(selected, Some(tree.nodes[0].key.clone()));
        // Reclaim tree focus and verify Tab is not trapped.
        let pos = tree_rect.min + egui::vec2(25., 12.);
        run!(click(pos));
        run!(vec![key(egui::Key::Tab)]);
        assert!(!ctx.memory(|m| m.has_focus(focus.unwrap())));
    }
}
