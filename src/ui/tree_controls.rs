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

/// One focus stop for a virtualized tree, independent of which rows are rendered.
/// The state belongs to egui, while selection and expansion retain their shared owners.
pub(super) struct Keyboard {
    id: egui::Id,
    scroll_id: egui::Id,
    height: f32,
    row_extent: f32,
}
#[derive(Clone, Default)]
struct KeyboardState {
    rect: Option<egui::Rect>,
    reveal: Option<Key>,
    focus_requested: bool,
}
pub(super) fn pointer_sense(drag: bool) -> Sense {
    let sense = if drag {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    sense.difference(Sense::FOCUSABLE)
}
impl Keyboard {
    pub(super) fn new(
        ui: &egui::Ui,
        salt: impl std::hash::Hash,
        height: f32,
        row_height: f32,
    ) -> Self {
        let scroll_id = ui.make_persistent_id(egui::Id::new(salt));
        Self {
            id: scroll_id.with("tree_keyboard"),
            scroll_id,
            height: height.min(ui.available_height()).max(row_height),
            row_extent: row_height + ui.spacing().item_spacing.y,
        }
    }
    pub(super) fn navigate(
        &self,
        ui: &egui::Ui,
        tree: &Tree,
        current: Option<&Key>,
        collapsed: &mut BTreeSet<Key>,
    ) -> Option<NavigationResult> {
        let state = ui
            .ctx()
            .data(|d| d.get_temp::<KeyboardState>(self.id))
            .unwrap_or_default();
        let clicked_elsewhere = ui.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .interact_pos()
                    .is_some_and(|p| state.rect.is_none_or(|r| !r.contains(p)))
        });
        if !ui.is_enabled()
            || !ui.memory(|m| m.has_focus(self.id))
            || egui::Popup::is_any_open(ui.ctx())
            || clicked_elsewhere
        {
            return None;
        }
        let page = ((state.rect.map_or(self.height, |r| r.height()) / self.row_extent).floor()
            as isize
            - 1)
        .max(1);
        let commands = ui.input_mut(|input| {
            let mut commands = Vec::new();
            input.events.retain(|event| {
                let egui::Event::Key {
                    key,
                    modifiers,
                    pressed,
                    ..
                } = event
                else {
                    return true;
                };
                if !modifiers.is_none() {
                    return true;
                }
                let command = match key {
                    egui::Key::ArrowUp => Navigation::Advance(-1),
                    egui::Key::ArrowDown => Navigation::Advance(1),
                    egui::Key::ArrowLeft => Navigation::Parent,
                    egui::Key::ArrowRight => Navigation::Child,
                    egui::Key::Home => Navigation::First,
                    egui::Key::End => Navigation::Last,
                    egui::Key::PageUp => Navigation::Advance(-page),
                    egui::Key::PageDown => Navigation::Advance(page),
                    _ => return true,
                };
                if *pressed {
                    commands.push(command);
                }
                false
            });
            commands
        });
        if commands.is_empty() {
            return None;
        }
        // A newly focused widget has no focus filter until its next pass. Cancel
        // egui's pending cardinal focus move for keys this tree just consumed.
        ui.memory_mut(|m| m.move_focus(egui::FocusDirection::None));
        let mut selected = current.cloned();
        let mut changed = false;
        for command in commands {
            if let Some(result) = tree.navigate(selected.as_ref(), command, collapsed) {
                selected = Some(result.key);
                changed |= result.expansion_changed;
            }
        }
        let key = selected?;
        ui.ctx().data_mut(|d| {
            let state = d.get_temp_mut_or_default::<KeyboardState>(self.id);
            state.reveal = Some(key.clone());
        });
        Some(NavigationResult {
            key,
            expansion_changed: changed,
        })
    }
    pub(super) fn scroll(
        &self,
        ui: &egui::Ui,
        scroll: egui::ScrollArea,
        tree: &Tree,
        visible: &[usize],
        projection_ready: bool,
    ) -> egui::ScrollArea {
        let key = ui
            .ctx()
            .data(|d| d.get_temp::<KeyboardState>(self.id))
            .and_then(|s| s.reveal);
        let Some(row) = key
            .as_ref()
            .and_then(|k| tree.lookup.get(k))
            .and_then(|i| visible.binary_search(i).ok())
        else {
            // Cull's background projection may not have published the newly
            // expanded child yet. Keep the reveal request until it does.
            return scroll;
        };
        if projection_ready {
            ui.ctx()
                .data_mut(|d| d.get_temp_mut_or_default::<KeyboardState>(self.id).reveal = None);
        }
        let offset =
            egui::scroll_area::State::load(ui.ctx(), self.scroll_id).map_or(0., |s| s.offset.y);
        let top = row as f32 * self.row_extent;
        if top < offset {
            scroll.vertical_scroll_offset(top)
        } else if top + self.row_extent > offset + self.height {
            scroll.vertical_scroll_offset((top + self.row_extent - self.height).max(0.))
        } else {
            scroll
        }
    }
    pub(super) fn focus_on_click(&self, response: &egui::Response) {
        if response.clicked() {
            response.ctx.data_mut(|d| {
                let state = d.get_temp_mut_or_default::<KeyboardState>(self.id);
                state.focus_requested = true;
                state.reveal = None;
            });
        }
    }
    pub(super) fn finish(&self, ui: &egui::Ui, rect: egui::Rect) {
        ui.ctx()
            .data_mut(|d| d.get_temp_mut_or_default::<KeyboardState>(self.id).rect = Some(rect));
        let response = ui.interact(rect, self.id, Sense::focusable_noninteractive());
        // Child rows own pointer hit testing, so registering the noninteractive
        // container can surrender focus. Apply their request after registration.
        let request = ui.ctx().data_mut(|d| {
            std::mem::take(
                &mut d
                    .get_temp_mut_or_default::<KeyboardState>(self.id)
                    .focus_requested,
            )
        });
        if request {
            ui.memory_mut(|m| m.request_focus(self.id));
        }
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, ui.is_enabled(),
            "Navigation tree. Arrow keys navigate and expand; Home, End and Page keys move through the tree."));
        ui.memory_mut(|m| {
            m.set_focus_lock_filter(
                self.id,
                egui::EventFilter {
                    horizontal_arrows: true,
                    vertical_arrows: true,
                    ..Default::default()
                },
            )
        });
        if response.has_focus() {
            ui.painter().rect_stroke(
                rect,
                2.,
                ui.visuals().selection.stroke,
                egui::StrokeKind::Inside,
            );
        }
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
