//! Application preferences, separate from per-library decisions and view bookmarks.
use super::{metadata::atomic_write, model::Step};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub show_shortcuts: bool,
    pub updates: super::updates::Preferences,
    pub easy_delete: super::analysis::Thresholds,
    /// Easy Delete stages its suggestions as the user's choices; off, every
    /// photo stays kept until the user accepts suggestions.
    pub easy_delete_auto: bool,
    /// The same for Pick Best: the suggested photo of each group is kept and
    /// the others are staged for Trash.
    pub pick_best_auto: bool,
    /// Fix Rotation stages its certain suggestions as the user's choices;
    /// off, every photo stays as it is until the user uses a suggestion.
    pub rotation_auto: bool,
    pub cache: super::cache::Limits,
    pub metadata_writes: super::metadata::WritePolicy,
    pub processing: super::processing::Preferences,
    pub location_layout: super::geography::Layout,
    pub similarity_distance: f32,
    pub subject_strictness: super::subjects::Strictness,
    pub navigation: super::navigation::Preferences,
    pub hidden_steps: HashSet<Step>,
    pub recent_folders: Vec<PathBuf>,
    /// Organize moves RAW, JPEG and XMP companions together.
    pub keep_companions: bool,
    /// Organize by faces: where photos without a recognized person go.
    pub faceless: Faceless,
    /// How RAW photos are displayed; analysis always uses the neutral render.
    pub raw_look: super::raw_look::RawLook,
    /// Finish every interface animation instantly.
    pub reduce_motion: bool,
    /// Where Review shows the strip of photos, in every step.
    pub strip_position: StripPosition,
    /// File names under thumbnails in every step's grid and photo strip.
    pub show_filenames: bool,
    /// Review's photo strip on the left lists photos as compact rows, a
    /// small thumbnail with its name beside it, rather than large tiles.
    /// Without names the rows become a grid of small thumbnails.
    pub compact_strip: bool,
    /// The height of the photos in every step's grid, in points: one size
    /// that every step's size slider shows and changes.
    pub photo_size: f32,
    /// The first-run intro has played. Menu → Watch the intro replays it.
    pub intro_shown: bool,
    /// Prompts the user turned off: about memory and cache limits, and
    /// about RAW files they kept whose JPEG goes to the Trash.
    pub limit_prompts: LimitPrompts,
}
/// Prompts the user answered for good. Settings shows them all again.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LimitPrompts {
    /// Never ask about these again.
    pub quiet: std::collections::BTreeSet<LimitPrompt>,
    /// Open photos that need more than half the memory without asking.
    pub open_large: bool,
    /// What Review's Apply does, without asking, with a RAW file the user
    /// rated, rotated or chose to keep when its JPEG goes to the Trash.
    pub wanted_raw: Option<WantedRaw>,
}
impl LimitPrompts {
    pub fn any(&self) -> bool {
        self.open_large || !self.quiet.is_empty() || self.wanted_raw.is_some()
    }
}
/// The answer to "Trash the RAW files you kept?".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WantedRaw {
    /// They stay, with their XMP, rating and rotation; only the JPEG goes.
    Keep,
    /// They go to the Trash with their JPEG.
    Trash,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum LimitPrompt {
    /// A full-size photo did not fit in full-size detail memory.
    DetailMemory,
    /// The disk cache removed previews made in this session.
    DiskCache,
}
/// The photo size every grid starts with: a photo's height in points.
pub const PHOTO_SIZE: f32 = 120.0;
/// The photo sizes the size slider offers.
pub const PHOTO_SIZES: std::ops::RangeInclusive<f32> = 90.0..=420.0;
/// Where grouping by faces puts photos without a recognized person.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Faceless {
    /// They stay in the folders they are in now.
    #[default]
    Stay,
    /// They go to one folder, [`NO_PEOPLE`].
    Folder,
}
/// The folder for photos without a recognized person, when chosen.
pub const NO_PEOPLE: &str = "No people";
/// The side of the Review canvas that holds the photo strip.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum StripPosition {
    #[default]
    Bottom,
    Left,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            show_shortcuts: true,
            updates: Default::default(),
            easy_delete: Default::default(),
            easy_delete_auto: false,
            pick_best_auto: false,
            rotation_auto: false,
            cache: Default::default(),
            processing: Default::default(),
            metadata_writes: Default::default(),
            navigation: Default::default(),
            subject_strictness: Default::default(),
            location_layout: Default::default(),
            similarity_distance: super::regional::DEFAULT_EPS,
            hidden_steps: HashSet::new(),
            recent_folders: vec![],
            keep_companions: true,
            faceless: Default::default(),
            raw_look: Default::default(),
            reduce_motion: false,
            strip_position: StripPosition::Bottom,
            show_filenames: true,
            compact_strip: true,
            photo_size: PHOTO_SIZE,
            intro_shown: false,
            limit_prompts: Default::default(),
        }
    }
}
impl Preferences {
    pub fn path() -> PathBuf {
        super::session::data_dir().join("preferences.json")
    }
    /// Organize, Cull and Review are always shown: Review applies what the
    /// other steps stage.
    pub fn can_hide(&self, step: Step) -> bool {
        !matches!(step, Step::Organize | Step::Cull | Step::Review)
    }
    pub fn visible(&self, step: Step) -> bool {
        !self.can_hide(step) || !self.hidden_steps.contains(&step)
    }
    pub fn neighbor(&self, step: Step, forward: bool) -> Option<Step> {
        if forward {
            Step::ALL
                .into_iter()
                .skip(step.index() + 1)
                .find(|s| self.visible(*s))
        } else {
            Step::ALL[..step.index()]
                .iter()
                .rev()
                .copied()
                .find(|s| self.visible(*s))
        }
    }
    pub fn normalize(&mut self) {
        self.easy_delete.normalize();
        self.similarity_distance = super::regional::normalize_distance(self.similarity_distance);
        self.cache.normalize();
        self.processing.normalize(super::processing::available());
        self.navigation.normalize();
        self.photo_size = if self.photo_size.is_finite() {
            self.photo_size
                .clamp(*PHOTO_SIZES.start(), *PHOTO_SIZES.end())
        } else {
            PHOTO_SIZE
        };
        self.hidden_steps.remove(&Step::Organize);
        self.hidden_steps.remove(&Step::Cull);
        self.hidden_steps.remove(&Step::Review);
        let mut seen = HashSet::new();
        self.recent_folders = self
            .recent_folders
            .iter()
            // JSON holds only UTF-8 paths; one other would fail every save.
            .filter(|p| p.is_absolute() && p.to_str().is_some())
            .map(|p| p.components().collect::<PathBuf>())
            .filter(|p| seen.insert(path_key(p)))
            .take(10)
            .collect();
    }
    /// Called only with the successfully opened catalog root; no scan or stat here.
    pub fn remember(&mut self, root: &Path) {
        self.recent_folders.insert(0, root.into());
        self.normalize();
    }
    pub fn last_existing_folder(&self) -> Option<PathBuf> {
        self.recent_folders.iter().find(|p| p.is_dir()).cloned()
    }
    /// The folder `--last-folder` opens. An unreadable file opens none
    /// rather than stopping the app; the window says why when it loads it.
    pub fn last_folder(path: &Path) -> Option<PathBuf> {
        Self::load(path)
            .inspect_err(|error| tracing::warn!(%error, "preferences unreadable"))
            .ok()?
            .last_existing_folder()
    }
    pub fn load(path: &Path) -> anyhow::Result<Self> {
        let bytes = match std::fs::read(path) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(e) => return Err(e.into()),
        };
        let (version, mut value): (u32, Self) = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(version == 1, "Unsupported preferences version {version}");
        value.normalize();
        Ok(value)
    }
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut value = self.clone();
        value.normalize();
        atomic_write(path, &serde_json::to_vec(&(1, value))?)
    }
}
fn path_key(path: &Path) -> std::ffi::OsString {
    #[cfg(windows)]
    {
        path.to_string_lossy().to_lowercase().into()
    }
    #[cfg(not(windows))]
    {
        path.as_os_str().into()
    }
}
/// Menu labels for folders: the folder name, plus as much of the parent path
/// as it takes to tell apart folders that share a name ("samples · tests").
pub fn folder_labels(paths: &[impl AsRef<Path>]) -> Vec<String> {
    let parents: Vec<Vec<String>> = paths
        .iter()
        .map(|p| {
            let parent = p.as_ref().parent().unwrap_or(Path::new(""));
            parent
                .components()
                .rev()
                .filter(|c| !matches!(c, Component::RootDir))
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .collect();
    let label = |i: usize, depth: usize| {
        let name = paths[i].as_ref().file_name().map_or_else(
            || paths[i].as_ref().to_string_lossy(),
            |n| n.to_string_lossy(),
        );
        if depth == 0 {
            return name.into_owned();
        }
        let shown: Vec<&str> = parents[i][..depth]
            .iter()
            .rev()
            .map(String::as_str)
            .collect();
        format!("{name} · {}", shown.join(std::path::MAIN_SEPARATOR_STR))
    };
    let mut depths = vec![0; paths.len()];
    loop {
        let labels: Vec<String> = (0..paths.len()).map(|i| label(i, depths[i])).collect();
        let mut grew = false;
        for i in 0..paths.len() {
            let shared = labels.iter().filter(|l| **l == labels[i]).count() > 1;
            if shared && depths[i] < parents[i].len() {
                depths[i] += 1;
                grew = true;
            }
        }
        if !grew {
            return labels;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unreadable_files_and_non_utf8_folders_never_stop_startup_or_saving() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        std::fs::write(&path, b"[2, {}]").unwrap();
        assert_eq!(Preferences::last_folder(&path), None);
        let mut preferences = Preferences::default();
        preferences.remember(dir.path());
        assert_eq!(Preferences::last_folder(&path), None);
        preferences.save(&path).unwrap();
        assert_eq!(Preferences::last_folder(&path).as_deref(), Some(dir.path()));
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let latin1 = dir.path().join(std::ffi::OsStr::from_bytes(b"F\xe9vrier"));
            preferences.remember(&latin1);
            preferences.save(&path).unwrap();
            assert_eq!(preferences.recent_folders, [dir.path()]);
        }
    }
}
