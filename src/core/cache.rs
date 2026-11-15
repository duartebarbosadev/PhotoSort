//! Regenerable cache ownership. Never traverses photo libraries or application data.
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
};
pub(crate) mod compiled;
pub const MIB: u64 = 1024 * 1024;
/// Saved only where they differ from this computer's defaults, so a limit the
/// user never changed keeps following the machine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    #[serde(skip_serializing_if = "Limits::default_previews")]
    pub previews_mib: u32,
    #[serde(skip_serializing_if = "Limits::default_originals")]
    pub originals_mib: u32,
    #[serde(skip_serializing_if = "Limits::default_disk")]
    pub disk_mib: u32,
}
/// The disk default when the free space is unknown, and the most a nearly
/// full disk gets.
const DISK_DEFAULT_MIB: u32 = 4096;
pub const PREVIEWS_MIB: std::ops::RangeInclusive<u32> = 64..=8192;
pub const DISK_MAX_MIB: u32 = 131_072;
const ORIGINALS_MIN_MIB: u32 = 256;
const ORIGINALS_MAX_MIB: u32 = 32_768;
const ORIGINALS_DEFAULT_MAX_MIB: u32 = 16_384;
impl Default for Limits {
    fn default() -> Self {
        Self::for_machine(super::memory::Machine::current())
    }
}
/// The power of two nearest `value`, kept within `floor..=ceiling`. Nearest,
/// not lower: a 32 GB computer reports a little under 32 GiB.
fn power_of_two(value: u64, floor: u32, ceiling: u32) -> u32 {
    let lower = if value == 0 { 0 } else { 1u64 << value.ilog2() };
    let rounded = if value - lower >= lower / 2 {
        lower * 2
    } else {
        lower
    };
    rounded.clamp(u64::from(floor), u64::from(ceiling)) as u32
}
impl Limits {
    /// Defaults for a computer: previews 1/16 of memory (128 MB to 4 GB),
    /// full-size detail 1/8 (512 MB to 16 GB) and disk 1/20 of the free space
    /// (4 to 32 GB), or at most a tenth of it when under 40 GB are free.
    ///
    /// On 32 GB that is 2 GB of previews, about 180 photos at 2048 px, and
    /// 4 GB of full-size detail, enough for nine 45 MP RAW files side by side.
    /// Both shrink to a half or a quarter while the system is short of memory
    /// (`memory::Pressure`).
    pub fn for_machine(machine: super::memory::Machine) -> Self {
        let ram = machine.ram / MIB;
        let disk = match machine.free_disk.map(|free| free / MIB) {
            None => DISK_DEFAULT_MIB,
            Some(free) if free < 40 * 1024 => (free / 10).min(u64::from(DISK_DEFAULT_MIB)) as u32,
            Some(free) => (free / 20 / 1024 * 1024).clamp(4096, 32_768) as u32,
        };
        Self {
            previews_mib: power_of_two(ram / 16, 128, 4096),
            originals_mib: power_of_two(ram / 8, 512, ORIGINALS_DEFAULT_MAX_MIB),
            disk_mib: disk,
        }
    }
    fn default_previews(mib: &u32) -> bool {
        *mib == Self::default().previews_mib
    }
    fn default_originals(mib: &u32) -> bool {
        *mib == Self::default().originals_mib
    }
    fn default_disk(mib: &u32) -> bool {
        *mib == Self::default().disk_mib
    }
    /// The most full-size detail memory Settings offers: half the memory.
    pub fn originals_max_mib() -> u32 {
        ((super::memory::Machine::current().ram / MIB / 2) as u32)
            .clamp(ORIGINALS_MIN_MIB, ORIGINALS_MAX_MIB)
    }
    pub fn originals_range() -> std::ops::RangeInclusive<u32> {
        ORIGINALS_MIN_MIB..=Self::originals_max_mib()
    }
    pub fn normalize(&mut self) {
        self.previews_mib = self
            .previews_mib
            .clamp(*PREVIEWS_MIB.start(), *PREVIEWS_MIB.end());
        self.originals_mib = self
            .originals_mib
            .clamp(ORIGINALS_MIN_MIB, ORIGINALS_MAX_MIB);
        self.disk_mib = self.disk_mib.min(DISK_MAX_MIB);
    }
}
#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub disk_bytes: u64,
    pub pinned_disk_bytes: u64,
    /// Of `disk_bytes`, the AI models prepared for this computer. They are
    /// kept outside the limit, so the rest is what the limit bounds.
    pub model_bytes: u64,
    pub files: usize,
    pub preview_bytes: usize,
    pub original_bytes: usize,
    pub metadata_entries: usize,
    pub analysis_entries: usize,
    /// Bytes a trim removed that were written after the `since` it was given.
    pub evicted_recent_bytes: u64,
}
#[derive(Clone, Debug, Default)]
pub struct Status {
    pub busy: bool,
    pub usage: Option<Usage>,
    pub message: Option<String>,
}
#[derive(Clone, Copy, Default)]
pub(crate) struct Request {
    pub limits: Limits,
    pub clear: bool,
    cancellation: u64,
}
#[derive(Default)]
struct State {
    limits: Limits,
    pending: Option<Request>,
    status: Status,
}
/// Independent of library generations: accepted maintenance survives a folder switch.
/// Coalesced requests cannot fill image queues or block the UI.
#[derive(Clone, Default)]
pub struct Control {
    directory: Option<Arc<PathBuf>>,
    state: Arc<Mutex<State>>,
    cancelled: Arc<AtomicU64>,
    writes: Arc<Writes>,
}
/// Disk cache bytes every pipeline of a runtime (the image worker and the
/// preview builder) wrote in this session, the photos whose previews they
/// made, and how many of those bytes trims already removed again.
#[derive(Debug, Default)]
pub struct Writes {
    pub written: AtomicU64,
    pub photos: AtomicU64,
    pub evicted: AtomicU64,
}
impl Control {
    pub(crate) fn with_directory(directory: PathBuf) -> Self {
        Self {
            directory: directory.is_absolute().then(|| Arc::new(directory)),
            ..Self::default()
        }
    }
    /// What this runtime's pipelines wrote to the disk cache.
    pub fn writes(&self) -> Arc<Writes> {
        self.writes.clone()
    }
    /// The same location injected into the image pipeline, without filesystem I/O.
    pub fn directory(&self) -> Option<&Path> {
        self.directory.as_deref().map(PathBuf::as_path)
    }
    pub fn status(&self) -> Status {
        self.state.lock().unwrap().status.clone()
    }
    pub fn configure(&self, mut limits: Limits) {
        limits.normalize();
        let mut state = self.state.lock().unwrap();
        if limits != state.limits {
            state.limits = limits;
            let clear = state.pending.is_some_and(|r| r.clear);
            state.pending = Some(Request {
                limits,
                clear,
                cancellation: self.cancelled.load(Ordering::Acquire),
            });
            state.status.busy = true;
        }
    }
    pub fn request(&self, clear: bool) -> bool {
        let mut state = self.state.lock().unwrap();
        if state.status.busy {
            return false;
        }
        state.pending = Some(Request {
            limits: state.limits,
            clear,
            cancellation: self.cancelled.load(Ordering::Acquire),
        });
        state.status.busy = true;
        state.status.message = None;
        true
    }
    pub fn cancel(&self) {
        self.cancelled.fetch_add(1, Ordering::AcqRel);
    }
    pub(crate) fn cancelled(&self, request: Request) -> bool {
        self.cancelled.load(Ordering::Acquire) != request.cancellation
    }
    pub(crate) fn take(&self) -> Option<Request> {
        self.state.lock().unwrap().pending.take()
    }
    pub(crate) fn finish(&self, result: anyhow::Result<Usage>, clear: bool) {
        let mut state = self.state.lock().unwrap();
        state.status.busy = state.pending.is_some();
        match result {
            Ok(usage) => {
                let pinned = usage.pinned_disk_bytes;
                state.status.usage = Some(usage);
                // A refresh shows its result in the usage line itself.
                state.status.message = match (clear, pinned > 0) {
                    (true, true) => Some(
                        "Caches cleared. Some model files are in use; clear again when processing finishes.".into(),
                    ),
                    (true, false) => Some("Caches cleared.".into()),
                    (false, _) => None,
                };
            }
            Err(e) => {
                state.status.usage = None;
                state.status.message = Some(e.to_string());
            }
        }
    }
}
/// Own only fingerprint-named proxy artifacts, including interrupted temporary
/// writes: `<hash>.<random>.tmp.png` previews and the `.photosort-*.tmp` files
/// of cache records and render notes. Those sort oldest first like any other
/// file, so a write in progress is the last to go.
/// Foreign files, directories and symlinks are never followed or removed.
fn owned(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    if name.starts_with(".photosort-") && name.ends_with(".tmp") {
        return true;
    }
    let Some((hash, suffix)) = name.split_once('.') else {
        return false;
    };
    hash.len() == 64
        && hash.bytes().all(|b| b.is_ascii_hexdigit())
        && (matches!(suffix, "png" | "jpg" | "json" | "bin")
            || suffix.ends_with("tmp.png")
            || suffix.ends_with("tmp.jpg"))
}
/// What a trim removes first. Files of one kind go by how long they went
/// unused; a kind goes only once nothing is left of the kinds before it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// Lossless images: the 1024 px input of an analysis, about a megabyte
    /// each and rarely read again once the photo's evidence is stored. The
    /// few previews with transparency or 16 bits are PNG too and go with
    /// them, as do interrupted image writes.
    Proxy,
    /// Display previews. The photo decodes again when it is shown.
    Preview,
    /// Grid thumbnails: small, and read for every tile of a folder.
    Thumbnail,
    /// Metadata, render notes and AI evidence: the smallest files and the
    /// slowest to make again, so the last to go.
    Record,
}
impl Kind {
    /// A grid thumbnail is a 320 px JPEG, well under this; a preview this
    /// small costs as little to keep.
    const THUMBNAIL_BYTES: u64 = 64 * 1024;
    fn of(path: &Path, bytes: u64) -> Self {
        match path.extension().and_then(|extension| extension.to_str()) {
            Some("png") => Self::Proxy,
            Some("jpg") if bytes > Self::THUMBNAIL_BYTES => Self::Preview,
            Some("jpg") => Self::Thumbnail,
            _ => Self::Record,
        }
    }
}
/// Marks a cache file as used now, so a trim removes what went unused the
/// longest rather than what was written first. A file is touched at most once
/// an hour, so reading a folder again writes almost nothing.
pub(crate) fn touch(path: &Path) {
    const EVERY: std::time::Duration = std::time::Duration::from_secs(3600);
    let now = std::time::SystemTime::now();
    let stale = std::fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .is_ok_and(|modified| now.duration_since(modified).is_ok_and(|age| age >= EVERY));
    if stale {
        let _ = std::fs::File::options()
            .write(true)
            .open(path)
            .and_then(|file| file.set_modified(now));
    }
}
/// Measures the cache and, with a limit, evicts what is above it: analysis
/// images first, then previews, thumbnails and last the records (`Kind`),
/// the longest unused of each first. A zero limit clears everything,
/// prepared models included.
pub fn maintain(
    path: &Path,
    max_bytes: Option<u64>,
    cancelled: impl Fn() -> bool,
) -> anyhow::Result<Usage> {
    prune(path, max_bytes, max_bytes == Some(0), None, cancelled)
}
/// The same, but keeps prepared models even at a zero limit. They are slow to
/// rebuild, so they sit outside the limit; a newer copy of the same model
/// replaces an older one (see `compiled::Lease::complete`).
pub fn maintain_keeping_models(
    path: &Path,
    max_bytes: Option<u64>,
    cancelled: impl Fn() -> bool,
) -> anyhow::Result<Usage> {
    prune(path, max_bytes, false, None, cancelled)
}
/// A trim to `max_bytes` that keeps prepared models and counts what it
/// removed of the previews, thumbnails and records written since `since`:
/// when that is not zero, the limit is smaller than what this session works
/// with. Analysis images are not counted: an analysis writes more of them
/// than any limit is meant to keep, and losing one costs a decode at most.
pub(crate) fn trim(
    path: &Path,
    max_bytes: u64,
    since: std::time::SystemTime,
    cancelled: impl Fn() -> bool,
) -> anyhow::Result<Usage> {
    prune(path, Some(max_bytes), false, Some(since), cancelled)
}
fn prune(
    path: &Path,
    max_bytes: Option<u64>,
    clear_models: bool,
    since: Option<std::time::SystemTime>,
    cancelled: impl Fn() -> bool,
) -> anyhow::Result<Usage> {
    let root = match std::fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Usage::default()),
        Err(e) => return Err(e.into()),
    };
    anyhow::ensure!(
        root.is_dir() && !root.file_type().is_symlink(),
        "Cache folder must be a real directory"
    );
    let namespace = compiled::namespace(path).ok();
    let mut files = Vec::new();
    let mut usage = Usage::default();
    let mut photo_bytes = 0u64;
    for entry in std::fs::read_dir(path)? {
        anyhow::ensure!(
            !cancelled(),
            "Cache maintenance cancelled; some reusable files may already have been removed"
        );
        let entry = entry?;
        if compiled::owned(&entry.path()) {
            let lock = namespace
                .as_ref()
                .and_then(|_| compiled::lease(&entry.path()).ok());
            if lock.is_some() && max_bytes.is_some() && clear_models {
                compiled::remove(&entry.path(), &cancelled)?;
                continue;
            }
            let (bytes, count) = compiled::usage(&entry.path(), &cancelled)?;
            usage.disk_bytes = usage.disk_bytes.saturating_add(bytes);
            usage.model_bytes = usage.model_bytes.saturating_add(bytes);
            usage.files += count;
            if lock.is_none() {
                usage.pinned_disk_bytes = usage.pinned_disk_bytes.saturating_add(bytes);
            }
            continue;
        }
        if !owned(&entry.path()) || !entry.file_type()?.is_file() {
            continue;
        }
        // Another writer may have renamed or trimmed it since the listing.
        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(e.into()),
        };
        usage.disk_bytes = usage.disk_bytes.saturating_add(metadata.len());
        usage.files += 1;
        photo_bytes += metadata.len();
        if max_bytes.is_some() {
            let path = entry.path();
            let kind = Kind::of(&path, metadata.len());
            files.push((kind, metadata.modified()?, metadata.len(), path));
        }
    }
    if let Some(max) = max_bytes {
        // Only previews and AI evidence count against the limit.
        files.sort_unstable_by_key(|f| (f.0, f.1));
        for (kind, modified, bytes, file) in files {
            if photo_bytes <= max && max != 0 {
                break;
            }
            anyhow::ensure!(
                !cancelled(),
                "Cache maintenance cancelled; some reusable files may already have been removed"
            );
            // Recheck after enumeration. Do not remove a replaced directory or symlink.
            let meta = match std::fs::symlink_metadata(&file) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    usage.disk_bytes = usage.disk_bytes.saturating_sub(bytes);
                    photo_bytes = photo_bytes.saturating_sub(bytes);
                    usage.files -= 1;
                    continue;
                }
                Err(e) => return Err(e.into()),
            };
            if !meta.is_file() || meta.file_type().is_symlink() {
                continue;
            }
            match std::fs::remove_file(file) {
                Ok(()) => {
                    if kind != Kind::Proxy && since.is_some_and(|since| modified >= since) {
                        usage.evicted_recent_bytes += bytes;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.into()),
            }
            usage.disk_bytes = usage.disk_bytes.saturating_sub(bytes);
            photo_bytes = photo_bytes.saturating_sub(bytes);
            usage.files -= 1;
        }
    }
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn defaults_follow_the_machine_and_chosen_limits_stay() {
        use crate::core::memory::{GIB, Machine};
        let machine = |ram: u64, free: Option<u64>| Machine {
            ram: ram * GIB,
            free_disk: free.map(|free| free * GIB),
        };
        let sizes = |limits: Limits| (limits.previews_mib, limits.originals_mib, limits.disk_mib);
        assert_eq!(
            sizes(Limits::for_machine(machine(4, Some(80)))),
            (256, 512, 4096)
        );
        assert_eq!(
            sizes(Limits::for_machine(machine(8, Some(80)))),
            (512, 1024, 4096)
        );
        assert_eq!(
            sizes(Limits::for_machine(machine(16, Some(200)))),
            (1024, 2048, 10240)
        );
        assert_eq!(
            sizes(Limits::for_machine(machine(36, Some(500)))),
            (2048, 4096, 25600)
        );
        assert_eq!(
            sizes(Limits::for_machine(machine(128, Some(2000)))),
            (4096, 16384, 32768)
        );
        // A 32 GB computer that reports 31.1 GiB still gets the 32 GB defaults.
        let reported = Machine {
            ram: 31_850 * MIB,
            free_disk: Some(500 * GIB),
        };
        assert_eq!(sizes(Limits::for_machine(reported)), (2048, 4096, 25600));
        // A nearly full disk gets a tenth of what is free; unknown keeps 4 GB.
        assert_eq!(Limits::for_machine(machine(16, Some(12))).disk_mib, 1228);
        assert_eq!(Limits::for_machine(machine(16, None)).disk_mib, 4096);

        // Limits left at this computer's defaults are not saved, so they keep
        // following the machine; a chosen one is saved.
        let mut defaults = Limits::default();
        assert_eq!(serde_json::to_string(&defaults).unwrap(), "{}");
        defaults.previews_mib = PREVIEWS_MIB.start() + 1;
        assert_eq!(
            serde_json::to_string(&defaults).unwrap(),
            format!("{{\"previews_mib\":{}}}", PREVIEWS_MIB.start() + 1)
        );

        // A chosen limit survives a save and reload.
        let mut chosen = Limits::for_machine(machine(32, Some(500)));
        chosen.disk_mib = 1024;
        let mut reloaded: Limits =
            serde_json::from_str(&serde_json::to_string(&chosen).unwrap()).unwrap();
        reloaded.normalize();
        assert_eq!(reloaded.disk_mib, 1024);
    }
    #[test]
    fn cancellation_is_scoped_to_accepted_tasks_and_configure_preserves_pending_clear() {
        let control = Control::default();
        assert!(control.request(true));
        assert!(!control.request(false));
        control.configure(Limits {
            disk_mib: 64,
            ..Default::default()
        });
        let request = control.take().unwrap();
        assert!(request.clear);
        control.cancel();
        assert!(control.cancelled(request));
        control.finish(Err(anyhow::anyhow!("cancelled")), true);
        control.configure(Limits {
            disk_mib: 128,
            ..Default::default()
        });
        let next = control.take().unwrap();
        assert!(!control.cancelled(next));
        control.finish(Ok(Usage::default()), false);
        assert!(!control.status().busy);
    }
    #[test]
    fn a_trim_reports_what_it_removed_of_this_sessions_files() {
        use std::time::{Duration, SystemTime};
        let dir = tempfile::tempdir().unwrap();
        let name = |n: u8| dir.path().join(format!("{n:064x}.jpg"));
        let started = SystemTime::now() - Duration::from_secs(3600);
        for n in 1..=3 {
            std::fs::write(name(n), [0u8; 100]).unwrap();
        }
        // A prepared model: reported apart and never trimmed by the limit.
        let model = dir.path().join(format!("{:064x}.coreml", 9));
        std::fs::create_dir(&model).unwrap();
        std::fs::write(model.join(".owner"), b"PhotoSort compiled models v1\n").unwrap();
        std::fs::write(model.join("weights.bin"), [0u8; 400]).unwrap();
        // Written in an earlier session: the first to go, and not reported.
        std::fs::File::options()
            .write(true)
            .open(name(1))
            .unwrap()
            .set_modified(started - Duration::from_secs(86_400))
            .unwrap();
        let usage = trim(dir.path(), 150, started, || false).unwrap();
        assert!(model.join("weights.bin").exists());
        assert!(usage.model_bytes >= 400, "{usage:?}");
        assert_eq!(usage.disk_bytes - usage.model_bytes, 100);
        assert!(!name(1).exists());
        assert_eq!(usage.evicted_recent_bytes, 100);
        // Within the limit, nothing of this session is removed.
        assert_eq!(
            trim(dir.path(), 150, started, || false)
                .unwrap()
                .evicted_recent_bytes,
            0
        );
    }
    #[test]
    fn trims_remove_what_went_unused_longest_not_what_was_written_first() {
        use std::time::{Duration, SystemTime};
        let dir = tempfile::tempdir().unwrap();
        let name = |n: u8| dir.path().join(format!("{n:064x}.jpg"));
        let age = |n: u8, hours: u64| {
            std::fs::File::options()
                .write(true)
                .open(name(n))
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(hours * 3600))
                .unwrap();
        };
        for n in 1..=2 {
            std::fs::write(name(n), [0u8; 100]).unwrap();
        }
        // Written first but read again since; the newer one went unused.
        age(1, 48);
        age(2, 24);
        touch(&name(1));
        trim(dir.path(), 100, SystemTime::now(), || false).unwrap();
        assert!(name(1).exists());
        assert!(!name(2).exists());
    }
    #[test]
    fn analysis_images_go_before_previews_and_evidence_goes_last() {
        use std::time::{Duration, SystemTime};
        let dir = tempfile::tempdir().unwrap();
        let file = |n: u8, extension: &str| dir.path().join(format!("{n:064x}.{extension}"));
        let started = SystemTime::now() - Duration::from_secs(3600);
        let write = |n: u8, extension: &str, bytes: usize, minutes_ago: u64| {
            let path = file(n, extension);
            std::fs::write(&path, vec![0u8; bytes]).unwrap();
            std::fs::File::options()
                .write(true)
                .open(&path)
                .unwrap()
                .set_modified(SystemTime::now() - Duration::from_secs(minutes_ago * 60))
                .unwrap();
        };
        // Opening the folder made a preview, a thumbnail and metadata; the
        // analysis after it made two analysis images and the evidence.
        write(1, "jpg", 300_000, 30);
        write(2, "jpg", 20_000, 30);
        write(3, "bin", 200, 30);
        write(4, "json", 43, 30);
        write(5, "png", 900_000, 2);
        write(6, "png", 900_000, 1);
        write(7, "bin", 9_000, 1);
        let left = |limit: u64| {
            let usage = trim(dir.path(), limit, started, || false).unwrap();
            let kept: Vec<u8> = (1..=7)
                .filter(|&n| {
                    ["jpg", "png", "bin", "json"]
                        .iter()
                        .any(|e| file(n, e).exists())
                })
                .collect();
            (kept, usage.evicted_recent_bytes)
        };
        // Over the limit by one analysis image: the older one goes, although
        // everything the folder made on opening went unused for longer. The
        // session lost nothing it shows or would have to work out again.
        assert_eq!(left(1_300_000), (vec![1, 2, 3, 4, 6, 7], 0));
        // Then the other, then the preview, which the session does show.
        assert_eq!(left(400_000), (vec![1, 2, 3, 4, 7], 0));
        assert_eq!(left(30_000), (vec![2, 3, 4, 7], 300_000));
        // The thumbnail outlives the preview, and records outlive both.
        assert_eq!(left(9_500), (vec![3, 4, 7], 20_000));
        assert_eq!(left(9_100), (vec![4, 7], 200));
    }
}
