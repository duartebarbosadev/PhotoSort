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
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Limits {
    pub previews_mib: u32,
    pub originals_mib: u32,
    pub disk_mib: u32,
    /// Which defaults these limits were saved with. Missing from files
    /// written while the disk default was 1 GiB.
    #[serde(default)]
    pub defaults: u32,
}
/// The disk default went from 1 GiB to 4 GiB in defaults version 2; in
/// version 3 every default scales with this computer's memory and free disk.
const DEFAULTS: u32 = 3;
const OLD_DISK_DEFAULT_MIB: u32 = 1024;
/// The fixed defaults of versions 1 and 2.
const FIXED: (u32, u32, u32) = (128, 256, 4096);
pub const PREVIEWS_MIB: std::ops::RangeInclusive<u32> = 64..=2048;
pub const DISK_MAX_MIB: u32 = 131_072;
const ORIGINALS_MIN_MIB: u32 = 256;
const ORIGINALS_MAX_MIB: u32 = 32_768;
impl Default for Limits {
    fn default() -> Self {
        Self::for_machine(super::memory::Machine::current())
    }
}
/// The largest power of two at most `value`, kept within `floor..=ceiling`.
fn power_of_two(value: u64, floor: u32, ceiling: u32) -> u32 {
    let rounded = if value == 0 { 0 } else { 1u64 << value.ilog2() };
    rounded.clamp(u64::from(floor), u64::from(ceiling)) as u32
}
impl Limits {
    /// Defaults for a computer: previews 1/64 of memory (128 MB to 1 GB),
    /// full-size detail 1/16 (512 MB to 8 GB) and disk 1/20 of the free space
    /// (4 to 32 GB), or at most a tenth of it when under 40 GB are free.
    pub fn for_machine(machine: super::memory::Machine) -> Self {
        let ram = machine.ram / MIB;
        let disk = match machine.free_disk.map(|free| free / MIB) {
            None => FIXED.2,
            Some(free) if free < 40 * 1024 => (free / 10).min(u64::from(FIXED.2)) as u32,
            Some(free) => (free / 20 / 1024 * 1024).clamp(4096, 32_768) as u32,
        };
        Self {
            previews_mib: power_of_two(ram / 64, 128, 1024),
            originals_mib: power_of_two(ram / 16, 512, 8192),
            disk_mib: disk,
            defaults: DEFAULTS,
        }
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
        self.migrate(Self::default());
        self.previews_mib = self
            .previews_mib
            .clamp(*PREVIEWS_MIB.start(), *PREVIEWS_MIB.end());
        self.originals_mib = self
            .originals_mib
            .clamp(ORIGINALS_MIN_MIB, ORIGINALS_MAX_MIB);
        self.disk_mib = self.disk_mib.min(DISK_MAX_MIB);
    }
    /// Settings still on an old default move to `current` once, and only
    /// upwards; a limit the user chose is kept.
    fn migrate(&mut self, current: Self) {
        if self.defaults >= DEFAULTS {
            return;
        }
        if self.defaults < 2 && self.disk_mib == OLD_DISK_DEFAULT_MIB {
            self.disk_mib = FIXED.2;
        }
        let raise = |value: &mut u32, old: u32, new: u32| {
            if *value == old {
                *value = new.max(old);
            }
        };
        raise(&mut self.previews_mib, FIXED.0, current.previews_mib);
        raise(&mut self.originals_mib, FIXED.1, current.originals_mib);
        raise(&mut self.disk_mib, FIXED.2, current.disk_mib);
        self.defaults = DEFAULTS;
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
}
impl Control {
    pub(crate) fn with_directory(directory: PathBuf) -> Self {
        Self {
            directory: directory.is_absolute().then(|| Arc::new(directory)),
            ..Self::default()
        }
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
                state.status.message = Some(
                    match (clear, pinned > 0) {
                        (true, true) => "Unused caches cleared. Some prepared model files are still in use; try clearing again when processing finishes.",
                        (true, false) => "Reusable caches cleared. Current reviews and saved positions are preserved.",
                        (false, _) => "Cache usage updated.",
                    }.into()
                );
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
/// Measures the cache and, with a limit, evicts the oldest previews and AI
/// evidence above it. A zero limit clears everything, prepared models included.
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
/// removed of the files written since `since`: when that is not zero, the
/// limit is smaller than what this session works with.
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
            if lock.is_some() && max_bytes.is_some() && compiled::retired(&entry.path()) {
                compiled::remove(&entry.path(), &cancelled)?;
                continue;
            }
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
            files.push((metadata.modified()?, metadata.len(), entry.path()));
        }
    }
    if let Some(max) = max_bytes {
        // Only previews and AI evidence count against the limit.
        files.sort_unstable_by_key(|f| f.0);
        for (modified, bytes, file) in files {
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
                    if since.is_some_and(|since| modified >= since) {
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
    fn defaults_follow_the_machine_and_old_defaults_move_up_once_but_chosen_limits_stay() {
        use crate::core::memory::{GIB, Machine};
        let machine = |ram: u64, free: Option<u64>| Machine {
            ram: ram * GIB,
            free_disk: free.map(|free| free * GIB),
        };
        let sizes = |limits: Limits| (limits.previews_mib, limits.originals_mib, limits.disk_mib);
        assert_eq!(
            sizes(Limits::for_machine(machine(8, Some(80)))),
            (128, 512, 4096)
        );
        assert_eq!(
            sizes(Limits::for_machine(machine(16, Some(200)))),
            (256, 1024, 10240)
        );
        assert_eq!(
            sizes(Limits::for_machine(machine(36, Some(500)))),
            (512, 2048, 25600)
        );
        assert_eq!(
            sizes(Limits::for_machine(machine(128, Some(2000)))),
            (1024, 8192, 32768)
        );
        // A nearly full disk gets a tenth of what is free; unknown keeps 4 GB.
        assert_eq!(Limits::for_machine(machine(16, Some(12))).disk_mib, 1228);
        assert_eq!(Limits::for_machine(machine(16, None)).disk_mib, 4096);

        let current = Limits::for_machine(machine(32, Some(500)));
        let mut old: Limits =
            serde_json::from_str(r#"{"previews_mib":128,"originals_mib":256,"disk_mib":1024}"#)
                .unwrap();
        old.migrate(current);
        assert_eq!(sizes(old), (512, 2048, 25600));
        // Never lower: on a small disk the old 4 GB default stays.
        let mut small: Limits = serde_json::from_str(
            r#"{"previews_mib":128,"originals_mib":256,"disk_mib":4096,"defaults":2}"#,
        )
        .unwrap();
        small.migrate(Limits::for_machine(machine(8, Some(12))));
        assert_eq!(sizes(small), (128, 512, 4096));
        let mut custom: Limits = serde_json::from_str(
            r#"{"previews_mib":96,"originals_mib":300,"disk_mib":2048,"defaults":2}"#,
        )
        .unwrap();
        custom.migrate(current);
        assert_eq!(sizes(custom), (96, 300, 2048));
        // Chosen again after the move, 1 GiB survives a save and reload.
        old.disk_mib = 1024;
        let mut reloaded: Limits =
            serde_json::from_str(&serde_json::to_string(&old).unwrap()).unwrap();
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
        let name = |n: u8| dir.path().join(format!("{n:064x}.png"));
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
}
