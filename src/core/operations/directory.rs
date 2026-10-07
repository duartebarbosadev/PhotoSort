//! One inventory preflight for destructive directory operations.
use super::*;
use std::time::UNIX_EPOCH;
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct DirectoryStamp {
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
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Inventory {
    pub files: Vec<Source>,
    pub(super) directories: Vec<(PathBuf, DirectoryStamp)>,
}
impl Inventory {
    /// Requested roots must be disjoint. Assign the existing discovery inventory
    /// in one pass; never rescan separately for every selected folder.
    pub(super) fn capture(
        catalog: &Catalog,
        roots: &[PathBuf],
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Self>> {
        if roots.is_empty() {
            return Ok(Vec::new());
        }
        let owners: HashMap<_, _> = roots
            .iter()
            .enumerate()
            .map(|(i, p)| (p.as_path(), i))
            .collect();
        anyhow::ensure!(owners.len() == roots.len(), "Duplicate folder selection");
        let known: HashSet<_> = catalog.directories.iter().collect();
        for root in roots {
            anyhow::ensure!(!cancelled(), "Folder planning cancelled");
            anyhow::ensure!(
                root.is_absolute()
                    && root != &catalog.root
                    && root.starts_with(&catalog.root)
                    && known.contains(root),
                "Choose an existing folder inside the library"
            );
            anyhow::ensure!(
                !root.ancestors().skip(1).any(|p| owners.contains_key(p)),
                "Folder selections overlap"
            );
            reject_symlink_components(root)?;
            if let Some(identity) = catalog.directory_identities.get(root) {
                anyhow::ensure!(
                    Fingerprint::read_directory(root)?.identity == *identity,
                    "Folder was replaced since discovery: {}",
                    root.display()
                );
            }
        }
        let fingerprints: HashMap<_, _> = catalog
            .media
            .iter()
            .map(|m| (m.id, m.fingerprint))
            .chain(
                catalog
                    .supporting_fingerprints
                    .iter()
                    .map(|(id, f)| (*id, *f)),
            )
            .collect();
        let owner = |path: &Path| path.ancestors().find_map(|p| owners.get(p).copied());
        let mut inventories = vec![Self::default(); roots.len()];
        for path in &catalog.inventory {
            anyhow::ensure!(!cancelled(), "Folder planning cancelled");
            if let Some(i) = owner(path) {
                let source = Source::capture(path.clone())?;
                if let Some(fingerprint) = fingerprints.get(&MediaId::for_path(path)) {
                    anyhow::ensure!(
                        source.fingerprint == *fingerprint,
                        "File changed since discovery: {}",
                        path.display()
                    );
                }
                inventories[i].files.push(source);
            }
        }
        for path in &catalog.directories {
            anyhow::ensure!(!cancelled(), "Folder planning cancelled");
            if let Some(i) = owner(path) {
                inventories[i]
                    .directories
                    .push((path.clone(), DirectoryStamp::read(path)?));
            }
        }
        for inventory in &mut inventories {
            inventory.files.sort_by(|a, b| a.path.cmp(&b.path));
            inventory.directories.sort_by(|a, b| a.0.cmp(&b.0));
        }
        Ok(inventories)
    }
    pub fn directory_count(&self) -> usize {
        self.directories.len()
    }
    pub(super) fn rebase(&mut self, map: impl Fn(&Path) -> PathBuf) {
        for source in &mut self.files {
            source.path = map(&source.path);
        }
        for (path, _) in &mut self.directories {
            *path = map(path);
        }
    }
    pub(super) fn verify(&self, root: &Path, cancelled: &dyn Fn() -> bool) -> Result<()> {
        reject_symlink_components(root)?;
        anyhow::ensure!(
            self.directories.iter().any(|(p, _)| p == root),
            "Folder inventory has no reviewed root"
        );
        let mut expected: HashSet<_> = self
            .files
            .iter()
            .map(|s| s.path.clone())
            .chain(self.directories.iter().map(|(p, _)| p.clone()))
            .collect();
        for entry in walkdir::WalkDir::new(root).follow_links(false) {
            anyhow::ensure!(!cancelled(), "Folder operation cancelled");
            let entry = entry?;
            anyhow::ensure!(
                !entry.file_type().is_symlink() && expected.remove(entry.path()),
                "Folder contents changed since review: {}",
                entry.path().display()
            );
        }
        anyhow::ensure!(expected.is_empty(), "Folder contents changed since review");
        for (path, stamp) in &self.directories {
            anyhow::ensure!(!cancelled(), "Folder operation cancelled");
            anyhow::ensure!(
                DirectoryStamp::read(path)? == *stamp,
                "Folder changed since review: {}",
                path.display()
            );
        }
        for source in &self.files {
            anyhow::ensure!(!cancelled(), "Folder operation cancelled");
            source.verify()?;
        }
        anyhow::ensure!(!cancelled(), "Folder operation cancelled");
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TrashDirectory {
    pub path: PathBuf,
    pub library: PathBuf,
    pub inventory: Inventory,
}
impl TrashDirectory {
    pub(super) fn apply(&self, cancelled: &dyn Fn() -> bool) -> Result<()> {
        anyhow::ensure!(
            self.path.is_absolute()
                && self.path != self.library
                && self.path.starts_with(&self.library),
            "Cannot Trash the library root or an outside folder"
        );
        self.inventory.verify(&self.path, cancelled)?;
        move_to_trash(&self.path)
            .with_context(|| format!("Could not move folder {} to OS Trash", self.path.display()))
    }
}
