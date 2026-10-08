//! Shared inference settings and session status, captured when a run is accepted.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
/// CPU preserves low initialization cost. Acceleration is an explicit choice;
/// Auto selects a provider for this platform and falls back per model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    #[default]
    Cpu,
    Auto,
    CoreMl,
    Cuda,
    DirectMl,
}
impl Backend {
    pub const ALL: [Self; 5] = [
        Self::Cpu,
        Self::Auto,
        Self::CoreMl,
        Self::Cuda,
        Self::DirectMl,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpu => "CPU",
            Self::Auto => "Automatic",
            Self::CoreMl => "Apple Core ML",
            Self::Cuda => "NVIDIA CUDA",
            Self::DirectMl => "Windows DirectML",
        }
    }
    pub fn candidates(self) -> Vec<Self> {
        match self {
            Self::Cpu => vec![],
            Self::Auto if cfg!(target_os = "macos") => vec![Self::CoreMl],
            Self::Auto if cfg!(target_os = "windows") => vec![Self::Cuda, Self::DirectMl],
            Self::Auto if cfg!(target_os = "linux") => vec![Self::Cuda],
            Self::Auto => vec![],
            other => vec![other],
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunConfig {
    pub threads: usize,
    pub backend: Backend,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModelStatus {
    pub model: &'static str,
    pub backend: Backend,
    pub detail: String,
    pub preparing: bool,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    #[default]
    Balanced,
    Maximum,
    Custom,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Preferences {
    pub mode: Mode,
    pub custom_threads: usize,
    pub backend: Backend,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            mode: Mode::Balanced,
            custom_threads: 2,
            backend: Backend::default(),
        }
    }
}
pub fn available() -> usize {
    std::thread::available_parallelism().map_or(1, usize::from)
}
impl Preferences {
    pub fn normalize(&mut self, available: usize) {
        self.custom_threads = self.custom_threads.clamp(1, available.max(1));
    }
    pub fn threads(self, available: usize) -> usize {
        let available = available.max(1);
        match self.mode {
            Mode::Balanced => available
                .saturating_mul(85)
                .checked_div(100)
                .unwrap_or(1)
                .max(1),
            Mode::Maximum => available,
            Mode::Custom => self.custom_threads.clamp(1, available),
        }
    }
}
/// Short settings locks never cover image work or native initialization.
/// In-flight jobs own a value snapshot, never this mutable preference.
#[derive(Clone)]
pub struct Control {
    requested: Arc<Mutex<RunConfig>>,
    status: Arc<Mutex<BTreeMap<&'static str, ModelStatus>>>,
    available: usize,
}
impl Default for Control {
    fn default() -> Self {
        let available = available();
        Self {
            requested: Arc::new(Mutex::new(RunConfig {
                threads: Preferences::default().threads(available),
                backend: Backend::default(),
            })),
            status: Default::default(),
            available,
        }
    }
}
impl Control {
    pub fn available(&self) -> usize {
        self.available
    }
    pub fn configure(&self, preferences: Preferences) {
        *self.requested.lock().unwrap() = RunConfig {
            threads: preferences.threads(self.available),
            backend: preferences.backend,
        };
    }
    pub fn snapshot_run(&self) -> RunConfig {
        *self.requested.lock().unwrap()
    }
    pub fn status(&self) -> Vec<ModelStatus> {
        self.status.lock().unwrap().values().cloned().collect()
    }
    #[cfg(feature = "onnx")]
    pub(crate) fn preparing(&self, model: &'static str, backend: Backend) {
        self.status.lock().unwrap().insert(
            model,
            ModelStatus {
                model,
                backend,
                detail: "Preparing model…".into(),
                preparing: true,
            },
        );
    }
    #[cfg(feature = "onnx")]
    pub(crate) fn report(&self, model: &'static str, backend: Backend, detail: String) {
        self.status.lock().unwrap().insert(
            model,
            ModelStatus {
                model,
                backend,
                detail,
                preparing: false,
            },
        );
    }
    #[cfg(feature = "onnx")]
    pub(crate) fn clear_model_status(&self, kind: &str) {
        self.status.lock().unwrap().remove(kind);
    }
    pub(crate) fn clear_status(&self) {
        self.status.lock().unwrap().clear();
    }
    pub fn snapshot(&self) -> usize {
        self.snapshot_run().threads
    }
}
