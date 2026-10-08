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

/// Transient dwell state for navigating a cached tree during an owned drag.
/// Call once per rendered tree frame, including with no eligible target.
#[derive(Default)]
pub(super) struct HoverExpand {
    pending: Option<(Key, f64)>,
    frame: Option<u64>,
}
impl HoverExpand {
    pub(super) fn update(
        &mut self,
        ui: &egui::Ui,
        tree: &Tree,
        target: Option<usize>,
        collapsed: &mut BTreeSet<Key>,
    ) -> bool {
        let frame = ui.ctx().cumulative_frame_nr();
        if self
            .frame
            .is_some_and(|previous| previous.saturating_add(1) < frame)
        {
            self.pending = None;
        }
        self.frame = Some(frame);
        let target = target.filter(|&index| {
            ui.input(|i| i.raw.focused && i.pointer.primary_down())
                && tree
                    .nodes
                    .get(index)
                    .is_some_and(|node| node.expandable(index) && collapsed.contains(&node.key))
        });
        let Some(index) = target else {
            self.pending = None;
            return false;
        };
        let key = &tree.nodes[index].key;
        let now = ui.input(|i| i.time);
        let started = match &self.pending {
            Some((previous, started)) if previous == key && now >= *started => *started,
            _ => {
                self.pending = Some((key.clone(), now));
                now
            }
        };
        let remaining = 0.6 - (now - started);
        if remaining > 0.0 {
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(remaining));
            return false;
        }
        self.pending = None;
        let changed = tree.set_expanded(key, Expansion::Node, true, collapsed);
        if changed {
            ui.ctx().request_repaint();
        }
        changed
    }
}

/// Pure predicate lets editors preserve event order when mixing navigation and actions.
pub(super) fn navigation_key(key: egui::Key, modifiers: egui::Modifiers, ranges: bool) -> bool {
    matches!(
        key,
        egui::Key::ArrowUp
            | egui::Key::ArrowDown
            | egui::Key::ArrowLeft
            | egui::Key::ArrowRight
            | egui::Key::Home
            | egui::Key::End
            | egui::Key::PageUp
            | egui::Key::PageDown
    ) && (modifiers.is_none()
        || (ranges
            && modifiers.shift
            && !modifiers.alt
            && !matches!(key, egui::Key::ArrowLeft | egui::Key::ArrowRight)))
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
    /// Workflow shortcuts use the same focus and popup guards as navigation.
    pub(super) fn collection(&self) -> &collection_controls::Keyboard {
        &self.focus
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
        self.focus
            .events(ui, |key, modifiers| navigation_key(key, modifiers, ranges))
            .into_iter()
            .map(|(key, modifiers, _)| (self.navigation_command(ui, key).unwrap(), modifiers))
            .collect()
    }
    pub(super) fn navigation_command(&self, ui: &egui::Ui, key: egui::Key) -> Option<Navigation> {
        let page = self.focus.page_rows(ui) as isize;
        match key {
            egui::Key::ArrowUp => Some(Navigation::Advance(-1)),
            egui::Key::ArrowDown => Some(Navigation::Advance(1)),
            egui::Key::ArrowLeft => Some(Navigation::Parent),
            egui::Key::ArrowRight => Some(Navigation::Child),
            egui::Key::Home => Some(Navigation::First),
            egui::Key::End => Some(Navigation::Last),
            egui::Key::PageUp => Some(Navigation::Advance(-page)),
            egui::Key::PageDown => Some(Navigation::Advance(page)),
            _ => None,
        }
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
    fn drag_hover_expansion_requires_continuous_dwell_and_schedules_idle_repaint() {
        for interruption in 0..5 {
            let tree = Tree::folders(
                ["a", "a/child", "b", "b/child"]
                    .into_iter()
                    .map(|p| (p.into(), p.into(), 0)),
                || false,
            )
            .unwrap();
            let a = Key::Folder("a".into());
            let b = Key::Folder("b".into());
            let a_index = tree.lookup[&a];
            let b_index = tree.lookup[&b];
            let mut collapsed = BTreeSet::from([a.clone(), b.clone()]);
            let ctx = egui::Context::default();
            let mut hover = HoverExpand::default();
            let mut frame = |time, target, pressed: Option<bool>, focused, render| {
                let events = pressed
                    .into_iter()
                    .map(|pressed| egui::Event::PointerButton {
                        pos: egui::pos2(30., 30.),
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    })
                    .collect();
                let mut changed = false;
                let output = ctx.run(
                    egui::RawInput {
                        time: Some(time),
                        focused,
                        events,
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            if render {
                                changed = hover.update(ui, &tree, target, &mut collapsed);
                            }
                        });
                    },
                );
                (
                    changed,
                    output.viewport_output[&egui::ViewportId::ROOT].repaint_delay,
                )
            };
            assert!(!frame(0., Some(a_index), Some(true), true, true).0);
            frame(0.1, Some(a_index), None, true, true);
            frame(0.2, Some(a_index), None, true, true);
            let (changed, delay) = frame(0.3, Some(a_index), None, true, true);
            assert!(!changed);
            assert!(
                delay > Duration::ZERO && delay <= Duration::from_millis(310),
                "{delay:?}"
            );
            match interruption {
                0 => {
                    frame(0.4, None, None, true, true);
                }
                1 => {
                    frame(0.4, Some(b_index), None, true, true);
                }
                2 => {
                    frame(0.4, Some(a_index), Some(false), true, true);
                }
                3 => {
                    frame(0.4, Some(a_index), None, false, true);
                }
                _ => {
                    frame(0.4, Some(a_index), None, true, false);
                }
            }
            assert!(!frame(0.8, Some(a_index), Some(true), true, true).0);
            assert!(!frame(1.2, Some(a_index), None, true, true).0);
            assert!(frame(1.5, Some(a_index), None, true, true).0);
            assert!(!frame(2., Some(a_index), None, true, true).0);
            assert!(!collapsed.contains(&a));
            assert!(collapsed.contains(&b));
        }
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
