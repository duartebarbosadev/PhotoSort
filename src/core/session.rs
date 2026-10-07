use super::{metadata::atomic_write, model::*};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

pub fn data_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("PhotoSort-Rust")
}
pub fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("PhotoSort-Rust")
        .join("proxy-v1")
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub manual_groups: std::sync::Arc<super::manual_groups::ManualGroups>,
    /// General scene membership is independent of Cull/Pick Best subject groups.
    #[serde(default)]
    pub scene_groups: std::sync::Arc<super::manual_groups::ManualGroups>,
    pub decisions: HashMap<MediaId, Decision>,
    pub fingerprints: HashMap<MediaId, Fingerprint>,
}
fn session_path(root: &Path) -> PathBuf {
    // Equivalent trailing separators must not create different session stores.
    let root: PathBuf = root.components().collect();
    data_dir().join("sessions").join(format!(
        "{}.json",
        blake3::hash(root.as_os_str().as_encoded_bytes()).to_hex()
    ))
}
impl Session {
    pub fn manual_group_ids(&self) -> impl Iterator<Item = MediaId> + '_ {
        self.manual_groups.ids().chain(self.scene_groups.ids())
    }
    pub fn load(root: &Path) -> Result<Self> {
        let p = session_path(root);
        if !p.exists() {
            return Ok(Self::default());
        }
        Ok(serde_json::from_slice(&std::fs::read(p)?)?)
    }
    pub fn save(&self, root: &Path) -> Result<()> {
        atomic_write(&session_path(root), &serde_json::to_vec(self)?)
    }
    /// Migrate review intent using the executor's verified results, not guessed
    /// paths or new filesystem reads. Moves do not apply ratings/Trash decisions.
    pub fn apply_report(&mut self, report: &super::operations::Report, media: &[Media]) {
        let decisions_applied = report.applied_decisions(media);
        for id in decisions_applied {
            self.decisions.remove(&id);
        }
        self.apply_verified_identities(&report.relocated);
    }
    /// Adopt executor-verified identities while retaining unrelated staged edits.
    pub fn apply_verified_identities(&mut self, changes: &[(MediaId, MediaId, Fingerprint)]) {
        for groups in [&mut self.manual_groups, &mut self.scene_groups] {
            std::sync::Arc::make_mut(groups).relocate(changes);
        }
        remap_media_ids(&mut self.decisions, changes);
        // Fingerprints come from the executor, not from the old entries. Remove
        // every old key first so a later source cannot erase a newly inserted key.
        for (old, _, _) in changes {
            self.fingerprints.remove(old);
        }
        self.fingerprints.extend(
            changes
                .iter()
                .map(|(_, new, fingerprint)| (*new, *fingerprint)),
        );
    }
    /// Preserve existing destination-library decisions while adding relocated
    /// sources. Fingerprints are reconciled by the destination scan afterwards.
    pub fn merge(&mut self, other: Self) {
        for (groups, incoming) in [
            (&mut self.manual_groups, &other.manual_groups),
            (&mut self.scene_groups, &other.scene_groups),
        ] {
            std::sync::Arc::make_mut(groups).merge(incoming);
        }
        self.decisions.extend(other.decisions);
        self.fingerprints.extend(other.fingerprints);
    }
    pub fn reconcile(&mut self, media: &[Media]) {
        self.reconcile_files(media, &HashMap::new());
    }
    pub fn reconcile_catalog(&mut self, catalog: &super::catalog::Catalog) {
        self.reconcile_files(&catalog.media, &catalog.supporting_fingerprints);
    }
    fn reconcile_files(&mut self, media: &[Media], supporting: &HashMap<MediaId, Fingerprint>) {
        let mut fingerprints: HashMap<_, _> = media.iter().map(|m| (m.id, m.fingerprint)).collect();
        for groups in [&mut self.manual_groups, &mut self.scene_groups] {
            std::sync::Arc::make_mut(groups).retain(|id| {
                self.fingerprints
                    .get(&id)
                    .zip(fingerprints.get(&id))
                    .is_some_and(|(a, b)| a == b)
            });
        }
        fingerprints.extend(
            supporting
                .iter()
                .map(|(id, fingerprint)| (*id, *fingerprint)),
        );
        self.decisions.retain(|id, decision| {
            if supporting.contains_key(id) {
                decision.rating = None;
                decision.turns = 0;
            }
            self.fingerprints
                .get(id)
                .zip(fingerprints.get(id))
                .is_some_and(|(saved, current)| {
                    // Migrate pre-identity decision sessions without silently discarding choices.
                    // New view bookmarks always require the complete identity snapshot.
                    saved == current
                        || (saved.identity == [0; 4]
                            && saved.bytes == current.bytes
                            && saved.modified_ns == current.modified_ns)
                })
        });
        self.fingerprints = fingerprints;
    }
}
