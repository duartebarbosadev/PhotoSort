//! Shared controls for cached actual and proposed folder trees.
use super::*;
use crate::core::browsing::tree::{Expansion, Key, Tree};
use std::collections::BTreeSet;

pub(super) fn expander(
    ui: &mut egui::Ui,
    tree: &Tree,
    index: usize,
    collapsed: &mut BTreeSet<Key>,
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
            egui::Button::new(if closed { ">" } else { "⌄" }).frame(false),
        )
        .on_hover_text(if closed { "Expand" } else { "Collapse" });
    #[cfg(test)]
    ui.ctx().data_mut(|d| {
        d.insert_temp(
            egui::Id::new(("test_tree_expander", &node.key)),
            response.rect,
        )
    });
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
