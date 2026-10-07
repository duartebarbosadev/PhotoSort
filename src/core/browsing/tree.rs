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
    pub fn project(self: &Arc<Self>, state: &State, cancelled: impl Fn() -> bool) -> Option<View> {
        let mut visible = Vec::new();
        let mut index = 0;
        while index < self.nodes.len() {
            if cancelled() {
                return None;
            }
            visible.push(index);
            let node = &self.nodes[index];
            index = if state.collapsed.contains(&node.key) {
                node.end
            } else {
                index + 1
            };
        }
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
            for (i, folder) in folders.iter().enumerate() {
                if i % 256 == 0 && cancelled() {
                    return None;
                }
                let key = Key::Folder(folder.path.clone());
                let parent = folder
                    .path
                    .parent()
                    .and_then(|p| tree.lookup.get(&Key::Folder(p.to_owned())))
                    .copied();
                let label = if parent.is_none() {
                    folder.label.clone()
                } else {
                    folder
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                };
                tree.push(key.clone(), label, counts.remove(&key).unwrap_or(0), parent);
            }
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
