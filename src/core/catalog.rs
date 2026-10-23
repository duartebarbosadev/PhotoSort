use super::model::*;
use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

#[derive(Default)]
pub struct Catalog {
    pub root: PathBuf,
    pub media: Vec<Media>,
    pub inventory: Vec<PathBuf>,
    /// Identity snapshots for Organize's supporting files; never image inputs.
    pub supporting_fingerprints: HashMap<MediaId, Fingerprint>,
    pub directories: Vec<PathBuf>,
    /// Stable identities from the shared scan; layout restoration never stats on the UI thread.
    pub directory_identities: HashMap<PathBuf, [u64; 4]>,
    pub warnings: Vec<String>,
}
impl Catalog {
    pub fn scan(
        root: &Path,
        cancelled: impl Fn() -> bool,
        mut batch: impl FnMut(Vec<Media>),
    ) -> Result<Self> {
        anyhow::ensure!(!cancelled(), "Cancelled");
        let root = root.canonicalize().context("Open photo folder")?;
        anyhow::ensure!(root.is_dir(), "Choose a folder, not a file");
        let mut catalog = Self {
            root: root.clone(),
            ..Default::default()
        };
        let mut pending = Vec::with_capacity(128);
        // Media and Organize inventory are discovered inside hidden folders
        // too. Keep one complete walk; do not follow file/directory links.
        for entry in WalkDir::new(&root).follow_links(false) {
            if cancelled() {
                anyhow::bail!("Cancelled");
            }
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    catalog.warnings.push(e.to_string());
                    continue;
                }
            };
            if entry.file_type().is_dir() {
                catalog.directories.push(entry.path().to_path_buf());
                match Fingerprint::read_directory(entry.path()) {
                    Ok(fingerprint) => {
                        catalog
                            .directory_identities
                            .insert(entry.path().to_path_buf(), fingerprint.identity);
                    }
                    Err(error) => catalog.warnings.push(error.to_string()),
                }
            }
            if !entry.file_type().is_file() {
                continue;
            }
            let path = entry.into_path();
            catalog.inventory.push(path.clone());
            if let Some(kind) = MediaKind::from_path(&path) {
                match Fingerprint::read(&path) {
                    Ok(fingerprint) => {
                        let item = Media {
                            id: MediaId::for_path(&path),
                            path,
                            kind,
                            fingerprint,
                        };
                        catalog.media.push(item.clone());
                        pending.push(item);
                    }
                    Err(e) => catalog.warnings.push(e.to_string()),
                }
            }
            if pending.len() >= 128 {
                batch(std::mem::take(&mut pending));
            }
        }
        if !pending.is_empty() {
            batch(pending);
        }
        // The single walk owns discovery. Capture only eligible supporting-file
        // identities once, after every media directory is known; no content reads.
        let directories: HashSet<_> = catalog
            .media
            .iter()
            .filter_map(|m| m.path.parent())
            .collect();
        for path in &catalog.inventory {
            anyhow::ensure!(!cancelled(), "Cancelled");
            if Self::is_supporting_file(path, &directories) {
                match Fingerprint::read(path) {
                    Ok(fingerprint) => {
                        catalog
                            .supporting_fingerprints
                            .insert(MediaId::for_path(path), fingerprint);
                    }
                    Err(error) => catalog.warnings.push(error.to_string()),
                }
            }
        }
        catalog.media.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(catalog)
    }
    /// Organize reuses the scan inventory. Its scope is the media plus any extra
    /// files in media directories or their descendants; XMP stays companion-owned.
    pub fn organize_files(&self) -> Vec<(MediaId, &Path)> {
        let directories: HashSet<_> = self.media.iter().filter_map(|m| m.path.parent()).collect();
        let mut files: Vec<_> = self
            .media
            .iter()
            .map(|m| (m.id, m.path.as_path()))
            .collect();
        let mut extras: Vec<_> = self
            .inventory
            .iter()
            .filter(|path| Self::is_supporting_file(path, &directories))
            .map(|path| (MediaId::for_path(path), path.as_path()))
            .collect();
        extras.sort_by(|a, b| a.1.cmp(b.1));
        extras.dedup_by(|a, b| a.1 == b.1);
        files.extend(extras);
        files
    }

    fn is_supporting_file(path: &Path, directories: &HashSet<&Path>) -> bool {
        MediaKind::from_path(path).is_none()
            && !path
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("xmp"))
            && path
                .parent()
                .is_some_and(|parent| parent.ancestors().any(|p| directories.contains(p)))
    }

    /// Build once from the inventory, never walk the filesystem per photo/workflow.
    pub fn companion_index(&self) -> HashMap<PathBuf, Vec<PathBuf>> {
        let mut index: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        for path in &self.inventory {
            if MediaKind::from_path(path).is_some_and(|k| k != MediaKind::Video)
                || path
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("xmp"))
            {
                index
                    .entry(path.with_extension(""))
                    .or_default()
                    .push(path.clone());
            }
        }
        index
    }
}

/// Why a chosen photo folder can't be opened. The raw OS error belongs in
/// the log; [`Unavailable::message`] is what the person reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// Gone, renamed, or on a drive that isn't connected.
    Missing,
    NotAFolder,
    NoPermission,
    Other,
}

impl Unavailable {
    fn of(error: &std::io::Error) -> Self {
        use std::io::ErrorKind;
        match error.kind() {
            ErrorKind::NotFound | ErrorKind::NotADirectory => Self::Missing,
            ErrorKind::PermissionDenied => Self::NoPermission,
            _ => Self::Other,
        }
    }

    /// One plain sentence or two naming the folder; never an OS error code.
    pub fn message(self, folder: &Path) -> String {
        let folder = shown_path(folder);
        match self {
            Self::Missing => format!(
                "Can't open “{folder}” — it doesn't exist or isn't connected. If it's on an external drive, connect it and try again."
            ),
            Self::NotAFolder => format!(
                "Can't open “{folder}” — it's a file, not a folder. Choose the folder that holds your photos."
            ),
            Self::NoPermission if cfg!(target_os = "macos") => format!(
                "PhotoSort isn't allowed to open “{folder}”. Allow access in System Settings › Privacy & Security › Files and Folders, then try again."
            ),
            Self::NoPermission => format!(
                "PhotoSort isn't allowed to open “{folder}”. Make sure your account can read this folder, then try again."
            ),
            Self::Other => format!(
                "Can't open “{folder}”. Check that its drive is connected and readable, then try again."
            ),
        }
    }
}

/// Checks that `root` is a folder PhotoSort can list before any scan or
/// recovery work touches it, and returns its canonical path.
pub fn open_root(root: &Path) -> std::result::Result<PathBuf, (Unavailable, std::io::Error)> {
    let canonical = root
        .canonicalize()
        .map_err(|error| (Unavailable::of(&error), error))?;
    if !canonical.is_dir() {
        return Err((
            Unavailable::NotAFolder,
            std::io::Error::from(std::io::ErrorKind::NotADirectory),
        ));
    }
    std::fs::read_dir(&canonical).map_err(|error| (Unavailable::of(&error), error))?;
    Ok(canonical)
}

/// Very long paths keep their start (the drive) and their end (the folder
/// itself) so the message stays a few lines long.
fn shown_path(path: &Path) -> String {
    const LIMIT: usize = 100;
    let full = path.display().to_string();
    let count = full.chars().count();
    if count <= LIMIT {
        return full;
    }
    let head: String = full.chars().take(30).collect();
    let tail: String = full.chars().skip(count - (LIMIT - 31)).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod unavailable_tests {
    use super::*;

    #[test]
    fn messages_name_the_folder_and_never_the_os_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("Photos 2024");
        let (problem, error) = open_root(&missing).unwrap_err();
        assert_eq!(problem, Unavailable::Missing);
        assert!(error.to_string().contains("os error"));
        let message = problem.message(&missing);
        assert!(message.contains(&format!("“{}”", missing.display())));
        assert!(message.contains("doesn't exist or isn't connected"));
        assert!(message.contains("external drive"));

        let file = dir.path().join("photo.jpg");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(open_root(&file).unwrap_err().0, Unavailable::NotAFolder);
        assert!(
            Unavailable::NotAFolder
                .message(&file)
                .contains("it's a file, not a folder")
        );
        // A path through a file is as missing as a path that never existed.
        assert_eq!(
            open_root(&file.join("inside")).unwrap_err().0,
            Unavailable::Missing
        );

        assert_eq!(
            open_root(dir.path()).unwrap(),
            dir.path().canonicalize().unwrap()
        );
        for problem in [
            Unavailable::Missing,
            Unavailable::NotAFolder,
            Unavailable::NoPermission,
            Unavailable::Other,
        ] {
            let message = problem.message(&missing);
            assert!(
                !message.contains("os error") && !message.contains(": "),
                "{message}"
            );
        }
        assert!(
            Unavailable::NoPermission
                .message(&missing)
                .contains("isn't allowed to open")
        );
    }

    #[test]
    fn long_paths_keep_the_drive_and_the_folder_name() {
        let long = PathBuf::from(format!(
            "/Volumes/Samsung T7/{}/Lisbon trip",
            "nested/".repeat(30)
        ));
        let message = Unavailable::Missing.message(&long);
        assert!(message.contains("“/Volumes/Samsung T7/"));
        assert!(message.contains("…"));
        assert!(message.contains("/Lisbon trip”"));
        assert!(shown_path(&long).chars().count() <= 100);
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_folders_ask_for_access() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let locked = dir.path().join("locked");
        std::fs::create_dir(&locked).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        let result = open_root(&locked);
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root reads anything; the check only means something for other users.
        let Err((problem, _)) = result else {
            return;
        };
        assert_eq!(problem, Unavailable::NoPermission);
    }
}
