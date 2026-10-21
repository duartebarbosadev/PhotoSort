//! Navigation over the current visible order, without IO or rebuilding filters.
use super::model::*;
use std::collections::HashMap;

/// Find the next eligible photo without wrapping at the ends. A missing current
/// item starts at the appropriate edge. Marks affect traversal, not visibility.
pub fn next_visible(
    media: &[Media],
    order: &[usize],
    current: Option<MediaId>,
    direction: isize,
    decisions: &HashMap<MediaId, Decision>,
    skip_marked: bool,
) -> Option<MediaId> {
    if direction == 0 {
        return None;
    }
    let position = current.and_then(|id| order.iter().position(|i| media[*i].id == id));
    next_position(order.len(), position, direction, |p| {
        !skip_marked
            || !decisions
                .get(&media[order[p]].id)
                .is_some_and(|d| d.rejected)
    })
    .map(|p| media[order[p]].id)
}

/// Walk a cached display order without wrapping, stopping at the first eligible
/// row. A missing cursor begins at the direction's edge. Shared by file and photo lists.
pub fn next_position(
    len: usize,
    current: Option<usize>,
    direction: isize,
    eligible: impl Fn(usize) -> bool,
) -> Option<usize> {
    let current = current.filter(|p| *p < len);
    if direction > 0 {
        (current.map_or(0, |p| p + 1)..len).find(|p| eligible(*p))
    } else if direction < 0 {
        (0..current.unwrap_or(len)).rev().find(|p| eligible(*p))
    } else {
        None
    }
}

/// Movement preferences do not alter the visible photo list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub rating: Option<u8>,
    pub skip_singletons: bool,
    pub cycle_similarity: bool,
}
impl Preferences {
    pub fn normalize(&mut self) {
        self.rating = self.rating.filter(|r| *r <= 5);
    }
}
/// A lightweight view snapshot built by the existing browse worker. All positions
/// refer to the final visible order, after every folder/search/rating filter.
#[derive(Default)]
pub struct Index {
    ids: Vec<MediaId>,
    positions: HashMap<MediaId, usize>,
    groups: HashMap<usize, Vec<usize>>,
    group_for: Vec<Option<usize>>,
    ratings: [Vec<usize>; 6],
    marked: Vec<bool>,
    multi: Vec<usize>,
    sibling_for: Vec<Option<usize>>,
    siblings: HashMap<usize, Vec<usize>>,
}
impl Index {
    pub fn build(
        rows: impl IntoIterator<Item = (MediaId, Option<usize>, Option<u8>, bool)>,
        cancelled: impl Fn() -> bool,
    ) -> Option<Self> {
        let mut result = Self::default();
        for (pos, (id, group, rating, marked)) in rows.into_iter().enumerate() {
            if pos % 256 == 0 && cancelled() {
                return None;
            }
            result.ids.push(id);
            result.positions.insert(id, pos);
            result.group_for.push(group);
            result.marked.push(marked);
            if let Some(group) = group {
                result.groups.entry(group).or_default().push(pos);
            }
            if let Some(rating) = rating.filter(|r| *r <= 5) {
                result.ratings[rating as usize].push(pos);
            }
        }
        for (pos, group) in result.group_for.iter().enumerate() {
            if pos % 256 == 0 && cancelled() {
                return None;
            }
            if group.is_some_and(|g| result.groups[&g].len() > 1) {
                result.multi.push(pos);
            }
        }
        if cancelled() { None } else { Some(result) }
    }
    /// Parent-level navigation reuses the final filtered order. Membership is
    /// supplied by the browse worker's cached folder/calendar tree.
    pub fn set_siblings(
        &mut self,
        rows: impl Iterator<Item = (MediaId, Option<usize>)>,
        cancelled: impl Fn() -> bool,
    ) -> Option<()> {
        self.sibling_for = vec![None; self.ids.len()];
        self.siblings.clear();
        for (n, (id, key)) in rows.enumerate() {
            if n % 256 == 0 && cancelled() {
                return None;
            }
            if let Some(&pos) = self.positions.get(&id)
                && let Some(key) = key
            {
                self.sibling_for[pos] = Some(key);
                self.siblings.entry(key).or_default().push(pos);
            }
        }
        (!cancelled()).then_some(())
    }
    fn eligible(&self, pos: usize, skip_marked: bool) -> bool {
        !skip_marked || !self.marked[pos]
    }
    fn next_candidate(
        &self,
        candidates: &[usize],
        current: Option<usize>,
        direction: isize,
        skip_marked: bool,
        exclude_group: Option<usize>,
        group_entry: bool,
    ) -> Option<MediaId> {
        let eligible = |p: &&usize| {
            self.eligible(**p, skip_marked)
                && exclude_group.is_none_or(|g| self.group_for[**p] != Some(g))
                && (!group_entry || {
                    let mut neighbor = **p as isize - direction.signum();
                    while neighbor >= 0
                        && (neighbor as usize) < self.ids.len()
                        && !self.eligible(neighbor as usize, skip_marked)
                    {
                        neighbor -= direction.signum();
                    }
                    neighbor < 0
                        || neighbor as usize >= self.ids.len()
                        || self.group_for[neighbor as usize] != self.group_for[**p]
                })
        };
        let found = if direction > 0 {
            let start = current.map_or(0, |c| candidates.partition_point(|p| *p <= c));
            candidates[start..].iter().find(eligible)
        } else {
            let end = current.map_or(candidates.len(), |c| candidates.partition_point(|p| *p < c));
            candidates[..end].iter().rev().find(eligible)
        };
        found.map(|p| self.ids[*p])
    }
    pub fn target(
        &self,
        current: Option<MediaId>,
        direction: isize,
        horizontal: bool,
        preferences: Preferences,
        include_marked: bool,
        trash_only: bool,
    ) -> Option<MediaId> {
        if direction == 0 || self.ids.is_empty() {
            return None;
        }
        let current = current.and_then(|id| self.positions.get(&id).copied());
        let skip_marked = !include_marked && !trash_only;
        if horizontal {
            let group = if preferences.cycle_similarity {
                current
                    .and_then(|p| self.group_for[p])
                    .and_then(|g| self.groups.get(&g))
            } else {
                current
                    .and_then(|p| self.sibling_for.get(p).copied().flatten())
                    .and_then(|g| self.siblings.get(&g))
            };
            let len = group.map_or(self.ids.len(), Vec::len);
            let local = current.and_then(|p| group.map_or(Some(p), |g| g.binary_search(&p).ok()));
            for offset in 1..=len {
                let pos = match (local, direction > 0) {
                    (Some(p), true) => (p + offset) % len,
                    (Some(p), false) => (p + len - offset) % len,
                    (None, true) => offset - 1,
                    (None, false) => len - offset,
                };
                let pos = group.map_or(pos, |g| g[pos]);
                if self.eligible(pos, skip_marked) {
                    return Some(self.ids[pos]);
                }
            }
            return None;
        }
        if !include_marked {
            if let Some(rating) = preferences.rating {
                return self.ratings.get(rating as usize).and_then(|c| {
                    self.next_candidate(c, current, direction, skip_marked, None, false)
                });
            }
            if preferences.skip_singletons && !self.groups.is_empty() {
                let group = current.and_then(|p| self.group_for[p]);
                if let Some(p) = current
                    && let Some(g) = group
                    && self.groups[&g].len() > 1
                {
                    let mut next = p as isize + direction.signum();
                    while next >= 0
                        && (next as usize) < self.ids.len()
                        && self.group_for[next as usize] == Some(g)
                    {
                        if self.eligible(next as usize, skip_marked) {
                            return Some(self.ids[next as usize]);
                        }
                        next += direction.signum();
                    }
                }
                if let Some(id) =
                    self.next_candidate(&self.multi, current, direction, skip_marked, group, true)
                {
                    return Some(id);
                }
                // Fall back to sequential movement when there is no further
                // multi-photo group, keeping the final singletons reachable.
            }
        }
        if direction > 0 {
            (current.map_or(0, |p| p + 1)..self.ids.len())
                .find(|p| self.eligible(*p, skip_marked))
                .map(|p| self.ids[p])
        } else {
            (0..current.unwrap_or(self.ids.len()))
                .rev()
                .find(|p| self.eligible(*p, skip_marked))
                .map(|p| self.ids[p])
        }
    }
}
