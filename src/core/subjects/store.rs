//! Shared model evidence, keyed by source fingerprint and model recipe.
use super::{DenseEmbedding, Evidence, Pair};
use crate::core::cache_record as codec;
use crate::core::faces::Faces;
use anyhow::Result;
use codec::Kind;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs::{self, File},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Serialize, Deserialize)]
pub struct Features {
    pub dense: DenseEmbedding,
    pub faces: Option<Faces>,
    pub identities: Option<Vec<Evidence>>,
}
impl Features {
    fn valid(&self) -> bool {
        let vector = |v: &[f32]| v.len() == 384 && v.iter().all(|v| v.is_finite());
        let evidence = |e: &Evidence| {
            vector(&e.descriptor) && e.bbox.iter().all(|v| v.is_finite()) && e.area.is_finite()
        };
        vector(&self.dense.global)
            && self.dense.regions.len() == 16
            && self.dense.regions.iter().all(evidence)
            && self
                .identities
                .as_ref()
                .is_none_or(|v| v.len() <= 10 && v.iter().all(evidence))
            && self
                .faces
                .as_ref()
                .is_none_or(|f| f.count == f.geometry.len() && f.count <= 10)
            && self
                .identities
                .as_ref()
                .is_none_or(|v| self.faces.as_ref().is_some_and(|f| f.count == v.len()))
    }
}
/// Every verdict one photo has with the photos after it in library order,
/// kept in one record. A library has about thirty candidate pairs per photo
/// and a burst many more; one file per pair meant millions of tiny files.
pub struct Verdicts<T> {
    key: String,
    entries: HashMap<[u8; 32], T>,
    used: HashSet<[u8; 32]>,
    /// How many verdicts this run did not ask for are written again.
    kept_unused: usize,
    changed: bool,
}
impl<T> Verdicts<T> {
    /// Verdicts of other runs kept when the record is written again, so a
    /// subfolder and its parent reuse each other's work without the record
    /// growing with every photo that ever stood next to this one.
    const KEPT_UNUSED: usize = 256;
    /// The most one record holds, well inside the cache record size limit.
    /// A photo with more partners than this has the rest worked out again.
    const KEPT: usize = 16_384;
    pub fn get(&mut self, pair: &[u8; 32]) -> Option<&T> {
        let value = self.entries.get(pair)?;
        self.used.insert(*pair);
        Some(value)
    }
    pub fn insert(&mut self, pair: [u8; 32], value: T) {
        self.entries.insert(pair, value);
        self.used.insert(pair);
        self.changed = true;
    }
    /// Every verdict the record holds; none of them counts as asked for.
    pub fn iter(&self) -> impl Iterator<Item = (&[u8; 32], &T)> {
        self.entries.iter()
    }
}

/// The folder of the cache that holds every working set, one subfolder each.
/// Cache maintenance only looks at the cache's own files, so it neither
/// counts nor trims what is in here.
const WORKING: &str = "working-sets";
const SET_PREFIX: &str = "set-";
/// Locked by the store that owns a working set for as long as it lives. The
/// system releases the lock when the process ends, however it ends, so a set
/// whose lease can be locked belongs to nobody.
const LEASE: &str = ".lease";

/// Held while a working set is created or judged stale, so a set that exists
/// always has its lease taken. None where the folder has no file locks.
fn namespace(sets: &Path) -> Option<File> {
    let file = File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(sets.join(".lock"))
        .ok()?;
    file.lock().ok()?;
    Some(file)
}

/// Removes a working set: its records, then the lease, then the folder. A
/// working set is flat, so nothing is followed and no folder inside is touched.
fn remove_working(path: &Path) {
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if entry.file_name() != LEASE {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    let _ = fs::remove_file(path.join(LEASE));
    let _ = fs::remove_dir(path);
}

/// Removes the working sets left in the cache `root` by a PhotoSort that
/// crashed or was killed, and returns how many. A set another running
/// PhotoSort (or this one) still uses keeps its lease locked and is left alone.
pub(crate) fn remove_stale_working_sets(root: &Path) -> usize {
    let sets = root.join(WORKING);
    // Never through a link: only real folders with a real lease are removed.
    let real = |path: &Path, dir: bool| {
        fs::symlink_metadata(path).is_ok_and(|m| if dir { m.is_dir() } else { m.is_file() })
    };
    if !root.is_absolute() || !real(&sets, true) {
        return 0;
    }
    let Some(guard) = namespace(&sets) else {
        return 0;
    };
    let stale: Vec<(PathBuf, File)> = fs::read_dir(&sets)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let named = entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with(SET_PREFIX));
            if !named || !real(&path, true) || !real(&path.join(LEASE), false) {
                return None;
            }
            let lease = File::options()
                .read(true)
                .write(true)
                .open(path.join(LEASE))
                .ok()?;
            lease.try_lock().ok()?;
            Some((path, lease))
        })
        .collect();
    drop(guard);
    for (path, _lease) in &stale {
        remove_working(path);
    }
    stale.len()
}

/// The same on its own thread: a large library leaves hundreds of thousands
/// of records, and nothing at startup waits for them.
pub(crate) fn sweep_stale_working_sets(root: PathBuf) {
    let _ = std::thread::Builder::new()
        .name("photosort-working-sweep".into())
        .spawn(move || {
            let removed = remove_stale_working_sets(&root);
            if removed > 0 {
                tracing::info!(removed, "removed working sets of an earlier run");
            }
        });
}

/// Immutable per-key snapshots in a working set protect an active job from
/// persistent-cache trimming. The working set is a folder inside the cache,
/// so a persisted record is pinned by a hard link and takes no more disk.
/// The last owner releases it, on a thread of its own: removing the records
/// of a large library takes long enough to keep a folder switch or quit
/// waiting. What a crash leaves behind is removed at the next start.
pub struct Store {
    root: PathBuf,
    working: PathBuf,
    lease: Option<File>,
    persistent: AtomicBool,
}
impl Drop for Store {
    fn drop(&mut self) {
        let working = std::mem::take(&mut self.working);
        let lease = self.lease.take();
        let path = working.clone();
        // The lease stays locked until the records are gone, so a starting
        // PhotoSort does not remove the same set a second time.
        let remove = move || {
            remove_working(&path);
            drop(lease);
        };
        if let Err(error) = std::thread::Builder::new()
            .name("photosort-working-release".into())
            .spawn(remove)
        {
            // Left for the next start, which finds the lease unlocked.
            tracing::debug!(%error, path = ?working, "working set not released");
        }
    }
}
impl Store {
    pub fn new(root: PathBuf, persistent: bool) -> Result<Self> {
        // A store without a cache folder of its own (nothing is persisted
        // there either) keeps its working set in the system's temporary folder.
        let (guard, working) = if root.is_absolute() {
            let sets = root.join(WORKING);
            fs::create_dir_all(&sets)?;
            let guard = namespace(&sets);
            let working = tempfile::Builder::new()
                .prefix(SET_PREFIX)
                .tempdir_in(&sets)?
                .keep();
            (guard, working)
        } else {
            (None, tempfile::tempdir()?.keep())
        };
        let lease = File::create(working.join(LEASE))
            .ok()
            .filter(|lease| lease.try_lock().is_ok());
        drop(guard);
        Ok(Self {
            root,
            working,
            lease,
            persistent: AtomicBool::new(persistent),
        })
    }
    pub fn key(source: &str, models: &str) -> String {
        blake3::hash(format!("subject-features-v1:{source}:{models}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn persistent(&self, enabled: bool) {
        self.persistent.store(enabled, Ordering::Release);
    }
    fn path(&self, key: &str) -> Result<PathBuf> {
        anyhow::ensure!(
            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid evidence key"
        );
        Ok(self.working.join(format!("{key}.bin")))
    }
    fn read<T: serde::de::DeserializeOwned + Serialize>(
        &self,
        key: &str,
        kind: Kind,
        valid: impl Fn(&T) -> bool,
    ) -> Option<T> {
        self.read_pinned(key, kind, valid, true)
    }
    /// `pin` keeps a persisted record in this run's working set so later stages
    /// can read it again even if cache maintenance prunes it meanwhile.
    fn read_pinned<T: serde::de::DeserializeOwned + Serialize>(
        &self,
        key: &str,
        kind: Kind,
        valid: impl Fn(&T) -> bool,
        pin: bool,
    ) -> Option<T> {
        let working = self.path(key).ok()?;
        let persistent = self.persistent.load(Ordering::Acquire);
        let disk = self.root.join(format!("{key}.bin"));
        // A corrupt record is a miss.
        let path = if working.try_exists().ok()? {
            working.clone()
        } else if persistent && disk.try_exists().ok()? {
            disk
        } else {
            return None;
        };
        let bytes = codec::read(&path)?;
        let value = codec::decode(kind, &bytes)?;
        if !valid(&value) {
            return None;
        }
        if pin && path != working {
            self::pin(&path, &working, &bytes).ok()?;
        }
        Some(value)
    }
    pub fn get(&self, key: &str) -> Option<Features> {
        self.read(key, Kind::Subjects, Features::valid)
    }
    pub fn measurements_key(source: &str) -> String {
        blake3::hash(
            format!("measurements-v2-neutral1024-area640-luma-tiles-source-size:{source}")
                .as_bytes(),
        )
        .to_hex()
        .to_string()
    }
    pub fn measurements(&self, key: &str) -> Option<crate::core::analysis::Measurements> {
        self.read(
            key,
            Kind::Measurements,
            crate::core::analysis::Measurements::valid,
        )
    }
    pub fn put_measurements(
        &self,
        key: &str,
        value: &crate::core::analysis::Measurements,
    ) -> Result<()> {
        anyhow::ensure!(value.valid(), "Invalid image measurements");
        self.write(key, Kind::Measurements, value)
    }
    pub fn rotation_key(source: &str, model: &str) -> String {
        blake3::hash(
            format!(
                "rotation-v3-neutral2048-triangle416-crop384-imagenet-consensus021:{source}:{model}"
            )
            .as_bytes(),
        )
        .to_hex()
        .to_string()
    }
    pub fn rotation(&self, key: &str) -> Option<crate::core::models::RotationSuggestion> {
        self.read(
            key,
            Kind::Rotation,
            crate::core::models::RotationSuggestion::valid,
        )
    }
    pub fn put_rotation(
        &self,
        key: &str,
        value: &crate::core::models::RotationSuggestion,
    ) -> Result<()> {
        anyhow::ensure!(value.valid(), "Invalid rotation evidence");
        self.write(key, Kind::Rotation, value)
    }
    pub fn aesthetic_key(source: &str, model: &str) -> String {
        blake3::hash(format!("aesthetic-v1-neutral1024:{source}:{model}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn aesthetic(&self, key: &str) -> Option<f32> {
        self.read(key, Kind::Aesthetic, |v| (0.0..=1.0).contains(v))
    }
    pub fn put_aesthetic(&self, key: &str, value: f32) -> Result<()> {
        anyhow::ensure!((0.0..=1.0).contains(&value), "Invalid aesthetic score");
        self.write(key, Kind::Aesthetic, &value)
    }
    /// One pair inside the first photo's `pairs` record.
    pub fn pair_id(a: &str, b: &str, ta: Option<i64>, tb: Option<i64>) -> [u8; 32] {
        *blake3::hash(format!("subject-pair-v2:{a}:{b}:{ta:?}:{tb:?}").as_bytes()).as_bytes()
    }
    /// The same-subject verdicts `first` has with later photos; empty when
    /// none were kept. Read once per grouping and then held in memory, so the
    /// record is not pinned.
    pub fn pairs(&self, first: &str) -> Verdicts<Pair> {
        self.verdicts(
            format!("subject-pairs-v2:{first}"),
            Kind::Pairs,
            Self::valid_pair,
            Verdicts::<Pair>::KEPT_UNUSED,
        )
    }
    fn valid_pair(p: &Pair) -> bool {
        [
            p.subject_similarity,
            p.scene_similarity,
            p.confidence,
            p.geometry.inlier_ratio,
            p.geometry.coverage_a,
            p.geometry.coverage_b,
        ]
        .iter()
        .all(|v| v.is_finite())
    }
    /// The verdicts `first` has with later photos that were not among its
    /// nearest: judged when two groups could merge only if they were
    /// accepted. A record of their own, written with `put_pairs`, so a long
    /// burst does not make every later run read them for the nearest ones.
    /// It is read, added to and written again within one run, so it keeps
    /// every verdict it holds.
    pub fn merge_pairs(&self, first: &str) -> Verdicts<Pair> {
        self.verdicts(
            format!("subject-merge-pairs-v2:{first}"),
            Kind::Pairs,
            Self::valid_pair,
            Verdicts::<Pair>::KEPT,
        )
    }
    pub fn put_pairs(&self, verdicts: Verdicts<Pair>) -> Result<()> {
        self.put_verdicts(Kind::Pairs, verdicts)
    }
    fn verdicts<T: serde::de::DeserializeOwned + Serialize>(
        &self,
        name: String,
        kind: Kind,
        valid: impl Fn(&T) -> bool,
        kept_unused: usize,
    ) -> Verdicts<T> {
        let key = blake3::hash(name.as_bytes()).to_hex().to_string();
        let entries = self
            .read_pinned(
                &key,
                kind,
                |entries: &Vec<([u8; 32], T)>| entries.iter().all(|(_, value)| valid(value)),
                false,
            )
            .unwrap_or_default();
        Verdicts {
            key,
            entries: entries.into_iter().collect(),
            used: HashSet::new(),
            kept_unused,
            changed: false,
        }
    }
    /// Writes the record when a verdict was added. It goes to the persistent
    /// cache alone, or to the working set while nothing is persisted.
    fn put_verdicts<T: Serialize>(&self, kind: Kind, verdicts: Verdicts<T>) -> Result<()> {
        if !verdicts.changed {
            return Ok(());
        }
        let Verdicts {
            key,
            entries,
            used,
            kept_unused,
            ..
        } = verdicts;
        // This run's verdicts first, then a bounded number of the others.
        let mut entries: Vec<_> = entries.into_iter().collect();
        entries.sort_unstable_by_key(|(pair, _)| (!used.contains(pair), *pair));
        let mut unused = 0;
        entries.retain(|(pair, _)| {
            used.contains(pair) || {
                unused += 1;
                unused <= kept_unused
            }
        });
        entries.truncate(Verdicts::<T>::KEPT);
        let bytes = codec::encode(kind, &entries)?;
        let working = self.path(&key)?;
        if self.persistent.load(Ordering::Acquire) {
            let _ = std::fs::create_dir_all(&self.root);
            if codec::write(&self.root.join(format!("{key}.bin")), &bytes).is_ok() {
                // An older copy from a time without persistence would be read first.
                let _ = std::fs::remove_file(working);
                return Ok(());
            }
        }
        codec::write(&working, &bytes)
    }
    pub fn regional_key(subject_key: &str) -> String {
        blake3::hash(format!("general-regions-v2-batched-five-crops:{subject_key}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn regional(&self, key: &str) -> Option<crate::core::regional::Embedding> {
        self.read(
            key,
            Kind::Regional,
            crate::core::regional::Embedding::valid_model_output,
        )
    }
    pub fn put_regional(&self, key: &str, value: &crate::core::regional::Embedding) -> Result<()> {
        anyhow::ensure!(
            value.valid_model_output(),
            "Invalid general regional embeddings"
        );
        self.write(key, Kind::Regional, value)
    }
    pub fn face_key(source: &str, models: &str) -> String {
        blake3::hash(format!("verified-faces-v1:{source}:{models}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn faces(&self, key: &str) -> Option<Faces> {
        self.read(key, Kind::Faces, Faces::complete)
    }
    pub fn put_faces(&self, key: &str, value: &Faces) -> Result<()> {
        anyhow::ensure!(value.complete(), "Invalid verified face evidence");
        self.write(key, Kind::Faces, value)
    }
    pub fn eyes_key(source: &str, recipe: &str) -> String {
        blake3::hash(format!("people-eyes:{source}:{recipe}").as_bytes())
            .to_hex()
            .to_string()
    }
    pub fn eyes(&self, key: &str) -> Option<Vec<crate::core::eyes::Person>> {
        self.read(
            key,
            Kind::Eyes,
            |people: &Vec<crate::core::eyes::Person>| {
                people.iter().all(crate::core::eyes::Person::valid)
            },
        )
    }
    pub fn put_eyes(&self, key: &str, value: &[crate::core::eyes::Person]) -> Result<()> {
        anyhow::ensure!(
            value.iter().all(crate::core::eyes::Person::valid),
            "Invalid eye evidence"
        );
        self.write(key, Kind::Eyes, &value)
    }
    pub fn recognition_key(face_key: &str, models: &str) -> String {
        blake3::hash(
            format!("face-recognition-v3-sface112-all-main60:{face_key}:{models}").as_bytes(),
        )
        .to_hex()
        .to_string()
    }
    pub fn recognition(&self, key: &str) -> Option<crate::core::face_groups::Recognition> {
        self.read(
            key,
            Kind::Recognition,
            crate::core::face_groups::Recognition::valid,
        )
    }
    pub fn put_recognition(
        &self,
        key: &str,
        value: &crate::core::face_groups::Recognition,
    ) -> Result<()> {
        anyhow::ensure!(value.valid(), "Invalid face recognition");
        self.write(key, Kind::Recognition, value)
    }
    /// One pair inside the first photo's `duplicates` record.
    pub fn duplicate_id(a: &str, b: &str) -> [u8; 32] {
        *blake3::hash(format!("duplicate-pair-v3-analysis1024-rgb15:{a}:{b}").as_bytes()).as_bytes()
    }
    /// The duplicate assessments `first` has with later photos.
    pub fn duplicates(&self, first: &str) -> Verdicts<crate::core::near_duplicates::Assessment> {
        self.verdicts(
            format!("duplicate-pairs-v3-analysis1024-rgb15:{first}"),
            Kind::Duplicates,
            |_| true,
            Verdicts::<crate::core::near_duplicates::Assessment>::KEPT_UNUSED,
        )
    }
    pub fn put_duplicates(
        &self,
        verdicts: Verdicts<crate::core::near_duplicates::Assessment>,
    ) -> Result<()> {
        self.put_verdicts(Kind::Duplicates, verdicts)
    }
    pub fn put(&self, key: &str, value: &Features) -> Result<()> {
        anyhow::ensure!(value.valid(), "Invalid extracted subject evidence");
        self.write(key, Kind::Subjects, value)
    }
    /// Writes the record once, to the persistent cache, and pins it in the
    /// working set; without persistence (or when the cache cannot be written)
    /// the working set holds the only copy.
    fn write(&self, key: &str, kind: Kind, value: &impl Serialize) -> Result<()> {
        let bytes = codec::encode(kind, value)?;
        let working = self.path(key)?;
        if self.persistent.load(Ordering::Acquire) {
            let disk = self.root.join(format!("{key}.bin"));
            if codec::write(&disk, &bytes).is_ok() {
                // An older snapshot would be read first.
                let _ = fs::remove_file(&working);
                // Unpinned, the record is still read from the cache.
                let _ = pin(&disk, &working, &bytes);
                return Ok(());
            }
        }
        codec::write(&working, &bytes)
    }
}

/// Keep a persisted record available for the rest of this run even if cache
/// maintenance prunes it. Records are only ever replaced by rename, never edited
/// in place, so a hard link pins a complete record without copying it (a racing
/// replacement can only substitute another complete record, and every read is
/// validated). A cache folder without hard links falls back to a copy.
fn pin(persisted: &std::path::Path, working: &std::path::Path, bytes: &[u8]) -> Result<()> {
    match std::fs::hard_link(persisted, working) {
        Ok(()) => Ok(()),
        // Another worker pinned the same record first.
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
        Err(_) => codec::write(working, bytes),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::subjects::Geometry;
    use std::path::Path;

    fn pair() -> Pair {
        Pair {
            subject_similarity: 0.9,
            face_similarity: None,
            scene_similarity: 0.8,
            complete: true,
            geometry: Geometry::default(),
            time_delta_seconds: None,
            confidence: 0.7,
            accepted: true,
            reason: String::new(),
        }
    }

    /// Released working sets are removed on their own thread.
    fn gone(path: &Path) -> bool {
        let deadline = std::time::Instant::now() + crate::test_support::worker_patience();
        while path.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        !path.exists()
    }

    #[test]
    fn reopened_pairs_are_read_without_pinning_but_other_evidence_is_pinned() {
        let root = tempfile::tempdir().unwrap();
        let (first, score_key) = ("a".repeat(64), "c".repeat(64));
        let ids = [0, 1, 2].map(|n| Store::pair_id(&first, &n.to_string(), None, None));
        let store = Store::new(root.path().into(), true).unwrap();
        let mut pairs = store.pairs(&first);
        assert!(pairs.get(&ids[0]).is_none());
        pairs.insert(ids[0], pair());
        pairs.insert(ids[1], pair());
        store.put_pairs(pairs).unwrap();
        store.put_aesthetic(&score_key, 0.5).unwrap();
        // The working set is a folder of the cache, never the system's
        // temporary folder, and goes when its store does.
        let released = store.working.clone();
        assert!(released.starts_with(root.path().join(WORKING)));
        drop(store);
        assert!(gone(&released));
        // One record for all of a photo's pairs, in the persistent cache only.
        let records = || {
            std::fs::read_dir(root.path())
                .unwrap()
                .filter(|entry| entry.as_ref().unwrap().file_type().unwrap().is_file())
                .count()
        };
        assert_eq!(records(), 2);

        let reopened = Store::new(root.path().into(), true).unwrap();
        let mut pairs = reopened.pairs(&first);
        assert!(pairs.get(&ids[0]).is_some() && pairs.get(&ids[2]).is_none());
        assert_eq!(reopened.aesthetic(&score_key), Some(0.5));
        // The pinned score next to the lease.
        assert_eq!(std::fs::read_dir(&reopened.working).unwrap().count(), 2);
        assert!(reopened.path(&score_key).unwrap().exists());
        // Nothing new: nothing is written. A new verdict joins the others,
        // also the one this run did not ask for.
        reopened.put_pairs(pairs).unwrap();
        let mut pairs = reopened.pairs(&first);
        pairs.insert(ids[2], pair());
        reopened.put_pairs(pairs).unwrap();
        assert_eq!(records(), 2);
        let mut pairs = reopened.pairs(&first);
        assert!(ids.iter().all(|id| pairs.get(id).is_some()));
        // Without persistence the working set holds the record for this run.
        reopened.persistent(false);
        let mut private = reopened.pairs(&"b".repeat(64));
        private.insert(ids[0], pair());
        reopened.put_pairs(private).unwrap();
        assert_eq!(records(), 2);
        assert!(reopened.pairs(&"b".repeat(64)).get(&ids[0]).is_some());
        // Clearing the cache mid-run neither counts nor removes what is
        // pinned, and a record written since is pinned as well.
        reopened.persistent(true);
        let written = "d".repeat(64);
        reopened.put_aesthetic(&written, 0.25).unwrap();
        let cleared = crate::core::cache::maintain(root.path(), Some(0), || false).unwrap();
        assert_eq!((cleared.disk_bytes, records()), (0, 0));
        assert_eq!(reopened.aesthetic(&score_key), Some(0.5));
        assert_eq!(reopened.aesthetic(&written), Some(0.25));
    }

    #[test]
    fn a_dead_runs_working_set_is_removed_at_startup_and_a_live_one_is_kept() {
        let root = tempfile::tempdir().unwrap();
        let key = "c".repeat(64);
        let live = Store::new(root.path().into(), true).unwrap();
        live.put_aesthetic(&key, 0.5).unwrap();
        // What a crash leaves: records and a lease nobody holds.
        let sets = root.path().join(WORKING);
        let dead = sets.join(format!("{SET_PREFIX}dead"));
        std::fs::create_dir(&dead).unwrap();
        std::fs::write(dead.join(LEASE), b"").unwrap();
        std::fs::write(dead.join(format!("{key}.bin")), b"record").unwrap();
        // Not a working set: no lease, or not named like one.
        let (unleased, foreign) = (sets.join(format!("{SET_PREFIX}other")), sets.join("mine"));
        for folder in [&unleased, &foreign] {
            std::fs::create_dir(folder).unwrap();
            std::fs::write(folder.join("keep.bin"), b"keep").unwrap();
        }
        std::fs::write(foreign.join(LEASE), b"").unwrap();

        assert_eq!(remove_stale_working_sets(root.path()), 1);
        assert!(!dead.exists());
        assert!(unleased.join("keep.bin").exists() && foreign.join("keep.bin").exists());
        assert!(live.path(&key).unwrap().exists());
        assert_eq!(live.aesthetic(&key), Some(0.5));
        // Once its owner is gone (here: released without removing), it goes too.
        let abandoned = live.working.clone();
        let mut live = std::mem::ManuallyDrop::new(live);
        drop(live.lease.take());
        assert_eq!(remove_stale_working_sets(root.path()), 1);
        assert!(!abandoned.exists());
    }
}
