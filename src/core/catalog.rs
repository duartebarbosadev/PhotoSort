use super::model::*;
use super::text;
use anyhow::{Context, Result};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use walkdir::WalkDir;

/// Folders an operating system or a NAS keeps for itself: trash, thumbnails
/// and indexes. What is inside is not the library's.
const SYSTEM_FOLDERS: [&str; 12] = [
    ".Trashes",
    ".Trash",
    "$RECYCLE.BIN",
    "System Volume Information",
    ".Spotlight-V100",
    ".fseventsd",
    ".TemporaryItems",
    ".DocumentRevisions-V100",
    "@eaDir",
    "#recycle",
    "@Recycle",
    ".@__thumb",
];

/// True for a folder the scan must not enter or list. The folder the walk
/// starts from is never one, so a library can still be opened anywhere.
pub(crate) fn is_system_folder(entry: &walkdir::DirEntry) -> bool {
    entry.depth() > 0
        && entry.file_type().is_dir()
        && entry.file_name().to_str().is_some_and(|name| {
            // Linux keeps one trash per user at the top of a volume: ".Trash-1000".
            name.starts_with(".Trash-")
                || SYSTEM_FOLDERS
                    .iter()
                    .any(|known| name.eq_ignore_ascii_case(known))
        })
}

/// True when the system itself keeps a file's "._name" sibling with it on the
/// volume holding `path`. macOS does on volumes without extended attributes
/// of their own (exFAT, FAT, most network shares): it moves and trashes the
/// sibling together with the file. Elsewhere "._name" is an ordinary file.
fn system_moves_apple_double(path: &Path) -> bool {
    #[cfg(target_os = "macos")]
    {
        use std::os::unix::ffi::OsStrExt;
        #[repr(C, packed(4))]
        struct Reply {
            length: u32,
            capabilities: libc::vol_capabilities_attr_t,
        }
        let Ok(path) = std::ffi::CString::new(path.as_os_str().as_bytes()) else {
            return true;
        };
        let mut request = libc::attrlist {
            bitmapcount: libc::ATTR_BIT_MAP_COUNT,
            reserved: 0,
            commonattr: 0,
            volattr: libc::ATTR_VOL_INFO | libc::ATTR_VOL_CAPABILITIES,
            dirattr: 0,
            fileattr: 0,
            forkattr: 0,
        };
        let mut reply = std::mem::MaybeUninit::<Reply>::zeroed();
        // SAFETY: `path` is a NUL-terminated string, `request` and `reply`
        // are valid for the call, and the reply size is the buffer's own.
        let status = unsafe {
            libc::getattrlist(
                path.as_ptr(),
                (&raw mut request).cast(),
                reply.as_mut_ptr().cast(),
                size_of::<Reply>(),
                0,
            )
        };
        if status != 0 {
            // Unknown: leave the files to the system, which is never wrong
            // about a file it does not manage.
            return true;
        }
        // SAFETY: zeroed before the call and filled by it.
        let capabilities = unsafe { reply.assume_init() }.capabilities;
        let interfaces = libc::VOL_CAPABILITIES_INTERFACES;
        let own = libc::VOL_CAP_INT_EXTENDED_ATTR;
        capabilities.valid[interfaces] & own == 0
            || capabilities.capabilities[interfaces] & own == 0
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
        false
    }
}

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
    /// The system moves "._name" files with their file on this library's
    /// volume, so PhotoSort plans nothing for them. See the function.
    pub system_moves_apple_double: bool,
    /// XMP files that name no photo or video beside them. Set by the scan;
    /// they are supporting files, so they follow their folder.
    pub unowned_sidecars: HashSet<PathBuf>,
    /// The library's volume ignores letter case in names: "anna" and "Anna"
    /// are one folder. Set by the scan. See [`Catalog::same_folder`].
    pub names_ignore_case: bool,
    /// The library's volume takes the composed and decomposed spellings of
    /// an accented name ("é" as one character, or as "e" and a combining
    /// accent) for one folder, as APFS does. Set by the scan.
    pub names_ignore_form: bool,
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
            system_moves_apple_double: system_moves_apple_double(&root),
            root: root.clone(),
            ..Default::default()
        };
        let mut pending = Vec::with_capacity(128);
        // Media and Organize inventory are discovered inside hidden folders
        // too. Keep one complete walk; do not follow file/directory links.
        // Only OS trash and system folders are left out, with all they hold.
        let walk = WalkDir::new(&root)
            .follow_links(false)
            .into_iter()
            .filter_entry(|entry| !is_system_folder(entry));
        for entry in walk {
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
        catalog.names_ignore_case = names_ignore_case(&catalog);
        catalog.names_ignore_form = names_ignore_form(&catalog);
        // A sidecar whose group holds no photo or video belongs to nothing.
        catalog.unowned_sidecars = catalog
            .companion_index()
            .into_values()
            .filter(|group| group.iter().all(|path| is_xmp(path)))
            .flatten()
            .collect();
        // The single walk owns discovery. Capture only eligible supporting-file
        // identities once, after every media directory is known; no content reads.
        let directories: HashSet<_> = catalog
            .media
            .iter()
            .filter_map(|m| m.path.parent())
            .collect();
        for path in &catalog.inventory {
            anyhow::ensure!(!cancelled(), "Cancelled");
            if catalog.is_supporting_file(path, &directories) {
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
    /// files in media directories or their descendants. An XMP file is one of
    /// them only when no photo or video beside it owns it.
    pub fn organize_files(&self) -> Vec<(MediaId, &Path)> {
        let directories: HashSet<_> = self.media.iter().filter_map(|m| m.path.parent()).collect();
        let mut files: Vec<_> = self
            .media
            .iter()
            .map(|m| (m.id, m.path.as_path()))
            .collect();
        files.extend(self.supporting_in(&directories));
        files
    }

    /// The files that are not photos or videos and follow their folder when
    /// it is renamed or moved: a lone XMP, a notes file, an AppleDouble file
    /// the system does not move itself.
    pub fn supporting_files(&self) -> Vec<(MediaId, &Path)> {
        let directories: HashSet<_> = self.media.iter().filter_map(|m| m.path.parent()).collect();
        self.supporting_in(&directories)
    }
    fn supporting_in(&self, directories: &HashSet<&Path>) -> Vec<(MediaId, &Path)> {
        let mut extras: Vec<_> = self
            .inventory
            .iter()
            .filter(|path| self.is_supporting_file(path, directories))
            .map(|path| (MediaId::for_path(path), path.as_path()))
            .collect();
        extras.sort_by(|a, b| a.1.cmp(b.1));
        extras.dedup_by(|a, b| a.1 == b.1);
        extras
    }

    /// AppleDouble files ("._IMG_0001.JPG", "._IMG_0001.xmp") are supporting
    /// files whatever their extension says, so they follow their folder.
    /// Not where the system moves one together with the file it belongs to:
    /// a second, planned move would find it already gone.
    fn is_supporting_file(&self, path: &Path, directories: &HashSet<&Path>) -> bool {
        let supporting = if is_apple_double(path) {
            !self.system_moves_apple_double
        } else if is_xmp(path) {
            self.unowned_sidecars.contains(path)
        } else {
            MediaKind::from_path(path).is_none()
        };
        supporting
            && path
                .parent()
                .is_some_and(|parent| parent.ancestors().any(|p| directories.contains(p)))
    }

    /// Files that share a name in one folder, keyed by that name without its
    /// extension: the photos, RAW files and videos called "IMG_1" and their
    /// XMP sidecars, both "IMG_1.xmp" and, as darktable and digiKam write
    /// them, "IMG_1.CR2.xmp". A sidecar that names nothing there is alone
    /// under its own name. Media come first in a group.
    /// Build once from the inventory, never walk the filesystem per photo/workflow.
    pub fn companion_index(&self) -> HashMap<PathBuf, Vec<PathBuf>> {
        let mut index: HashMap<PathBuf, Vec<PathBuf>> = HashMap::new();
        let mut sidecars = Vec::new();
        for path in &self.inventory {
            if is_apple_double(path) {
                continue;
            }
            if MediaKind::from_path(path).is_some() {
                index
                    .entry(path.with_extension(""))
                    .or_default()
                    .push(path.clone());
            } else if is_xmp(path) {
                sidecars.push(path);
            }
        }
        for path in sidecars {
            let named = path.with_extension("");
            let key = match MediaKind::from_path(&named) {
                // "IMG_1.CR2.xmp" beside "IMG_1.CR2" joins the group "IMG_1".
                Some(_) => {
                    let name = named.with_extension("");
                    match index.get(&name) {
                        Some(group) if group.contains(&named) => name,
                        _ => named,
                    }
                }
                None => named,
            };
            index.entry(key).or_default().push(path.clone());
        }
        index
    }

    /// Whether `a` and `b` are one folder of this library: the same path, or
    /// paths that differ only in what the volume ignores, letter case or the
    /// composed and decomposed spelling of accents. No filesystem access.
    /// Not valid Unicode: only the same path.
    pub fn same_folder(&self, a: &Path, b: &Path) -> bool {
        a == b
            || ((self.names_ignore_case || self.names_ignore_form)
                && a.to_str().zip(b.to_str()).is_some_and(|(a, b)| {
                    text::same_name(a, b, self.names_ignore_case, self.names_ignore_form)
                }))
    }

    /// The key under which the library's volume keeps one name: two paths
    /// with the same key may be one folder there. Only a hint where it is
    /// not the path itself; the volume has the last word.
    pub fn name_key<'a>(&self, path: &'a str) -> std::borrow::Cow<'a, str> {
        text::name_key(path, self.names_ignore_case, self.names_ignore_form)
    }
}

/// True for an XMP sidecar, by its extension in any letter case.
pub fn is_xmp(path: &Path) -> bool {
    path.extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("xmp"))
}

/// The one file in `group` (an entry of [`Catalog::companion_index`]) that
/// `sidecar` names in full: "IMG_1.CR2" for "IMG_1.CR2.xmp". `None` for
/// "IMG_1.xmp", which belongs to every photo of that name.
pub fn sidecar_owner<'a>(group: &'a [PathBuf], sidecar: &Path) -> Option<&'a PathBuf> {
    if !is_xmp(sidecar) {
        return None;
    }
    let named = sidecar.with_extension("");
    group
        .iter()
        .find(|path| **path == named && MediaKind::from_path(path).is_some())
}

/// Whether `a` and `b` are one folder on disk under two spellings. The
/// volume answers (device and inode, or volume and file index on Windows).
/// Neither may be a link, and a folder that cannot be opened counts as no.
pub(crate) fn one_folder_on_disk(a: &Path, b: &Path) -> bool {
    let folder = |path: &Path| std::fs::symlink_metadata(path).is_ok_and(|m| m.is_dir());
    folder(a) && folder(b) && same_file::is_same_file(a, b).unwrap_or(false)
}

/// Asks the volume once, with a folder the scan found: spelled in the other
/// letter case, is it still that folder? Unknown counts as no, which plans
/// as before.
fn names_ignore_case(catalog: &Catalog) -> bool {
    let swap = |c: char| {
        if c.is_ascii_lowercase() {
            c.to_ascii_uppercase()
        } else {
            c.to_ascii_lowercase()
        }
    };
    catalog
        .directories
        .iter()
        .find_map(|folder| {
            let name = folder.file_name()?.to_str()?;
            let identity = catalog.directory_identities.get(folder)?;
            (name.contains(|c: char| c.is_ascii_alphabetic()) && *identity != [0; 4]).then(|| {
                let other = folder.with_file_name(name.chars().map(swap).collect::<String>());
                one_folder_on_disk(folder, &other)
            })
        })
        .unwrap_or(false)
}

/// Asks the volume once, with a folder the scan found whose name has
/// accents: spelled in the other form, composed for decomposed, is it still
/// that folder? A library with no such folder has nothing to merge. Unknown
/// counts as no, which plans as before.
fn names_ignore_form(catalog: &Catalog) -> bool {
    catalog
        .directories
        .iter()
        .find_map(|folder| {
            let other = text::other_form(folder.file_name()?.to_str()?)?;
            let identity = catalog.directory_identities.get(folder)?;
            (*identity != [0; 4]).then(|| one_folder_on_disk(folder, &folder.with_file_name(other)))
        })
        .unwrap_or(false)
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
                "Can't open “{folder}” because it doesn't exist or isn't connected. If it's on an external drive, connect it and try again."
            ),
            Self::NotAFolder => format!(
                "Can't open “{folder}” because it's a file, not a folder. Choose the folder that holds your photos."
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
