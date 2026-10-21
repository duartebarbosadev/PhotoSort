//! Cached folder/calendar/group navigation, built by the shared browse worker.
use super::Folder;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    path::PathBuf,
    sync::Arc,
};
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Folders,
    Dates,
    Groups,
}
impl Mode {
    pub const ALL: [Self; 3] = [Self::Folders, Self::Dates, Self::Groups];
    pub fn title(self) -> &'static str {
        match self {
            Self::Folders => "Folders",
            Self::Dates => "Dates",
            Self::Groups => "Groups",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Key {
    Folder(PathBuf),
    Date(String),
    Group(usize),
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub enabled: bool,
    pub mode: Mode,
    pub collapsed: BTreeSet<Key>,
}
#[derive(Debug)]
pub struct Node {
    pub key: Key,
    pub label: String,
    pub count: usize,
    pub depth: usize,
    pub parent: Option<usize>,
    pub previous: Option<usize>,
    pub next: Option<usize>,
    pub end: usize,
}
impl Node {
    pub fn expandable(&self, index: usize) -> bool {
        self.end > index + 1
    }
}
/// How an explicit expansion command affects an existing branch.
#[derive(Clone, Copy)]
pub enum Expansion {
    Node,
    Subtree,
    Children,
}
/// Keyboard movement over the cached hierarchy. Advance is measured in visible rows.
#[derive(Clone, Copy, Debug)]
pub enum Navigation {
    Advance(isize),
    First,
    Last,
    Parent,
    Child,
}
pub struct NavigationResult {
    pub key: Key,
    pub expansion_changed: bool,
}
#[derive(Default)]
pub struct Tree {
    pub nodes: Vec<Node>,
    pub lookup: HashMap<Key, usize>,
}
#[derive(Clone)]
pub struct View {
    pub mode: Mode,
    pub tree: Arc<Tree>,
    pub visible: Vec<usize>,
}
impl Tree {
    fn push(&mut self, key: Key, label: String, count: usize, parent: Option<usize>) {
        let index = self.nodes.len();
        let depth = parent.map_or(0, |p| self.nodes[p].depth + 1);
        self.lookup.insert(key.clone(), index);
        self.nodes.push(Node {
            key,
            label,
            count,
            depth,
            parent,
            previous: None,
            next: None,
            end: index + 1,
        });
    }
    fn finish(mut self, cancelled: &impl Fn() -> bool) -> Option<Self> {
        let mut last = HashMap::new();
        for i in 0..self.nodes.len() {
            if i % 256 == 0 && cancelled() {
                return None;
            }
            if let Some(previous) = last.insert(self.nodes[i].parent, i) {
                self.nodes[i].previous = Some(previous);
                self.nodes[previous].next = Some(i);
            }
        }
        for i in (0..self.nodes.len()).rev() {
            if i % 256 == 0 && cancelled() {
                return None;
            }
            if let Some(p) = self.nodes[i].parent {
                self.nodes[p].count += self.nodes[i].count;
                self.nodes[p].end = self.nodes[p].end.max(self.nodes[i].end);
            }
        }
        (!cancelled()).then_some(self)
    }
    /// Build proposed or actual folders without filesystem access. Path-component
    /// ordering keeps descendants contiguous even beside names such as A-1.
    pub fn folders(
        folders: impl IntoIterator<Item = (PathBuf, String, usize)>,
        cancelled: impl Fn() -> bool,
    ) -> Option<Self> {
        let mut sorted = BTreeMap::new();
        for (i, (path, label, count)) in folders.into_iter().enumerate() {
            if i % 256 == 0 && cancelled() {
                return None;
            }
            sorted.insert(path, (label, count));
        }
        let mut tree = Self::default();
        for (i, (path, (label, count))) in sorted.into_iter().enumerate() {
            if i % 256 == 0 && cancelled() {
                return None;
            }
            let parent = path
                .parent()
                .and_then(|p| tree.lookup.get(&Key::Folder(p.into())))
                .copied();
            tree.push(Key::Folder(path), label, count, parent);
        }
        tree.finish(&cancelled)
    }
    pub fn set_expanded(
        &self,
        key: &Key,
        scope: Expansion,
        expanded: bool,
        collapsed: &mut BTreeSet<Key>,
    ) -> bool {
        let Some(&index) = self.lookup.get(key) else {
            return false;
        };
        let range = match scope {
            Expansion::Node => index..index + 1,
            Expansion::Subtree => index..self.nodes[index].end,
            Expansion::Children => index + 1..self.nodes[index].end,
        };
        let mut changed = false;
        for i in range {
            let node = &self.nodes[i];
            if node.expandable(i) {
                changed |= if expanded {
                    collapsed.remove(&node.key)
                } else {
                    collapsed.insert(node.key.clone())
                };
            }
        }
        changed
    }
    /// Resolve movement without rebuilding the visible projection. Descendant spans
    /// skip collapsed subtrees; only the requested rows and their ancestors are visited.
    pub fn navigate(
        &self,
        current: Option<&Key>,
        command: Navigation,
        collapsed: &mut BTreeSet<Key>,
    ) -> Option<NavigationResult> {
        if self.nodes.is_empty() {
            return None;
        }
        let current = current.and_then(|key| self.lookup.get(key)).copied();
        let mut index = self.visible_ancestor(current.unwrap_or(0), collapsed);
        let mut expansion_changed = false;
        match command {
            Navigation::First => index = 0,
            Navigation::Last => index = self.visible_ancestor(self.nodes.len() - 1, collapsed),
            Navigation::Advance(rows) if current.is_some() => {
                for _ in 0..rows.unsigned_abs() {
                    let next = if rows < 0 {
                        self.visible_ancestor(index.saturating_sub(1), collapsed)
                    } else if collapsed.contains(&self.nodes[index].key) {
                        self.nodes[index].end
                    } else {
                        index + 1
                    };
                    if next == index || next >= self.nodes.len() {
                        break;
                    }
                    index = next;
                }
            }
            Navigation::Parent => {
                let node = &self.nodes[index];
                if node.expandable(index) && !collapsed.contains(&node.key) {
                    expansion_changed = collapsed.insert(node.key.clone());
                } else if let Some(parent) = node.parent {
                    index = parent;
                }
            }
            Navigation::Child => {
                let node = &self.nodes[index];
                if node.expandable(index) {
                    expansion_changed = collapsed.remove(&node.key);
                    if !expansion_changed {
                        index += 1;
                    }
                }
            }
            Navigation::Advance(_) => {}
        }
        Some(NavigationResult {
            key: self.nodes[index].key.clone(),
            expansion_changed,
        })
    }
    fn visible_ancestor(&self, mut index: usize, collapsed: &BTreeSet<Key>) -> usize {
        let mut parent = self.nodes[index].parent;
        while let Some(i) = parent {
            if collapsed.contains(&self.nodes[i].key) {
                index = i;
            }
            parent = self.nodes[i].parent;
        }
        index
    }
    pub fn retain_branches(&self, collapsed: &mut BTreeSet<Key>) {
        collapsed.retain(|key| {
            self.lookup
                .get(key)
                .is_some_and(|&i| self.nodes[i].expandable(i))
        });
    }
    /// Reveal the selected destination after a structural edit, without losing
    /// independent collapsed branches or changing the selection itself.
    pub fn reveal(&self, key: &Key, collapsed: &mut BTreeSet<Key>) {
        let mut parent = self.lookup.get(key).and_then(|&i| self.nodes[i].parent);
        while let Some(i) = parent {
            collapsed.remove(&self.nodes[i].key);
            parent = self.nodes[i].parent;
        }
    }
    pub fn visible(
        &self,
        collapsed: &BTreeSet<Key>,
        cancelled: impl Fn() -> bool,
    ) -> Option<Vec<usize>> {
        let mut visible = Vec::new();
        let mut index = 0;
        while index < self.nodes.len() {
            if cancelled() {
                return None;
            }
            visible.push(index);
            let node = &self.nodes[index];
            index = if collapsed.contains(&node.key) {
                node.end
            } else {
                index + 1
            };
        }
        Some(visible)
    }
    pub fn project(self: &Arc<Self>, state: &State, cancelled: impl Fn() -> bool) -> Option<View> {
        let visible = self.visible(&state.collapsed, cancelled)?;
        Some(View {
            mode: state.mode,
            tree: self.clone(),
            visible,
        })
    }
}
pub struct Entry<'a> {
    pub path: &'a std::path::Path,
    pub date: Option<&'a str>,
    pub group: Option<usize>,
}
pub fn build<'a>(
    mode: Mode,
    folders: &[Folder],
    entries: impl Iterator<Item = Entry<'a>>,
    cancelled: impl Fn() -> bool,
) -> Option<Tree> {
    let mut counts = BTreeMap::<Key, usize>::new();
    for (n, entry) in entries.enumerate() {
        if n % 256 == 0 && cancelled() {
            return None;
        }
        let key = match mode {
            Mode::Folders => entry.path.parent().map(|p| Key::Folder(p.to_owned())),
            Mode::Dates => Some(Key::Date(entry.date.unwrap_or("").into())),
            Mode::Groups => entry.group.map(Key::Group),
        };
        if let Some(key) = key {
            *counts.entry(key).or_default() += 1;
        }
    }
    let mut tree = Tree::default();
    match mode {
        Mode::Folders => {
            return Tree::folders(
                folders.iter().map(|folder| {
                    let key = Key::Folder(folder.path.clone());
                    let label = if folder.path.parent().is_some_and(|parent| {
                        folders
                            .first()
                            .is_some_and(|root| parent.starts_with(&root.path))
                    }) {
                        folder
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned()
                    } else {
                        folder.label.clone()
                    };
                    (folder.path.clone(), label, counts.remove(&key).unwrap_or(0))
                }),
                cancelled,
            );
        }
        Mode::Dates => {
            let mut dates = BTreeSet::new();
            for (n, key) in counts.keys().enumerate() {
                if n % 256 == 0 && cancelled() {
                    return None;
                }
                if let Key::Date(date) = key {
                    if date.len() == 10 && date.is_ascii() {
                        dates.insert(date[..4].to_string());
                        dates.insert(date[..7].to_string());
                    }
                    dates.insert(date.clone());
                }
            }
            for (i, date) in dates.into_iter().enumerate() {
                if i % 256 == 0 && cancelled() {
                    return None;
                }
                let parent = match date.len() {
                    7 => date.get(..4),
                    10 => date.get(..7),
                    _ => None,
                }
                .and_then(|p| tree.lookup.get(&Key::Date(p.into())))
                .copied();
                let key = Key::Date(date.clone());
                let label = if date.is_empty() {
                    "Unknown date".into()
                } else {
                    date
                };
                tree.push(key.clone(), label, counts.remove(&key).unwrap_or(0), parent);
            }
        }
        Mode::Groups => {
            for (i, (key, count)) in counts.into_iter().enumerate() {
                if i % 256 == 0 && cancelled() {
                    return None;
                }
                let Key::Group(g) = key else { unreachable!() };
                tree.push(Key::Group(g), format!("Group {g}"), count, None);
            }
        }
    }
    tree.finish(&cancelled)
}
/// Empty scope is the explicit Unknown date node. Other scopes come from
/// calendar nodes (year/month/day); separator boundaries prevent prefix leakage.
pub fn date_matches(scope: Option<&str>, date: Option<&str>) -> bool {
    scope.is_none_or(|scope| {
        if scope.is_empty() {
            date.is_none()
        } else {
            date.is_some_and(|date| {
                date == scope || date.strip_prefix(scope).is_some_and(|s| s.starts_with('-'))
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(path: &str) -> Key {
        Key::Folder(path.into())
    }
    #[test]
    fn folder_expansion_scopes_preserve_siblings_counts_and_selection_visibility() {
        let tree = Tree::folders(
            [
                ("", 0),
                ("A-1", 1),
                ("A/B/C", 2),
                ("A", 0),
                ("A/B", 3),
                ("A/D", 4),
                ("Z", 5),
            ]
            .into_iter()
            .map(|(p, n)| (p.into(), p.into(), n)),
            || false,
        )
        .unwrap();
        let names = |visible: Vec<usize>| {
            visible
                .into_iter()
                .map(|i| tree.nodes[i].key.clone())
                .collect::<Vec<_>>()
        };
        let mut collapsed = BTreeSet::new();
        assert_eq!(
            names(tree.visible(&collapsed, || false).unwrap()),
            ["", "A", "A/B", "A/B/C", "A/D", "A-1", "Z"].map(key)
        );
        assert_eq!(tree.nodes[0].count, 15);
        assert_eq!(tree.nodes[tree.lookup[&key("A")]].count, 9);
        assert!(tree.set_expanded(&key("A"), Expansion::Children, false, &mut collapsed));
        assert_eq!(collapsed, [key("A/B")].into());
        assert!(tree.set_expanded(&key("A"), Expansion::Node, false, &mut collapsed));
        assert_eq!(
            names(tree.visible(&collapsed, || false).unwrap()),
            ["", "A", "A-1", "Z"].map(key)
        );
        assert!(tree.set_expanded(&key("A"), Expansion::Node, true, &mut collapsed));
        assert_eq!(
            names(tree.visible(&collapsed, || false).unwrap()),
            ["", "A", "A/B", "A/D", "A-1", "Z"].map(key)
        );
        assert!(tree.set_expanded(&key("A"), Expansion::Subtree, true, &mut collapsed));
        tree.set_expanded(&key(""), Expansion::Subtree, false, &mut collapsed);
        assert_eq!(tree.visible(&collapsed, || false).unwrap(), vec![0]);
        tree.reveal(&key("A/B/C"), &mut collapsed);
        assert!(collapsed.is_empty());
        collapsed.extend([key("A/D"), key("gone"), Key::Date("2026".into())]);
        tree.retain_branches(&mut collapsed);
        assert!(collapsed.is_empty());
        assert!(!tree.set_expanded(&key("gone"), Expansion::Subtree, false, &mut collapsed));
    }
    #[test]
    fn folder_expansion_large_tree_reuses_projection_and_honors_cancellation() {
        let folders = || {
            std::iter::once((PathBuf::new(), "Library".into(), 0)).chain(
                (0..100_000).map(|i| (format!("Folder-{i:06}").into(), format!("Folder {i}"), 1)),
            )
        };
        assert!(Tree::folders(folders(), || true).is_none());
        let tree = Arc::new(Tree::folders(folders(), || false).unwrap());
        let mut state = State::default();
        tree.set_expanded(&key(""), Expansion::Subtree, false, &mut state.collapsed);
        let view = tree.project(&state, || false).unwrap();
        assert!(Arc::ptr_eq(&tree, &view.tree));
        assert_eq!(view.visible, vec![0]);
        assert_eq!(tree.nodes[0].count, 100_000);
        assert!(tree.project(&state, || true).is_none());
        tree.set_expanded(&key(""), Expansion::Subtree, true, &mut state.collapsed);
        assert_eq!(
            tree.project(&state, || false).unwrap().visible.len(),
            100_001
        );
    }
}

#[cfg(test)]
mod keyboard_tests {
    use super::*;
    fn folders() -> Tree {
        Tree::folders(
            ["", "A", "A/B", "A/B/C", "A/D", "A-1", "Z", "Z/X"].map(|p| (p.into(), p.into(), 0)),
            || false,
        )
        .unwrap()
    }
    fn key(path: &str) -> Key {
        Key::Folder(path.into())
    }
    #[test]
    fn tree_keyboard_walk_matches_visible_projection_for_every_collapse_combination() {
        let tree = folders();
        let address = tree.nodes.as_ptr();
        for mask in 0u32..256 {
            let mut collapsed: BTreeSet<_> = tree
                .nodes
                .iter()
                .enumerate()
                .filter(|(i, _)| mask & (1 << i) != 0)
                .map(|(_, n)| n.key.clone())
                .collect();
            tree.retain_branches(&mut collapsed);
            let visible = tree.visible(&collapsed, || false).unwrap();
            for (row, &i) in visible.iter().enumerate() {
                for delta in [-100isize, -3, -1, 0, 1, 3, 100] {
                    let result = tree
                        .navigate(
                            Some(&tree.nodes[i].key),
                            Navigation::Advance(delta),
                            &mut collapsed,
                        )
                        .unwrap();
                    let expected =
                        (row as isize + delta).clamp(0, visible.len() as isize - 1) as usize;
                    assert_eq!(result.key, tree.nodes[visible[expected]].key);
                    assert!(!result.expansion_changed);
                }
            }
            assert_eq!(
                tree.navigate(None, Navigation::First, &mut collapsed)
                    .unwrap()
                    .key,
                tree.nodes[visible[0]].key
            );
            assert_eq!(
                tree.navigate(None, Navigation::Last, &mut collapsed)
                    .unwrap()
                    .key,
                tree.nodes[*visible.last().unwrap()].key
            );
        }
        assert_eq!(tree.nodes.as_ptr(), address);
    }
    #[test]
    fn tree_keyboard_parent_child_hidden_selection_and_missing_selection() {
        let tree = folders();
        let mut collapsed = BTreeSet::new();
        let result = tree
            .navigate(Some(&key("A")), Navigation::Parent, &mut collapsed)
            .unwrap();
        assert_eq!(result.key, key("A"));
        assert!(result.expansion_changed);
        assert_eq!(
            tree.navigate(Some(&key("A/B/C")), Navigation::Advance(1), &mut collapsed)
                .unwrap()
                .key,
            key("A-1")
        );
        let result = tree
            .navigate(Some(&key("A")), Navigation::Child, &mut collapsed)
            .unwrap();
        assert_eq!(result.key, key("A"));
        assert!(result.expansion_changed);
        assert_eq!(
            tree.navigate(Some(&key("A")), Navigation::Child, &mut collapsed)
                .unwrap()
                .key,
            key("A/B")
        );
        assert_eq!(
            tree.navigate(Some(&key("A/B/C")), Navigation::Parent, &mut collapsed)
                .unwrap()
                .key,
            key("A/B")
        );
        assert_eq!(
            tree.navigate(Some(&key("A/B/C")), Navigation::Child, &mut collapsed)
                .unwrap()
                .key,
            key("A/B/C")
        );
        assert_eq!(
            tree.navigate(
                Some(&key("missing")),
                Navigation::Advance(1),
                &mut collapsed
            )
            .unwrap()
            .key,
            key("")
        );
        assert!(
            Tree::default()
                .navigate(None, Navigation::First, &mut collapsed)
                .is_none()
        );
    }
    #[test]
    fn tree_keyboard_large_cached_tree_and_flat_date_group_keys() {
        let tree = Tree::folders(
            std::iter::once(("".into(), "root".into(), 0))
                .chain((0..100_000).map(|i| (format!("{i:06}").into(), i.to_string(), 0))),
            || false,
        )
        .unwrap();
        let address = tree.nodes.as_ptr();
        let mut collapsed = BTreeSet::new();
        assert_eq!(
            tree.navigate(None, Navigation::Last, &mut collapsed)
                .unwrap()
                .key,
            key("099999")
        );
        assert_eq!(
            tree.navigate(
                Some(&key("050000")),
                Navigation::Advance(30),
                &mut collapsed
            )
            .unwrap()
            .key,
            key("050030")
        );
        assert_eq!(tree.nodes.as_ptr(), address);
        for keys in [
            vec![
                Key::Date("2026-01-01".into()),
                Key::Date("2026-01-02".into()),
            ],
            vec![Key::Group(1), Key::Group(2)],
        ] {
            let mut tree = Tree::default();
            for key in &keys {
                tree.push(key.clone(), String::new(), 0, None);
            }
            let tree = tree.finish(&|| false).unwrap();
            assert_eq!(
                tree.navigate(Some(&keys[0]), Navigation::Advance(1), &mut collapsed)
                    .unwrap()
                    .key,
                keys[1]
            );
        }
    }
}
