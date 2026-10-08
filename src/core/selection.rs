//! Browser selection in the displayed order. No IO, image requests or workflow state.
use super::model::{Media, MediaId};
use std::collections::HashSet;

#[derive(Default)]
pub struct Selection {
    ids: HashSet<MediaId>,
    anchor: Option<MediaId>,
}
impl Selection {
    pub fn len(&self) -> usize {
        self.ids.len()
    }
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }
    pub fn contains(&self, id: MediaId) -> bool {
        self.ids.contains(&id)
    }
    /// Snapshot only selected IDs, not every row in a large catalog.
    pub fn snapshot(&self) -> (Vec<MediaId>, Option<MediaId>) {
        let mut ids: Vec<_> = self.ids.iter().copied().collect();
        ids.sort_unstable_by_key(|id| id.0);
        (ids, self.anchor)
    }
    pub fn restore(&mut self, ids: impl IntoIterator<Item = MediaId>, anchor: Option<MediaId>) {
        self.ids = ids.into_iter().collect();
        self.anchor = anchor;
    }
    pub fn clear(&mut self) {
        self.ids.clear();
        self.anchor = None;
    }
    pub fn replace(&mut self, ids: impl IntoIterator<Item = MediaId>) {
        self.clear();
        for id in ids {
            self.anchor.get_or_insert(id);
            self.ids.insert(id);
        }
    }
    pub fn single(&mut self, id: MediaId) {
        self.clear();
        self.ids.insert(id);
        self.anchor = Some(id);
    }
    pub fn ordered(&self, media: &[Media], order: &[usize]) -> Vec<MediaId> {
        order
            .iter()
            .map(|i| media[*i].id)
            .filter(|id| self.ids.contains(id))
            .collect()
    }
    pub fn all(&mut self, media: &[Media], order: &[usize]) {
        self.ids = order.iter().map(|i| media[*i].id).collect();
        self.anchor = order.first().map(|i| media[*i].id);
    }
    pub fn select(
        &mut self,
        id: MediaId,
        media: &[Media],
        order: &[usize],
        toggle: bool,
        extend: bool,
    ) {
        select_ordered(
            &mut self.ids,
            &mut self.anchor,
            id,
            order.iter().map(|i| media[*i].id),
            toggle,
            extend,
        );
    }
    /// Drop hidden/missing items so a bulk action cannot affect another filter's rows.
    pub fn reconcile(&mut self, media: &[Media], order: &[usize]) {
        let mut anchor_visible = false;
        self.ids = order
            .iter()
            .map(|i| media[*i].id)
            .filter(|id| {
                anchor_visible |= Some(*id) == self.anchor;
                self.ids.contains(id)
            })
            .collect();
        if !anchor_visible {
            self.anchor = None;
        }
    }
}

/// Apply the same range/toggle rules to media browsers and supporting-file lists.
/// The caller supplies its cached display order; no synthetic media or IO is needed.
pub fn select_ordered(
    ids: &mut HashSet<MediaId>,
    anchor: &mut Option<MediaId>,
    id: MediaId,
    order: impl Iterator<Item = MediaId> + Clone,
    toggle: bool,
    extend: bool,
) {
    let Some(target) = order.clone().position(|candidate| candidate == id) else {
        return;
    };
    let start = anchor
        .filter(|_| extend)
        .and_then(|a| order.clone().position(|candidate| candidate == a));
    let start = select_positions(
        ids,
        start,
        target,
        |range| {
            // A single click already resolved its ID; do not walk the prefix again.
            (!extend).then_some(id).into_iter().chain(
                order
                    .clone()
                    .skip(range.start)
                    .take(if extend { range.len() } else { 0 }),
            )
        },
        toggle,
        extend,
    );
    *anchor = if extend {
        order.clone().nth(start)
    } else {
        Some(id)
    };
}

/// Collection operations needed by ordered selection; callers retain their existing
/// storage (hashed media IDs or sorted folder paths).
pub trait SelectionSet<T>: Extend<T> {
    fn clear_selection(&mut self);
    fn toggle_selection(&mut self, item: T);
}
impl<T: Eq + std::hash::Hash> SelectionSet<T> for HashSet<T> {
    fn clear_selection(&mut self) {
        self.clear();
    }
    fn toggle_selection(&mut self, item: T) {
        if !self.remove(&item) {
            self.insert(item);
        }
    }
}
impl<T: Ord> SelectionSet<T> for std::collections::BTreeSet<T> {
    fn clear_selection(&mut self) {
        self.clear();
    }
    fn toggle_selection(&mut self, item: T) {
        if !self.remove(&item) {
            self.insert(item);
        }
    }
}

/// Apply range/toggle rules to already resolved display positions. The caller
/// supplies only the requested span, so cached trees need neither a full walk
/// nor a cloned list of path strings. Returns the new anchor position.
pub fn select_positions<T, I: Iterator<Item = T>>(
    ids: &mut impl SelectionSet<T>,
    anchor: Option<usize>,
    target: usize,
    items: impl FnOnce(std::ops::Range<usize>) -> I,
    toggle: bool,
    extend: bool,
) -> usize {
    let start = if extend {
        anchor.unwrap_or(target)
    } else {
        target
    };
    if !toggle {
        ids.clear_selection();
    }
    let mut range = items(start.min(target)..start.max(target) + 1);
    if toggle && !extend {
        if let Some(item) = range.next() {
            ids.toggle_selection(item);
        }
    } else {
        ids.extend(range);
    }
    start
}

#[cfg(test)]
mod range_tests {
    use super::*;
    use std::{cell::Cell, collections::BTreeSet};

    #[test]
    fn folder_ranges_shared_rules_do_not_add_media_prefix_walks_for_single_clicks() {
        let visits = Cell::new(0);
        let order = (0..100_000).map(|i| {
            visits.set(visits.get() + 1);
            MediaId(i)
        });
        let mut ids = HashSet::new();
        let mut anchor = Some(MediaId(0));
        select_ordered(&mut ids, &mut anchor, MediaId(99_999), order, true, false);
        assert_eq!(
            visits.get(),
            100_000,
            "Only the existing target lookup visits the prefix"
        );
        assert_eq!(ids, [MediaId(99_999)].into());
        assert_eq!(anchor, Some(MediaId(99_999)));
    }

    #[test]
    fn folder_ranges_visit_only_the_requested_span_and_share_toggle_rules() {
        let visits = Cell::new(0);
        let items = |range: std::ops::Range<usize>| {
            range.inspect(|_| {
                visits.set(visits.get() + 1);
            })
        };
        let mut folders = BTreeSet::from([1]);
        let anchor = select_positions(&mut folders, Some(89_998), 90_000, items, false, true);
        assert_eq!(anchor, 89_998);
        assert_eq!(folders, BTreeSet::from([89_998, 89_999, 90_000]));
        assert_eq!(
            visits.get(),
            3,
            "A range near the end of a large tree must not walk its prefix"
        );
        select_positions(&mut folders, Some(anchor), 89_999, items, false, true);
        assert_eq!(folders, BTreeSet::from([89_998, 89_999]));
        select_positions(&mut folders, None, 1, items, true, false);
        select_positions(&mut folders, Some(anchor), 90_000, items, true, true);
        assert_eq!(folders, BTreeSet::from([1, 89_998, 89_999, 90_000]));
        select_positions(&mut folders, None, 1, items, true, false);
        assert!(!folders.contains(&1));
        assert_eq!(visits.get(), 10);
    }
}
