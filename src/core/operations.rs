mod case_rename;
mod directory;
#[cfg(target_os = "macos")]
mod macos_attributes;
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
    /// A reviewed plan can be retried after its drive was mounted again, so
    /// the saved device and inode are not compared. Returns the file as it is
    /// now, for the exact checks that follow within this run.
    fn verify(&self) -> Result<Fingerprint> {
        let fingerprint = Fingerprint::read(&self.path)
            .with_context(|| format!("Could not verify reviewed source {}", self.path.display()))?;
        anyhow::ensure!(
            fingerprint.unchanged(&self.fingerprint),
            "File changed since review: {}",
            self.path.display()
        );
        Ok(fingerprint)
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
        /// What the review listed as still in the folder once its files have
        /// moved out, by name: thumbnail folders and Finder leftovers. They
        /// go to the Trash with the folder. Empty when the folder is expected
        /// to be empty. A folder that keeps anything else is never planned.
        #[serde(default)]
        leftovers: Vec<String>,
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
            Self::RemoveEmptyDirectory {
                path, leftovers, ..
            } if !leftovers.is_empty() => {
                format!(
                    "Move to Trash with {}: {}",
                    leftovers.join(", "),
                    path.display()
                )
            }
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
                        metadata::WriteTarget::Pixels => "re-encoded pixels",
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
    /// Folders the review sent to the Trash with their leftovers that are
    /// still there: the drive has no Trash, or the system refused.
    pub kept_folders: Vec<PathBuf>,
    /// Folders this run made, parents included: a destination that was not
    /// there before its first photo arrived, or a folder the plan creates.
    pub created_folders: Vec<PathBuf>,
    /// Where each current destination sits in `relocated`, so a chained move
    /// is found without scanning every earlier one. Rebuilt when stale.
    #[doc(hidden)]
    pub relocation_index: RelocationIndex,
}
/// Positions in [`Report::relocated`] by current destination.
#[derive(Debug, Default)]
pub struct RelocationIndex {
    by_destination: HashMap<MediaId, Vec<usize>>,
    indexed: usize,
}

impl Report {
    fn record_relocation(&mut self, old: MediaId, new: MediaId, fingerprint: Fingerprint) {
        let index = &mut self.relocation_index;
        if index.indexed != self.relocated.len() {
            // `relocated` is public: a caller that filled it directly is indexed here.
            index.by_destination.clear();
            for (position, (_, destination, _)) in self.relocated.iter().enumerate() {
                index
                    .by_destination
                    .entry(*destination)
                    .or_default()
                    .push(position);
            }
        }
        let positions = match index.by_destination.remove(&old) {
            Some(chained) => {
                for &position in &chained {
                    self.relocated[position].1 = new;
                    self.relocated[position].2 = fingerprint;
                }
                chained
            }
            None => {
                self.relocated.push((old, new, fingerprint));
                vec![self.relocated.len() - 1]
            }
        };
        index
            .by_destination
            .entry(new)
            .or_default()
            .extend(positions);
        index.indexed = self.relocated.len();
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
            // A folder made for them is removed once they are in the Trash.
            Action::Trash { files } => self
                .remaining
                .completed_dependencies
                .extend(files.iter().map(|source| source.path.clone())),
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

/// The files that go to the Trash with the file at `path` when everything in
/// `rejected` is trashed, as Apply plans it. `index` is
/// [`Catalog::companion_index`].
/// - A sidecar that names the file in full ("IMG.CR2.xmp", "clip.MOV.xmp")
///   goes with that file.
/// - With companions on, a RAW beside a JPEG/HEIC/... follows the photo, and
///   takes its own sidecar along. Photos with the same name in other formats
///   are separate shots and never follow.
/// - "IMG.xmp" belongs to every photo of that name and follows once none of
///   them is kept, so a kept photo keeps its RAW and XMP. Beside videos only,
///   it follows once every video of that name is trashed.
pub fn trash_followers_for(
    index: &HashMap<PathBuf, Vec<PathBuf>>,
    path: &Path,
    companions: bool,
    rejected: &HashSet<MediaId>,
) -> Vec<PathBuf> {
    use super::catalog::sidecar_owner;
    let kind = |p: &Path| MediaKind::from_path(p);
    let (Some(own), Some(group)) = (kind(path), index.get(&path.with_extension(""))) else {
        return Vec::new();
    };
    let is_rejected = |p: &Path| rejected.contains(&MediaId::for_path(p));
    let still = |p: &Path| matches!(kind(p), Some(MediaKind::Photo | MediaKind::Raw));
    let has_photo = group.iter().any(|p| kind(p) == Some(MediaKind::Photo));
    let raw_follows = |p: &Path| companions && has_photo && kind(p) == Some(MediaKind::Raw);
    // Whether the photos of this name all go, and their RAW and XMP with them.
    let photos_go = own != MediaKind::Video
        && group
            .iter()
            .filter(|p| still(p) && !raw_follows(p))
            .all(|p| is_rejected(p));
    let videos_only = !group.iter().any(|p| still(p));
    let videos_go = group
        .iter()
        .filter(|p| kind(p) == Some(MediaKind::Video))
        .all(|p| is_rejected(p));
    group
        .iter()
        .filter(|p| p.as_path() != path)
        .filter(|p| match (kind(p), sidecar_owner(group, p)) {
            (Some(_), _) => photos_go && raw_follows(p),
            // A RAW with a mark of its own takes its sidecar itself.
            (None, Some(owner)) => {
                owner == path || (photos_go && raw_follows(owner) && !is_rejected(owner))
            }
            (None, None) if videos_only => videos_go,
            (None, None) => photos_go,
        })
        .cloned()
        .collect()
}

/// The files Organize moves and renames as one with the file at `path`: its
/// XMP sidecars and, with `companions`, the photos and RAW files of the same
/// name with theirs. A video takes only its own sidecars, "clip.MOV.xmp" and,
/// when no photo is called "clip", "clip.xmp". `index` is
/// [`Catalog::companion_index`].
fn move_cohort(
    index: &HashMap<PathBuf, Vec<PathBuf>>,
    path: &Path,
    companions: bool,
) -> Vec<PathBuf> {
    use super::catalog::sidecar_owner;
    let kind = |p: &Path| MediaKind::from_path(p);
    let still = |p: &Path| matches!(kind(p), Some(MediaKind::Photo | MediaKind::Raw));
    let group = match (kind(path), index.get(&path.with_extension(""))) {
        (Some(_), Some(group)) if group.iter().any(|p| p == path) => group,
        _ => return vec![path.to_path_buf()],
    };
    let together = companions && still(path);
    let moves = |p: &Path| p == path || (together && still(p));
    let videos_only = !group.iter().any(|p| still(p));
    group
        .iter()
        .filter(|p| match (kind(p), sidecar_owner(group, p)) {
            (Some(_), _) => moves(p),
            (None, Some(owner)) => moves(owner),
            (None, None) => still(path) || videos_only,
        })
        .cloned()
        .collect()
}

/// Shared Trash expansion for staged decisions and explicit context-menu requests.
/// With `only`, a file follows a rejected photo only if it is in that set.
fn trash_actions(
    catalog: &Catalog,
    rejected: &HashSet<MediaId>,
    companions: bool,
    only: Option<&HashSet<MediaId>>,
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
            let mut files = vec![path.to_path_buf()];
            files.extend(
                trash_followers_for(&index, path, companions, rejected)
                    .into_iter()
                    .filter(|p| only.is_none_or(|only| only.contains(&MediaId::for_path(p)))),
            );
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
    let (mut actions, _) = trash_actions(catalog, &rejected, companions, None, &cancelled)?;
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
    load: impl FnMut(&Media) -> Result<Metadata>,
    cancelled: impl Fn() -> bool,
) -> Result<Plan> {
    decisions_plan_with_followers(
        catalog, decisions, companions, None, policy, load, cancelled,
    )
}
/// [`decisions_plan_with_metadata`] where, with `followers`, only those files
/// may go to the Trash with a rejected photo: the RAW and XMP files a review
/// listed, so a file that reached the photo's name since never follows.
pub fn decisions_plan_with_followers(
    catalog: &Catalog,
    decisions: &HashMap<MediaId, Decision>,
    companions: bool,
    followers: Option<&HashSet<MediaId>>,
    policy: metadata::WritePolicy,
    mut load: impl FnMut(&Media) -> Result<Metadata>,
    cancelled: impl Fn() -> bool,
) -> Result<Plan> {
    let rejected = decisions
        .iter()
        .filter(|(_, d)| d.rejected)
        .map(|(id, _)| *id)
        .collect();
    let (mut actions, seen) = trash_actions(catalog, &rejected, companions, followers, &cancelled)?;
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
        let meta = if d.turns > 0 || policy.embeds() {
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
        prepared.push((
            media,
            d,
            sidecar,
            target,
            orientation,
            meta.embedded_metadata_writable,
        ));
    }
    let mut stems: HashMap<PathBuf, usize> = HashMap::new();
    for media in &catalog.media {
        *stems.entry(media.path.with_extension("")).or_default() += 1;
    }
    for (media, d, sidecar, mut target, orientation, embeddable) in prepared {
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
        let sidecar_before = Fingerprint::read(&sidecar).ok();
        // Only a rotation-only edit on a file that cannot hold orientation and has
        // no sidecar (its own or a companion's) may rotate pixels, so a leftover
        // sidecar orientation can never rotate the image twice.
        if policy == metadata::WritePolicy::EmbedOrRotatePixels
            && target == metadata::WriteTarget::Sidecar
            && !embeddable
            && d.rating.is_none()
            && orientation.is_some_and(|o| o != 1)
            && sidecar_before.is_none()
            && stems.get(&media.path.with_extension("")) == Some(&1)
            && super::reencode::supported(&media.path)
        {
            target = metadata::WriteTarget::Pixels;
        }
        actions.push(Action::Metadata {
            source: Source {
                path: media.path.clone(),
                fingerprint: media.fingerprint,
            },
            sidecar_before,
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

/// What the review announces before an Apply: the folders that were there
/// before and hold nothing, or only leftovers, once `moves` are done and the
/// files of `trashed` (named where they are now) are in the Trash. The
/// moves plan already names the folders its moves alone empty; these are the
/// ones the Trash marks finish emptying, with their parents when nothing
/// else is in them, deepest first. Never the library folder, a folder that
/// keeps or receives anything else, or one the moves rename or make.
///
/// The result is a plan of `RemoveEmptyDirectory` actions for the review to
/// list. Nothing in it is applied as it is: the Trash marks are planned
/// after the moves, by [`remove_folders_emptied_by_trash`], which removes
/// only folders listed here.
pub fn folders_emptied_by_trash(
    catalog: &Catalog,
    moves: &Plan,
    trashed: &[PathBuf],
    cancelled: impl Fn() -> bool,
) -> Result<Plan> {
    let going_to_trash: HashSet<&Path> = trashed.iter().map(PathBuf::as_path).collect();
    // Every file that leaves its folder, by a move or to the Trash.
    let mut leaving: HashSet<PathBuf> = trashed.iter().cloned().collect();
    // The files a move takes to another folder.
    let mut moved_out = HashSet::<&Path>::new();
    // Folders something stays in or arrives at, or that change their name.
    let mut kept = BTreeSet::<&Path>::new();
    // Folders the moves rename: everything in them has another path by the
    // time the Trash marks apply, so nothing in them is listed here.
    let mut renamed = Vec::<&Path>::new();
    // Folders the moves alone empty: gone by the time the Trash marks apply.
    let mut going = HashSet::<PathBuf>::new();
    let mut candidates = BTreeSet::<&Path>::new();
    for action in &moves.actions {
        anyhow::ensure!(!cancelled(), "File planning cancelled");
        match action {
            Action::Move { files } | Action::FinishMove { files } => {
                for (source, to) in files {
                    let (from, into) = (source.path.parent(), to.parent());
                    if from != into {
                        leaving.insert(source.path.clone());
                        moved_out.insert(&source.path);
                    }
                    if let Some(into) = into {
                        if going_to_trash.contains(source.path.as_path()) {
                            // It moves first and goes to the Trash from there.
                            candidates.insert(into);
                        } else {
                            kept.insert(into);
                        }
                    }
                }
            }
            Action::RenameDirectory(rename) => {
                kept.insert(&rename.from);
                kept.insert(&rename.to);
                renamed.push(&rename.from);
            }
            Action::CreateDirectory { path } => {
                kept.insert(path);
            }
            Action::RemoveEmptyDirectory { path, .. } => {
                going.insert(path.clone());
            }
            _ => {}
        }
    }
    for path in trashed {
        if let Some(folder) = path
            .parent()
            .filter(|_| !moved_out.contains(path.as_path()))
        {
            candidates.insert(folder);
        }
    }
    // With every folder above them, up to the library folder.
    let mut folders = BTreeSet::<&Path>::new();
    for folder in candidates {
        for path in folder.ancestors() {
            if path == catalog.root || !path.starts_with(&catalog.root) || !folders.insert(path) {
                break;
            }
        }
    }
    let mut folders: Vec<&Path> = folders.into_iter().collect();
    folders.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    let mut plan = Plan {
        description: "Folders left empty by the Trash marks".into(),
        ..Default::default()
    };
    for path in folders {
        anyhow::ensure!(!cancelled(), "File planning cancelled");
        // Paths order by component, so what is inside a folder sorts right
        // after it.
        if going.contains(path)
            || renamed.iter().any(|folder| path.starts_with(folder))
            || kept
                .range::<&Path, _>(path..)
                .next()
                .is_some_and(|folder| folder.starts_with(path))
        {
            continue;
        }
        if let Some(leftovers) =
            leftovers_after(path, &leaving, &going, catalog.system_moves_apple_double)
        {
            going.insert(path.to_path_buf());
            plan.actions.push(Action::RemoveEmptyDirectory {
                path: path.to_path_buf(),
                root: catalog.root.clone(),
                after: Vec::new(),
                leftovers,
            });
        }
    }
    Ok(plan)
}

/// The second half of an Apply: `plan` sends the photos marked for Trash
/// there, after the moves of the same Apply, if it had any. Two kinds of
/// folder are removed after them, by the same plan, deepest first:
///
/// - a folder of `created`, made by those moves a moment ago, that holds
///   nothing at all once the photos are gone. One that keeps anything, a
///   system file included, stays.
/// - a folder of `listed`, which the review named as left empty by the
///   Trash marks, with the leftovers it listed for it. It goes when it
///   holds nothing, or to the Trash when it holds only those leftovers.
///   Anything else in it keeps it.
///
/// Every other folder stays, and so does the library's own. Each removal
/// waits for the Trash actions that empty the folder: where one fails or is
/// stopped, the folder stays and is retried with it.
pub fn remove_folders_emptied_by_trash(
    plan: &mut Plan,
    catalog: &Catalog,
    created: &[PathBuf],
    listed: &[(PathBuf, Vec<String>)],
    cancelled: impl Fn() -> bool,
) -> Result<()> {
    let inside = |path: &&Path| *path != catalog.root && path.starts_with(&catalog.root);
    // Each folder with the leftovers that may go with it: none for a folder
    // the moves made.
    let mut allowed: HashMap<&Path, &[String]> = created
        .iter()
        .map(PathBuf::as_path)
        .filter(inside)
        .map(|path| (path, &[][..]))
        .collect();
    for (path, leftovers) in listed {
        if inside(&path.as_path()) {
            allowed.insert(path, leftovers);
        }
    }
    if allowed.is_empty() {
        return Ok(());
    }
    let mut trashed = HashSet::new();
    let mut emptied = BTreeMap::<&Path, Vec<PathBuf>>::new();
    for action in &plan.actions {
        let Action::Trash { files } = action else {
            continue;
        };
        for source in files {
            anyhow::ensure!(!cancelled(), "File planning cancelled");
            trashed.insert(source.path.clone());
            for folder in source.path.ancestors().skip(1) {
                if let Some((folder, _)) = allowed.get_key_value(folder) {
                    emptied.entry(folder).or_default().push(source.path.clone());
                }
            }
        }
    }
    // Deepest first: a folder is gone for its parent once it is known to go.
    let mut folders: Vec<_> = emptied.into_iter().collect();
    folders.sort_by_key(|(path, _)| std::cmp::Reverse(path.components().count()));
    let mut going = HashSet::new();
    for (path, after) in folders {
        anyhow::ensure!(!cancelled(), "File planning cancelled");
        let Some(left) = leftovers_after(path, &trashed, &going, catalog.system_moves_apple_double)
        else {
            continue;
        };
        // Only what the review listed may go with the folder.
        if left.iter().all(|name| allowed[path].contains(name)) {
            going.insert(path.to_path_buf());
            plan.actions.push(Action::RemoveEmptyDirectory {
                path: path.to_path_buf(),
                root: catalog.root.clone(),
                after,
                leftovers: left,
            });
        }
    }
    Ok(())
}

/// Clean a name PhotoSort creates (a generated grouping or a typed name) for
/// this platform. Never apply it to a name read from the library.
pub fn safe_component(value: &str) -> String {
    super::names::NameRules::CURRENT.sanitize(value)
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
    // The plan review asks this every frame: build each folder path once.
    let mut folders: HashMap<&str, PathBuf> = HashMap::new();
    for (id, path) in catalog.organize_files() {
        let renamed = || {
            names
                .get(&id)
                .is_some_and(|name| super::organize::requested_stem(path, name).is_some())
        };
        // A file the grouping leaves where it is can still be renamed there.
        let Some(label) = labels.get(&id) else {
            if renamed() {
                return true;
            }
            continue;
        };
        let folder = folders
            .entry(label)
            .or_insert_with(|| folder_destination(output, label));
        // A label that spells an existing folder with other capital letters,
        // or with its accents in the other form, names that folder where the
        // volume ignores the difference: nothing moves.
        let in_place = path
            .parent()
            .is_some_and(|parent| catalog.same_folder(parent, folder));
        if !in_place || renamed() {
            return true;
        }
    }
    false
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
    // Keyed without letter case or the form of accents: two planned names
    // that differ only in those are one file on macOS and Windows, and the
    // second move would fail.
    let mut reserved = HashSet::new();
    let reservation = |path: &Path| super::text::folded(&path.to_string_lossy());
    let mut actions = casing.actions.clone();
    let mut file_names = case_rename::FileNames::default();
    // The library's folders as they will be spelled, keyed by what the
    // volume ignores: letter case, the composed or decomposed form of
    // accents, or both. Where it ignores neither, "anna" and "Anna" are two
    // folders and nothing is looked up.
    let one_folder = catalog.names_ignore_case || catalog.names_ignore_form;
    let spelled: HashMap<String, PathBuf> = if one_folder {
        catalog
            .directories
            .iter()
            .map(|folder| casing.rebase(folder))
            .filter_map(|folder| Some((catalog.name_key(folder.to_str()?).into_owned(), folder)))
            .collect()
    } else {
        HashMap::new()
    };
    let mut folders: HashMap<&str, PathBuf> = HashMap::new();
    for (id, path) in organize_files {
        anyhow::ensure!(!cancelled(), "Planning cancelled");
        if seen.contains(path) {
            continue;
        }
        let new_name = names
            .get(&id)
            .filter(|s| !s.trim().is_empty())
            .map(|s| safe_component(s))
            .filter(|name| path.file_stem().and_then(|s| s.to_str()) != Some(name.as_str()));
        let dir = match labels.get(&id) {
            Some(label) => folders
                .entry(label)
                .or_insert_with(|| {
                    spelling_on_disk(folder_destination(&output, label), &spelled, catalog)
                })
                .clone(),
            // The grouping leaves it where it is: only its new name applies.
            None if new_name.is_some() => match casing.rebase(path).parent() {
                Some(parent) => parent.to_path_buf(),
                None => continue,
            },
            None => continue,
        };
        let sources: Vec<_> = move_cohort(&index, path, companions)
            .into_iter()
            .filter(|p| !seen.contains(p))
            .collect();
        // "IMG.CR2.xmp" is named after "IMG.CR2", whatever that becomes.
        let named_after: Vec<Option<usize>> = sources
            .iter()
            .map(|p| {
                super::catalog::sidecar_owner(&sources, p)
                    .and_then(|owner| sources.iter().position(|s| s == owner))
            })
            .collect();
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
            let mut destinations: Vec<_> = sources
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
            for (i, owner) in named_after.iter().enumerate() {
                if let Some(owner) = owner
                    && let Some(name) = destinations[*owner].file_name()
                {
                    destinations[i] =
                        dir.join(super::organize::renamed_file_name(&sources[i], name));
                }
            }
            let mut available = true;
            for (source, destination) in sources.iter().zip(&destinations) {
                if reserved.contains(&reservation(destination))
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
            reserved.insert(reservation(&dest));
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
            let adjusted =
                super::names::relative_label(adjusted.strip_prefix(&catalog.root).ok()?)?;
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
/// `folder` as the library spells it. A generated label can name an existing
/// folder with other capital letters ("Anna" for "anna"), or with composed
/// accents where the folder's are decomposed; where the volume ignores that
/// difference it is the same folder, and planning with the label's spelling
/// would move every file already there onto itself and rename it "name_1".
/// `spelled` maps the library's folders by [`Catalog::name_key`]. The
/// deepest folder that exists decides, so "Anna/2024" becomes "anna/2024".
fn spelling_on_disk(
    folder: PathBuf,
    spelled: &HashMap<String, PathBuf>,
    catalog: &Catalog,
) -> PathBuf {
    if spelled.is_empty() {
        return folder;
    }
    for ancestor in folder.ancestors() {
        let Some(existing) = ancestor
            .to_str()
            .and_then(|path| spelled.get(catalog.name_key(path).as_ref()))
        else {
            continue;
        };
        // The key is only a hint: the volume says whether they are one.
        if existing == ancestor || !super::catalog::one_folder_on_disk(ancestor, existing) {
            break;
        }
        return match folder.strip_prefix(ancestor) {
            Ok(rest) => existing.join(rest),
            Err(_) => folder,
        };
    }
    folder
}
pub(crate) fn folder_destination(output: &Path, label: &str) -> PathBuf {
    let mut path = output.to_path_buf();
    // Labels name existing folders verbatim, including ones like `23:Dec`.
    // Only a component that cannot be a single folder name is cleaned.
    for component in label.split('/').filter(|c| !c.is_empty()) {
        if super::names::is_single_component(component) {
            path.push(component);
        } else {
            path.push(safe_component(component));
        }
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
    let known: HashSet<_> = catalog.directories.iter().map(PathBuf::as_path).collect();
    // Folders a file ends up in: one that arrives from another folder, or
    // one that stays where it is under a new name. Such a folder keeps
    // something, and so does every folder above it.
    let receiving: BTreeSet<PathBuf> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Move { files } => Some(files),
            _ => None,
        })
        .flatten()
        .filter_map(|(_, to)| to.parent().map(Path::to_path_buf))
        .collect();
    // Paths order by component, so what is inside a folder sorts right
    // after it.
    let receives = |path: &Path| {
        receiving
            .range::<Path, _>((std::ops::Bound::Included(path), std::ops::Bound::Unbounded))
            .next()
            .is_some_and(|folder| folder.starts_with(path))
    };
    let mut cleanup = BTreeMap::<PathBuf, BTreeSet<PathBuf>>::new();
    let mut candidate = |folder: &Path, dependency: &Path| {
        for path in folder.ancestors() {
            if path == catalog.root || !path.starts_with(&catalog.root) {
                break;
            }
            if !required.contains(path) && !receives(path) {
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
            known.contains(source.as_path()),
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
        if (!path.exists() && !known.contains(path.as_path())) || moved_targets.contains(path) {
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
    // Deepest first: a folder is gone for its parent once it is known to be
    // empty or to hold only leftovers after the moves.
    let moving: HashSet<PathBuf> = actions
        .iter()
        .filter_map(|action| match action {
            Action::Move { files } => Some(files.iter().map(|(source, _)| source.path.clone())),
            _ => None,
        })
        .flatten()
        .collect();
    let mut going = HashSet::<PathBuf>::new();
    for (path, after) in cleanup {
        anyhow::ensure!(!cancelled(), "Planning cancelled");
        // A folder that keeps anything else stays, and is not planned: the
        // review lists every folder planned here as going.
        let Some(leftovers) =
            leftovers_after(&path, &moving, &going, catalog.system_moves_apple_double)
        else {
            continue;
        };
        going.insert(path.clone());
        actions.push(Action::RemoveEmptyDirectory {
            path,
            root: catalog.root.clone(),
            after: after.into_iter().collect(),
            leftovers,
        });
    }
    Ok(())
}

/// What stays in `folder` once the plan's moves are done, when all of it is
/// leftovers (see `catalog::is_leftover`): their names, for the review to
/// list. `None` when anything else stays, so the folder does too.
fn leftovers_after(
    folder: &Path,
    moving: &HashSet<PathBuf>,
    going: &HashSet<PathBuf>,
    system_moves_apple_double: bool,
) -> Option<Vec<String>> {
    let mut names = Vec::new();
    for entry in fs::read_dir(folder).ok()? {
        let entry = entry.ok()?;
        let kind = entry.file_type().ok()?;
        let (path, name) = (entry.path(), entry.file_name());
        if kind.is_dir() {
            if going.contains(&path) {
                continue;
            }
        } else if kind.is_file() {
            // The system takes a "._name" file along with "name", a file
            // that moves or a folder that goes.
            let follows = system_moves_apple_double
                && name
                    .as_encoded_bytes()
                    .strip_prefix(b"._")
                    .and_then(|owner| std::str::from_utf8(owner).ok())
                    .map(|owner| folder.join(owner))
                    .is_some_and(|owner| moving.contains(&owner) || going.contains(&owner));
            if follows || moving.contains(&path) {
                continue;
            }
        } else {
            return None;
        }
        if !super::catalog::is_leftover(&name, kind.is_dir()) {
            return None;
        }
        names.push(name.to_str()?.to_owned());
    }
    names.sort();
    Some(names)
}

/// True when everything in `folder` now is a leftover the review listed.
fn holds_only_listed(folder: &Path, listed: &[String]) -> bool {
    let Ok(entries) = fs::read_dir(folder) else {
        return false;
    };
    entries.into_iter().all(|entry| {
        entry.is_ok_and(|entry| {
            let name = entry.file_name();
            entry.file_type().is_ok_and(|kind| {
                (kind.is_dir() || kind.is_file())
                    && super::catalog::is_leftover(&name, kind.is_dir())
                    && listed.iter().any(|known| name == known.as_str())
            })
        })
    })
}

/// A reviewed folder with leftovers that could not go to the Trash. It
/// stays as it is; the result says so instead of failing the whole apply.
#[derive(Debug)]
struct FolderKept;
impl std::fmt::Display for FolderKept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Could not move the folder to the Trash")
    }
}
impl std::error::Error for FolderKept {}

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
/// Most reviewed moves whose "pending" records share one journal sync.
const MOVE_BATCH: usize = 64;
#[cfg(test)]
thread_local! {
    /// Journal syncs on this test thread, and the journal length each made durable.
    static JOURNAL_SYNCS: std::cell::RefCell<Vec<u64>> = const { std::cell::RefCell::new(Vec::new()) };
}
/// A full journal sync is the slowest step of a small action (`F_FULLFSYNC`
/// on macOS), so the executor asks for one only where recovery needs it.
fn sync_journal(journal: &File) -> Result<()> {
    journal.sync_all()?;
    #[cfg(test)]
    JOURNAL_SYNCS.with_borrow_mut(|syncs| syncs.push(journal.metadata().map_or(0, |m| m.len())));
    Ok(())
}
/// The reviewed moves from `start` that may share one "pending" sync. Only a
/// move with a filesystem identity can prove on disk, without its completion
/// record, whether it ran: recovery reads that identity. A move that reads or
/// writes a path an earlier one in the batch touches would hide that proof.
fn batch_end(actions: &[Action], start: usize) -> usize {
    if !cfg!(unix) {
        return start + 1;
    }
    let key = |path: &Path| super::text::folded(&path.to_string_lossy());
    let mut sources = HashSet::new();
    let mut destinations = HashSet::new();
    let mut end = start;
    while end < actions.len() && end - start < MOVE_BATCH {
        let Action::Move { files } = &actions[end] else {
            break;
        };
        if files.is_empty()
            || files.iter().any(|(source, destination)| {
                source.fingerprint.identity[1] == 0
                    || destinations.contains(&key(&source.path))
                    || sources.contains(&key(destination))
            })
        {
            break;
        }
        for (source, destination) in files {
            sources.insert(key(&source.path));
            destinations.insert(key(destination));
        }
        end += 1;
    }
    end.max(start + 1)
}
/// Whether recovery would call this completed move complete from the disk
/// alone: every destination has the identity its reviewed source had, as a
/// hard link or a rename leaves it. A copy is another file and does not.
fn proves_itself(action: &Action, relocated: &[(MediaId, MediaId, Fingerprint)]) -> bool {
    let Action::Move { files } = action else {
        return false;
    };
    cfg!(unix)
        && relocated.len() == files.len()
        && files
            .iter()
            .zip(relocated)
            .all(|((source, _), (_, _, moved))| {
                source.fingerprint.identity[1] != 0 && *moved == source.fingerprint
            })
}
/// Journal durability, for every crash point:
/// - A "pending" record is synced before its action touches the disk.
/// - A "completed", "failed", "skipped" or "cancelled" record is not synced on
///   its own. It becomes durable with the next "pending" sync, which happens
///   before the next action touches the disk, or with the final sync. Power
///   loss in between leaves that one action pending without a completion, the
///   state recovery already resolves by identity or reports as interrupted.
/// - Consecutive moves may share one "pending" sync ([`batch_end`]). Inside a
///   batch, an outcome recovery could not read back from the disk (a copy, a
///   failure) is synced at once, so at most one such action is ever unrecorded.
///   Recovery sorts the rest by identity into completed, partly done and not
///   started.
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
    let mut staging = Staging::default();
    // Actions before this index have a durable "pending" record.
    let mut armed = 0;
    // Whether the last move proved itself on disk, so the next ones may batch.
    let mut moves_prove_themselves = false;
    // Batched actions that will not run still need a receipt: without one
    // they would read as interrupted when the library is reopened.
    let disarm = |journal: &mut File, range: std::ops::Range<usize>| -> Result<()> {
        for index in range {
            write_record(journal, "cancelled", index, &[], None)?;
        }
        Ok(())
    };
    for (i, action) in plan.actions.iter().enumerate() {
        if cancelled() {
            report.cancelled = true;
            report
                .remaining
                .actions
                .extend_from_slice(&plan.actions[i..]);
            disarm(journal, i..armed)?;
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
                write_record(journal, "skipped", i, &[], Some("Unfinished prerequisite"))?;
                report.remaining.actions.push(action.clone());
                progress(i + 1, plan.actions.len());
                continue;
            }
        }
        if i >= armed {
            let end = match moves_prove_themselves {
                true => batch_end(&plan.actions, i),
                false => i + 1,
            };
            for index in i..end {
                write_pending(journal, index, end - i > 1)?;
            }
            sync_journal(journal)?;
            armed = end;
        }
        // The next action of this batch starts without another sync.
        let batch_continues = i + 1 < armed;
        match apply(action, &cancelled, &mut file_names, &mut staging) {
            Ok(()) => {
                let relocated = completion_identities(action);
                let proven = proves_itself(action, &relocated);
                report.complete(action, &relocated);
                write_record(journal, "completed", i, &relocated, None)?;
                if batch_continues && !proven {
                    sync_journal(journal)?;
                }
                if matches!(action, Action::Move { .. }) {
                    moves_prove_themselves = proven;
                }
            }
            Err(error) if error.is::<FolderKept>() => {
                // The tidy step is over either way: the folder stays, a retry
                // would only fail the same way. The result names it.
                if let Action::RemoveEmptyDirectory { path, .. } = action {
                    report.kept_folders.push(path.clone());
                }
                report.complete(action, &[]);
                write_record(journal, "completed", i, &[], None)?;
            }
            Err(error) if error.is::<MoveCancelled>() => {
                report.cancelled = true;
                report
                    .remaining
                    .actions
                    .extend_from_slice(&plan.actions[i..]);
                write_record(journal, "cancelled", i, &[], None)?;
                disarm(journal, i + 1..armed)?;
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
                write_record(journal, "cancelled", i, &[], None)?;
                disarm(journal, i + 1..armed)?;
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
                    Action::Trash { files } => unfinished_dependencies
                        .extend(files.iter().map(|source| source.path.clone())),
                    _ => {}
                }
                let detail = format!("{e:#}");
                report.errors.push(action.failure_message(&detail));
                // A move that failed after staging its destinations cannot be
                // retried as is: they already exist. Receipt its progress, as
                // reopening after a crash would, so a retry only removes staged
                // sources and moves the untouched rest. The executor knows which
                // sources it removed, so copies count too, not only hard links.
                let receipt = match (e.downcast_ref::<PartialMove>(), action) {
                    (Some(partial), Action::Move { .. }) => Ok(Some(partial.states.clone())),
                    _ => recovery::moves::verify(action, &|| false),
                };
                match receipt {
                    Ok(Some(states)) => {
                        recovery::moves::write(journal, i, action, &states, Some(&detail))?;
                        let remainder = recovery::moves::restore(action, &states, &mut report)?;
                        report.remaining.actions.extend(remainder);
                    }
                    _ => {
                        let trashed = e.downcast_ref::<PartialTrash>().map_or(0, |p| p.trashed);
                        if let Some((done, rest)) = split_trash(action, trashed) {
                            report.complete(&done, &[]);
                            report.remaining.actions.push(rest);
                        } else {
                            report.remaining.actions.push(action.clone());
                        }
                        write_failure(journal, i, trashed, &detail)?;
                    }
                }
                if batch_continues {
                    sync_journal(journal)?;
                }
                if matches!(action, Action::Move { .. }) {
                    moves_prove_themselves = false;
                }
                if matches!(action, Action::RenameDirectory(_)) {
                    // Later sources depend on the reviewed spelling. Never run them
                    // after an unsuccessful prerequisite, including on retry.
                    report
                        .remaining
                        .actions
                        .extend_from_slice(&plan.actions[i + 1..]);
                    disarm(journal, i + 1..armed)?;
                    progress(i + 1, plan.actions.len());
                    break;
                }
            }
        }
        progress(i + 1, plan.actions.len());
    }
    report.created_folders = staging.created;
    // Everything recorded since the last "pending" sync, on every way out.
    sync_journal(journal)?;
    Ok(report)
}
/// Records name their action by its index in the plan the journal's header
/// holds. They never repeat the action: a folder's whole inventory, or every
/// file of a move, would otherwise be written up to three times per Apply.
/// Journals written before this, which did repeat it, read the same way.
///
/// `batched` marks a start that shared its sync with other moves: reopening
/// may then find it never ran, which recovery checks on disk.
fn write_pending(journal: &mut File, index: usize, batched: bool) -> Result<()> {
    if !batched {
        return write_record(journal, "pending", index, &[], None);
    }
    #[derive(Serialize)]
    struct Record {
        status: &'static str,
        index: usize,
        batched: bool,
    }
    write_line(
        journal,
        &Record {
            status: "pending",
            index,
            batched,
        },
    )
}
/// One record, one write: the journal is unbuffered so that a crash of the
/// app alone loses nothing, and a record written whole is not torn by it.
pub(in crate::core::operations) fn write_line(
    journal: &mut File,
    record: &impl Serialize,
) -> Result<()> {
    let mut line = serde_json::to_vec(record)?;
    line.push(b'\n');
    journal.write_all(&line)?;
    Ok(())
}
fn write_record(
    journal: &mut File,
    status: &str,
    index: usize,
    relocated: &[(MediaId, MediaId, Fingerprint)],
    error: Option<&str>,
) -> Result<()> {
    #[derive(Serialize)]
    struct Record<'a> {
        status: &'a str,
        index: usize,
        #[serde(skip_serializing_if = "<[_]>::is_empty")]
        relocated: &'a [(MediaId, MediaId, Fingerprint)],
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<&'a str>,
    }
    write_line(
        journal,
        &Record {
            status,
            index,
            relocated,
            error,
        },
    )
}
fn write_failure(journal: &mut File, index: usize, trashed: usize, error: &str) -> Result<()> {
    if trashed == 0 {
        return write_record(journal, "failed", index, &[], Some(error));
    }
    #[derive(Serialize)]
    struct Record<'a> {
        status: &'a str,
        index: usize,
        trashed: usize,
        error: &'a str,
    }
    write_line(
        journal,
        &Record {
            status: "failed",
            index,
            trashed,
            error,
        },
    )
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
            target: metadata::WriteTarget::Embedded | metadata::WriteTarget::Pixels,
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
    staging: &mut Staging,
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
            staging.create_folder(path)?;
        }
        Action::RemoveEmptyDirectory {
            path,
            root,
            leftovers,
            ..
        } => {
            anyhow::ensure!(
                path.is_absolute() && root.is_absolute() && path != root && path.starts_with(root),
                "Cleanup must stay below the library root"
            );
            reject_symlink_components(path)?;
            match fs::remove_dir(path) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) if e.kind() == std::io::ErrorKind::DirectoryNotEmpty => {
                    // Only what the review listed goes to the Trash with the
                    // folder. Anything else in it keeps the folder, as before.
                    if !leftovers.is_empty() && holds_only_listed(path, leftovers) {
                        move_to_trash(path).map_err(|error| {
                            tracing::warn!(%error, "could not trash a folder left with only system files");
                            anyhow::Error::new(FolderKept)
                        })?;
                    }
                }
                Err(e) => return Err(e.into()),
            }
        }
        Action::FinishMove { files } => {
            anyhow::ensure!(
                files.len() == 1,
                "Staged source cleanup requires one receipt per file"
            );
            // Preflight the entire reviewed cohort before removing a source.
            let mut checked = Vec::with_capacity(files.len());
            for (source, destination) in files {
                anyhow::ensure!(!cancelled(), "Finishing move cancelled");
                checked.push(verify_finishable_move(source, destination, cancelled)?);
            }
            for ((source, destination), staged) in files.iter().zip(checked) {
                anyhow::ensure!(!cancelled(), "Finishing move cancelled");
                source.verify()?;
                anyhow::ensure!(
                    Fingerprint::read(destination)? == staged,
                    "Staged move destination changed: {}",
                    destination.display()
                );
                fs::remove_file(&source.path)?;
            }
        }
        Action::Move { files } => apply_move(files, cancelled, staging)?,
        Action::Trash { files } => {
            for source in files {
                source.verify()?;
            }
            for (trashed, source) in files.iter().enumerate() {
                move_to_trash(&source.path)
                    .with_context(|| {
                        format!("Could not move {} to OS Trash", source.path.display())
                    })
                    .map_err(|error| match trashed {
                        0 => error,
                        _ => PartialTrash {
                            trashed,
                            detail: format!("{error:#}"),
                        }
                        .into(),
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
            let current = source.verify()?;
            let path = metadata::sidecar(&source.path);
            anyhow::ensure!(
                Fingerprint::read(&path).ok().map(Fingerprint::stable)
                    == sidecar_before.map(Fingerprint::stable),
                "Sidecar changed since review"
            );
            reject_symlink_components(&source.path)?;
            match target {
                metadata::WriteTarget::Sidecar => {
                    metadata::write_sidecar(&source.path, *rating, *orientation)?
                }
                metadata::WriteTarget::Embedded => {
                    metadata::write_embedded(&source.path, current, *rating, *orientation)?
                }
                metadata::WriteTarget::Pixels => {
                    anyhow::ensure!(rating.is_none(), "Pixel rotation cannot carry a rating");
                    super::reencode::rotate(
                        &source.path,
                        current,
                        orientation.context("Pixel rotation needs an orientation")?,
                    )?
                }
            }
        }
    }
    Ok(())
}
/// One reviewed cohort (a photo with its RAW and XMP), never replacing a
/// destination. Each member reaches its new name the cheapest safe way:
/// - a hard link, where the volume has them: both names exist until the whole
///   cohort is staged, then the sources are removed;
/// - on another volume, a copy, staged the same way;
/// - where the volume refuses hard links (exFAT, FAT, most network shares), an
///   exclusive rename, which is instant but has no staged state: the source
///   name is gone the moment the destination exists. An empty member is
///   copied instead, like one on another volume.
///
/// So renames wait until every other member is staged and nothing can be
/// cancelled any more, and run before any staged source is removed. If the
/// first rename is refused, nothing irreversible has happened yet and those
/// members are staged by copying instead, as before. After the first rename
/// the cohort is finished like a started source removal: a later failure is
/// reported member by member ([`PartialMove`]).
///
/// An interrupted cohort needs no extra record. Each member is read from the
/// disk by identity: source gone and destination holding the source's
/// identity is completed (a rename keeps the inode, like a link), both
/// present is staged, source intact and destination absent is untouched.
fn apply_move(
    files: &[(Source, PathBuf)],
    cancelled: &dyn Fn() -> bool,
    staging: &mut Staging,
) -> Result<()> {
    use recovery::moves::State;
    for (s, d) in files {
        check_move_cancelled(cancelled)?;
        s.verify()?;
        reject_symlink_components(d)?;
        anyhow::ensure!(!d.exists(), "Destination already exists: {}", d.display());
    }
    // Members whose destination exists beside the still-present source.
    let mut staged: Vec<usize> = Vec::new();
    // Members a hard link refused on this volume: renamed at commit.
    let mut deferred: Vec<usize> = Vec::new();
    // Cancellation and failure roll back staging only: no source is touched.
    let unstage = |staged: &[usize], error: anyhow::Error| -> anyhow::Error {
        let mut cleanup_errors = Vec::new();
        for &member in staged {
            let d = &files[member].1;
            if let Err(cleanup) = fs::remove_file(d) {
                cleanup_errors.push(format!("{}: {cleanup}", d.display()));
            }
        }
        if cleanup_errors.is_empty() {
            return error;
        }
        anyhow::anyhow!(
            "{error:#}; could not remove staged destinations: {}",
            cleanup_errors.join("; ")
        )
    };
    let stage = (|| -> Result<()> {
        for (member, (s, d)) in files.iter().enumerate() {
            check_move_cancelled(cancelled)?;
            staging.create_folder(d.parent().context("Missing destination parent")?)?;
            check_move_cancelled(cancelled)?;
            match link_exclusive(&s.path, d) {
                Link::Made => staged.push(member),
                Link::OtherVolume => {
                    staging.copy(&s.path, d, cancelled)?;
                    staged.push(member);
                }
                // An empty file (a sidecar can be one) cannot be renamed
                // where the name has to be reserved first: it looks like its
                // own reservation, and [`Staging::rename`] refuses it. Found
                // only at commit, after another member was renamed, that
                // would leave the cohort half moved. Copying nothing costs
                // nothing, so it is staged here, while all can be undone.
                Link::Refused if s.fingerprint.bytes == 0 => {
                    staging.copy(&s.path, d, cancelled)?;
                    staged.push(member);
                }
                Link::Refused => deferred.push(member),
            }
            s.verify()?;
        }
        check_move_cancelled(cancelled)?;
        Ok(())
    })();
    if let Err(e) = stage {
        return Err(unstage(&staged, e));
    }
    let mut states = vec![State::Untouched; files.len()];
    let mut renamed = false;
    for (position, &member) in deferred.iter().enumerate() {
        let (s, d) = &files[member];
        let verified = match s.verify() {
            // A changed source stops the cohort while it can still be undone.
            Err(error) if !renamed => return Err(unstage(&staged, error)),
            verified => verified,
        };
        let moved = verified
            .and_then(|_| staging.rename(&s.path, d))
            .with_context(|| format!("Could not move {}", s.path.display()));
        match moved {
            Ok(()) => {
                states[member] = State::Completed;
                renamed = true;
            }
            Err(error) if renamed => {
                for &member in &staged {
                    states[member] = State::Staged;
                }
                return Err(PartialMove {
                    states,
                    detail: format!("{error:#}"),
                }
                .into());
            }
            Err(error) => {
                // No source is gone yet, so this is still staging: copy the
                // members that could not be renamed, cancellable as before.
                tracing::debug!(%error, "staging by copy where the volume refused a rename");
                let copied = (|| -> Result<()> {
                    for &member in &deferred[position..] {
                        let (s, d) = &files[member];
                        check_move_cancelled(cancelled)?;
                        staging.copy(&s.path, d, cancelled)?;
                        staged.push(member);
                        s.verify()?;
                    }
                    check_move_cancelled(cancelled)?;
                    Ok(())
                })();
                if let Err(e) = copied {
                    return Err(unstage(&staged, e));
                }
                break;
            }
        }
    }
    for &member in &staged {
        states[member] = State::Staged;
    }
    // Cancellation rolls back staging only. Once a source is renamed or removed,
    // finish this companion cohort rather than introducing partial moves.
    // All linked and copied companions have durable destinations before any
    // source is removed. If removal fails, leave them and report the exact source.
    for &member in &staged {
        let s = &files[member].0;
        if let Err(error) = fs::remove_file(&s.path) {
            return Err(PartialMove {
                states,
                detail: format!(
                    "Copied safely but could not remove source {}: {error}",
                    s.path.display()
                ),
            }
            .into());
        }
        states[member] = State::Completed;
    }
    Ok(())
}
/// Proof that `destination` is the reviewed source under a second name, a
/// hard link, so that removing the source name loses nothing. The plan may
/// have been saved in an earlier run and the drive mounted again since, which
/// changes the device number and nothing else, so the proof has two halves:
/// - right now both names are one file: device, inode, size and dates agree
///   exactly, both read under this mount;
/// - that file is the reviewed one: its inode, size, modified time and birth
///   time are the saved ones. Size and dates alone would not do, a copy made
///   by PhotoSort carries them too.
///
/// Where a volume numbers its inodes anew per mount this fails, as it did
/// when the device number was compared: the caller then proves the bytes or
/// reports the move as interrupted. Returns the destination as it is now.
fn verify_staged_move(source: &Source, destination: &Path) -> Result<Fingerprint> {
    anyhow::ensure!(
        cfg!(unix) && source.fingerprint.identity[1] != 0,
        "Staged moves require a verified filesystem identity"
    );
    check_staged_paths(source, destination)?;
    let staged = Fingerprint::read(destination)?;
    anyhow::ensure!(
        staged == Fingerprint::read(&source.path)? && staged.same_file(&source.fingerprint),
        "Staged move destination changed: {}",
        destination.display()
    );
    Ok(staged)
}
/// Before a retry removes the source of a staged move. A hard link proves
/// itself by identity. A copy (another volume, or no inode identity, as on
/// Windows) must be a separate file with the source's size and bytes. Its
/// modified time is not compared: FAT, exFAT and network shares round it.
/// Returns the destination it checked, as read in this run: the caller
/// compares it exactly before each source is removed.
fn verify_finishable_move(
    source: &Source,
    destination: &Path,
    cancelled: &dyn Fn() -> bool,
) -> Result<Fingerprint> {
    if let Ok(staged) = verify_staged_move(source, destination) {
        return Ok(staged);
    }
    check_staged_paths(source, destination)?;
    let staged = Fingerprint::read(destination)?;
    anyhow::ensure!(
        staged.bytes == source.fingerprint.bytes
            && content_hash(&source.path, cancelled)? == content_hash(destination, cancelled)?,
        "Staged move destination changed: {}",
        destination.display()
    );
    Ok(staged)
}
fn check_staged_paths(source: &Source, destination: &Path) -> Result<()> {
    anyhow::ensure!(
        source.path.is_absolute() && destination.is_absolute(),
        "Staged move paths must be absolute"
    );
    reject_symlink_components(&source.path)?;
    source.verify()?;
    reject_symlink_components(destination)?;
    // Equal identity or bytes do not prove two directory entries: a same-path,
    // dot-component or case-insensitive alias must never unlink its destination.
    // Folder identity is device and inode on Unix, volume and file index on Windows.
    anyhow::ensure!(
        cfg!(any(unix, windows)),
        "Staged moves require a folder identity"
    );
    let source_parent = source.path.parent().context("Missing source parent")?;
    let destination_parent = destination.parent().context("Missing destination parent")?;
    if Fingerprint::read_directory(source_parent)?.identity
        == Fingerprint::read_directory(destination_parent)?.identity
    {
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
    Ok(())
}
/// A Trash group that failed after its first `trashed` files left: those are
/// gone, so a retry may only repeat the rest.
#[derive(Debug)]
struct PartialTrash {
    trashed: usize,
    detail: String,
}
impl std::fmt::Display for PartialTrash {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}
impl std::error::Error for PartialTrash {}
/// A move that stopped after its first source was renamed or removed, with
/// what became of each member: completed, staged beside its source, or
/// untouched (still to be renamed).
#[derive(Debug)]
struct PartialMove {
    states: Vec<recovery::moves::State>,
    detail: String,
}
impl std::fmt::Display for PartialMove {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}
impl std::error::Error for PartialMove {}
/// The trashed and the remaining part of a Trash group, for the executor and
/// for a journal reader, which must agree exactly.
fn split_trash(action: &Action, trashed: usize) -> Option<(Action, Action)> {
    let Action::Trash { files } = action else {
        return None;
    };
    (0 < trashed && trashed < files.len()).then(|| {
        let (done, rest) = files.split_at(trashed);
        (
            Action::Trash {
                files: done.to_vec(),
            },
            Action::Trash {
                files: rest.to_vec(),
            },
        )
    })
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
#[cfg(test)]
thread_local! {
    /// Stages moves by copying, as across volumes, on this test thread.
    static COPY_ONLY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Refuses hard links on this test thread, as exFAT, FAT and network shares do.
    static NO_HARD_LINKS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Also refuses the atomic exclusive rename, as macOS does on exFAT and FAT.
    static NO_EXCLUSIVE_RENAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
enum Link {
    Made,
    /// The destination is on another volume: only a copy can reach it.
    OtherVolume,
    /// This volume has no hard links (or refused this one): rename instead.
    Refused,
}
fn link_exclusive(source: &Path, dest: &Path) -> Link {
    #[cfg(test)]
    if COPY_ONLY.get() {
        return Link::OtherVolume;
    } else if NO_HARD_LINKS.get() {
        return Link::Refused;
    }
    match fs::hard_link(source, dest) {
        Ok(()) => Link::Made,
        Err(error) if error.kind() == std::io::ErrorKind::CrossesDevices => Link::OtherVolume,
        Err(_) => Link::Refused,
    }
}
/// The names PhotoSort gives its own short-lived files in a destination
/// folder: a copy still being written, or the marker beside a name it holds
/// for a rename. They are never library files; the scan skips them.
const TEMPORARY_PREFIX: &str = ".photosort-";
const TEMPORARY_SUFFIX: &str = ".tmp";
pub fn is_temporary_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        // Where a volume has no extended attributes, macOS keeps them in a
        // `._` file beside each file, a temporary one included.
        let name = name.strip_prefix("._").unwrap_or(name);
        name.len() > TEMPORARY_PREFIX.len() + TEMPORARY_SUFFIX.len()
            && name.starts_with(TEMPORARY_PREFIX)
            && name.ends_with(TEMPORARY_SUFFIX)
    })
}
/// The temporary file a `._` name belongs to: where a volume has no extended
/// attributes, macOS keeps them in such a companion beside each file.
fn companion_owner(name: &std::ffi::OsStr) -> Option<&str> {
    name.to_str()?
        .strip_prefix("._")
        .filter(|owner| is_temporary_name(std::ffi::OsStr::new(owner)))
}
/// The final name a temporary file was made for: `.photosort-<random>.<name>.tmp`.
fn temporary_target(name: &std::ffi::OsStr) -> Option<&str> {
    let inner = name
        .to_str()?
        .strip_prefix(TEMPORARY_PREFIX)?
        .strip_suffix(TEMPORARY_SUFFIX)?;
    inner
        .split_once('.')
        .map(|(_, target)| target)
        .filter(|target| !target.is_empty())
}
/// A new private file in `folder`, locked while this process uses it so that
/// another PhotoSort never mistakes it for one a crash left behind. With
/// `target`, its name carries the final name it stands for.
fn create_temporary(folder: &Path, target: Option<&std::ffi::OsStr>) -> Result<(File, PathBuf)> {
    // A name the volume could not hold is left out: 255 bytes fit every one.
    let suffix = match target.and_then(|name| name.to_str()) {
        Some(name) if name.len() <= 200 => format!(".{name}{TEMPORARY_SUFFIX}"),
        _ => TEMPORARY_SUFFIX.to_string(),
    };
    let mut options = OpenOptions::new();
    options.create_new(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut attempt = 0;
    loop {
        // Each hasher state is seeded from the system's random source.
        use std::hash::{BuildHasher, Hasher};
        let random = std::collections::hash_map::RandomState::new()
            .build_hasher()
            .finish();
        let path = folder.join(format!("{TEMPORARY_PREFIX}{random:016x}{suffix}"));
        match options.open(&path) {
            Ok(file) => {
                // Advisory, and missing on some network shares: age decides there.
                let _ = file.try_lock();
                return Ok((file, path));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt < 8 => {
                attempt += 1;
            }
            Err(error) => return Err(error.into()),
        }
    }
}
/// Whether no running PhotoSort owns this temporary file. Where the volume
/// has no locks, only a file untouched for a day counts.
fn temporary_is_stale(path: &Path) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    let Ok(file) = File::open(path) else {
        return false;
    };
    match file.try_lock() {
        Ok(()) => true,
        Err(fs::TryLockError::WouldBlock) => false,
        Err(fs::TryLockError::Error(_)) => metadata
            .modified()
            .ok()
            .and_then(|modified| modified.elapsed().ok())
            .is_some_and(|age| age > std::time::Duration::from_secs(24 * 60 * 60)),
    }
}
/// A companion whose temporary file is gone is left alone this long after it
/// was last written. A finished copy gets its final name first and macOS
/// moves the companion along a moment later: in between it looks orphaned.
const COMPANION_SETTLE: std::time::Duration = std::time::Duration::from_secs(60);
/// Whether the `._` companion at `path` belongs to no temporary file in use.
/// Nothing ever locks a companion, so its own lock says nothing. It is stale
/// when this sweep just removed its temporary file as stale, or when that
/// file is gone and the companion has settled. While the temporary file is
/// there, a running PhotoSort may own it, and its copy checks the attributes
/// kept in the companion: it stays.
fn companion_is_stale(path: &Path, owner: &Path, owner_removed: bool) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    owner_removed
        || (matches!(fs::symlink_metadata(owner), Err(e) if e.kind() == std::io::ErrorKind::NotFound)
            && metadata
                .modified()
                .ok()
                .and_then(|modified| modified.elapsed().ok())
                .is_some_and(|age| age > COMPANION_SETTLE))
}
/// Removes what an interrupted run left in `folder`: partial copies under
/// their temporary names, and an empty file still holding a final name for a
/// transfer that never arrived. Runs before PhotoSort next writes to the
/// folder and when recovery reopens an interrupted move. Only names PhotoSort
/// generates are touched, and never a file that has any content under a final
/// name. Another PhotoSort may be copying into the folder meanwhile: its
/// temporary files are locked, and they and their companions are left.
pub(crate) fn sweep_temporaries(folder: &Path) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    // Companions are judged after the temporary files they belong to.
    let (companions, temporaries): (Vec<_>, Vec<_>) = entries
        .flatten()
        .map(|entry| entry.file_name())
        .filter(|name| is_temporary_name(name))
        .partition(|name| companion_owner(name).is_some());
    let mut removed = HashSet::new();
    for name in temporaries {
        let path = folder.join(&name);
        if !temporary_is_stale(&path) {
            continue;
        }
        // A named temporary file is only made for a transfer with content, so
        // an empty file under that name is the reservation, not the photo.
        if let Some(target) = temporary_target(&name) {
            let reserved = folder.join(target);
            if fs::symlink_metadata(&reserved).is_ok_and(|m| m.is_file() && m.len() == 0) {
                match fs::remove_file(&reserved) {
                    Ok(()) => {
                        tracing::info!(path = %reserved.display(), "removed a name held by an interrupted move")
                    }
                    Err(error) => {
                        tracing::warn!(path = %reserved.display(), %error, "could not remove a name held by an interrupted move")
                    }
                }
            }
        }
        match fs::remove_file(&path) {
            Ok(()) => {
                tracing::info!(path = %path.display(), "removed a temporary file left by an interrupted move");
                removed.insert(name);
            }
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "could not remove a stale temporary file")
            }
        }
    }
    for name in companions {
        let Some(owner) = companion_owner(&name) else {
            continue;
        };
        let path = folder.join(&name);
        let owner_removed = removed.contains(std::ffi::OsStr::new(owner));
        if !companion_is_stale(&path, &folder.join(owner), owner_removed) {
            continue;
        }
        match fs::remove_file(&path) {
            Ok(()) => {
                tracing::info!(path = %path.display(), "removed the attribute file of a temporary file left by an interrupted move")
            }
            // macOS removes it with the file it belongs to.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "could not remove a stale attribute file")
            }
        }
    }
}
fn exclusive_rename(from: &Path, to: &Path) -> std::io::Result<()> {
    #[cfg(test)]
    if NO_EXCLUSIVE_RENAME.get() {
        return match fs::symlink_metadata(to) {
            Ok(_) => Err(std::io::ErrorKind::AlreadyExists.into()),
            Err(_) => Err(std::io::ErrorKind::Unsupported.into()),
        };
    }
    case_rename::rename_exclusive(from, to).map_err(|error| {
        error
            .downcast::<std::io::Error>()
            .unwrap_or_else(std::io::Error::other)
    })
}
/// The volume has no atomic "rename unless the name is taken", as opposed to
/// refusing this particular rename.
fn rename_unsupported(error: &std::io::Error) -> bool {
    // Some of these share a number on Linux, so this is not a match.
    #[cfg(unix)]
    if error.raw_os_error().is_some_and(|code| {
        [libc::ENOTSUP, libc::EOPNOTSUPP, libc::ENOSYS, libc::EINVAL].contains(&code)
    }) {
        return true;
    }
    matches!(
        error.kind(),
        std::io::ErrorKind::Unsupported | std::io::ErrorKind::InvalidInput
    )
}
#[cfg(unix)]
fn same_entry(a: &fs::Metadata, b: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    (a.dev(), a.ino()) == (b.dev(), b.ino())
}
#[cfg(not(unix))]
fn same_entry(_: &fs::Metadata, _: &fs::Metadata) -> bool {
    true
}
/// Rename for a volume with neither hard links nor an atomic exclusive
/// rename (exFAT and FAT on macOS). Creating the destination name exclusively
/// is atomic everywhere, so that claims it; the rename then replaces only
/// that empty file, which this call created a moment ago and checks again.
/// An existing destination is never replaced: claiming it fails first.
///
/// A crash between the two steps leaves an empty file under the final name.
/// The caller's temporary file names it, so [`sweep_temporaries`] removes it.
fn rename_reserved(from: &Path, to: &Path) -> std::io::Result<()> {
    let reservation = OpenOptions::new().write(true).create_new(true).open(to)?;
    let ours = reservation.metadata()?;
    // Closed first: Windows cannot always rename over an open file.
    drop(reservation);
    let still_ours = || {
        fs::symlink_metadata(to)
            .is_ok_and(|found| found.is_file() && found.len() == 0 && same_entry(&ours, &found))
    };
    let renamed = match still_ours() {
        true => fs::rename(from, to),
        false => Err(std::io::Error::other(
            "The reserved destination name was changed by something else",
        )),
    };
    if renamed.is_err() && still_ours() {
        let _ = fs::remove_file(to);
    }
    renamed
}
/// What one executor run remembers about the destination folders it wrote to.
#[derive(Default)]
struct Staging {
    swept: HashSet<PathBuf>,
    /// The folders this run made, each parent before its subfolders.
    created: Vec<PathBuf>,
}
impl Staging {
    /// Makes `folder` and its missing parents, and remembers which it made.
    fn create_folder(&mut self, folder: &Path) -> std::io::Result<()> {
        if folder.is_dir() {
            return Ok(());
        }
        let missing: Vec<&Path> = folder.ancestors().take_while(|p| !p.exists()).collect();
        fs::create_dir_all(folder)?;
        self.created
            .extend(missing.into_iter().rev().map(Path::to_path_buf));
        Ok(())
    }
    /// Before the first copy or reservation in a folder, clear what an
    /// interrupted run left there. Once per folder: a listing per file would
    /// cost more than the move on a slow card.
    fn sweep(&mut self, dest: &Path) {
        if let Some(folder) = dest.parent()
            && self.swept.insert(folder.to_path_buf())
        {
            sweep_temporaries(folder);
        }
    }
    fn copy(&mut self, source: &Path, dest: &Path, cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.sweep(dest);
        copy_contents_exclusive(source, dest, cancelled)
    }
    /// Moves `from` to `to` on its own volume without copying and without ever
    /// replacing `to`. An error means nothing changed and the caller may copy.
    fn rename(&mut self, from: &Path, to: &Path) -> Result<()> {
        let refused = match exclusive_rename(from, to) {
            Ok(()) => return Ok(()),
            Err(error) => error,
        };
        if !rename_unsupported(&refused) {
            return Err(refused.into());
        }
        // An empty source would be indistinguishable from its own reservation.
        anyhow::ensure!(
            fs::symlink_metadata(from)?.len() > 0,
            "An empty file is copied, not renamed"
        );
        self.sweep(to);
        let folder = to.parent().context("Missing destination parent")?;
        let (marker, marker_path) = create_temporary(folder, to.file_name())?;
        let renamed = rename_reserved(from, to);
        drop(marker);
        let _ = fs::remove_file(&marker_path);
        Ok(renamed?)
    }
}
// The fallback is shared by every move when the destination cannot hard-link the
// source. Bound cancellation work without allocating the entire photo or video.
fn copy_contents_exclusive(source: &Path, dest: &Path, cancelled: &dyn Fn() -> bool) -> Result<()> {
    // Under heavy concurrent I/O, macOS can drop a large resource fork from a
    // fresh copy. The attempt detects it and rolls back; a new copy usually
    // keeps it, so try a few times before reporting the error.
    let mut attempt = 0;
    loop {
        match copy_contents_once(source, dest, cancelled) {
            Err(error)
                if attempt < 2
                    && !error.is::<MoveCancelled>()
                    && !dest.exists()
                    && format!("{error:#}").contains("extended attributes") =>
            {
                attempt += 1;
                tracing::warn!(attempt, %error, "retrying fallback copy");
                std::thread::sleep(std::time::Duration::from_millis(50 * attempt));
            }
            result => return result,
        }
    }
}
/// The source's dates for its copy: modified and accessed everywhere, and
/// where the platform can set it (macOS, Windows) the creation date too.
/// The second value leaves the creation date out, for a volume that refuses it.
fn copied_times(metadata: &fs::Metadata) -> std::io::Result<(fs::FileTimes, fs::FileTimes)> {
    let mut times = fs::FileTimes::new().set_modified(metadata.modified()?);
    if let Ok(accessed) = metadata.accessed() {
        times = times.set_accessed(accessed);
    }
    #[cfg(any(target_os = "macos", windows))]
    if let Ok(created) = metadata.created() {
        #[cfg(target_os = "macos")]
        use std::os::macos::fs::FileTimesExt;
        #[cfg(windows)]
        use std::os::windows::fs::FileTimesExt;
        return Ok((times.set_created(created), times));
    }
    Ok((times, times))
}
/// The copy is written under a temporary name in the destination folder and
/// only given its final name once complete and synced, so a crash, a power
/// cut or an unplugged drive can never leave a partial photo under the name
/// the plan promised. The final step never replaces a file.
fn copy_contents_once(source: &Path, dest: &Path, cancelled: &dyn Fn() -> bool) -> Result<()> {
    check_move_cancelled(cancelled)?;
    match fs::symlink_metadata(dest) {
        Ok(_) => anyhow::bail!("Destination already exists: {}", dest.display()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    let folder = dest.parent().context("Missing destination parent")?;
    let mut input = File::open(source)?;
    let metadata = input.metadata()?;
    let (times, times_without_created) = copied_times(&metadata)?;
    // Created exclusively, readable and writable by this user only: do not
    // expose a private source while its copy is still partial. Read access
    // matters too: macOS resource-fork attributes are read back through this
    // descriptor to verify the copy. An empty source gets an unnamed temporary
    // file: its finished copy is as empty as a reservation.
    let target = dest.file_name().filter(|_| metadata.len() > 0);
    let (mut output, temporary) = create_temporary(folder, target)?;
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
        #[cfg(target_os = "macos")]
        macos_attributes::copy(&input, &output, cancelled)?;
        // Capture times before reading and preserve them before making the copy
        // read-only. Failure leaves the source intact and rolls back this copy.
        if output.set_times(times).is_err() {
            output.set_times(times_without_created)?;
        }
        output.set_permissions(metadata.permissions())?;
        output.sync_all()?;
        check_move_cancelled(cancelled)?;
        // Still locked where that is advisory: nothing may take this file for
        // a stale one until it has its final name. A Windows lock is released
        // first, so it cannot stand in the way of the rename.
        #[cfg(windows)]
        let _ = output.unlock();
        place_temporary(&temporary, dest)
    })();
    drop(output);
    if let Err(error) = result {
        // A failed rollback is an operation error, never a clean cancellation.
        if let Err(cleanup) = fs::remove_file(&temporary) {
            anyhow::bail!(
                "{error}; could not remove partial destination {}: {cleanup}",
                temporary.display()
            );
        }
        return Err(error);
    }
    Ok(())
}
/// Gives a finished temporary copy its final name without ever replacing a
/// file: an atomic exclusive rename, else a hard link (the name is created
/// only if free, then the temporary name goes), else a reserved rename.
fn place_temporary(temporary: &Path, dest: &Path) -> Result<()> {
    let exists = |error: &std::io::Error| error.kind() == std::io::ErrorKind::AlreadyExists;
    let taken = || anyhow::anyhow!("Destination already exists: {}", dest.display());
    match exclusive_rename(temporary, dest) {
        Ok(()) => return Ok(()),
        Err(error) if exists(&error) => return Err(taken()),
        Err(_) => {}
    }
    #[cfg(test)]
    let link = match NO_HARD_LINKS.get() {
        true => Err(std::io::ErrorKind::Unsupported.into()),
        false => fs::hard_link(temporary, dest),
    };
    #[cfg(not(test))]
    let link = fs::hard_link(temporary, dest);
    match link {
        Ok(()) => {
            // The copy has its final name. A temporary name that cannot be
            // removed now is cleared by the next sweep of this folder.
            if let Err(error) = fs::remove_file(temporary) {
                tracing::warn!(path = %temporary.display(), %error, "could not remove a temporary name");
            }
            return Ok(());
        }
        Err(error) if exists(&error) => return Err(taken()),
        Err(_) => {}
    }
    rename_reserved(temporary, dest).map_err(|error| match exists(&error) {
        true => taken(),
        false => error.into(),
    })
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
/// Tests never fill the computer's Trash. For a library a test names here,
/// what an Apply sends to the Trash is moved into `trash` instead.
#[cfg(any(test, feature = "test-isolation"))]
#[doc(hidden)]
pub fn trash_stand_in(library: &Path, trash: &Path) {
    TRASH_STAND_INS
        .lock()
        .unwrap()
        .push((library.to_path_buf(), trash.to_path_buf()));
}
#[cfg(any(test, feature = "test-isolation"))]
static TRASH_STAND_INS: std::sync::Mutex<Vec<(PathBuf, PathBuf)>> =
    std::sync::Mutex::new(Vec::new());
#[cfg(any(test, feature = "test-isolation"))]
fn move_to_stand_in(path: &Path) -> Option<Result<()>> {
    let trash = TRASH_STAND_INS
        .lock()
        .unwrap()
        .iter()
        .find(|(library, _)| path.starts_with(library))
        .map(|(_, trash)| trash.clone())?;
    Some((|| {
        fs::create_dir_all(&trash)?;
        let name = path.file_name().context("Nothing to trash")?;
        let mut to = trash.join(name);
        let mut copy = 0;
        while to.exists() {
            copy += 1;
            to = trash.join(format!("{copy} {}", name.to_string_lossy()));
        }
        Ok(fs::rename(path, to)?)
    })())
}
fn move_to_trash(path: &Path) -> Result<()> {
    #[cfg(any(test, feature = "test-isolation"))]
    if let Some(moved) = move_to_stand_in(path) {
        return moved;
    }
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
