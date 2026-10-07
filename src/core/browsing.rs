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
    pub path: Option<std::path::PathBuf>,
    pub recursive: bool,
}
impl Default for FolderFilter {
    fn default() -> Self {
        Self {
            path: None,
            recursive: true,
        }
    }
}
impl FolderFilter {
    pub fn matches(&self, media: &std::path::Path) -> bool {
        self.path.as_ref().is_none_or(|folder| {
            media.parent().is_some_and(|parent| {
                if self.recursive {
                    parent.starts_with(folder)
                } else {
                    parent == folder
                }
            })
        })
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
