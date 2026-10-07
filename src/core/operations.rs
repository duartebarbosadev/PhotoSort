mod case_rename;
mod directory;
pub use directory::TrashDirectory;
#[cfg(test)]
mod move_tests;
pub mod recovery;
use super::{catalog::Catalog, metadata, model::*, session};
use anyhow::{Context, Result};
pub use case_rename::DirectoryRename;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Source {
    pub path: PathBuf,
    pub fingerprint: Fingerprint,
}
impl Source {
    pub fn capture(path: PathBuf) -> Result<Self> {
        let fingerprint = Fingerprint::read(&path)?;
        Ok(Self { path, fingerprint })
    }
    fn verify(&self) -> Result<()> {
        let fingerprint = Fingerprint::read(&self.path)
            .with_context(|| format!("Could not verify reviewed source {}", self.path.display()))?;
        anyhow::ensure!(
            fingerprint == self.fingerprint,
            "File changed since review: {}",
            self.path.display()
        );
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum Action {
    RenameDirectory(DirectoryRename),
    TrashDirectory(TrashDirectory),
    CreateDirectory {
        path: PathBuf,
    },
    RemoveEmptyDirectory {
        path: PathBuf,
        root: PathBuf,
        after: Vec<PathBuf>,
    },
    Move {
        files: Vec<(Source, PathBuf)>,
    },
    /// Recovery-only intent: a verified same-filesystem destination already
    /// exists. Explicit retry rechecks both identities before removing sources.
    FinishMove {
        files: Vec<(Source, PathBuf)>,
    },
    Trash {
        files: Vec<Source>,
    },
    /// One atomic spelling change per receipt, so partial companion progress
    /// remains explicit and retry never repeats already completed files.
    RenameFile {
        source: Source,
        to: PathBuf,
    },
    Metadata {
        source: Source,
        sidecar_before: Option<Fingerprint>,
        #[serde(default)]
        target: metadata::WriteTarget,
        rating: Option<u8>,
        orientation: Option<u8>,
    },
}
impl Action {
    fn failure_message(&self, detail: &str) -> String {
        format!("{}: {detail}", self.label())
    }

    pub fn label(&self) -> String {
        match self {
            Self::TrashDirectory(folder) => format!(
                "Trash folder {} ({} files)",
                folder.path.display(),
                folder.inventory.files.len()
            ),
            Self::RenameFile { source, to } => {
                format!("Rename file {} → {}", source.path.display(), to.display())
            }
            Self::RenameDirectory(rename) => format!(
                "Rename folder {} → {}",
                rename.from.display(),
                rename.to.display()
            ),
            Self::CreateDirectory { path } => format!("Create folder {}", path.display()),
            Self::RemoveEmptyDirectory { path, .. } => {
                format!("Remove only if empty: {}", path.display())
            }
            Self::FinishMove { files } => format!("Finish {} staged file move(s)", files.len()),
            Self::Move { files } => files.first().map_or_else(
                || "Move 0 files".into(),
                |(_, destination)| {
                    format!(
                        "Move {} file(s) to {}",
                        files.len(),
                        destination.parent().unwrap_or(Path::new("")).display()
                    )
                },
            ),
            Self::Trash { files } => files.first().map_or_else(
                || "Trash 0 files".into(),
                |source| {
                    format!(
                        "Trash {} file(s): {}",
                        files.len(),
                        source
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                    )
                },
            ),
            Self::Metadata {
                source,
                target,
                rating,
                orientation,
                ..
            } => {
                let mut details = Vec::new();
                if let Some(rating) = rating {
                    details.push(if *rating == 0 {
                        "clear rating".to_string()
                    } else {
                        format!("{rating} stars")
                    });
                }
                if let Some(orientation) = orientation {
                    details.push(format!(
                        "orientation {}",
                        match orientation {
                            1 => "upright",
                            2 => "mirrored",
                            3 => "180 degrees",
                            4 => "flipped",
                            5 => "transposed",
                            6 => "90 degrees clockwise",
                            7 => "transverse",
                            8 => "90 degrees counterclockwise",
                            _ => "unknown",
                        }
                    ));
                }
                format!(
                    "{} · {} · {}",
                    source
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy(),
                    match target {
                        metadata::WriteTarget::Embedded => "embedded metadata",
                        metadata::WriteTarget::Sidecar => "XMP sidecar",
                    },
                    details.join(" · ")
                )
            }
        }
    }
    pub fn sources(&self) -> Vec<&Path> {
        match self {
            Self::TrashDirectory(folder) => folder
                .inventory
                .files
                .iter()
                .map(|s| s.path.as_path())
                .chain(std::iter::once(folder.path.as_path()))
                .collect(),
            Self::RenameDirectory(rename) => rename
                .inventory
                .files
                .iter()
                .map(|s| s.path.as_path())
                .collect(),
            Self::Move { files } | Self::FinishMove { files } => {
                files.iter().map(|(s, _)| s.path.as_path()).collect()
            }
            Self::Trash { files } => files.iter().map(|s| s.path.as_path()).collect(),
            Self::Metadata { source, .. } | Self::RenameFile { source, .. } => {
                vec![source.path.as_path()]
            }
            Self::CreateDirectory { .. } | Self::RemoveEmptyDirectory { .. } => Vec::new(),
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Plan {
    pub actions: Vec<Action>,
    pub description: String,
    /// Organize opens its destination after all reviewed operations finish.
    #[serde(default)]
    pub output_root: Option<PathBuf>,
    /// Completed prerequisites from an earlier attempt, for conditional cleanup.
    #[serde(default)]
    pub completed_dependencies: HashSet<PathBuf>,
}
#[derive(Debug, Default)]
pub struct Report {
    pub recovery: Option<recovery::Ticket>,
    pub completed: Vec<Action>,
    pub errors: Vec<String>,
    pub cancelled: bool,
    pub remaining: Plan,
    /// Verified identity transitions; embedded edits retain their path ID.
    pub relocated: Vec<(MediaId, MediaId, Fingerprint)>,
}

impl Report {
    fn record_relocation(&mut self, old: MediaId, new: MediaId, fingerprint: Fingerprint) {
        let mut chained = false;
        for (_, destination, saved) in &mut self.relocated {
            if *destination == old {
                *destination = new;
                *saved = fingerprint;
                chained = true;
            }
        }
        if !chained {
            self.relocated.push((old, new, fingerprint));
        }
    }

    fn complete(&mut self, action: &Action, relocated: &[(MediaId, MediaId, Fingerprint)]) {
        for &(old, new, fingerprint) in relocated {
            self.record_relocation(old, new, fingerprint);
        }
        match action {
            Action::Move { files } | Action::FinishMove { files } => self
                .remaining
                .completed_dependencies
                .extend(files.iter().map(|(s, _)| s.path.clone())),
            Action::RenameFile { source, .. } => {
                self.remaining
                    .completed_dependencies
                    .insert(source.path.clone());
            }
            Action::CreateDirectory { path } => {
                self.remaining.completed_dependencies.insert(path.clone());
            }
            _ => {}
        }
        self.completed.push(action.clone());
    }

    /// Verified final identities whose completed move destination is in this
    /// library. Component-aware paths exclude similarly named sibling roots.
    pub fn relocations_into(&self, root: &Path) -> Vec<(MediaId, MediaId, Fingerprint)> {
        let mut destinations = HashSet::new();
        for action in &self.completed {
            match action {
                Action::Move { files } | Action::FinishMove { files } => {
                    destinations.extend(
                        files
                            .iter()
                            .filter(|(_, to)| to.starts_with(root))
                            .map(|(_, to)| MediaId::for_path(to)),
                    );
                }
                Action::RenameFile { to, .. } => {
                    if to.starts_with(root) {
                        destinations.insert(MediaId::for_path(to));
                    }
                }
                Action::RenameDirectory(rename) => {
                    destinations.extend(
                        rename
                            .inventory
                            .files
                            .iter()
                            .map(|source| rename.destination(&source.path))
                            .filter(|to| to.starts_with(root))
                            .map(|to| MediaId::for_path(&to)),
                    );
                }
                _ => {}
            }
        }
        self.relocated
            .iter()
            .filter(|(old, new, _)| old != new && destinations.contains(new))
            .copied()
            .collect()
    }

    pub fn applied_decisions(&self, media: &[Media]) -> HashSet<MediaId> {
        Self {
            completed: self
                .completed
                .iter()
                .filter(|a| {
                    matches!(
                        a,
                        Action::Trash { .. } | Action::TrashDirectory(_) | Action::Metadata { .. }
                    )
                })
                .cloned()
                .collect(),
            ..Default::default()
        }
        .affected_media(media)
    }

    /// A shared XMP write also affects RAW/JPEG companions. Resolve from the
    /// catalog, without filesystem reads or another scan on the UI thread.
    pub fn affected_media(&self, media: &[Media]) -> HashSet<MediaId> {
        let mut ids: HashSet<_> = self
            .completed
            .iter()
            .flat_map(Action::sources)
            .map(MediaId::for_path)
            .collect();
        let stems: HashSet<_> = self
            .completed
            .iter()
            .filter_map(|action| match action {
                Action::Metadata {
                    source,
                    target: metadata::WriteTarget::Sidecar,
                    ..
                } => Some(source.path.with_extension("")),
                _ => None,
            })
            .collect();
        for item in media {
            if item.kind != MediaKind::Video && stems.contains(&item.path.with_extension("")) {
                ids.insert(item.id);
            }
        }
        ids
    }
}

/// Shared Trash expansion for staged decisions and explicit context-menu requests.
fn trash_actions(
    catalog: &Catalog,
    rejected: &HashSet<MediaId>,
    companions: bool,
    cancelled: &dyn Fn() -> bool,
) -> Result<(Vec<Action>, HashSet<PathBuf>)> {
    let index = catalog.companion_index();
    let mut seen = HashSet::new();
    let mut actions = Vec::new();
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
    for (id, path) in catalog.organize_files() {
        anyhow::ensure!(!cancelled(), "File planning cancelled");
        if rejected.contains(&id) {
            let photo = MediaKind::from_path(path).is_some_and(|kind| kind != MediaKind::Video);
            let files = if companions && photo {
                index
                    .get(&path.with_extension(""))
                    .cloned()
                    .unwrap_or_else(|| vec![path.to_path_buf()])
            } else {
                let mut f = vec![path.to_path_buf()];
                if photo && let Some(pair) = index.get(&path.with_extension("")) {
                    f.extend(
                        pair.iter()
                            .filter(|p| {
                                p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xmp"))
                            })
                            .cloned(),
                    );
                }
                f
            };
            let files = files
                .into_iter()
                .filter(|p| seen.insert(p.clone()))
                .map(|path| {
                    let source = Source::capture(path)?;
                    if let Some(expected) = fingerprints.get(&MediaId::for_path(&source.path)) {
                        anyhow::ensure!(
                            source.fingerprint == *expected,
                            "File changed since discovery: {}",
                            source.path.display()
                        );
                    } else if MediaId::for_path(&source.path) == id {
                        anyhow::bail!("File identity is unavailable: {}", source.path.display());
                    }
                    Ok(source)
                })
                .collect::<Result<Vec<_>>>()?;
            if !files.is_empty() {
                actions.push(Action::Trash { files });
            }
        }
    }
    Ok((actions, seen))
}
#[derive(Clone, Debug, Default)]
pub struct TrashRequest {
    pub files: Vec<MediaId>,
    pub folders: Vec<PathBuf>,
}
/// Build one reviewable plan from the shared discovery inventory. No metadata or
/// image decoding is needed. The executor repeats preflight before OS Trash.
pub fn trash_plan(
    catalog: &Catalog,
    request: &TrashRequest,
    companions: bool,
    cancelled: impl Fn() -> bool,
) -> Result<Plan> {
    anyhow::ensure!(!cancelled(), "Trash planning cancelled");
    let mut roots = request.folders.clone();
    roots.sort();
    roots.dedup();
    let selected: HashSet<_> = roots.iter().cloned().collect();
    roots.retain(|p| !p.ancestors().skip(1).any(|a| selected.contains(a)));
    let files: HashMap<_, _> = catalog.organize_files().into_iter().collect();
    let mut rejected = HashSet::new();
    for id in &request.files {
        anyhow::ensure!(!cancelled(), "Trash planning cancelled");
        let path = files
            .get(id)
            .context("Selected file is no longer available")?;
        if !path.ancestors().any(|p| selected.contains(p)) {
            rejected.insert(*id);
        }
    }
    let inventories = directory::Inventory::capture(catalog, &roots, &cancelled)?;
    let (mut actions, _) = trash_actions(catalog, &rejected, companions, &cancelled)?;
    for (path, inventory) in roots.into_iter().zip(inventories) {
        inventory.verify(&path, &cancelled)?;
        actions.push(Action::TrashDirectory(TrashDirectory {
            path,
            library: catalog.root.clone(),
            inventory,
        }));
    }
    Ok(Plan {
        actions,
        description: "Move the listed files and original folders to system Trash. Folder entries include every discovered file and subfolder. Changed contents stop that operation. Unrelated marks, ratings and Organize edits remain staged.".into(),
        ..Default::default()
    })
}

pub fn decisions_plan(
    catalog: &Catalog,
    decisions: &HashMap<MediaId, Decision>,
    companions: bool,
) -> Result<Plan> {
    decisions_plan_with_metadata(
        catalog,
        decisions,
        companions,
        metadata::WritePolicy::SidecarOnly,
        |m| Ok(metadata::read(&m.path)),
        || false,
    )
}
/// Runtime supplies its shared image actor's metadata cache. Standalone callers
/// may use decisions_plan for the original sidecar-only plan contract.
pub fn decisions_plan_with_metadata(
    catalog: &Catalog,
    decisions: &HashMap<MediaId, Decision>,
    companions: bool,
    policy: metadata::WritePolicy,
    mut load: impl FnMut(&Media) -> Result<Metadata>,
    cancelled: impl Fn() -> bool,
) -> Result<Plan> {
    let rejected = decisions
        .iter()
        .filter(|(_, d)| d.rejected)
        .map(|(id, _)| *id)
        .collect();
    let (mut actions, seen) = trash_actions(catalog, &rejected, companions, &cancelled)?;
    let mut writes: HashMap<PathBuf, (Option<u8>, Option<u8>)> = HashMap::new();
    let mut prepared = Vec::new();
    let mut shared_sidecars = HashSet::new();
    for media in &catalog.media {
        anyhow::ensure!(!cancelled(), "File planning cancelled");
        let Some(d) = decisions.get(&media.id) else {
            continue;
        };
        if d.rejected
            || seen.contains(&media.path)
            || media.kind == MediaKind::Video
            || (d.rating.is_none() && d.turns == 0)
        {
            continue;
        }
        let meta = if d.turns > 0 || policy == metadata::WritePolicy::EmbedWhenSupported {
            load(media)?
        } else {
            Metadata::default()
        };
        let sidecar = metadata::sidecar(&media.path);
        let target = metadata::write_target(&media.path, &meta, policy);
        if target == metadata::WriteTarget::Sidecar {
            shared_sidecars.insert(sidecar.clone());
        }
        let orientation =
            (d.turns > 0).then(|| metadata::rotate_orientation(meta.orientation, d.turns));
        prepared.push((media, d, sidecar, target, orientation));
    }
    for (media, d, sidecar, mut target, orientation) in prepared {
        anyhow::ensure!(!cancelled(), "File planning cancelled");
        // A planned shared sidecar must never silently override a companion's
        // separately embedded edit. Resolve the whole edited stem consistently.
        if shared_sidecars.contains(&sidecar) {
            target = metadata::WriteTarget::Sidecar;
        }
        let values = (d.rating, orientation);
        if target == metadata::WriteTarget::Sidecar
            && let Some(old) = writes.insert(sidecar.clone(), values)
        {
            anyhow::ensure!(
                old == values,
                "RAW/JPEG pair has conflicting sidecar edits: {}",
                sidecar.display()
            );
            continue;
        }
        actions.push(Action::Metadata {
            source: Source {
                path: media.path.clone(),
                fingerprint: media.fingerprint,
            },
            sidecar_before: Fingerprint::read(&sidecar).ok(),
            target,
            rating: d.rating,
            orientation,
        });
    }
    Ok(Plan {
        actions,
        description: "Apply reviewed ratings, rotations and Trash marks".into(),
        ..Default::default()
    })
}

pub fn safe_component(value: &str) -> String {
    let clean: String = value
        .chars()
        .map(|c| {
            if c.is_control() || "/\\:*?\"<>|".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    // Leading dots are valid names, not navigation. Trimming them would
    // silently rename existing hidden folders in an unchanged Current plan.
    let clean = clean.trim().trim_end_matches('.');
    if clean.is_empty() {
        "Unsorted".into()
    } else {
        clean.chars().take(100).collect()
    }
}
/// Cheap, filesystem-free departure check. The worker still builds the exact
/// collision-safe plan before asking the user to resolve changes.
pub fn organize_changes_pending(
    catalog: &Catalog,
    labels: &HashMap<MediaId, String>,
    names: &HashMap<MediaId, String>,
    output: &Path,
    directories: &super::organize::DirectoryEdits,
) -> bool {
    if !directories.moved_empty.is_empty() || !directories.renamed.is_empty() {
        return true;
    }
    let existing: HashSet<_> = catalog.directories.iter().collect();
    if directories
        .required
        .iter()
        .any(|label| !existing.contains(&folder_destination(output, label)))
    {
        return true;
    }
    catalog.organize_files().iter().any(|(id, path)| {
        labels.get(id).is_some_and(|label| {
            path.parent() != Some(folder_destination(output, label).as_path())
                || names
                    .get(id)
                    .filter(|name| !name.trim().is_empty())
                    .is_some_and(|name| {
                        path.file_stem().and_then(|s| s.to_str())
                            != Some(safe_component(name).as_str())
                    })
        })
    })
}

pub fn organize_plan(
    catalog: &Catalog,
    labels: &HashMap<MediaId, String>,
    output: &Path,
    companions: bool,
) -> Result<Plan> {
    organize_plan_with_names(catalog, labels, &HashMap::new(), output, companions)
}

pub fn organize_plan_with_names(
    catalog: &Catalog,
    labels: &HashMap<MediaId, String>,
    names: &HashMap<MediaId, String>,
    output: &Path,
    companions: bool,
) -> Result<Plan> {
    organize_plan_with_layout(
        catalog,
        labels,
        names,
        output,
        companions,
        &Default::default(),
    )
}

pub fn organize_plan_with_layout(
    catalog: &Catalog,
    labels: &HashMap<MediaId, String>,
    names: &HashMap<MediaId, String>,
    output: &Path,
    companions: bool,
    directories: &super::organize::DirectoryEdits,
) -> Result<Plan> {
    organize_plan_with_layout_cancellable(
        catalog,
        labels,
        names,
        output,
        companions,
        directories,
        || false,
    )
}
pub fn organize_plan_with_layout_cancellable(
    catalog: &Catalog,
    labels: &HashMap<MediaId, String>,
    names: &HashMap<MediaId, String>,
    output: &Path,
    companions: bool,
    directories: &super::organize::DirectoryEdits,
    cancelled: impl Fn() -> bool,
) -> Result<Plan> {
    anyhow::ensure!(!cancelled(), "Planning cancelled");
    anyhow::ensure!(
        output.is_absolute(),
        "Output must be an absolute folder path"
    );
    let output = normalized_destination(output)?;
    reject_symlink_components(&output)?;
    let casing = case_rename::prepare(catalog, directories, &output, &cancelled)?;
    let index = catalog.companion_index();
    let organize_files = catalog.organize_files();
    let names =
        super::organize::resolve_file_names(&organize_files, names, companions, &cancelled)?;
    let mut seen = HashSet::new();
    let mut reserved = HashSet::new();
    let mut actions = casing.actions.clone();
    let mut file_names = case_rename::FileNames::default();
    for (id, path) in organize_files {
        anyhow::ensure!(!cancelled(), "Planning cancelled");
        if seen.contains(path) {
            continue;
        }
        let Some(label) = labels.get(&id) else {
            continue;
        };
        let has_companions =
            MediaKind::from_path(path).is_some_and(|kind| kind != MediaKind::Video);
        let sources = if companions && has_companions {
            index
                .get(&path.with_extension(""))
                .cloned()
                .unwrap_or_else(|| vec![path.to_path_buf()])
        } else {
            let mut sources = vec![path.to_path_buf()];
            if has_companions && let Some(pair) = index.get(&path.with_extension("")) {
                sources.extend(
                    pair.iter()
                        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("xmp")))
                        .cloned(),
                );
            }
            sources
        };
        let sources: Vec<_> = sources.into_iter().filter(|p| !seen.contains(p)).collect();
        let dir = folder_destination(&output, label);
        let new_name = names
            .get(&id)
            .filter(|s| !s.trim().is_empty())
            .map(|s| safe_component(s))
            .filter(|name| path.file_stem().and_then(|s| s.to_str()) != Some(name.as_str()));
        if new_name.is_none()
            && sources
                .iter()
                .all(|p| casing.rebase(p).parent() == Some(dir.as_path()))
        {
            seen.extend(sources);
            continue;
        }
        let mut suffix = 0u32;
        let destinations = loop {
            anyhow::ensure!(!cancelled(), "Planning cancelled");
            let destinations: Vec<_> = sources
                .iter()
                .map(|p| {
                    if new_name.is_none() && suffix == 0 {
                        return dir.join(p.file_name().unwrap_or_default());
                    }
                    let mut name = new_name
                        .as_ref()
                        .map(std::ffi::OsString::from)
                        .unwrap_or_else(|| p.file_stem().unwrap_or_default().to_os_string());
                    if suffix > 0 {
                        name.push(format!("_{suffix}"));
                    }
                    dir.join(super::organize::renamed_file_name(p, &name))
                })
                .collect();
            let mut available = true;
            for (source, destination) in sources.iter().zip(&destinations) {
                if reserved.contains(destination)
                    || (casing.occupied(destination)
                        && !file_names.source_alias(
                            source,
                            &casing.original(destination),
                            &cancelled,
                        )?)
                {
                    available = false;
                    break;
                }
            }
            if available {
                break destinations;
            }
            suffix += 1;
        };
        let mut files = Vec::new();
        for (source, dest) in sources.into_iter().zip(destinations) {
            seen.insert(source.clone());
            reserved.insert(dest.clone());
            let mut reviewed = Source::capture(source)?;
            reviewed.path = casing.rebase(&reviewed.path);
            files.push((reviewed, dest));
        }
        if !files.is_empty() {
            if files
                .iter()
                .all(|(source, to)| case_rename::file_spelling_change(&source.path, to))
            {
                actions.extend(
                    files
                        .into_iter()
                        .map(|(source, to)| Action::RenameFile { source, to }),
                );
            } else {
                actions.push(Action::Move { files });
            }
        }
    }
    let adjusted = casing.directory_catalog(catalog);
    let mut directories = directories.clone();
    directories.moved_empty = directories
        .moved_empty
        .iter()
        .filter_map(|(source, target)| {
            let adjusted = casing.rebase(&folder_destination(&catalog.root, source));
            let adjusted = adjusted
                .strip_prefix(&catalog.root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            (adjusted != *target).then(|| (adjusted, target.clone()))
        })
        .collect();
    add_directory_actions(
        &mut actions,
        adjusted.as_ref().unwrap_or(catalog),
        &output,
        &directories,
        &cancelled,
    )?;
    Ok(Plan {
        actions,
        description: "Move reviewed files and companions into the previewed folders".into(),
        output_root: Some(output),
        ..Default::default()
    })
}
pub(crate) fn folder_destination(output: &Path, label: &str) -> PathBuf {
    let mut path = output.to_path_buf();
    for component in label.split('/').filter(|c| !c.is_empty()) {
        path.push(safe_component(component));
    }
    path
}

fn add_directory_actions(
    actions: &mut Vec<Action>,
    catalog: &Catalog,
    output: &Path,
    edits: &super::organize::DirectoryEdits,
    cancelled: &dyn Fn() -> bool,
) -> Result<()> {
    let required: BTreeSet<_> = edits
        .required
        .iter()
        .map(|p| folder_destination(output, p))
        .collect();
    let moved_targets: HashSet<_> = edits
        .moved_empty
        .iter()
        .map(|(_, p)| folder_destination(output, p))
        .collect();
    let mut cleanup = BTreeMap::<PathBuf, BTreeSet<PathBuf>>::new();
    let mut candidate = |folder: &Path, dependency: &Path| {
        for path in folder.ancestors() {
            if path == catalog.root || !path.starts_with(&catalog.root) {
                break;
            }
            if !required.contains(path) {
                cleanup
                    .entry(path.into())
                    .or_default()
                    .insert(dependency.into());
            }
        }
    };
    for action in actions.iter() {
        anyhow::ensure!(!cancelled(), "Planning cancelled");
        if let Action::Move { files } = action {
            for (source, _) in files {
                if let Some(parent) = source.path.parent() {
                    candidate(parent, &source.path);
                }
            }
        }
    }
    for (source, target) in &edits.moved_empty {
        anyhow::ensure!(!cancelled(), "Planning cancelled");
        let source = folder_destination(&catalog.root, source);
        anyhow::ensure!(
            catalog.directories.contains(&source),
            "Empty folder was not in the reviewed catalog"
        );
        let target = folder_destination(output, target);
        anyhow::ensure!(
            required.contains(&target),
            "Empty folder destination is missing"
        );
        candidate(&source, &target);
    }
    for path in &required {
        anyhow::ensure!(!cancelled(), "Planning cancelled");
        reject_symlink_components(path)?;
        if path.exists() {
            anyhow::ensure!(
                path.is_dir(),
                "A file occupies folder destination {}",
                path.display()
            );
        }
        if (!path.exists() && !catalog.directories.contains(path)) || moved_targets.contains(path) {
            actions.push(Action::CreateDirectory { path: path.clone() });
        }
    }
    let mut cleanup: Vec<_> = cleanup.into_iter().collect();
    cleanup.sort_by(|(a, _), (b, _)| {
        b.components()
            .count()
            .cmp(&a.components().count())
            .then_with(|| a.cmp(b))
    });
    for (path, after) in cleanup {
        anyhow::ensure!(!cancelled(), "Planning cancelled");
        actions.push(Action::RemoveEmptyDirectory {
            path,
            root: catalog.root.clone(),
            after: after.into_iter().collect(),
        });
    }
    Ok(())
}

fn normalized_destination(path: &Path) -> Result<PathBuf> {
    anyhow::ensure!(
        !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "Parent traversal is not allowed"
    );
    for ancestor in path.ancestors() {
        if ancestor.exists() {
            anyhow::ensure!(
                ancestor.is_dir(),
                "A file occupies folder destination {}",
                ancestor.display()
            );
            let suffix = path.strip_prefix(ancestor)?;
            let canonical = ancestor.canonicalize()?;
            return Ok(if suffix.as_os_str().is_empty() {
                canonical
            } else {
                canonical.join(suffix)
            });
        }
    }
    anyhow::bail!("Destination has no existing parent")
}
pub(crate) fn reject_symlink_components(path: &Path) -> Result<()> {
    anyhow::ensure!(
        !path
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)),
        "Parent traversal is not allowed"
    );
    for part in path.ancestors() {
        if let Ok(meta) = fs::symlink_metadata(part) {
            anyhow::ensure!(
                !meta.file_type().is_symlink(),
                "Symlink destination is not allowed: {}",
                part.display()
            );
        }
    }
    Ok(())
}

pub fn execute(
    plan: &Plan,
    journal_dir: &Path,
    cancelled: impl Fn() -> bool,
    progress: impl FnMut(usize, usize),
) -> Result<Report> {
    fs::create_dir_all(journal_dir)?;
    // Concurrent applications/workers must never share a clock-derived name.
    // Keep the unique journal on disk, including when execution is interrupted.
    let (mut journal, _) = tempfile::Builder::new()
        .prefix("operations-")
        .suffix(".jsonl")
        .tempfile_in(journal_dir)?
        .keep()
        .map_err(|error| error.error)?;
    execute_logged(plan, &mut journal, cancelled, progress)
}
fn execute_logged(
    plan: &Plan,
    journal: &mut File,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Report> {
    let mut report = Report {
        remaining: Plan {
            description: plan.description.clone(),
            output_root: plan.output_root.clone(),
            completed_dependencies: plan.completed_dependencies.clone(),
            ..Default::default()
        },
        ..Default::default()
    };
    let mut unfinished_dependencies = HashSet::<PathBuf>::new();
    let mut file_names = case_rename::FileNames::default();
    for (i, action) in plan.actions.iter().enumerate() {
        if cancelled() {
            report.cancelled = true;
            report
                .remaining
                .actions
                .extend_from_slice(&plan.actions[i..]);
            break;
        }
        if let Action::RemoveEmptyDirectory { after, .. } = action {
            let unfinished_prerequisite = after
                .iter()
                .any(|path| unfinished_dependencies.contains(path));
            if unfinished_prerequisite
                || !after
                    .iter()
                    .any(|path| report.remaining.completed_dependencies.contains(path))
            {
                write_record(
                    journal,
                    "skipped",
                    i,
                    action,
                    &[],
                    Some("Unfinished prerequisite"),
                )?;
                journal.sync_all()?;
                report.remaining.actions.push(action.clone());
                progress(i + 1, plan.actions.len());
                continue;
            }
        }
        write_record(journal, "pending", i, action, &[], None)?;
        journal.sync_all()?;
        match apply(action, &cancelled, &mut file_names) {
            Ok(()) => {
                let relocated = completion_identities(action);
                report.complete(action, &relocated);
                write_record(journal, "completed", i, action, &relocated, None)?;
            }
            Err(error) if error.is::<MoveCancelled>() => {
                report.cancelled = true;
                report
                    .remaining
                    .actions
                    .extend_from_slice(&plan.actions[i..]);
                write_record(journal, "cancelled", i, action, &[], None)?;
                journal.sync_all()?;
                break;
            }
            Err(_)
                if cancelled()
                    && matches!(
                        action,
                        Action::RenameDirectory(_)
                            | Action::TrashDirectory(_)
                            | Action::RenameFile { .. }
                    ) =>
            {
                report.cancelled = true;
                report
                    .remaining
                    .actions
                    .extend_from_slice(&plan.actions[i..]);
                write_record(journal, "cancelled", i, action, &[], None)?;
                journal.sync_all()?;
                break;
            }
            Err(e) => {
                match action {
                    Action::Move { files } | Action::FinishMove { files } => {
                        unfinished_dependencies
                            .extend(files.iter().map(|(source, _)| source.path.clone()))
                    }
                    Action::CreateDirectory { path } => {
                        unfinished_dependencies.insert(path.clone());
                    }
                    _ => {}
                }
                report.remaining.actions.push(action.clone());
                let detail = format!("{e:#}");
                report.errors.push(action.failure_message(&detail));
                write_record(journal, "failed", i, action, &[], Some(&detail))?;
                if matches!(action, Action::RenameDirectory(_)) {
                    // Later sources depend on the reviewed spelling. Never run them
                    // after an unsuccessful prerequisite, including on retry.
                    report
                        .remaining
                        .actions
                        .extend_from_slice(&plan.actions[i + 1..]);
                    journal.sync_all()?;
                    progress(i + 1, plan.actions.len());
                    break;
                }
            }
        }
        journal.sync_all()?;
        progress(i + 1, plan.actions.len());
    }
    Ok(report)
}
fn write_record(
    journal: &mut File,
    status: &str,
    index: usize,
    action: &Action,
    relocated: &[(MediaId, MediaId, Fingerprint)],
    error: Option<&str>,
) -> Result<()> {
    #[derive(Serialize)]
    struct Record<'a> {
        status: &'a str,
        index: usize,
        action: &'a Action,
        relocated: &'a [(MediaId, MediaId, Fingerprint)],
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<&'a str>,
    }
    serde_json::to_writer(
        &mut *journal,
        &Record {
            status,
            index,
            action,
            relocated,
            error,
        },
    )?;
    journal.write_all(b"\n")?;
    Ok(())
}
fn completion_identities(action: &Action) -> Vec<(MediaId, MediaId, Fingerprint)> {
    let mut result = Vec::new();
    let mut record = |from: &Path, to: &Path| {
        if let Ok(fingerprint) = Fingerprint::read(to) {
            result.push((MediaId::for_path(from), MediaId::for_path(to), fingerprint));
        }
    };
    match action {
        Action::RenameDirectory(rename) => {
            for source in &rename.inventory.files {
                record(&source.path, &rename.destination(&source.path));
            }
        }
        Action::RenameFile { source, to } => record(&source.path, to),
        Action::Move { files } | Action::FinishMove { files } => {
            for (source, destination) in files {
                record(&source.path, destination);
            }
        }
        Action::Metadata {
            source,
            target: metadata::WriteTarget::Embedded,
            ..
        } => record(&source.path, &source.path),
        _ => {}
    }
    result
}
fn apply(
    action: &Action,
    cancelled: &dyn Fn() -> bool,
    file_names: &mut case_rename::FileNames,
) -> Result<()> {
    if !matches!(action, Action::RenameFile { .. }) {
        file_names.invalidate();
    }
    match action {
        Action::RenameDirectory(rename) => rename.apply(cancelled)?,
        Action::TrashDirectory(folder) => folder.apply(cancelled)?,
        Action::RenameFile { source, to } => {
            case_rename::apply_file(source, to, cancelled, file_names)?
        }
        Action::CreateDirectory { path } => {
            anyhow::ensure!(path.is_absolute(), "Folder path must be absolute");
            reject_symlink_components(path)?;
            fs::create_dir_all(path)?;
        }
        Action::RemoveEmptyDirectory { path, root, .. } => {
            anyhow::ensure!(
                path.is_absolute() && root.is_absolute() && path != root && path.starts_with(root),
                "Cleanup must stay below the library root"
            );
            reject_symlink_components(path)?;
            match fs::remove_dir(path) {
                Ok(()) => {}
                Err(e)
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Action::FinishMove { files } => {
            anyhow::ensure!(
                files.len() == 1,
                "Staged source cleanup requires one receipt per file"
            );
            // Preflight the entire reviewed cohort before removing a source.
            for (source, destination) in files {
                anyhow::ensure!(!cancelled(), "Finishing move cancelled");
                verify_staged_move(source, destination)?;
            }
            for (source, destination) in files {
                anyhow::ensure!(!cancelled(), "Finishing move cancelled");
                verify_staged_move(source, destination)?;
                fs::remove_file(&source.path)?;
            }
        }
        Action::Move { files } => {
            for (s, d) in files {
                check_move_cancelled(cancelled)?;
                s.verify()?;
                reject_symlink_components(d)?;
                anyhow::ensure!(!d.exists(), "Destination already exists: {}", d.display());
            }
            let mut staged: Vec<PathBuf> = Vec::new();
            let stage = (|| -> Result<()> {
                for (s, d) in files {
                    check_move_cancelled(cancelled)?;
                    fs::create_dir_all(d.parent().context("Missing destination parent")?)?;
                    copy_exclusive(&s.path, d, cancelled)?;
                    staged.push(d.clone());
                    s.verify()?;
                }
                check_move_cancelled(cancelled)?;
                Ok(())
            })();
            if let Err(e) = stage {
                let mut cleanup_errors = Vec::new();
                for d in staged {
                    if let Err(cleanup) = fs::remove_file(&d) {
                        cleanup_errors.push(format!("{}: {cleanup}", d.display()));
                    }
                }
                anyhow::ensure!(
                    cleanup_errors.is_empty(),
                    "{e:#}; could not remove staged destinations: {}",
                    cleanup_errors.join("; ")
                );
                return Err(e);
            }
            // Cancellation rolls back staging only. Once source removal starts,
            // finish this companion cohort rather than introducing partial moves.
            // All companions have durable destinations before any source is removed.
            // If removal fails, leave the durable copies and report the exact source.
            for (s, _) in files {
                fs::remove_file(&s.path).with_context(|| {
                    format!(
                        "Copied safely but could not remove source {}",
                        s.path.display()
                    )
                })?;
            }
        }
        Action::Trash { files } => {
            for source in files {
                source.verify()?;
            }
            for source in files {
                move_to_trash(&source.path).with_context(|| {
                    format!("Could not move {} to OS Trash", source.path.display())
                })?;
            }
        }
        Action::Metadata {
            source,
            sidecar_before,
            target,
            rating,
            orientation,
        } => {
            source.verify()?;
            let path = metadata::sidecar(&source.path);
            anyhow::ensure!(
                Fingerprint::read(&path).ok() == *sidecar_before,
                "Sidecar changed since review"
            );
            reject_symlink_components(&source.path)?;
            match target {
                metadata::WriteTarget::Sidecar => {
                    metadata::write_sidecar(&source.path, *rating, *orientation)?
                }
                metadata::WriteTarget::Embedded => metadata::write_embedded(
                    &source.path,
                    source.fingerprint,
                    *rating,
                    *orientation,
                )?,
            }
        }
    }
    Ok(())
}
fn verify_staged_move(source: &Source, destination: &Path) -> Result<()> {
    anyhow::ensure!(
        cfg!(unix) && source.fingerprint.identity[1] != 0,
        "Staged moves require a verified filesystem identity"
    );
    anyhow::ensure!(
        source.path.is_absolute() && destination.is_absolute(),
        "Staged move paths must be absolute"
    );
    reject_symlink_components(&source.path)?;
    source.verify()?;
    reject_symlink_components(destination)?;
    // Equal identity alone does not prove two directory entries: a same-path,
    // dot-component or case-insensitive alias must never unlink its destination.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let source_parent = source.path.parent().context("Missing source parent")?;
        let destination_parent = destination.parent().context("Missing destination parent")?;
        let from = fs::metadata(source_parent)?;
        let to = fs::metadata(destination_parent)?;
        if (from.dev(), from.ino()) == (to.dev(), to.ino()) {
            let source_name = source.path.file_name().context("Missing source name")?;
            let destination_name = destination
                .file_name()
                .context("Missing destination name")?;
            anyhow::ensure!(
                source_name != destination_name,
                "Staged source and destination are the same entry"
            );
            let mut source_found = false;
            let mut destination_found = false;
            for entry in fs::read_dir(source_parent)? {
                let name = entry?.file_name();
                source_found |= name == source_name;
                destination_found |= name == destination_name;
                if source_found && destination_found {
                    break;
                }
            }
            anyhow::ensure!(
                source_found && destination_found,
                "Staged move must have two distinct directory entries"
            );
        }
    }
    anyhow::ensure!(
        Fingerprint::read(destination)? == source.fingerprint,
        "Staged move destination changed: {}",
        destination.display()
    );
    Ok(())
}
#[derive(Debug)]
struct MoveCancelled;
impl std::fmt::Display for MoveCancelled {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Move cancelled before removing sources")
    }
}
impl std::error::Error for MoveCancelled {}

fn check_move_cancelled(cancelled: &dyn Fn() -> bool) -> Result<()> {
    if cancelled() {
        return Err(MoveCancelled.into());
    }
    Ok(())
}
fn copy_exclusive(source: &Path, dest: &Path, cancelled: &dyn Fn() -> bool) -> Result<()> {
    check_move_cancelled(cancelled)?;
    if fs::hard_link(source, dest).is_ok() {
        return Ok(());
    }
    copy_contents_exclusive(source, dest, cancelled)
}
// The fallback is shared by every move when the destination cannot hard-link the
// source. Bound cancellation work without allocating the entire photo or video.
fn copy_contents_exclusive(source: &Path, dest: &Path, cancelled: &dyn Fn() -> bool) -> Result<()> {
    check_move_cancelled(cancelled)?;
    let mut input = File::open(source)?;
    let metadata = input.metadata()?;
    let mut times = fs::FileTimes::new().set_modified(metadata.modified()?);
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    // Do not expose a private source while its fallback copy is still partial.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut output = options.open(dest)?;
    let result = (|| -> Result<()> {
        let mut buffer = vec![0u8; 1024 * 1024];
        loop {
            check_move_cancelled(cancelled)?;
            let count = match input.read(&mut buffer) {
                Ok(count) => count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(error.into()),
            };
            if count == 0 {
                break;
            }
            check_move_cancelled(cancelled)?;
            output.write_all(&buffer[..count])?;
        }
        check_move_cancelled(cancelled)?;
        // Capture times before reading and preserve them before making the copy
        // read-only. Failure leaves the source intact and rolls back this copy.
        output.set_times(times)?;
        output.set_permissions(metadata.permissions())?;
        output.sync_all()?;
        check_move_cancelled(cancelled)?;
        Ok(())
    })();
    if let Err(error) = result {
        drop(output);
        // A failed rollback is an operation error, never a clean cancellation.
        if let Err(cleanup) = fs::remove_file(dest) {
            anyhow::bail!(
                "{error}; could not remove partial destination {}: {cleanup}",
                dest.display()
            );
        }
        return Err(error);
    }
    Ok(())
}
pub fn journal_dir() -> PathBuf {
    session::data_dir().join("operations")
}
pub fn content_hash(path: &Path, cancelled: impl Fn() -> bool) -> Result<String> {
    let mut file = File::open(path)?;
    let mut buffer = [0u8; 64 * 1024];
    let mut hash = blake3::Hasher::new();
    loop {
        if cancelled() {
            anyhow::bail!("Cancelled");
        }
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    Ok(hash.finalize().to_hex().to_string())
}

// All workflows use this boundary through the reviewed executor. The macOS
// crate default launches Finder AppleScript and can wait on automation events.
fn trash_context() -> trash::TrashContext {
    #[cfg(target_os = "macos")]
    {
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        let mut context = trash::TrashContext::default();
        context.set_delete_method(DeleteMethod::NsFileManager);
        context
    }
    #[cfg(not(target_os = "macos"))]
    trash::TrashContext::default()
}
fn move_to_trash(path: &Path) -> Result<()> {
    // trash 5.2.9 percent-encodes non-UTF-8 bytes before fileURLWithPath. That
    // can name a different literal %-encoded sibling. Refuse rather than target
    // a file other than the exact identity verified by the executor.
    #[cfg(target_os = "macos")]
    anyhow::ensure!(
        path.to_str().is_some(),
        "macOS Trash cannot safely represent this file name; rename it to valid Unicode before retrying"
    );
    trash_context().delete(path)?;
    Ok(())
}
#[cfg(all(test, target_os = "macos"))]
mod macos_trash_tests {
    use super::*;
    #[test]
    fn organize_trash_uses_native_backend_and_rejects_ambiguous_byte_names() {
        use std::os::unix::ffi::OsStringExt;
        use trash::macos::{DeleteMethod, TrashContextExtMacos};
        assert!(matches!(
            trash_context().delete_method(),
            DeleteMethod::NsFileManager
        ));
        let temp = tempfile::tempdir().unwrap();
        let source = temp
            .path()
            .join(std::ffi::OsString::from_vec(b"unsafe-\xff.txt".to_vec()));
        let encoded = temp.path().join("unsafe-%FF.txt");
        // APFS can reject creating this name. The boundary must still reject
        // its bytes before the dependency can reinterpret the encoded sibling.
        fs::write(&encoded, b"unrelated sibling").unwrap();
        assert!(
            move_to_trash(&source)
                .unwrap_err()
                .to_string()
                .contains("safely represent")
        );
        assert_eq!(fs::read(&encoded).unwrap(), b"unrelated sibling");
    }
}
