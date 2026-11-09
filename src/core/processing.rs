//! Shared inference settings and session status, captured when a run is accepted.
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
};
/// Auto selects a provider for this platform and falls back to CPU per model.
/// New installs use Auto; a saved choice, CPU included, is kept as it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Backend {
    Cpu,
    #[default]
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
    /// Whether this backend can run on this platform at all, so Settings only
    /// offers what could work here (Core ML is Apple's, DirectML is Windows').
    pub fn on_this_platform(self) -> bool {
        match self {
            Self::Cpu | Self::Auto => true,
            Self::CoreMl => cfg!(target_os = "macos"),
            Self::Cuda => cfg!(any(target_os = "windows", target_os = "linux")),
            Self::DirectMl => cfg!(target_os = "windows"),
        }
    }
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
/// Where the prepared models run, in the words the UI shows. Each model
/// resolves its own provider, so an accelerator and the CPU can share a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The setting the models were prepared under.
    pub requested: Backend,
    /// Each accelerator a ready model runs on, in model order, once each.
    pub accelerators: Vec<Backend>,
    /// Ready models that run on the CPU.
    pub on_cpu: usize,
}
impl Placement {
    /// From the status of each model; `None` until a model is ready.
    /// Models still preparing or whose preparation stopped are left out.
    pub fn of(requested: Backend, status: &[ModelStatus]) -> Option<Self> {
        let ready: Vec<_> = status
            .iter()
            .filter(|s| !s.preparing && !s.detail.starts_with("Initialization stopped"))
            .collect();
        if ready.is_empty() {
            return None;
        }
        let mut accelerators = Vec::new();
        for status in &ready {
            if status.backend != Backend::Cpu && !accelerators.contains(&status.backend) {
                accelerators.push(status.backend);
            }
        }
        Some(Self {
            requested,
            accelerators,
            on_cpu: ready.iter().filter(|s| s.backend == Backend::Cpu).count(),
        })
    }
    /// "Running on NVIDIA CUDA", "Running on CPU", or the mix with how
    /// many models fell back to the CPU.
    pub fn label(&self) -> String {
        let names = self
            .accelerators
            .iter()
            .map(|b| b.label())
            .collect::<Vec<_>>()
            .join(" and ");
        match (self.accelerators.is_empty(), self.on_cpu) {
            (true, _) if self.requested == Backend::Cpu => "Running on CPU".into(),
            (true, _) if self.requested == Backend::Auto => {
                "Running on CPU · no accelerator available".into()
            }
            (true, _) => format!("Running on CPU · {} unavailable", self.requested.label()),
            (false, 0) => format!("Running on {names}"),
            (false, 1) => format!("Running on {names} · 1 model on CPU"),
            (false, n) => format!("Running on {names} · {n} models on CPU"),
        }
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
    /// The backend the models in `status` were prepared under, which stays
    /// the previous setting until the next analysis starts.
    prepared_for: Arc<Mutex<Option<Backend>>>,
    status: Arc<Mutex<BTreeMap<&'static str, ModelStatus>>>,
    /// Last finished duration of each slow step, keyed by model, backend and step.
    timings: Arc<Mutex<Timings>>,
    available: usize,
}
/// Step durations from earlier preparations, kept in a small file so the
/// next launch can say how long a step usually takes.
#[derive(Default)]
struct Timings {
    path: Option<std::path::PathBuf>,
    millis: BTreeMap<String, u64>,
}
impl Timings {
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
            prepared_for: Default::default(),
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
    /// The backend the shown model status belongs to; the requested one
    /// until a run has configured the models.
    pub fn prepared_for(&self) -> Backend {
        self.prepared_for
            .lock()
            .unwrap()
            .unwrap_or_else(|| self.snapshot_run().backend)
    }
    /// Records the backend a run configured the models with.
    pub(crate) fn set_prepared_for(&self, backend: Backend) {
        *self.prepared_for.lock().unwrap() = Some(backend);
    }
    /// Where the ready models run now; `None` before any model is ready.
    pub fn placement(&self) -> Option<Placement> {
        Placement::of(self.prepared_for(), &self.status())
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
    fn usual(&self, model: &str, backend: Backend, step: PrepStep) -> Option<std::time::Duration> {
        let timings = self.timings.lock().unwrap();
        let millis = timings.millis.get(&Timings::key(model, backend, step))?;
        Some(std::time::Duration::from_millis(*millis))
    }
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

#[cfg(test)]
mod platform_tests {
    use super::Backend;
    #[test]
    fn settings_offer_only_backends_that_can_run_here() {
        assert!(Backend::Cpu.on_this_platform() && Backend::Auto.on_this_platform());
        assert_eq!(
            Backend::CoreMl.on_this_platform(),
            cfg!(target_os = "macos")
        );
        assert_eq!(
            Backend::DirectMl.on_this_platform(),
            cfg!(target_os = "windows")
        );
        assert_eq!(
            Backend::Cuda.on_this_platform(),
            cfg!(any(target_os = "windows", target_os = "linux"))
        );
        // Whatever Automatic may try is always offered on its own platform.
        for candidate in Backend::Auto.candidates() {
            assert!(candidate.on_this_platform(), "{candidate:?}");
        }
    }
}

#[cfg(test)]
mod placement_tests {
    use super::*;
    fn ready(model: &'static str, backend: Backend) -> ModelStatus {
        ModelStatus {
            model,
            backend,
            detail: "Provider enabled".into(),
            preparing: false,
            prep: None,
        }
    }
    #[test]
    fn new_installs_run_ai_on_automatic_and_a_saved_cpu_choice_stays() {
        assert_eq!(Preferences::default().backend, Backend::Auto);
        assert_eq!(Control::default().snapshot_run().backend, Backend::Auto);
        let missing: Preferences = serde_json::from_str("{}").unwrap();
        assert_eq!(missing.backend, Backend::Auto);
        let saved: Preferences = serde_json::from_str(r#"{"backend":"Cpu"}"#).unwrap();
        assert_eq!(saved.backend, Backend::Cpu);
    }
    #[test]
    fn placement_names_the_resolved_backend_not_the_setting() {
        assert_eq!(Placement::of(Backend::Auto, &[]), None);
        let mut preparing = ready("dinov2", Backend::Cuda);
        preparing.preparing = true;
        let mut stopped = ready("sface", Backend::Cuda);
        stopped.detail = "Initialization stopped: cancelled".into();
        assert_eq!(
            Placement::of(Backend::Auto, &[preparing.clone(), stopped]),
            None
        );
        let label =
            |requested, status: &[ModelStatus]| Placement::of(requested, status).unwrap().label();
        let cuda = [
            ready("dinov2", Backend::Cuda),
            ready("aesthetic", Backend::Cuda),
        ];
        assert_eq!(label(Backend::Auto, &cuda), "Running on NVIDIA CUDA");
        assert_eq!(
            label(
                Backend::Auto,
                &[ready("dinov2", Backend::DirectMl), preparing]
            ),
            "Running on Windows DirectML"
        );
        // Automatic with every accelerator failing reports the CPU fallback.
        let cpu = [
            ready("dinov2", Backend::Cpu),
            ready("aesthetic", Backend::Cpu),
        ];
        assert_eq!(
            label(Backend::Auto, &cpu),
            "Running on CPU · no accelerator available"
        );
        assert_eq!(
            label(Backend::Cuda, &cpu),
            "Running on CPU · NVIDIA CUDA unavailable"
        );
        assert_eq!(label(Backend::Cpu, &cpu), "Running on CPU");
        let mixed = [
            ready("dinov2", Backend::Cuda),
            ready("aesthetic", Backend::Cpu),
            ready("face_detector", Backend::DirectMl),
            ready("sface", Backend::Cuda),
        ];
        assert_eq!(
            label(Backend::Auto, &mixed),
            "Running on NVIDIA CUDA and Windows DirectML · 1 model on CPU"
        );
        assert_eq!(
            label(
                Backend::Auto,
                &[
                    ready("dinov2", Backend::Cuda),
                    cpu[0].clone(),
                    cpu[1].clone()
                ]
            ),
            "Running on NVIDIA CUDA · 2 models on CPU"
        );
    }
    #[test]
    fn control_reports_placement_for_the_backend_the_models_were_prepared_under() {
        let control = Control::default();
        assert_eq!(control.placement(), None);
        // Before a run, the shown status belongs to the requested backend.
        control.configure(Preferences {
            backend: Backend::Cuda,
            ..Default::default()
        });
        assert_eq!(control.prepared_for(), Backend::Cuda);
        control.report(
            "dinov2",
            Backend::Cpu,
            "CPU fallback · NVIDIA CUDA: missing".into(),
        );
        assert_eq!(
            control.placement().unwrap().label(),
            "Running on CPU · NVIDIA CUDA unavailable"
        );
        // A run under Automatic, then a new setting applied before the next run:
        // the status still describes the run that prepared the models.
        control.set_prepared_for(Backend::Auto);
        control.configure(Preferences {
            backend: Backend::Cpu,
            ..Default::default()
        });
        assert_eq!(control.prepared_for(), Backend::Auto);
        assert_eq!(
            control.placement().unwrap().label(),
            "Running on CPU · no accelerator available"
        );
    }
}
