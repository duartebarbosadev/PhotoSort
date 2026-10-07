//! Durable photo membership, independent of regenerated AI group numbers.
//! Pure, cancellable policy used by the existing application IO worker.
use super::model::*;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    sync::Arc,
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManualGroups {
    members: HashMap<MediaId, u64>,
}
#[derive(Clone)]
pub enum Edit {
    Move {
        selected: Vec<MediaId>,
        target: Option<usize>,
    },
    Restore(Vec<MediaId>),
    RestoreAll,
    Replace(Arc<ManualGroups>),
    Resolve,
}
pub struct Resolved {
    pub changed: bool,
    pub run: u64,
    pub automatic: Arc<HashMap<MediaId, usize>>,
    pub previous: Arc<ManualGroups>,
    pub manual: Arc<ManualGroups>,
    pub groups: HashMap<MediaId, usize>,
    pub choices: Vec<(usize, usize, MediaId)>,
}
impl ManualGroups {
    pub fn is_empty(&self) -> bool {
        self.members.is_empty()
    }
    pub fn len(&self) -> usize {
        self.members.len()
    }
    pub fn ids(&self) -> impl Iterator<Item = MediaId> + '_ {
        self.members.keys().copied()
    }
    pub fn contains(&self, id: MediaId) -> bool {
        self.members.contains_key(&id)
    }
    pub fn retain(&mut self, mut keep: impl FnMut(MediaId) -> bool) {
        self.members.retain(|id, _| keep(*id));
    }
    pub fn relocate(&mut self, changes: &[(MediaId, MediaId, Fingerprint)]) {
        remap_media_ids(&mut self.members, changes);
    }
    pub fn merge(&mut self, other: &Self) {
        // Tokens have library-local meaning. Never join unrelated destination groups.
        let mut own = BTreeMap::new();
        for token in self.members.values() {
            own.insert(*token, 0);
        }
        for (i, token) in own.values_mut().enumerate() {
            *token = i as u64 + 1;
        }
        for token in self.members.values_mut() {
            *token = own[token];
        }
        let mut next = own.len() as u64;
        let mut tokens = BTreeMap::new();
        for token in other.members.values() {
            tokens.insert(*token, 0);
        }
        for token in tokens.values_mut() {
            next += 1;
            *token = next;
        }
        for (id, token) in &other.members {
            self.members.insert(*id, tokens[token]);
        }
    }
    pub fn resolve(
        &self,
        automatic: &HashMap<MediaId, usize>,
        cancelled: impl Fn() -> bool,
    ) -> Result<HashMap<MediaId, usize>> {
        let mut groups = HashMap::with_capacity(automatic.len().max(self.members.len()));
        for (i, (id, group)) in automatic.iter().enumerate() {
            if i % 256 == 0 {
                ensure!(!cancelled(), "Grouping cancelled");
            }
            groups.insert(*id, *group);
        }
        let mut tokens = BTreeMap::new();
        for (i, token) in self.members.values().enumerate() {
            if i % 256 == 0 {
                ensure!(!cancelled(), "Grouping cancelled");
            }
            tokens.insert(*token, 0);
        }
        let mut next = automatic.values().copied().max().unwrap_or(0);
        for group in tokens.values_mut() {
            next = next
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Too many groups"))?;
            *group = next;
        }
        for (i, (id, token)) in self.members.iter().enumerate() {
            if i % 256 == 0 {
                ensure!(!cancelled(), "Grouping cancelled");
            }
            groups.insert(*id, tokens[token]);
        }
        ensure!(!cancelled(), "Grouping cancelled");
        Ok(groups)
    }
}

pub fn edit(
    run: u64,
    automatic: Arc<HashMap<MediaId, usize>>,
    previous: Arc<ManualGroups>,
    media: &[Media],
    edit: Edit,
    cancelled: impl Fn() -> bool,
) -> Result<Resolved> {
    ensure!(!cancelled(), "Grouping cancelled");
    let mut manual = previous.clone();
    match edit {
        Edit::Resolve => {}
        Edit::Replace(replacement) => manual = replacement,
        Edit::RestoreAll => manual = Arc::default(),
        Edit::Restore(selected) => {
            let editable = Arc::make_mut(&mut manual);
            for (i, id) in selected.into_iter().enumerate() {
                if i % 256 == 0 {
                    ensure!(!cancelled(), "Grouping cancelled");
                }
                editable.members.remove(&id);
            }
        }
        Edit::Move { selected, target } => {
            let mut allowed = HashSet::with_capacity(media.len());
            for (i, m) in media.iter().enumerate() {
                if i % 256 == 0 {
                    ensure!(!cancelled(), "Grouping cancelled");
                }
                if m.kind != MediaKind::Video {
                    allowed.insert(m.id);
                }
            }
            let mut members: HashSet<_> = selected
                .into_iter()
                .filter(|id| allowed.contains(id))
                .collect();
            ensure!(
                !members.is_empty(),
                "Select at least one photo; videos are not similarity-grouped"
            );
            if let Some(target) = target {
                let current = previous.resolve(&automatic, &cancelled)?;
                let mut found = false;
                for (i, (id, group)) in current.into_iter().enumerate() {
                    if i % 256 == 0 {
                        ensure!(!cancelled(), "Grouping cancelled");
                    }
                    if group == target && allowed.contains(&id) {
                        found = true;
                        members.insert(id);
                    }
                }
                ensure!(found, "That group is no longer available");
            }
            // A new token pins the exact resulting cohort, including the target's
            // existing members. Later AI results cannot silently expand it.
            let editable = Arc::make_mut(&mut manual);
            let token = editable
                .members
                .values()
                .copied()
                .max()
                .unwrap_or(0)
                .checked_add(1)
                .ok_or_else(|| anyhow::anyhow!("Too many manual groups"))?;
            for (i, id) in members.into_iter().enumerate() {
                if i % 256 == 0 {
                    ensure!(!cancelled(), "Grouping cancelled");
                }
                editable.members.insert(id, token);
            }
        }
    }
    let groups = manual.resolve(&automatic, &cancelled)?;
    let changed = !Arc::ptr_eq(&manual, &previous) && manual != previous;
    ensure!(!cancelled(), "Grouping cancelled");
    let choices = group_choices(&groups, &cancelled)?;
    Ok(Resolved {
        changed,
        run,
        automatic,
        previous,
        manual,
        groups,
        choices,
    })
}

/// Index once on the worker; menus render only visible target rows.
pub fn group_choices(
    groups: &HashMap<MediaId, usize>,
    cancelled: impl Fn() -> bool,
) -> Result<Vec<(usize, usize, MediaId)>> {
    let mut choices = BTreeMap::<usize, (usize, MediaId)>::new();
    for (i, (&id, &group)) in groups.iter().enumerate() {
        if i % 256 == 0 {
            ensure!(!cancelled(), "Grouping cancelled");
        }
        let entry = choices.entry(group).or_insert((0, id));
        entry.0 += 1;
        if id.0 < entry.1.0 {
            entry.1 = id;
        }
    }
    ensure!(!cancelled(), "Grouping cancelled");
    Ok(choices
        .into_iter()
        .map(|(group, (count, example))| (group, count, example))
        .collect())
}

/// Apply only to successfully analyzed photos. Missing evidence must not become
/// a movable photo merely because it has saved membership from an earlier pass.
pub fn resolve_scenes(
    automatic: &HashMap<MediaId, usize>,
    manual: &ManualGroups,
    cancelled: impl Fn() -> bool,
) -> Result<HashMap<MediaId, usize>> {
    let mut groups = manual.resolve(automatic, &cancelled)?;
    groups.retain(|id, _| automatic.contains_key(id));
    ensure!(!cancelled(), "Grouping cancelled");
    Ok(groups)
}
