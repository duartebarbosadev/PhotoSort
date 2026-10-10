//! Stable native cache directories. A session lease prevents pruning in another
//! process; a namespace lock makes acquiring a lease atomic with deletion.
use anyhow::{Context, Result, ensure};
use std::{
    fs::{self, File, OpenOptions},
    path::{Path, PathBuf},
};
const OWNER: &[u8] = b"PhotoSort compiled models v1\n";

/// Prepared CPU graphs (`.onnx-cache`) are no longer created; existing ones are
/// still recognized as owned so maintenance can delete them.
pub(super) fn retired(path: &Path) -> bool {
    path.extension().is_some_and(|e| e == "onnx-cache")
}
pub(super) fn named(path: &Path) -> bool {
    path.file_name().and_then(|p| p.to_str()).is_some_and(|n| {
        [".coreml", ".onnx-cache"].iter().any(|suffix| {
            n.strip_suffix(suffix)
                .is_some_and(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()))
        })
    })
}
pub(super) struct Lock(File);
impl Drop for Lock {
    fn drop(&mut self) {
        // Closing this descriptor alone may leave flock held by a duplicate
        // briefly inherited by a concurrently spawned process. Release ownership
        // explicitly before close; no retry or blocking lock is needed.
        let _ = self.0.unlock();
    }
}
fn lock_file(path: &Path) -> Result<Lock> {
    crate::core::operations::reject_symlink_components(path)?;
    if let Ok(meta) = fs::symlink_metadata(path) {
        ensure!(meta.is_file(), "Invalid model-cache lock");
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.try_lock().context("Model cache is in use")?;
    Ok(Lock(file))
}
pub(super) fn namespace(root: &Path) -> Result<Lock> {
    lock_file(&root.join(".photosort-compiled-cache.lock"))
}
pub(super) fn owned(path: &Path) -> bool {
    named(path)
        && fs::symlink_metadata(path).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink())
        && fs::symlink_metadata(path.join(".owner")).is_ok_and(|m| {
            m.is_file() && !m.file_type().is_symlink() && m.len() == OWNER.len() as u64
        })
        && fs::read(path.join(".owner")).is_ok_and(|b| b == OWNER)
}
pub(super) fn lease(path: &Path) -> Result<Lock> {
    lock_file(&path.join(".lease"))
}
/// Return regular descendants only. Never follow a native cache symlink.
fn files(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    for (i, entry) in walkdir::WalkDir::new(path)
        .follow_links(false)
        .into_iter()
        .enumerate()
    {
        ensure!(!cancelled(), "Model cache cancelled");
        ensure!(i < 50_000, "Model cache contains too many entries");
        let entry = entry?;
        ensure!(entry.depth() <= 32, "Model cache is too deeply nested");
        ensure!(
            entry.file_type().is_dir() || entry.file_type().is_file(),
            "Model cache contains a link or special file"
        );
        if entry.file_type().is_file() {
            files.push(entry.into_path());
        }
    }
    files.sort();
    Ok(files)
}
pub(super) fn usage(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<(u64, usize)> {
    let mut bytes = 0u64;
    let mut count = 0;
    for path in files(path, cancelled)? {
        let meta = fs::symlink_metadata(&path)?;
        bytes = bytes.saturating_add(meta.len());
        count += 1;
    }
    Ok((bytes, count))
}
pub(super) fn remove(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<()> {
    ensure!(owned(path), "Not a PhotoSort compiled cache");
    files(path, cancelled)?;
    ensure!(!cancelled(), "Model cache cancelled");
    fs::remove_dir_all(path)?;
    Ok(())
}

#[cfg(any(test, all(feature = "onnx", target_os = "macos")))]
mod writer;
#[cfg(all(feature = "onnx", target_os = "macos"))]
pub(crate) use writer::{Format, Lease, key};

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guard_release_does_not_wait_for_inherited_descriptors() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let guard = namespace(&root).unwrap();
        let inherited = guard.0.try_clone().unwrap();
        assert!(namespace(&root).is_err());
        drop(guard);
        // Simulates an inherited descriptor that has not yet closed at exec.
        let next = namespace(&root).expect("The original owner explicitly released its lock");
        drop(inherited);
        assert!(
            namespace(&root).is_err(),
            "An old duplicate must not release a new owner's lock"
        );
        drop(next);
        assert!(namespace(&root).is_ok());
    }
}
