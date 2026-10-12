//! Application preferences, separate from per-library decisions and view bookmarks.
use super::{metadata::atomic_write, model::Step};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub show_shortcuts: bool,
    pub updates: super::updates::Preferences,
    pub easy_delete: super::analysis::Thresholds,
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
    /// How RAW photos are displayed; analysis always uses the neutral render.
    pub raw_look: super::raw_look::RawLook,
    /// Finish every interface animation instantly.
    pub reduce_motion: bool,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            show_shortcuts: true,
            updates: Default::default(),
            easy_delete: Default::default(),
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
            raw_look: Default::default(),
            reduce_motion: false,
        }
    }
}
impl Preferences {
    pub fn path() -> PathBuf {
        super::session::data_dir().join("preferences.json")
    }
    pub fn visible(&self, step: Step) -> bool {
        matches!(step, Step::Organize | Step::Cull) || !self.hidden_steps.contains(&step)
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
        self.hidden_steps.remove(&Step::Organize);
        self.hidden_steps.remove(&Step::Cull);
        let mut seen = HashSet::new();
        self.recent_folders = self
            .recent_folders
            .iter()
            .filter(|p| p.is_absolute())
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
