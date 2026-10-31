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
/// The disk default went from 1 GiB to 4 GiB in defaults version 2.
const DEFAULTS: u32 = 2;
const OLD_DISK_DEFAULT_MIB: u32 = 1024;
impl Default for Limits {
    fn default() -> Self {
        Self {
            previews_mib: 128,
            originals_mib: 256,
            disk_mib: 4096,
            defaults: DEFAULTS,
        }
    }
}
impl Limits {
    pub fn normalize(&mut self) {
        // Settings still on the old default move to the new one once; a
        // limit chosen after that, 1 GiB included, is kept.
        if self.defaults < DEFAULTS {
            if self.disk_mib == OLD_DISK_DEFAULT_MIB {
                self.disk_mib = Self::default().disk_mib;
            }
            self.defaults = DEFAULTS;
        }
        self.previews_mib = self.previews_mib.clamp(32, 1024);
        self.originals_mib = self.originals_mib.clamp(64, 2048);
        self.disk_mib = self.disk_mib.min(32768);
    }
}
#[derive(Clone, Debug, Default)]
pub struct Usage {
    pub disk_bytes: u64,
    pub pinned_disk_bytes: u64,
    pub files: usize,
    pub preview_bytes: usize,
    pub original_bytes: usize,
    pub metadata_entries: usize,
    pub analysis_entries: usize,
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
/// Own only fingerprint-named proxy artifacts, including interrupted temporary writes.
/// Foreign files, directories and symlinks are never followed or removed.
fn owned(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let Some((hash, suffix)) = name.split_once('.') else {
        return false;
    };
    hash.len() == 64
        && hash.bytes().all(|b| b.is_ascii_hexdigit())
        && matches!(suffix, "png" | "json" | "bin" | "tmp.png" | "tmp")
}
/// Measures the cache and, with a limit, evicts the oldest previews and AI
/// evidence above it. A zero limit clears everything, prepared models included.
pub fn maintain(
    path: &Path,
    max_bytes: Option<u64>,
    cancelled: impl Fn() -> bool,
) -> anyhow::Result<Usage> {
    prune(path, max_bytes, max_bytes == Some(0), cancelled)
}
/// The same, but keeps prepared models even at a zero limit. They are slow to
/// rebuild, so they sit outside the limit; a newer copy of the same model
/// replaces an older one (see `compiled::Lease::complete`).
pub fn maintain_keeping_models(
    path: &Path,
    max_bytes: Option<u64>,
    cancelled: impl Fn() -> bool,
) -> anyhow::Result<Usage> {
    prune(path, max_bytes, false, cancelled)
}
fn prune(
    path: &Path,
    max_bytes: Option<u64>,
    clear_models: bool,
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
            usage.files += count;
            if lock.is_none() {
                usage.pinned_disk_bytes = usage.pinned_disk_bytes.saturating_add(bytes);
            }
            continue;
        }
        if !owned(&entry.path()) || !entry.file_type()?.is_file() {
            continue;
        }
        let metadata = entry.metadata()?;
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
        for (_, bytes, file) in files {
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
                Ok(()) => {}
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
    fn old_one_gib_default_moves_to_four_gib_once_but_a_chosen_limit_stays() {
        let mut old: Limits =
            serde_json::from_str(r#"{"previews_mib":128,"originals_mib":256,"disk_mib":1024}"#)
                .unwrap();
        old.normalize();
        assert_eq!(old.disk_mib, 4096);
        let mut custom: Limits = serde_json::from_str(r#"{"disk_mib":2048}"#).unwrap();
        custom.normalize();
        assert_eq!(custom.disk_mib, 2048);
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
}
