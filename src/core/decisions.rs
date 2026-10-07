//! Shared manual staging. File mutations remain owned by reviewed operation plans.
use super::model::*;
use std::collections::{HashMap, HashSet};
#[derive(Clone, Copy)]
pub enum Change {
    Rating(u8),
    ToggleTrash,
    Trash(bool),
    Rotate(u8),
}

pub fn stage(
    media: &[Media],
    index: &HashMap<MediaId, usize>,
    decisions: &mut HashMap<MediaId, Decision>,
    ids: impl IntoIterator<Item = MediaId>,
    change: Change,
) -> Vec<MediaId> {
    if matches!(change, Change::Rating(n) if n > 5) {
        return Vec::new();
    }
    let mut changed = Vec::new();
    let mut seen = HashSet::new();
    for id in ids {
        let Some(item) = index
            .get(&id)
            .and_then(|i| media.get(*i))
            .filter(|m| m.id == id)
        else {
            continue;
        };
        if !seen.insert(id)
            || (item.kind == MediaKind::Video
                && matches!(change, Change::Rating(_) | Change::Rotate(_)))
        {
            continue;
        }
        let decision = decisions.entry(id).or_default();
        match change {
            Change::Rating(n) => decision.rating = Some(n),
            Change::ToggleTrash => decision.rejected ^= true,
            Change::Trash(value) => decision.rejected = value,
            Change::Rotate(turns) => decision.turns = (decision.turns % 4 + turns % 4) % 4,
        }
        changed.push(id);
    }
    changed
}
