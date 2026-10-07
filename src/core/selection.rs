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
        let Some(target) = order.iter().position(|i| media[*i].id == id) else {
            return;
        };
        if extend {
            let anchor = self
                .anchor
                .and_then(|anchor| order.iter().position(|i| media[*i].id == anchor))
                .unwrap_or(target);
            if !toggle {
                self.ids.clear();
            }
            self.ids.extend(
                order[anchor.min(target)..=anchor.max(target)]
                    .iter()
                    .map(|i| media[*i].id),
            );
            self.anchor = Some(media[order[anchor]].id);
        } else if toggle {
            if !self.ids.remove(&id) {
                self.ids.insert(id);
            }
            self.anchor = Some(id);
        } else {
            self.single(id);
        }
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
