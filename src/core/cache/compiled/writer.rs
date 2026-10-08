use super::*;
use crate::core::{cache_record, model::Fingerprint};
use serde::{Deserialize, Serialize};
use std::io::Read;
const READY: &str = ".ready.bin";
#[derive(Serialize, Deserialize, PartialEq, Eq)]
struct Record {
    path: PathBuf,
    bytes: u64,
    hash: [u8; 32],
}
fn hash_file(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<[u8; 32]> {
    let before = Fingerprint::read(path)?;
    let mut file = File::open(path)?;
    let mut hash = blake3::Hasher::new();
    let mut buf = [0; 64 * 1024];
    loop {
        ensure!(!cancelled(), "Model cache cancelled");
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hash.update(&buf[..n]);
    }
    ensure!(
        Fingerprint::read(path)? == before,
        "Model cache input changed"
    );
    Ok(*hash.finalize().as_bytes())
}
fn snapshot(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<Vec<Record>> {
    files(path, cancelled)?
        .into_iter()
        .filter(|p| {
            let relative = p.strip_prefix(path).unwrap();
            ![Path::new(".owner"), Path::new(".lease"), Path::new(READY)].contains(&relative)
        })
        .map(|p| {
            Ok(Record {
                path: p.strip_prefix(path)?.into(),
                bytes: fs::metadata(&p)?.len(),
                hash: hash_file(&p, cancelled)?,
            })
        })
        .collect()
}
/// Content identities, not filenames or timestamps alone, invalidate native weights.
pub(crate) fn key(
    model: &Path,
    runtime: &Path,
    recipe: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<String> {
    let mut hash = blake3::Hasher::new();
    hash.update(b"photosort-coreml-cache-v1");
    for path in [model, runtime] {
        hash.update(&hash_file(path, cancelled)?);
    }
    hash.update(recipe.as_bytes());
    Ok(hash.finalize().to_hex().to_string())
}
/// Formats share integrity, leasing, eviction and budget ownership.
#[derive(Clone, Copy)]
pub(crate) enum Format {
    CoreMl,
    Onnx,
}
impl Format {
    fn suffix(self) -> &'static str {
        match self {
            Self::CoreMl => "coreml",
            Self::Onnx => "onnx-cache",
        }
    }
}
pub(crate) struct Lease {
    pub path: PathBuf,
    pub reused: bool,
    root: PathBuf,
    lock: Option<Lock>,
    keep: bool,
    max_bytes: u64,
}
impl Lease {
    pub fn acquire(
        format: Format,
        root: &Path,
        key: &str,
        max_bytes: u64,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Self> {
        ensure!(max_bytes > 0, "Persistent model cache is disabled");
        ensure!(
            key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()),
            "Invalid model cache identity"
        );
        ensure!(!cancelled(), "Model cache cancelled");
        crate::core::operations::reject_symlink_components(root)?;
        fs::create_dir_all(root)?;
        let root = root.canonicalize()?;
        let guard = namespace(&root)?;
        let path = root.join(format!("{key}.{}", format.suffix()));
        ensure!(named(&path), "Invalid model cache identity");
        if path.try_exists()? {
            ensure!(owned(&path), "Foreign model cache directory");
        } else {
            fs::create_dir(&path)?;
            fs::write(path.join(".owner"), OWNER)?;
        }
        let lock = lease(&path)?;
        drop(guard);
        let mut result = Self {
            path,
            reused: false,
            root,
            lock: Some(lock),
            keep: false,
            max_bytes,
        };
        let saved: Option<Vec<Record>> = cache_record::read(&result.path.join(READY))
            .and_then(|b| cache_record::decode(cache_record::Kind::Compiled, &b));
        result.keep = saved.is_some();
        let current = snapshot(&result.path, cancelled)?;
        if saved
            .as_ref()
            .is_some_and(|s| !s.is_empty() && *s == current)
        {
            result.reused = true;
            result.keep = true;
            // Update whole-entry LRU without changing native file paths or mtimes.
            result.publish(&current)?;
        } else {
            result.keep = false;
            for entry in fs::read_dir(&result.path)? {
                ensure!(!cancelled(), "Model cache cancelled");
                let entry = entry?;
                if [".lease", ".owner"].iter().any(|n| entry.file_name() == *n) {
                    continue;
                }
                if entry.file_type()?.is_dir() {
                    fs::remove_dir_all(entry.path())?;
                } else {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(result)
    }
    fn publish(&self, records: &[Record]) -> Result<()> {
        cache_record::write(
            &self.path.join(READY),
            &cache_record::encode(cache_record::Kind::Compiled, &records)?,
        )
    }
    pub fn complete(&mut self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        let records = snapshot(&self.path, cancelled)?;
        ensure!(
            !records.is_empty(),
            "Model preparation produced no cache artifacts"
        );
        self.publish(&records)?;
        self.keep = usage(&self.path, cancelled)?.0 <= self.max_bytes;
        // Active leases may temporarily exceed the budget, but are never evicted.
        crate::core::cache::maintain(&self.root, Some(self.max_bytes), cancelled)?;
        Ok(())
    }
    pub fn retained(&self) -> bool {
        self.keep
    }
    pub fn invalidate(&mut self) {
        self.keep = false;
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Retain the lease until namespace ownership prevents a new opener.
        if let Ok(guard) = namespace(&self.root) {
            drop(self.lock.take());
            if !self.keep {
                let _ = remove(&self.path, &|| false);
            }
            drop(guard);
        } else {
            drop(self.lock.take());
        }
        let _ = crate::core::cache::maintain(&self.root, Some(self.max_bytes), || false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> (tempfile::TempDir, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap().join("cache");
        (temp, root)
    }
    fn ready(root: &Path, n: usize, bytes: &[u8]) -> Lease {
        let mut entry = Lease::acquire(
            Format::CoreMl,
            root,
            &format!("{n:064x}"),
            1_000_000,
            &|| false,
        )
        .unwrap();
        fs::create_dir_all(entry.path.join("native/weights")).unwrap();
        fs::write(entry.path.join("native/weights/model.bin"), bytes).unwrap();
        entry.complete(&|| false).unwrap();
        entry
    }
    #[test]
    fn stable_native_cache_reuses_verified_bytes_and_holds_a_process_lease() {
        let (_temp, root) = root();
        let first = ready(&root, 1, b"weights");
        let path = first.path.clone();
        assert!(!first.reused);
        assert!(first.retained());
        assert!(
            Lease::acquire(
                Format::CoreMl,
                &root,
                &format!("{:064x}", 1),
                1_000_000,
                &|| false
            )
            .is_err()
        );
        let active = crate::core::cache::maintain(&root, Some(0), || false).unwrap();
        assert!(active.disk_bytes > 0 && active.pinned_disk_bytes == active.disk_bytes);
        assert!(path.join("native/weights/model.bin").exists());
        drop(first);
        let second = Lease::acquire(
            Format::CoreMl,
            &root,
            &format!("{:064x}", 1),
            1_000_000,
            &|| false,
        )
        .unwrap();
        assert!(second.reused);
        assert_eq!(second.path, path);
        drop(second);
        assert_eq!(
            crate::core::cache::maintain(&root, Some(0), || false)
                .unwrap()
                .disk_bytes,
            0
        );
        assert!(!path.exists());
    }
    #[test]
    fn onnx_and_coreml_entries_share_budget_without_aliasing() {
        let (_temp, root) = root();
        let coreml = ready(&root, 1, b"coreml");
        let mut onnx = Lease::acquire(
            Format::Onnx,
            &root,
            &format!("{:064x}", 1),
            1_000_000,
            &|| false,
        )
        .unwrap();
        fs::write(onnx.path.join("optimized.onnx"), b"onnx").unwrap();
        onnx.complete(&|| false).unwrap();
        assert_ne!(coreml.path, onnx.path);
        let usage = crate::core::cache::maintain(&root, Some(0), || false).unwrap();
        assert!(usage.disk_bytes > 0 && usage.disk_bytes == usage.pinned_disk_bytes);
        let path = onnx.path.clone();
        drop(onnx);
        let reused = Lease::acquire(
            Format::Onnx,
            &root,
            &format!("{:064x}", 1),
            1_000_000,
            &|| false,
        )
        .unwrap();
        assert!(reused.reused);
        drop(reused);
        fs::write(path.join("optimized.onnx"), b"torn").unwrap();
        let fresh = Lease::acquire(
            Format::Onnx,
            &root,
            &format!("{:064x}", 1),
            1_000_000,
            &|| false,
        )
        .unwrap();
        assert!(!fresh.reused && !path.join("optimized.onnx").exists());
        drop(fresh);
        drop(coreml);
        assert_eq!(
            crate::core::cache::maintain(&root, Some(0), || false)
                .unwrap()
                .disk_bytes,
            0
        );
        assert!(!path.exists());
    }
    #[test]
    fn changed_weights_runtime_or_recipe_cannot_reuse_an_old_directory() {
        let (temp, _) = root();
        let model = temp.path().join("model");
        let runtime = temp.path().join("runtime");
        fs::write(&model, b"weights one").unwrap();
        fs::write(&runtime, b"runtime one").unwrap();
        let original = key(&model, &runtime, "recipe1", &|| false).unwrap();
        assert_eq!(
            original,
            key(&model, &runtime, "recipe1", &|| false).unwrap()
        );
        fs::write(&model, b"weights two").unwrap();
        assert_ne!(
            original,
            key(&model, &runtime, "recipe1", &|| false).unwrap()
        );
        fs::write(&model, b"weights one").unwrap();
        fs::write(&runtime, b"runtime two").unwrap();
        assert_ne!(
            original,
            key(&model, &runtime, "recipe1", &|| false).unwrap()
        );
        fs::write(&runtime, b"runtime one").unwrap();
        assert_ne!(
            original,
            key(&model, &runtime, "recipe2", &|| false).unwrap()
        );
    }
    #[test]
    fn corrupt_incomplete_or_extra_native_files_are_regenerated_before_loading() {
        for damage in [0, 1, 2] {
            let (_temp, root) = root();
            let old = ready(&root, 1, b"weights");
            let path = old.path.clone();
            drop(old);
            match damage {
                0 => fs::write(path.join("native/weights/model.bin"), b"changed").unwrap(),
                1 => fs::write(path.join(READY), b"torn").unwrap(),
                _ => fs::write(path.join("unexpected.bin"), b"extra").unwrap(),
            }
            let fresh = Lease::acquire(
                Format::CoreMl,
                &root,
                &format!("{:064x}", 1),
                1_000_000,
                &|| false,
            )
            .unwrap();
            assert!(!fresh.reused);
            assert!(!path.join("native").exists());
            drop(fresh);
            assert!(!path.exists());
        }
    }
    #[test]
    fn cancelled_cache_validation_preserves_previously_complete_artifacts() {
        let (_temp, root) = root();
        let old = ready(&root, 1, &vec![7; 200_000]);
        let path = old.path.clone();
        drop(old);
        let calls = std::cell::Cell::new(0);
        assert!(
            Lease::acquire(
                Format::CoreMl,
                &root,
                &format!("{:064x}", 1),
                1_000_000,
                &|| {
                    calls.set(calls.get() + 1);
                    calls.get() > 5
                }
            )
            .is_err()
        );
        assert!(path.join(READY).exists());
        assert!(
            Lease::acquire(
                Format::CoreMl,
                &root,
                &format!("{:064x}", 1),
                1_000_000,
                &|| false
            )
            .unwrap()
            .reused
        );
    }
    #[test]
    fn zero_budget_foreign_directories_and_unsafe_keys_are_refused() {
        let (temp, root) = root();
        assert!(
            Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), 0, &|| false).is_err()
        );
        assert!(!root.exists());
        for key in [format!("../{:064x}", 1), "bad".into()] {
            assert!(Lease::acquire(Format::CoreMl, &root, &key, 1000, &|| false).is_err());
        }
        fs::create_dir_all(root.join(format!("{:064x}.coreml", 1))).unwrap();
        let foreign = root.join(format!("{:064x}.coreml/precious", 1));
        fs::write(&foreign, b"keep").unwrap();
        assert!(
            Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), 1000, &|| {
                false
            })
            .is_err()
        );
        crate::core::cache::maintain(&root, Some(0), || false).unwrap();
        assert_eq!(fs::read(foreign).unwrap(), b"keep");
        assert!(!temp.path().join(format!("{:064x}.coreml", 1)).exists());
    }
    #[cfg(unix)]
    #[test]
    fn native_cache_never_follows_or_deletes_link_targets() {
        let (temp, root) = root();
        let entry = ready(&root, 1, b"weights");
        let path = entry.path.clone();
        drop(entry);
        let photo = temp.path().join("photo");
        fs::write(&photo, b"original").unwrap();
        std::os::unix::fs::symlink(&photo, path.join("foreign-link")).unwrap();
        assert!(
            Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), 1000, &|| {
                false
            })
            .is_err()
        );
        assert!(crate::core::cache::maintain(&root, Some(0), || false).is_err());
        assert_eq!(fs::read(photo).unwrap(), b"original");
        assert!(path.join("foreign-link").is_symlink());
    }
    #[test]
    fn oversize_and_failed_native_compilation_leave_no_persistent_artifact() {
        let (_temp, root) = root();
        let mut entry =
            Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 1), 64, &|| false).unwrap();
        let path = entry.path.clone();
        fs::write(path.join("weights"), [1; 100]).unwrap();
        entry.complete(&|| false).unwrap();
        assert!(!entry.retained());
        assert!(
            crate::core::cache::maintain(&root, Some(64), || false)
                .unwrap()
                .pinned_disk_bytes
                > 64
        );
        drop(entry);
        assert!(!path.exists());
        let failed = Lease::acquire(Format::CoreMl, &root, &format!("{:064x}", 2), 1000, &|| {
            false
        })
        .unwrap();
        let path = failed.path.clone();
        drop(failed);
        assert!(!path.exists());
    }
    #[test]
    fn failed_session_invalidates_previously_complete_native_files() {
        let (_temp, root) = root();
        let mut entry = ready(&root, 1, b"weights");
        let path = entry.path.clone();
        assert!(entry.retained());
        entry.invalidate();
        assert!(!entry.retained());
        drop(entry);
        assert!(!path.exists());
    }
    #[test]
    fn shared_budget_prunes_unused_native_and_flat_files_but_keeps_live_sessions() {
        let (_temp, root) = root();
        let live = ready(&root, 1, b"active weights");
        let unused = ready(&root, 2, b"unused weights");
        let unused_path = unused.path.clone();
        drop(unused);
        let flat = root.join(format!("{:064x}.bin", 3));
        fs::write(&flat, [7; 100]).unwrap();
        let live_bytes = usage(&live.path, &|| false).unwrap().0;
        let result = crate::core::cache::maintain(&root, Some(live_bytes), || false).unwrap();
        assert_eq!(result.disk_bytes, live_bytes);
        assert_eq!(result.pinned_disk_bytes, live_bytes);
        assert!(!flat.exists());
        assert!(!unused_path.exists());
        assert!(live.path.exists());
    }
    // Invoked in isolated test processes by the lifetime regression below.
    #[test]
    fn cache_lease_process_probe() {
        let Some(root) = std::env::var_os("PHOTOSORT_TEST_COMPILED_CACHE") else {
            return;
        };
        let root = PathBuf::from(root);
        let mode = std::env::var("PHOTOSORT_TEST_COMPILED_MODE").unwrap();
        if mode == "held" {
            let result = crate::core::cache::maintain(&root, Some(0), || false).unwrap();
            assert!(result.disk_bytes > 0 && result.pinned_disk_bytes == result.disk_bytes);
            assert!(
                Lease::acquire(
                    Format::CoreMl,
                    &root,
                    &format!("{:064x}", 1),
                    1_000_000,
                    &|| false
                )
                .is_err()
            );
        } else {
            let entry = Lease::acquire(
                Format::CoreMl,
                &root,
                &format!("{:064x}", 1),
                1_000_000,
                &|| false,
            )
            .unwrap();
            assert!(entry.reused);
            if mode == "crash" {
                // Bypass Rust drops to verify that OS-owned locks do not strand entries.
                std::process::exit(0);
            }
        }
    }
    #[test]
    fn another_process_cannot_prune_a_live_model_and_exit_releases_its_lease() {
        let (_temp, root) = root();
        let entry = ready(&root, 1, b"native weights");
        let probe = |mode: &str| {
            let output = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "core::cache::compiled::writer::tests::cache_lease_process_probe",
                    "--nocapture",
                ])
                .env("PHOTOSORT_TEST_COMPILED_CACHE", &root)
                .env("PHOTOSORT_TEST_COMPILED_MODE", mode)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr)
            );
        };
        probe("held");
        assert!(entry.path.exists());
        drop(entry);
        probe("crash");
        probe("released");
        assert_eq!(
            crate::core::cache::maintain(&root, Some(0), || false)
                .unwrap()
                .disk_bytes,
            0
        );
    }
}
