//! Editable organization intent. No scanning, decoding or filesystem mutations.
use super::{
    model::{Media, MediaId, MediaKind},
    operations::safe_component,
};
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Draft {
    labels: HashMap<MediaId, String>,
    folders: BTreeSet<String>,
    explicit: BTreeSet<String>,
    empty_origins: BTreeMap<String, String>,
    directory_origins: BTreeMap<String, String>,
}
/// A staged selection shared by drag/drop and explicit move controls.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub files: HashSet<MediaId>,
    pub folders: BTreeSet<String>,
}
impl Selection {
    // A selected ancestor already owns its descendants. Keep only subtree roots.
    fn roots(&self) -> Vec<&str> {
        self.folders
            .iter()
            .filter(|path| {
                let mut parent = path.as_str();
                while let Some((prefix, _)) = parent.rsplit_once('/') {
                    if self.folders.contains(prefix) {
                        return false;
                    }
                    parent = prefix;
                }
                true
            })
            .map(String::as_str)
            .collect()
    }
}
/// Directory intent travels with the same file plan; no UI filesystem actions.
#[derive(Clone, Debug, Default)]
pub struct DirectoryEdits {
    pub required: BTreeSet<String>,
    pub moved_empty: Vec<(String, String)>,
    pub renamed: Vec<(String, String)>,
}
fn within(path: &str, parent: &str) -> bool {
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|s| s.starts_with('/'))
}
fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
fn join(parent: &str, name: &str) -> String {
    if parent.is_empty() {
        name.to_owned()
    } else {
        format!("{parent}/{name}")
    }
}
fn validate_leaf(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && safe_component(name) == name,
        "Use a folder name without slashes, reserved characters, surrounding spaces or trailing dots (up to 100 characters)"
    );
    Ok(())
}
impl Draft {
    pub fn new(labels: HashMap<MediaId, String>) -> Self {
        let mut draft = Self {
            labels,
            folders: BTreeSet::from([String::new()]),
            explicit: BTreeSet::new(),
            empty_origins: BTreeMap::new(),
            directory_origins: BTreeMap::new(),
        };
        for label in draft.labels.values().cloned().collect::<Vec<_>>() {
            draft.include_parents(&label);
        }
        draft
    }
    pub fn from_catalog(
        catalog: &super::catalog::Catalog,
        labels: HashMap<MediaId, String>,
    ) -> Self {
        let mut draft = Self::new(labels);
        let occupied: HashSet<_> = catalog
            .inventory
            .iter()
            .flat_map(|path| path.parent().into_iter().flat_map(|p| p.ancestors()))
            .collect();
        for path in &catalog.directories {
            if let Ok(relative) = path.strip_prefix(&catalog.root) {
                let label = relative.to_string_lossy().replace('\\', "/");
                draft.include_parents(&label);
                if !label.is_empty() {
                    draft.directory_origins.insert(label.clone(), label.clone());
                }
                if !label.is_empty() && !occupied.contains(path.as_path()) {
                    draft.empty_origins.insert(label.clone(), label);
                }
            }
        }
        draft
    }
    /// A destination may combine several original folders. Expose an original
    /// only when it is unambiguous; newly created destinations have none.
    pub fn directory_source(&self, destination: &str) -> Option<&str> {
        if destination.is_empty() {
            return Some("");
        }
        let mut sources = self
            .directory_origins
            .iter()
            .filter(|(_, target)| target.as_str() == destination)
            .map(|(source, _)| source.as_str());
        let first = sources.next()?;
        sources.next().is_none().then_some(first)
    }
    pub fn directory_edits(&self) -> DirectoryEdits {
        DirectoryEdits {
            renamed: self
                .directory_origins
                .iter()
                .filter(|(source, target)| source != target)
                .map(|(source, target)| (source.clone(), target.clone()))
                .collect(),
            required: self
                .explicit
                .iter()
                .cloned()
                .chain(self.empty_origins.values().cloned())
                .collect(),
            moved_empty: self
                .empty_origins
                .iter()
                .filter(|(source, target)| source != target)
                .map(|(source, target)| (source.clone(), target.clone()))
                .collect(),
        }
    }
    fn include_parents(&mut self, path: &str) {
        self.folders.insert(path.into());
        let mut parent = path;
        while let Some((prefix, _)) = parent.rsplit_once('/') {
            self.folders.insert(prefix.into());
            parent = prefix;
        }
    }
    pub fn labels(&self) -> &HashMap<MediaId, String> {
        &self.labels
    }
    pub fn folders(&self) -> &BTreeSet<String> {
        &self.folders
    }
    pub fn create_folder(&mut self, parent: &str, name: &str) -> Result<String> {
        ensure!(
            self.folders.contains(parent),
            "Parent folder no longer exists"
        );
        validate_leaf(name)?;
        let path = join(parent, name);
        ensure!(
            !self.folders.contains(&path),
            "A folder with that name already exists"
        );
        self.folders.insert(path.clone());
        self.explicit.insert(path.clone());
        Ok(path)
    }
    pub fn move_photos(&mut self, ids: &[MediaId], target: &str) -> Result<()> {
        ensure!(
            self.folders.contains(target),
            "Destination folder no longer exists"
        );
        ensure!(
            ids.iter().all(|id| self.labels.contains_key(id)),
            "Selection contains an unavailable photo"
        );
        for id in ids {
            self.labels.insert(*id, target.to_owned());
        }
        Ok(())
    }
    /// Cheap validation for hover feedback. Does not clone the catalog/draft or
    /// access files. Execution validates again before changing any intent.
    pub fn validate_move(&self, selection: &Selection, target: &str) -> Result<()> {
        ensure!(
            self.folders.contains(target),
            "Destination folder no longer exists"
        );
        ensure!(
            !selection.files.is_empty() || !selection.folders.is_empty(),
            "Nothing selected"
        );
        ensure!(
            selection
                .files
                .iter()
                .all(|id| self.labels.contains_key(id)),
            "Selection contains an unavailable file"
        );
        ensure!(
            selection
                .folders
                .iter()
                .all(|p| !p.is_empty() && self.folders.contains(p)),
            "Selection contains the library root or an unavailable folder"
        );
        let mut destinations = HashSet::new();
        for source in selection.roots() {
            ensure!(!within(target, source), "A folder cannot contain itself");
            let destination = join(target, leaf(source));
            ensure!(
                destination == source || !self.folders.contains(&destination),
                "Destination folder already exists"
            );
            ensure!(
                destinations.insert(destination),
                "Selected folders have the same destination name"
            );
        }
        Ok(())
    }
    /// All selected items move or none do. Companion expansion belongs here so
    /// buttons and drag/drop cannot produce different RAW/JPEG destinations.
    pub fn move_selection(
        &mut self,
        selection: &Selection,
        target: &str,
        media: &[Media],
        companions: bool,
    ) -> Result<()> {
        self.validate_move(selection, target)?;
        let mut files = selection.files.clone();
        if companions {
            let stems: HashSet<_> = media
                .iter()
                .filter(|m| m.kind != MediaKind::Video && files.contains(&m.id))
                .map(|m| m.path.with_extension(""))
                .collect();
            files.extend(
                media
                    .iter()
                    .filter(|m| {
                        m.kind != MediaKind::Video
                            && stems.contains(&m.path.with_extension(""))
                            && self.labels.contains_key(&m.id)
                    })
                    .map(|m| m.id),
            );
        }
        let mut next = self.clone();
        // Explicitly selected files move directly to the target, matching the
        // reference's file-then-folder order for mixed selections.
        next.move_photos(&files.into_iter().collect::<Vec<_>>(), target)?;
        for source in selection.roots() {
            next.nest_folder(source, target)?;
        }
        *self = next;
        Ok(())
    }
    /// Rewrite the whole subtree atomically. Refuse ambiguous folder merges.
    fn relocate(&mut self, source: &str, target: &str) -> Result<String> {
        ensure!(!source.is_empty(), "The library root cannot be moved");
        ensure!(
            self.folders.contains(source),
            "Source folder no longer exists"
        );
        if source == target {
            return Ok(target.into());
        }
        ensure!(!within(target, source), "A folder cannot contain itself");
        ensure!(
            !self.folders.contains(target),
            "Destination folder already exists"
        );
        let rewrite = |path: &str| {
            if within(path, source) {
                format!("{target}{}", &path[source.len()..])
            } else {
                path.to_owned()
            }
        };
        self.folders = self.folders.iter().map(|p| rewrite(p)).collect();
        self.explicit = self.explicit.iter().map(|p| rewrite(p)).collect();
        for target in self.directory_origins.values_mut() {
            *target = rewrite(target);
        }
        for target in self.empty_origins.values_mut() {
            *target = rewrite(target);
        }
        for path in self.labels.values_mut() {
            *path = rewrite(path);
        }
        self.include_parents(target);
        Ok(target.into())
    }
    pub fn rename_folder(&mut self, source: &str, name: &str) -> Result<String> {
        validate_leaf(name)?;
        let parent = source.rsplit_once('/').map_or("", |(p, _)| p);
        self.relocate(source, &join(parent, name))
    }
    pub fn nest_folder(&mut self, source: &str, parent: &str) -> Result<String> {
        ensure!(
            self.folders.contains(parent),
            "Parent folder no longer exists"
        );
        ensure!(
            source != parent && !within(parent, source),
            "A folder cannot contain itself"
        );
        self.relocate(source, &join(parent, leaf(source)))
    }
    pub fn unnest_folder(&mut self, source: &str) -> Result<String> {
        self.relocate(source, leaf(source))
    }
    pub fn create_parent(&mut self, source: &str, name: &str) -> Result<String> {
        validate_leaf(name)?;
        let parent = source.rsplit_once('/').map_or("", |(p, _)| p);
        let new_parent = join(parent, name);
        ensure!(
            !self.folders.contains(&new_parent),
            "Parent folder already exists"
        );
        let target = self.relocate(source, &join(&new_parent, leaf(source)))?;
        self.explicit.insert(new_parent);
        Ok(target)
    }
}

/// Additional files keep their current relative folder until the user edits the
/// hierarchy or moves a selection. Generated media groups do not guess where
/// unrelated documents belong. Uses only the shared scan inventory.
pub fn include_inventory(catalog: &super::catalog::Catalog, labels: &mut HashMap<MediaId, String>) {
    for (id, path) in catalog.organize_files() {
        if super::model::MediaKind::from_path(path).is_none()
            && let Some(parent) = path
                .parent()
                .and_then(|p| p.strip_prefix(&catalog.root).ok())
        {
            labels
                .entry(id)
                .or_insert_with(|| parent.to_string_lossy().replace('\\', "/"));
        }
    }
}

// A photo cohort is distinct from an extensionless supporting file at that path.
fn name_key(path: &std::path::Path, companions: bool) -> (std::path::PathBuf, bool) {
    let cohort = companions && MediaKind::from_path(path).is_some_and(|k| k != MediaKind::Video);
    (
        if cohort {
            path.with_extension("")
        } else {
            path.to_path_buf()
        },
        cohort,
    )
}
/// Resolve naming intent before visiting files, so RAW/JPEG order cannot discard
/// a request. This uses the existing inventory only; no scanning or image work.
pub fn resolve_file_names(
    files: &[(MediaId, &std::path::Path)],
    names: &HashMap<MediaId, String>,
    companions: bool,
    cancelled: impl Fn() -> bool,
) -> Result<HashMap<MediaId, String>> {
    ensure!(!cancelled(), "Naming cancelled");
    if names.values().all(|name| name.trim().is_empty()) {
        return Ok(HashMap::new());
    }
    let mut requested = HashMap::new();
    for (i, (id, path)) in files.iter().enumerate() {
        if i % 256 == 0 {
            ensure!(!cancelled(), "Naming cancelled");
        }
        let Some(name) = names.get(id).filter(|v| !v.trim().is_empty()) else {
            continue;
        };
        if path.file_stem().and_then(|v| v.to_str()) == Some(name.as_str()) {
            continue;
        }
        let name = safe_component(name);
        if path.file_stem().and_then(|v| v.to_str()) == Some(name.as_str()) {
            continue;
        }
        let key = name_key(path, companions);
        if let Some(previous) = requested.insert(key, name.clone()) {
            ensure!(
                previous == name,
                "Photo companions have conflicting names: {previous} and {name}"
            );
        }
    }
    let mut resolved = HashMap::new();
    for (i, (id, path)) in files.iter().enumerate() {
        if i % 256 == 0 {
            ensure!(!cancelled(), "Naming cancelled");
        }
        if let Some(name) = requested.get(&name_key(path, companions)) {
            resolved.insert(*id, name.clone());
        }
    }
    ensure!(!cancelled(), "Naming cancelled");
    Ok(resolved)
}
/// Atomically edit or restore a base name and its enabled photo companions.
/// The editor owns this disposable draft until Keep/Preview accepts it.
pub fn set_file_name(
    catalog: &super::catalog::Catalog,
    names: &mut HashMap<MediaId, String>,
    id: MediaId,
    name: &str,
    companions: bool,
) -> Result<()> {
    let files = catalog.organize_files();
    let path = files
        .iter()
        .find(|(candidate, _)| *candidate == id)
        .map(|(_, path)| *path)
        .ok_or_else(|| anyhow::anyhow!("File is no longer available"))?;
    let unchanged = path.file_stem().and_then(|stem| stem.to_str()) == Some(name);
    if !name.is_empty() && !unchanged {
        validate_file_stem(name)?;
    }
    let key = name_key(path, companions);
    let mut next = names.clone();
    for (id, path) in &files {
        if name_key(path, companions) == key {
            next.remove(id);
        }
    }
    if !name.is_empty() && !unchanged {
        next.insert(id, name.to_owned());
    }
    *names = resolve_file_names(&files, &next, companions, || false)?;
    Ok(())
}
/// Shared validation for new base names in drafts and saved move plans.
pub fn validate_file_stem(name: &str) -> Result<()> {
    ensure!(
        !name.is_empty() && safe_component(name) == name,
        "Use a name without slashes, reserved characters, surrounding spaces or trailing dots (up to 100 characters)"
    );
    Ok(())
}
/// Preserve the original extension and spelling, including extensionless files.
pub fn renamed_file_name(path: &std::path::Path, stem: &std::ffi::OsStr) -> std::ffi::OsString {
    let mut name = stem.to_os_string();
    if let Some(extension) = path.extension() {
        name.push(".");
        name.push(extension);
    }
    name
}
