//! Shared inference settings and session status, captured when a run is accepted.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
/// Auto selects a provider for this platform and falls back to CPU per model.
/// `Default` stays CPU, the value preference files written before the backend
/// setting existed must keep; new installs use [`Backend::platform_default`].
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
    /// The backend for new installs: Automatic where it has been verified on
    /// real hardware (macOS, Core ML with CPU fallback), CPU elsewhere until the
    /// CUDA and DirectML paths are tested.
    pub fn platform_default() -> Self {
        if cfg!(target_os = "macos") {
            Self::Auto
        } else {
            Self::Cpu
        }
    }
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
    /// What preparation is doing now; `None` once it has finished.
    #[serde(skip)]
    pub prep: Option<Prep>,
}
/// One step of preparing a model, in the order they run. Native compilation
/// and loading are single blocking calls with no fraction to report.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PrepStep {
    /// Hashing the model and any saved copy before trusting them.
    Checking,
    /// Converting the model for this machine's accelerator.
    Compiling,
    /// Opening a copy compiled by an earlier launch.
    Reusing,
    /// Opening the model on the CPU.
    Loading,
    /// Recording the compiled copy so the next launch can reuse it.
    Saving,
}
impl PrepStep {
    pub fn label(self) -> &'static str {
        match self {
            Self::Checking => "Checking model files",
            Self::Compiling => "Optimizing for this computer",
            Self::Reusing => "Loading the saved optimized copy",
            Self::Loading => "Loading model",
            Self::Saving => "Saving the optimized copy for next time",
        }
    }
    /// How long this step may take before the UI calls it slow, when this
    /// machine has no earlier timing for the model.
    pub fn patience(self) -> std::time::Duration {
        std::time::Duration::from_secs(match self {
            Self::Checking | Self::Saving => 60,
            Self::Reusing | Self::Loading => 90,
            Self::Compiling => 300,
        })
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prep {
    pub step: PrepStep,
    /// When preparing this model began.
    pub started: std::time::Instant,
    /// When the current step began.
    pub step_started: std::time::Instant,
    /// How long this step took the last time it finished on this machine.
    pub usual: Option<std::time::Duration>,
}
impl Prep {
    /// How long the current step has run.
    pub fn elapsed(&self, now: std::time::Instant) -> std::time::Duration {
        now.saturating_duration_since(self.step_started)
    }
    /// Past the usual time with some slack, or past the step's patience
    /// when this machine has no earlier timing.
    fn limit(&self) -> std::time::Duration {
        self.usual.map_or(self.step.patience(), |usual| {
            (usual + usual / 2).max(usual + std::time::Duration::from_secs(15))
        })
    }
    pub fn slow(&self, now: std::time::Instant) -> bool {
        self.elapsed(now) > self.limit()
    }
    /// Three times past slow: likely stuck rather than merely slow.
    pub fn stalled(&self, now: std::time::Instant) -> bool {
        self.elapsed(now) > self.limit() * 3
    }
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
    /// Missing from preference files that predate this setting; those keep CPU.
    #[serde(default)]
    pub backend: Backend,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            mode: Mode::Balanced,
            custom_threads: 2,
            backend: Backend::platform_default(),
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
    /// Last finished duration of each slow step, keyed by model, backend and step.
    timings: Arc<Mutex<Timings>>,
    available: usize,
}
/// Step durations from earlier preparations, kept in a small file so the
/// next launch can say how long a step usually takes.
#[derive(Default)]
#[cfg_attr(not(feature = "onnx"), allow(dead_code))]
struct Timings {
    path: Option<std::path::PathBuf>,
    millis: BTreeMap<String, u64>,
}
impl Timings {
    #[cfg(feature = "onnx")]
    fn key(model: &str, backend: Backend, step: PrepStep) -> String {
        format!("{model}:{backend:?}:{step:?}")
    }
}
impl Default for Control {
    fn default() -> Self {
        let available = available();
        let preferences = Preferences::default();
        Self {
            requested: Arc::new(Mutex::new(RunConfig {
                threads: preferences.threads(available),
                backend: preferences.backend,
            })),
            status: Default::default(),
            timings: Default::default(),
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
    /// Shows a status as given, without a model; for screenshots.
    pub fn show_status(&self, status: ModelStatus) {
        self.status.lock().unwrap().insert(status.model, status);
    }
    /// Where step timings persist between launches; read once here.
    pub fn remember_timings(&self, path: std::path::PathBuf) {
        let millis = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        *self.timings.lock().unwrap() = Timings {
            path: Some(path),
            millis,
        };
    }
    #[cfg(feature = "onnx")]
    fn usual(&self, model: &str, backend: Backend, step: PrepStep) -> Option<std::time::Duration> {
        let timings = self.timings.lock().unwrap();
        let millis = timings.millis.get(&Timings::key(model, backend, step))?;
        Some(std::time::Duration::from_millis(*millis))
    }
    #[cfg(feature = "onnx")]
    /// Logs how long the finished step took and keeps that time for next launch.
    fn finish_step(&self, model: &'static str, backend: Backend, prep: Prep) {
        let millis = prep.step_started.elapsed().as_millis() as u64;
        tracing::info!(model, ?backend, step = ?prep.step, millis, "model preparation step finished");
        let mut timings = self.timings.lock().unwrap();
        timings
            .millis
            .insert(Timings::key(model, backend, prep.step), millis);
        if let Some(path) = &timings.path
            && let Ok(bytes) = serde_json::to_vec(&timings.millis)
        {
            let _ = std::fs::write(path, bytes);
        }
    }
    #[cfg(feature = "onnx")]
    pub(crate) fn preparing(&self, model: &'static str, backend: Backend) {
        tracing::info!(model, ?backend, "model preparation started");
        self.status.lock().unwrap().insert(
            model,
            ModelStatus {
                model,
                backend,
                detail: "Preparing model…".into(),
                preparing: true,
                prep: None,
            },
        );
    }
    #[cfg(feature = "onnx")]
    /// Moves a preparing model to its next step, closing the previous one.
    pub(crate) fn step(&self, model: &'static str, backend: Backend, step: PrepStep) {
        let now = std::time::Instant::now();
        let usual = self.usual(model, backend, step);
        let finished = {
            let mut status = self.status.lock().unwrap();
            let Some(entry) = status.get_mut(model).filter(|s| s.preparing) else {
                return;
            };
            let previous = entry.prep;
            let finished = previous.map(|prep| (entry.backend, prep));
            entry.backend = backend;
            entry.prep = Some(Prep {
                step,
                started: previous.map_or(now, |p| p.started),
                step_started: now,
                usual,
            });
            finished
        };
        if let Some((backend, prep)) = finished {
            self.finish_step(model, backend, prep);
        }
    }
    #[cfg(feature = "onnx")]
    pub(crate) fn report(&self, model: &'static str, backend: Backend, detail: String) {
        let finished = self
            .status
            .lock()
            .unwrap()
            .get(model)
            .and_then(|s| Some((s.backend, s.prep?)));
        let total = finished.map(|(_, prep)| prep.started.elapsed().as_millis() as u64);
        if let Some((backend, prep)) = finished
            && !detail.starts_with("Initialization stopped")
        {
            self.finish_step(model, backend, prep);
        }
        tracing::info!(model, ?backend, %detail, total_millis = total, "model preparation finished");
        self.status.lock().unwrap().insert(
            model,
            ModelStatus {
                model,
                backend,
                detail,
                preparing: false,
                prep: None,
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
