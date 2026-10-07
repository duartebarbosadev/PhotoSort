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
    stage_files(media, index, &HashMap::new(), decisions, ids, change)
}

/// Supporting files share Trash intent, but never enter image-only operations.
/// Both indexes come from discovery; staging performs no filesystem work.
pub fn stage_files(
    media: &[Media],
    index: &HashMap<MediaId, usize>,
    supporting: &HashMap<MediaId, Fingerprint>,
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
        let item = index
            .get(&id)
            .and_then(|i| media.get(*i))
            .filter(|m| m.id == id);
        let valid = item.is_some() || supporting.contains_key(&id);
        let image = item.is_some_and(|m| m.kind != MediaKind::Video);
        if !valid
            || !seen.insert(id)
            || (!image && matches!(change, Change::Rating(_) | Change::Rotate(_)))
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
