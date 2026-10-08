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
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            previews_mib: 128,
            originals_mib: 256,
            disk_mib: 1024,
        }
    }
}
impl Limits {
    pub fn normalize(&mut self) {
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
pub fn maintain(
    path: &Path,
    max_bytes: Option<u64>,
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
            let (bytes, count) = compiled::usage(&entry.path(), &cancelled)?;
            usage.disk_bytes = usage.disk_bytes.saturating_add(bytes);
            usage.files += count;
            if lock.is_none() {
                usage.pinned_disk_bytes = usage.pinned_disk_bytes.saturating_add(bytes);
            } else if max_bytes.is_some() {
                let stamp = std::fs::metadata(entry.path().join(".ready.bin"))
                    .or_else(|_| entry.metadata())?
                    .modified()?;
                files.push((stamp, bytes, entry.path(), count, lock));
            }
            continue;
        }
        if !owned(&entry.path()) || !entry.file_type()?.is_file() {
            continue;
        }
        let metadata = entry.metadata()?;
        usage.disk_bytes = usage.disk_bytes.saturating_add(metadata.len());
        usage.files += 1;
        if max_bytes.is_some() {
            files.push((metadata.modified()?, metadata.len(), entry.path(), 1, None));
        }
    }
    if let Some(max) = max_bytes {
        files.sort_unstable_by_key(|f| f.0);
        for (_, bytes, file, count, lease) in files {
            if usage.disk_bytes <= max && max != 0 {
                break;
            }
            anyhow::ensure!(
                !cancelled(),
                "Cache maintenance cancelled; some reusable files may already have been removed"
            );
            if lease.is_some() {
                compiled::remove(&file, &cancelled)?;
                usage.disk_bytes = usage.disk_bytes.saturating_sub(bytes);
                usage.files -= count;
                continue;
            }
            // Recheck after enumeration. Do not remove a replaced directory or symlink.
            let meta = match std::fs::symlink_metadata(&file) {
                Ok(m) => m,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    usage.disk_bytes = usage.disk_bytes.saturating_sub(bytes);
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
            usage.files -= 1;
        }
    }
    Ok(usage)
}

#[cfg(test)]
mod tests {
    use super::*;
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
