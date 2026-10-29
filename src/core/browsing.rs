//! Library browsing rules. Uses already scanned media and shared metadata only.
use super::{dates, model::*};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

mod group_order;
pub mod tree;
mod worker;
pub use worker::{GroupChoice, Query, ResultSet, Seed, Update, Worker};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RatingFilter {
    #[default]
    All,
    Exactly(u8),
    AtLeast(u8),
}
impl RatingFilter {
    pub const ALL: [Self; 11] = [
        Self::All,
        Self::Exactly(0),
        Self::Exactly(1),
        Self::AtLeast(1),
        Self::Exactly(2),
        Self::AtLeast(2),
        Self::Exactly(3),
        Self::AtLeast(3),
        Self::Exactly(4),
        Self::AtLeast(4),
        Self::Exactly(5),
    ];
    pub fn title(self) -> String {
        match self {
            Self::All => "Any rating".into(),
            Self::Exactly(0) => "Unrated".into(),
            Self::Exactly(1) => "Exactly 1 star".into(),
            Self::Exactly(n) => format!("Exactly {n} stars"),
            Self::AtLeast(1) => "1 star or more".into(),
            Self::AtLeast(n) => format!("{n} stars or more"),
        }
    }
    pub fn matches(self, rating: Option<u8>) -> bool {
        match self {
            Self::All => true,
            Self::Exactly(n) => rating == Some(n),
            Self::AtLeast(n) => rating.is_some_and(|v| v >= n),
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Sort {
    Name,
    #[default]
    Oldest,
    Newest,
}
impl Sort {
    pub const ALL: [Self; 3] = [Self::Name, Self::Oldest, Self::Newest];
    pub fn title(self) -> &'static str {
        match self {
            Self::Name => "File name",
            Self::Oldest => "Capture time · oldest first",
            Self::Newest => "Capture time · newest first",
        }
    }
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum GroupSort {
    #[default]
    Flat,
    Time,
    Similarity,
}
impl GroupSort {
    pub const ALL: [Self; 3] = [Self::Flat, Self::Time, Self::Similarity];
    pub fn title(self) -> &'static str {
        match self {
            Self::Flat => "Photos · no group ordering",
            Self::Time => "Groups · earliest photo",
            Self::Similarity => "Groups · similarity then time",
        }
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Options {
    #[serde(default)]
    pub group_sort: GroupSort,
    #[serde(default)]
    pub date: Option<String>,
    pub rating: RatingFilter,
    pub group: Option<usize>,
    pub sort: Sort,
    pub folder: FolderFilter,
}
/// A folder scope uses component-aware paths from the existing catalog.
/// It never scans the filesystem or conflates siblings such as Trip and Trip-2.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FolderFilter {
    /// The folder shown, or of several the last one clicked, which a
    /// Shift-click range starts from. `None` shows every folder.
    pub path: Option<std::path::PathBuf>,
    /// More folders shown with `path`, chosen with Ctrl/Cmd or Shift.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub also: Vec<std::path::PathBuf>,
    pub recursive: bool,
}
impl Default for FolderFilter {
    fn default() -> Self {
        Self {
            path: None,
            also: Vec::new(),
            recursive: true,
        }
    }
}
impl FolderFilter {
    pub fn matches(&self, media: &std::path::Path) -> bool {
        self.path.is_none()
            || media.parent().is_some_and(|parent| {
                self.folders().any(|folder| {
                    if self.recursive {
                        parent.starts_with(folder)
                    } else {
                        parent == folder
                    }
                })
            })
    }
    /// Every folder shown; none when every folder is.
    pub fn folders(&self) -> impl Iterator<Item = &std::path::PathBuf> {
        let also = if self.path.is_some() {
            &self.also[..]
        } else {
            &[]
        };
        self.path.iter().chain(also)
    }
    pub fn shows(&self, folder: &std::path::Path) -> bool {
        self.folders().any(|f| f == folder)
    }
    /// Shows just this folder, or every folder.
    pub fn show_only(&mut self, folder: Option<std::path::PathBuf>) {
        self.path = folder;
        self.also.clear();
    }
    /// Ctrl/Cmd-click: adds `folder` to those shown, or takes it away again.
    pub fn toggle(&mut self, folder: std::path::PathBuf) {
        if self.path.as_ref() == Some(&folder) {
            self.path = self.also.pop();
        } else if let Some(at) = self.also.iter().position(|f| *f == folder) {
            self.also.remove(at);
        } else {
            self.also.extend(self.path.replace(folder));
        }
    }
    /// Shift-click: shows the `span` of folders, from `path` through the
    /// one clicked; `add` (Ctrl/Cmd) keeps those already shown too.
    pub fn show_span(&mut self, span: Vec<std::path::PathBuf>, add: bool) {
        if !add {
            self.also.clear();
        }
        for folder in span {
            if self.path.is_none() {
                self.path = Some(folder);
            } else if !self.shows(&folder) {
                self.also.push(folder);
            }
        }
    }
}
/// Cached folder navigation entries, built once by the browse worker.
#[derive(Clone, Debug)]
pub struct Folder {
    pub path: std::path::PathBuf,
    pub label: String,
}
/// Unknown metadata is not an unrated photo; staged zero explicitly is.
pub fn rating(
    id: MediaId,
    decisions: &HashMap<MediaId, Decision>,
    metadata: &HashMap<MediaId, Metadata>,
) -> Option<u8> {
    decisions
        .get(&id)
        .and_then(|d| d.rating)
        .or_else(|| metadata.get(&id).map(|m| m.rating))
}
pub fn apply(
    indices: &mut Vec<usize>,
    media: &[Media],
    metadata: &HashMap<MediaId, Metadata>,
    decisions: &HashMap<MediaId, Decision>,
    groups: &HashMap<MediaId, usize>,
    options: &Options,
) {
    indices.retain(|i| {
        let id = media[*i].id;
        options.rating.matches(rating(id, decisions, metadata))
            && options.group.is_none_or(|g| groups.get(&id) == Some(&g))
            && options.folder.matches(&media[*i].path)
            && options.date.as_deref().is_none_or(|scope| {
                tree::date_matches(
                    Some(scope),
                    dates::calendar(
                        dates::capture_millis(&media[*i], metadata.get(&id)),
                        metadata.get(&id),
                    )
                    .as_deref(),
                )
            })
    });
    // Resolve strings and timestamps once per row, rather than for every comparison.
    let mut rows: Vec<_> = indices
        .iter()
        .map(|i| {
            let m = &media[*i];
            (
                *i,
                dates::capture_millis(m, metadata.get(&m.id)),
                m.name().to_lowercase(),
            )
        })
        .collect();
    rows.sort_by(|a, b| {
        compare(options.sort, a.1, b.1, &a.2, &b.2)
            .then_with(|| media[a.0].path.cmp(&media[b.0].path))
    });
    *indices = rows.into_iter().map(|r| r.0).collect();
}

fn compare(sort: Sort, a: Option<i64>, b: Option<i64>, an: &str, bn: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    let time = match sort {
        Sort::Name => Equal,
        _ => match (a, b) {
            (Some(a), Some(b)) if sort == Sort::Newest => b.cmp(&a),
            (Some(a), Some(b)) => a.cmp(&b),
            (None, Some(_)) => Greater,
            (Some(_), None) => Less,
            _ => Equal,
        },
    };
    time.then_with(|| an.cmp(bn))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Filter {
    #[default]
    All,
    Unmarked,
    Rejected,
    Rated,
    Issues,
    Duplicates,
}
impl Filter {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::Unmarked,
        Self::Rejected,
        Self::Rated,
        Self::Issues,
        Self::Duplicates,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::All => "All photos",
            Self::Unmarked => "Unmarked",
            Self::Rejected => "Marked for Trash",
            Self::Rated => "Rated",
            Self::Issues => "Quality issues",
            Self::Duplicates => "Duplicate candidates",
        }
    }
    pub fn matches(
        self,
        rejected: bool,
        rating: Option<u8>,
        issues: bool,
        duplicate: bool,
    ) -> bool {
        match self {
            Self::All => true,
            Self::Unmarked => !rejected,
            Self::Rejected => rejected,
            Self::Rated => rating.is_some_and(|n| n > 0),
            Self::Issues => issues,
            Self::Duplicates => duplicate,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn a_folder_filter_shows_every_folder_chosen() {
        let root = Path::new("/photos");
        let [a, b, c] = ["A", "B", "C"].map(|name| root.join(name));
        let mut filter = FolderFilter::default();
        assert!(filter.matches(&c.join("x.jpg")));
        // Ctrl/Cmd-click adds folders, and takes them away again.
        filter.show_only(Some(a.clone()));
        filter.toggle(b.clone());
        assert_eq!(filter.folders().collect::<Vec<_>>(), [&b, &a]);
        assert!(filter.matches(&a.join("Day1/x.jpg")) && filter.matches(&b.join("x.jpg")));
        assert!(!filter.matches(&c.join("x.jpg")));
        filter.recursive = false;
        assert!(!filter.matches(&a.join("Day1/x.jpg")) && filter.matches(&a.join("x.jpg")));
        filter.toggle(b.clone());
        assert_eq!(filter.folders().collect::<Vec<_>>(), [&a]);
        filter.toggle(a.clone());
        assert!(filter.path.is_none() && filter.matches(&c.join("x.jpg")));
        // Shift shows a range from the folder clicked last, replacing or
        // (with Ctrl/Cmd) adding to those shown.
        filter.show_only(Some(c.clone()));
        filter.show_span(vec![a.clone(), b.clone(), c.clone()], false);
        assert_eq!(filter.folders().collect::<Vec<_>>(), [&c, &a, &b]);
        filter.show_span(vec![b.clone(), c.clone()], false);
        assert_eq!(filter.folders().collect::<Vec<_>>(), [&c, &b]);
        filter.show_span(vec![a.clone()], true);
        assert_eq!(filter.folders().count(), 3);
    }

    #[test]
    fn views_saved_with_one_folder_still_load() {
        let saved = r#"{"path":"/photos/A","recursive":true}"#;
        let filter: FolderFilter = serde_json::from_str(saved).unwrap();
        assert_eq!(filter.path, Some(PathBuf::from("/photos/A")));
        assert!(filter.also.is_empty());
        // One folder is still saved the way it was.
        assert_eq!(serde_json::to_string(&filter).unwrap(), saved);
    }
}
