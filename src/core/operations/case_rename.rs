//! Explicit spelling changes, planned from shared inventory and executed exclusively.
use super::*;
use std::time::UNIX_EPOCH;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct DirectoryStamp {
    modified_ns: u128,
    created_ns: Option<u128>,
    identity: [u64; 2],
}
impl DirectoryStamp {
    fn read(path: &Path) -> Result<Self> {
        let meta = fs::symlink_metadata(path)?;
        anyhow::ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "Folder was replaced: {}",
            path.display()
        );
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            [meta.dev(), meta.ino()]
        };
        #[cfg(not(unix))]
        let identity = [0, 0];
        Ok(Self {
            modified_ns: meta
                .modified()?
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos(),
            created_ns: meta
                .created()
                .ok()
                .map(|t| t.duration_since(UNIX_EPOCH).unwrap_or_default().as_nanos()),
            identity,
        })
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DirectoryRename {
    pub from: PathBuf,
    pub to: PathBuf,
    pub files: Vec<Source>,
    directories: Vec<(PathBuf, DirectoryStamp)>,
}
impl DirectoryRename {
    pub(super) fn destination(&self, path: &Path) -> PathBuf {
        self.to
            .join(path.strip_prefix(&self.from).expect("Reviewed descendant"))
    }
    pub(super) fn apply(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        anyhow::ensure!(
            self.from.is_absolute() && self.from.parent() == self.to.parent(),
            "Folder spelling change must keep its parent"
        );
        reject_symlink_components(&self.from)?;
        reject_symlink_components(&self.to)?;
        // Mutation preflight, not another catalog/index/metadata scan. Refuse
        // additions, removals, links and replaced files since the plan was reviewed.
        let mut expected: HashSet<_> = self
            .files
            .iter()
            .map(|s| s.path.clone())
            .chain(self.directories.iter().map(|(path, _)| path.clone()))
            .collect();
        for entry in walkdir::WalkDir::new(&self.from).follow_links(false) {
            anyhow::ensure!(!cancelled(), "Folder rename cancelled");
            let entry = entry?;
            anyhow::ensure!(
                !entry.file_type().is_symlink() && expected.remove(entry.path()),
                "Folder contents changed since review: {}",
                entry.path().display()
            );
        }
        anyhow::ensure!(expected.is_empty(), "Folder contents changed since review");
        for (path, stamp) in &self.directories {
            anyhow::ensure!(!cancelled(), "Folder rename cancelled");
            anyhow::ensure!(
                DirectoryStamp::read(path)? == *stamp,
                "Folder changed since review: {}",
                path.display()
            );
        }
        for source in &self.files {
            anyhow::ensure!(!cancelled(), "Folder rename cancelled");
            source.verify()?;
        }
        anyhow::ensure!(!cancelled(), "Folder rename cancelled");
        rename_exclusive(&self.from, &self.to).with_context(|| {
            format!(
                "Cannot rename {} without overwriting a destination",
                self.from.display()
            )
        })
    }
}

pub(super) struct Prepared {
    pub actions: Vec<Action>,
    rewrites: Vec<(PathBuf, PathBuf)>,
}
impl Prepared {
    pub fn rebase(&self, path: &Path) -> PathBuf {
        let mut path = path.to_path_buf();
        for (from, to) in &self.rewrites {
            if let Ok(suffix) = path.strip_prefix(from) {
                path = to.join(suffix);
            }
        }
        path
    }
    pub fn occupied(&self, path: &Path) -> bool {
        if path.exists() {
            return true;
        }
        // On a case-sensitive volume the future spelling is not present yet.
        // Check the reviewed pre-rename location as well when reserving names.
        let original = self.original(path);
        original != path && original.exists()
    }
    pub fn original(&self, path: &Path) -> PathBuf {
        let mut original = path.to_path_buf();
        for (from, to) in self.rewrites.iter().rev() {
            if let Ok(suffix) = original.strip_prefix(to) {
                original = from.join(suffix);
            }
        }
        original
    }
    pub fn directory_catalog(&self, catalog: &Catalog) -> Option<Catalog> {
        (!self.rewrites.is_empty()).then(|| Catalog {
            root: catalog.root.clone(),
            directories: catalog.directories.iter().map(|p| self.rebase(p)).collect(),
            ..Default::default()
        })
    }
}
pub(super) fn prepare(
    catalog: &Catalog,
    edits: &super::super::organize::DirectoryEdits,
    output: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Prepared> {
    let mut result = Prepared {
        actions: Vec::new(),
        rewrites: Vec::new(),
    };
    if output != catalog.root {
        return Ok(result);
    }
    let mut renamed: Vec<_> = edits
        .renamed
        .iter()
        .filter(|(from, to)| from != to && from.to_lowercase() == to.to_lowercase())
        .collect();
    renamed.sort_by_key(|(from, _)| from.split('/').count());
    for (original, target) in renamed {
        anyhow::ensure!(!cancelled(), "Planning cancelled");
        let original = folder_destination(output, original);
        let from = result.rebase(&original);
        let to = folder_destination(output, target);
        if from == to {
            continue;
        }
        anyhow::ensure!(
            catalog.directories.contains(&original),
            "Renamed folder was not reviewed"
        );
        anyhow::ensure!(
            from.parent() == to.parent(),
            "Folder spelling changes have conflicting parent destinations"
        );
        let files = catalog
            .inventory
            .iter()
            .filter(|path| path.starts_with(&original))
            .map(|path| {
                anyhow::ensure!(!cancelled(), "Planning cancelled");
                let mut source = Source::capture(path.clone())?;
                source.path = result.rebase(path);
                Ok(source)
            })
            .collect::<Result<Vec<_>>>()?;
        let directories = catalog
            .directories
            .iter()
            .filter(|path| path.starts_with(&original))
            .map(|path| {
                anyhow::ensure!(!cancelled(), "Planning cancelled");
                Ok((result.rebase(path), DirectoryStamp::read(path)?))
            })
            .collect::<Result<Vec<_>>>()?;
        result
            .actions
            .push(Action::RenameDirectory(DirectoryRename {
                from: from.clone(),
                to: to.clone(),
                files,
                directories,
            }));
        result.rewrites.push((from, to));
    }
    Ok(result)
}

pub(super) fn file_spelling_change(from: &Path, to: &Path) -> bool {
    from != to
        && from.parent() == to.parent()
        && from
            .file_name()
            .and_then(|s| s.to_str())
            .zip(to.file_name().and_then(|s| s.to_str()))
            .is_some_and(|(a, b)| a.to_lowercase() == b.to_lowercase())
}

/// One directory listing per parent for alias checks in an accepted planning
/// pass. Identity alone cannot distinguish two hardlinks on a case-sensitive
/// volume. Exact directory entries distinguish those from one spelling alias.
#[derive(Default)]
pub(super) struct FileNames {
    directories: HashMap<PathBuf, HashSet<std::ffi::OsString>>,
    #[cfg(test)]
    scans: usize,
}
impl FileNames {
    pub fn invalidate(&mut self) {
        self.directories.clear();
    }
    fn contains(&mut self, path: &Path, cancelled: &dyn Fn() -> bool) -> Result<bool> {
        let parent = path.parent().context("Missing source parent")?;
        if !self.directories.contains_key(parent) {
            #[cfg(test)]
            {
                self.scans += 1;
            }
            let mut names = HashSet::new();
            for entry in fs::read_dir(parent)? {
                anyhow::ensure!(!cancelled(), "File rename cancelled");
                names.insert(entry?.file_name());
            }
            self.directories.insert(parent.into(), names);
        }
        Ok(path
            .file_name()
            .is_some_and(|name| self.directories[parent].contains(name)))
    }
    pub fn source_alias(
        &mut self,
        from: &Path,
        to: &Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<bool> {
        if !file_spelling_change(from, to) {
            return Ok(false);
        }
        let Ok(source) = Fingerprint::read(from) else {
            return Ok(false);
        };
        if Fingerprint::read(to).ok() != Some(source) {
            return Ok(false);
        }
        Ok(self.contains(from, cancelled)? && !self.contains(to, cancelled)?)
    }
}

pub(super) fn apply_file(
    source: &Source,
    to: &Path,
    cancelled: &dyn Fn() -> bool,
    names: &mut FileNames,
) -> Result<()> {
    anyhow::ensure!(
        source.path.is_absolute() && file_spelling_change(&source.path, to),
        "File spelling change must keep its parent and change only letter case"
    );
    anyhow::ensure!(!cancelled(), "File rename cancelled");
    source.verify()?;
    reject_symlink_components(&source.path)?;
    reject_symlink_components(to)?;
    anyhow::ensure!(
        names.contains(&source.path, cancelled)?,
        "Source spelling changed since review"
    );
    anyhow::ensure!(
        !names.contains(to, cancelled)?,
        "Destination already exists: {}",
        to.display()
    );
    anyhow::ensure!(!cancelled(), "File rename cancelled");
    // The OS primitive rejects a destination created after our preflight. No
    // temporary spelling, copy/unlink or overwrite fallback is permitted.
    source.verify()?;
    rename_exclusive(&source.path, to).with_context(|| {
        format!(
            "Cannot rename {} without overwriting a destination",
            source.path.display()
        )
    })?;
    let entries = names
        .directories
        .get_mut(source.path.parent().unwrap())
        .unwrap();
    entries.remove(source.path.file_name().unwrap());
    entries.insert(to.file_name().unwrap().to_os_string());
    Ok(())
}

#[cfg(target_os = "macos")]
fn rename_exclusive(from: &Path, to: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn renamex_np(
            from: *const std::ffi::c_char,
            to: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    // RENAME_EXCL permits a spelling change of the same entry, never replacement.
    if unsafe { renamex_np(from.as_ptr(), to.as_ptr(), 4) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
#[cfg(target_os = "linux")]
fn rename_exclusive(from: &Path, to: &Path) -> Result<()> {
    use std::{ffi::CString, os::unix::ffi::OsStrExt};
    unsafe extern "C" {
        fn renameat2(
            fromfd: i32,
            from: *const std::ffi::c_char,
            tofd: i32,
            to: *const std::ffi::c_char,
            flags: u32,
        ) -> i32;
    }
    let from = CString::new(from.as_os_str().as_bytes())?;
    let to = CString::new(to.as_os_str().as_bytes())?;
    if unsafe { renameat2(-100, from.as_ptr(), -100, to.as_ptr(), 1) } != 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
#[cfg(target_os = "windows")]
fn rename_exclusive(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(from: *const u16, to: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    anyhow::ensure!(
        !from[..from.len() - 1].contains(&0) && !to[..to.len() - 1].contains(&0),
        "Invalid path"
    );
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0) } == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    Ok(())
}
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn rename_exclusive(_from: &Path, _to: &Path) -> Result<()> {
    anyhow::bail!("Exclusive renaming is unavailable on this platform")
}

#[cfg(test)]
mod file_tests {
    use super::*;
    #[test]
    fn filename_snapshot_is_reused_for_every_companion_and_lookup() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        for ext in ["arw", "jpg", "xmp"] {
            fs::write(root.join(format!("sample.{ext}")), ext).unwrap();
        }
        let mut names = FileNames::default();
        for _ in 0..100 {
            for ext in ["arw", "jpg", "xmp"] {
                assert!(
                    names
                        .contains(&root.join(format!("sample.{ext}")), &|| false)
                        .unwrap()
                );
                assert!(
                    !names
                        .contains(&root.join(format!("Sample.{ext}")), &|| false)
                        .unwrap()
                );
            }
        }
        assert_eq!(names.scans, 1);
        for ext in ["arw", "jpg", "xmp"] {
            let source = Source::capture(root.join(format!("sample.{ext}"))).unwrap();
            let target = root.join(format!("Sample.{ext}"));
            apply_file(&source, &target, &|| false, &mut names).unwrap();
            assert!(names.contains(&target, &|| false).unwrap());
            assert!(!names.contains(&source.path, &|| false).unwrap());
        }
        assert_eq!(
            names.scans, 1,
            "Successful renames update the shared directory snapshot"
        );
        names.invalidate();
        assert!(names.contains(&root.join("Sample.jpg"), &|| false).unwrap());
        assert_eq!(
            names.scans, 2,
            "Other file operations invalidate spelling snapshots"
        );
        let mut cancelled = FileNames::default();
        assert!(
            cancelled
                .contains(&root.join("Sample.jpg"), &|| true)
                .is_err()
        );
        assert!(
            cancelled.directories.is_empty(),
            "Cancelled snapshots never enter the cache"
        );
    }
}
