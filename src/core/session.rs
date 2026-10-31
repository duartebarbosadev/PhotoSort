use super::{metadata::atomic_write, model::*};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

/// Application data (preferences, sessions, saved views, models). Set
/// `PHOTOSORT_DATA_ROOT` to relocate it, e.g. for portable or isolated runs.
pub fn data_dir() -> PathBuf {
    root_override("PHOTOSORT_DATA_ROOT")
        .or_else(|| isolated_root("data"))
        .unwrap_or_else(|| {
            dirs::data_local_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("PhotoSort-Rust")
        })
}
/// Regenerable caches. `PHOTOSORT_CACHE_ROOT` relocates them.
pub fn cache_dir() -> PathBuf {
    root_override("PHOTOSORT_CACHE_ROOT")
        .or_else(|| isolated_root("cache"))
        .unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(std::env::temp_dir)
                .join("PhotoSort-Rust")
        })
        .join("proxy-v1")
}
/// Tests and examples never touch the user's real data or caches: each
/// process gets its own temporary root (in the system temp folder).
fn isolated_root(kind: &str) -> Option<PathBuf> {
    if !cfg!(any(test, feature = "test-isolation")) {
        return None;
    }
    static ROOT: std::sync::OnceLock<Option<PathBuf>> = std::sync::OnceLock::new();
    // Statics are never dropped, so the folder is removed when the process exits.
    // nextest runs every test in its own process, which would otherwise leave
    // one folder behind per test.
    extern "C" fn remove_root() {
        if let Some(Some(path)) = ROOT.get() {
            let _ = std::fs::remove_dir_all(path);
        }
    }
    unsafe extern "C" {
        fn atexit(callback: extern "C" fn()) -> std::ffi::c_int;
    }
    let root = ROOT.get_or_init(|| {
        let dir = tempfile::Builder::new()
            .prefix("photosort-test-")
            .tempdir()
            .ok()?;
        // SAFETY: registers a plain function with the C runtime, once per process.
        unsafe { atexit(remove_root) };
        Some(dir.keep())
    });
    // Journals refuse symlinked paths; macOS temp lives under the /var link.
    let path = root.as_deref()?;
    Some(
        path.canonicalize()
            .unwrap_or_else(|_| path.to_path_buf())
            .join(kind),
    )
}
/// Only absolute paths are honoured, so a stray relative value cannot scatter
/// state into whatever the working directory happens to be.
fn root_override(name: &str) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os(name)?);
    path.is_absolute().then_some(path)
}
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Session {
    #[serde(default)]
    pub manual_groups: std::sync::Arc<super::manual_groups::ManualGroups>,
    /// General scene membership is independent of Cull/Pick Best subject groups.
    #[serde(default)]
    pub scene_groups: std::sync::Arc<super::manual_groups::ManualGroups>,
    #[serde(default)]
    pub organize: Option<super::organize::Intent>,
    pub decisions: HashMap<MediaId, Decision>,
    pub fingerprints: HashMap<MediaId, Fingerprint>,
    /// Photos whose Easy Delete suggestion was already staged. A later analysis
    /// never stages it again, so a photo the user chose to keep stays kept.
    /// Discarding staged choices forgets it, so the suggestions return.
    #[serde(default, rename = "staged_suggestions")]
    pub suggested: HashSet<MediaId>,
    /// The same for Pick Best's suggestions, kept apart: a photo Easy Delete
    /// kept can still be one Pick Best suggests moving to Trash.
    #[serde(default, rename = "staged_pick_suggestions")]
    pub pick_suggested: HashSet<MediaId>,
    /// Photos whose Fix Rotation suggestion was already staged, kept apart
    /// from Easy Delete's: one photo can be a suggestion in both steps.
    #[serde(default, rename = "staged_rotations")]
    pub rotations_staged: HashSet<MediaId>,
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
        if report.remaining.output_root.is_some() && report.remaining.actions.is_empty() {
            self.organize = None;
        }
    }
    /// Adopt executor-verified identities while retaining unrelated staged edits.
    pub fn apply_verified_identities(&mut self, changes: &[(MediaId, MediaId, Fingerprint)]) {
        if let Some(intent) = &mut self.organize {
            intent.relocate(changes);
        }
        for groups in [&mut self.manual_groups, &mut self.scene_groups] {
            std::sync::Arc::make_mut(groups).relocate(changes);
        }
        remap_media_ids(&mut self.decisions, changes);
        for (old, new, _) in changes {
            for staged in [
                &mut self.suggested,
                &mut self.pick_suggested,
                &mut self.rotations_staged,
            ] {
                if staged.remove(old) {
                    staged.insert(*new);
                }
            }
        }
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
        self.suggested.extend(other.suggested);
        self.pick_suggested.extend(other.pick_suggested);
        self.rotations_staged.extend(other.rotations_staged);
        self.fingerprints.extend(other.fingerprints);
    }
    pub fn reconcile(&mut self, media: &[Media]) {
        self.reconcile_files(media, &HashMap::new());
    }
    pub fn reconcile_catalog(&mut self, catalog: &super::catalog::Catalog) {
        self.reconcile_files(&catalog.media, &catalog.supporting_fingerprints);
        if let Some(intent) = &mut self.organize {
            intent.reconcile(catalog);
        }
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
        // An edited file is a new photo: its suggestion may be staged again.
        for staged in [
            &mut self.suggested,
            &mut self.pick_suggested,
            &mut self.rotations_staged,
        ] {
            staged.retain(|id| {
                self.fingerprints
                    .get(id)
                    .zip(fingerprints.get(id))
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
