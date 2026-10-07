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
        // The reference discovers media and Organize inventory inside hidden
        // folders too. Keep one complete walk; do not follow file/directory links.
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
    /// Organize reuses the scan inventory. Match the reference scope: extra
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
