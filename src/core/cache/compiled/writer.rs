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
mod tests;
