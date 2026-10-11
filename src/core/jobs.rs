use super::{
    analysis::{Analysis, Thresholds},
    catalog::Catalog,
    model::*,
    operations::{self, Plan, Report},
    pipeline::Stats,
    session::Session,
};
use image::RgbaImage;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

#[path = "jobs/diagnostics.rs"]
mod diagnostics;
mod files;
mod images;
mod similarity;
mod subjects;

use similarity::SimilarityJob;
use subjects::SubjectJob;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AnalysisStats {
    pub decodes: u64,
    pub quality_measurements: u64,
    pub technical_measurements: u64,
    pub content_hashes: u64,
    pub model_initializations: usize,
    pub model_inferences: u64,
    pub model_threads: usize,
}
pub enum Event {
    ModelsNeeded {
        run: u64,
        issue: super::model_assets::Issue,
    },
    GroupsEdited(super::manual_groups::Resolved),
    CacheStats(Stats),
    Batch(Vec<Media>),
    Opened(Arc<Catalog>, Session, super::views::Saved),
    Asset {
        id: MediaId,
        preview: bool,
        decoder_warning: Option<String>,
        pixels: Arc<RgbaImage>,
        metadata: Metadata,
        stats: Stats,
    },
    MetadataBatch {
        run: u64,
        items: Vec<(MediaId, Result<Metadata, String>)>,
        done: usize,
        total: usize,
        stats: Stats,
    },
    Inspected {
        request: super::inspection::Request,
        result: Result<Arc<RgbaImage>, String>,
        decoder_warning: Option<String>,
        stats: Stats,
    },
    AnalysisError {
        run: u64,
        id: MediaId,
        error: String,
    },
    AssetError {
        id: MediaId,
        preview: bool,
        error: String,
    },
    Analyzed {
        run: u64,
        id: MediaId,
        analysis: Analysis,
        embedding: Option<Arc<Vec<f32>>>,
        group: usize,
        duplicate: Option<MediaId>,
        metadata: Metadata,
    },
    Progress {
        run: u64,
        done: usize,
        total: usize,
    },
    MixedGroups {
        run: u64,
        labels: HashMap<MediaId, String>,
    },
    GeneralStarted {
        run: u64,
    },
    GeneralGroups {
        run: u64,
        automatic: Arc<HashMap<MediaId, usize>>,
        choices: Vec<(usize, usize, MediaId)>,
        groups: HashMap<MediaId, usize>,
    },
    DuplicateStarted {
        run: u64,
    },
    DuplicatePairs {
        run: u64,
        pairs: Vec<super::near_duplicates::batch::Pair>,
    },
    SubjectStarted {
        run: u64,
    },
    FaceGroups {
        run: u64,
        groups: super::regional::grouping::Output,
    },
    SubjectGroups {
        run: u64,
        groups: HashMap<MediaId, usize>,
    },
    AnalysisDone {
        run: u64,
        stats: AnalysisStats,
    },
    Rotation {
        run: u64,
        id: MediaId,
        suggestion: super::models::RotationSuggestion,
    },
    Planned(Plan),
    PlanningCancelled,
    Applied(Report),
    OperationsRestored(operations::recovery::Restored),
    OperationsDiscarded,
    OperationsAmended(Result<operations::recovery::Restored, String>),
    ApplyFailed(String),
    Saved,
    Checkpointed(Result<(), String>),
    ViewError(String),
    PreferencesError(String),
    Error(String),
    /// A task panicked outside per-item work; the named worker carries on.
    WorkerStopped(Worker),
}
pub struct Envelope {
    pub epoch: u64,
    pub event: Event,
}
pub enum IoWork {
    EditGroups {
        scene: bool,
        cancelled: Arc<AtomicBool>,
        run: u64,
        catalog: Arc<Catalog>,
        automatic: Arc<HashMap<MediaId, usize>>,
        manual: Arc<super::manual_groups::ManualGroups>,
        edit: super::manual_groups::Edit,
    },
    SavePreferences {
        path: PathBuf,
        preferences: super::preferences::Preferences,
    },
    Checkpoint {
        root: PathBuf,
        session: Session,
        views: super::views::Saved,
    },
    SaveViews {
        root: PathBuf,
        views: super::views::Saved,
    },
    Scan(PathBuf),
    PlanTrash {
        catalog: Arc<Catalog>,
        request: operations::TrashRequest,
        companions: bool,
        cancelled: Arc<AtomicBool>,
    },
    PlanOrganize {
        cancelled: Arc<AtomicBool>,
        directories: super::organize::DirectoryEdits,
        catalog: Arc<Catalog>,
        labels: HashMap<MediaId, String>,
        names: HashMap<MediaId, String>,
        output: PathBuf,
        companions: bool,
    },
    Apply(Plan),
    ApplyRecoverable {
        context: Box<operations::recovery::Context>,
        plan: Plan,
        ticket: Option<operations::recovery::Ticket>,
        stop_requested: Arc<AtomicBool>,
    },
    AmendRecovery {
        root: PathBuf,
        ticket: operations::recovery::Ticket,
        group: usize,
        folder: PathBuf,
        stem: Option<String>,
        cancelled: Arc<AtomicBool>,
    },
    DiscardRecovery {
        root: PathBuf,
        ticket: operations::recovery::Ticket,
    },
    MergeSession {
        root: PathBuf,
        session: Session,
    },
    Save {
        root: PathBuf,
        session: Session,
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnalysisProfile {
    Technical,
    Duplicates,
    Subjects,
    Visual,
    Mixed,
    Quality,
    Faces,
}
pub enum ImageWork {
    PlanDecisions {
        catalog: Arc<Catalog>,
        decisions: HashMap<MediaId, Decision>,
        companions: bool,
        policy: super::metadata::WritePolicy,
    },

    IndexMetadata {
        run: u64,
        media: Arc<Vec<Media>>,
    },
    Asset(Media, bool),
    Analyze {
        run: u64,
        media: Arc<Vec<Media>>,
        profile: AnalysisProfile,
        thresholds: Thresholds,
        strictness: super::subjects::Strictness,
        similarity_distance: f32,
        scene_groups: Arc<super::manual_groups::ManualGroups>,
        model_config: super::processing::RunConfig,
    },
    Orient {
        run: u64,
        media: Arc<Vec<Media>>,
        model: PathBuf,
        model_config: super::processing::RunConfig,
    },
    Invalidate,
}
/// Admission and cancellation share a short gate. Running work only reads the
/// atomic; no gate is held during decoding, model setup or result publication.
#[derive(Default)]
struct RunGeneration {
    current: AtomicU64,
    admission: Mutex<()>,
}
impl RunGeneration {
    fn current(&self) -> u64 {
        self.current.load(Ordering::Acquire)
    }
    fn submit(&self, send: impl FnOnce(u64) -> Result<(), String>) -> Result<u64, String> {
        let _admission = self.admission.lock().unwrap();
        let run = self.current().wrapping_add(1);
        // A rejected request must leave the accepted job running. Rollback after
        // publishing is unsafe: workers may already have observed cancellation.
        send(run)?;
        self.current.store(run, Ordering::Release);
        Ok(run)
    }
    fn accepts(&self, run: u64) -> bool {
        // A receiver can obtain the message before submit publishes its run.
        // Synchronize once at receipt, then keep hot cancellation reads lock-free.
        let _admission = self.admission.lock().unwrap();
        self.current() == run
    }
    fn cancel(&self) {
        let _admission = self.admission.lock().unwrap();
        self.current.fetch_add(1, Ordering::AcqRel);
    }
}
impl ImageWork {
    fn is_current(&self, analysis: &RunGeneration, metadata: &RunGeneration) -> bool {
        match self {
            Self::Analyze { run, .. } | Self::Orient { run, .. } => analysis.accepts(*run),
            Self::IndexMetadata { run, .. } => metadata.accepts(*run),
            _ => true,
        }
    }
}

struct Message<T> {
    epoch: u64,
    work: T,
}
#[derive(Clone)]
struct Publisher {
    tx: SyncSender<Envelope>,
    generation: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
}
/// The error a worker publishes when one of its tasks panicked. The UI resets
/// its busy state on it; the crash notice explains what happened.
pub const WORKER_STOPPED: &str =
    "A background task stopped unexpectedly. Try again; restart PhotoSort if it keeps happening.";

/// The long-lived background workers, so the UI resets only what each owns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Worker {
    Files,
    Images,
    Similarity,
    Subjects,
}

impl Publisher {
    /// Runs one unit of worker work. A panic becomes an error for the current
    /// folder instead of ending the worker thread and leaving the UI waiting.
    /// Returns false when the work panicked.
    fn guard(&self, worker: Worker, work: impl FnOnce()) -> bool {
        let ok = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_ok();
        if !ok {
            let epoch = self.generation.load(Ordering::Acquire);
            self.send(epoch, Event::WorkerStopped(worker));
        }
        ok
    }
    fn cancelled(&self, epoch: u64) -> bool {
        self.stop.load(Ordering::Relaxed) || self.generation.load(Ordering::Acquire) != epoch
    }
    fn send(&self, epoch: u64, event: Event) {
        self.send_checked(epoch, event, || false);
    }
    fn send_checked(&self, epoch: u64, event: Event, cancelled: impl Fn() -> bool) {
        // Record the worker outcome once, independently of publication retries.
        // This describes completed work, not acceptance by the current UI.
        event.record_diagnostic(epoch);
        let mut envelope = Envelope { epoch, event };
        loop {
            if self.cancelled(epoch) || cancelled() {
                return;
            }
            match self.tx.try_send(envelope) {
                Ok(()) => {
                    // Publish before waking: the UI may drain the queue as soon
                    // as the callback runs, without another scheduled frame.
                    (self.wake)();
                    return;
                }
                Err(TrySendError::Disconnected(_)) => return,
                Err(TrySendError::Full(e)) => {
                    envelope = e;
                    // Existing results need draining before this one can fit.
                    (self.wake)();
                    thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }
}
pub struct Runtime {
    pub playback: super::playback::Player,
    pub updates: super::updates::Worker,
    pub desktop: super::desktop::Worker,
    pub model_assets: super::model_assets::Worker,
    pub processing: super::processing::Control,
    pub cache: super::cache::Control,
    pub browsing: super::browsing::Worker,
    pub events: Receiver<Envelope>,
    io: SyncSender<Message<IoWork>>,
    images: SyncSender<Message<ImageWork>>,
    priority: SyncSender<Message<ImageWork>>,
    generation: Arc<AtomicU64>,
    analysis: Arc<RunGeneration>,
    metadata_generation: Arc<RunGeneration>,
    inspection: Arc<Mutex<super::inspection::Queue>>,
    stop: Arc<AtomicBool>,
    threads: Vec<thread::JoinHandle<()>>,
}
impl Runtime {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Self {
        Self::with_cache(wake, super::session::cache_dir())
    }
    pub fn with_cache(wake: impl Fn() + Send + Sync + 'static, cache: PathBuf) -> Self {
        tracing::debug!("shared runtime starting");
        let (tx, events) = mpsc::sync_channel(8);
        let (io, io_rx) = mpsc::sync_channel::<Message<IoWork>>(8);
        let (images, image_rx) = mpsc::sync_channel::<Message<ImageWork>>(48);
        let (priority, priority_rx) = mpsc::sync_channel::<Message<ImageWork>>(4);
        let generation = Arc::new(AtomicU64::new(0));
        let analysis = Arc::new(RunGeneration::default());
        let metadata_generation = Arc::new(RunGeneration::default());
        let stop = Arc::new(AtomicBool::new(false));
        let publisher = Publisher {
            tx,
            generation: generation.clone(),
            stop: stop.clone(),
            wake: Arc::new(wake),
        };
        let updates = super::updates::Worker::new(publisher.wake.clone());
        let desktop = super::desktop::Worker::new(generation.clone(), publisher.wake.clone());
        let playback = super::playback::Player::new(publisher.wake.clone());
        let browsing = super::browsing::Worker::new(publisher.wake.clone());
        let model_assets = super::model_assets::Worker::new(
            super::models::resource_roots(),
            publisher.wake.clone(),
        );
        let io_thread = files::spawn(publisher.clone(), io_rx);
        let cache_control = super::cache::Control::with_directory(cache.clone());
        let (duplicate_tx, duplicate_rx) = mpsc::sync_channel::<SimilarityJob>(1);
        let (evidence_tx, evidence_rx) = mpsc::sync_channel::<similarity::EvidenceRequest>(1);
        let duplicate_thread = similarity::spawn(
            publisher.clone(),
            duplicate_rx,
            evidence_tx,
            analysis.clone(),
            cache_control.clone(),
        );
        let (subject_tx, subject_rx) = mpsc::sync_channel::<SubjectJob>(1);
        let subject_thread = subjects::spawn(
            publisher.clone(),
            subject_rx,
            analysis.clone(),
            cache_control.clone(),
        );
        let processing = super::processing::Control::default();
        let inspection = Arc::new(Mutex::new(super::inspection::Queue::default()));
        let image_thread = images::spawn(images::Wiring {
            publisher,
            cache,
            cache_control: cache_control.clone(),
            processing: processing.clone(),
            inspection: inspection.clone(),
            analysis: analysis.clone(),
            metadata: metadata_generation.clone(),
            images: image_rx,
            priority: priority_rx,
            evidence: evidence_rx,
            similarity: duplicate_tx,
            subjects: subject_tx,
        });
        Self {
            desktop,
            updates,
            playback,
            model_assets,
            cache: cache_control,
            processing,
            browsing,
            events,
            io,
            images,
            priority,
            generation,
            analysis,
            metadata_generation,
            inspection,
            stop,
            threads: vec![io_thread, image_thread, subject_thread, duplicate_thread],
        }
    }
    pub fn inspect(&self, requests: Vec<super::inspection::Request>) {
        self.inspection
            .lock()
            .unwrap()
            .update(self.epoch(), requests);
    }
    pub fn epoch(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }
    pub fn next_epoch(&self) -> u64 {
        self.playback.select(None);
        self.browsing.reset(None);
        self.cancel_analysis();
        self.cancel_metadata();
        let epoch = self.generation.fetch_add(1, Ordering::AcqRel) + 1;
        tracing::debug!(epoch, "folder generation changed");
        self.inspection.lock().unwrap().update(epoch, Vec::new());
        epoch
    }
    pub fn index_metadata(&self, media: Arc<Vec<Media>>) -> Result<u64, String> {
        self.metadata_generation
            .submit(|run| self.image(ImageWork::IndexMetadata { run, media }))
    }
    pub fn cancel_metadata(&self) {
        self.metadata_generation.cancel();
    }
    pub fn orient(&self, media: Arc<Vec<Media>>, model: PathBuf) -> Result<u64, String> {
        self.analysis.submit(|run| {
            tracing::info!(
                epoch = self.epoch(),
                run,
                photos = media.len(),
                "orientation requested"
            );
            self.image(ImageWork::Orient {
                run,
                media,
                model,
                model_config: self.processing.snapshot_run(),
            })
        })
    }
    pub fn shutdown(&mut self) {
        tracing::debug!("shared runtime shutdown requested");
        self.desktop.shutdown();
        self.updates.shutdown();
        self.playback.shutdown();
        self.model_assets.shutdown();
        self.browsing.shutdown();
        self.stop.store(true, Ordering::Release);
        self.cancel_analysis();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
        tracing::debug!("shared runtime shutdown finished");
    }
    pub fn cancel_analysis(&self) {
        tracing::debug!(
            epoch = self.epoch(),
            run = self.analysis.current(),
            "analysis cancellation requested"
        );
        self.analysis.cancel();
    }
    pub fn analyze(&self, media: Arc<Vec<Media>>, profile: AnalysisProfile) -> Result<u64, String> {
        self.analyze_with_thresholds(media, profile, Thresholds::default())
    }
    pub fn analyze_with_thresholds(
        &self,
        media: Arc<Vec<Media>>,
        profile: AnalysisProfile,
        thresholds: Thresholds,
    ) -> Result<u64, String> {
        self.analyze_with_options(
            media,
            profile,
            thresholds,
            Default::default(),
            super::regional::DEFAULT_EPS,
        )
    }
    pub fn analyze_with_options(
        &self,
        media: Arc<Vec<Media>>,
        profile: AnalysisProfile,
        thresholds: Thresholds,
        strictness: super::subjects::Strictness,
        similarity_distance: f32,
    ) -> Result<u64, String> {
        self.analyze_with_scene_groups(
            media,
            profile,
            thresholds,
            strictness,
            similarity_distance,
            Arc::default(),
        )
    }
    pub fn analyze_with_scene_groups(
        &self,
        media: Arc<Vec<Media>>,
        profile: AnalysisProfile,
        mut thresholds: Thresholds,
        strictness: super::subjects::Strictness,
        similarity_distance: f32,
        scene_groups: Arc<super::manual_groups::ManualGroups>,
    ) -> Result<u64, String> {
        thresholds.normalize();
        self.analysis.submit(|run| {
            tracing::info!(
                epoch = self.epoch(),
                run,
                ?profile,
                photos = media.len(),
                "analysis requested"
            );
            self.image(ImageWork::Analyze {
                run,
                media,
                profile,
                thresholds,
                strictness,
                similarity_distance: super::regional::normalize_distance(similarity_distance),
                scene_groups,
                model_config: self.processing.snapshot_run(),
            })
        })
    }
    pub fn io(&self, work: IoWork) -> Result<(), String> {
        self.io
            .try_send(Message {
                epoch: self.epoch(),
                work,
            })
            .map_err(|_| "File worker is busy; try again shortly".into())
    }
    pub fn image(&self, work: ImageWork) -> Result<(), String> {
        let queue = if matches!(work, ImageWork::Asset(_, true)) {
            &self.priority
        } else {
            &self.images
        };
        queue
            .try_send(Message {
                epoch: self.epoch(),
                work,
            })
            .map_err(|_| "Image queue is full".into())
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
#[path = "jobs/admission_tests.rs"]
mod admission_tests;

#[cfg(test)]
#[path = "jobs/publication_tests.rs"]
mod publication_tests;
