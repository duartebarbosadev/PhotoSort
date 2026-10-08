use super::{
    analysis::{self, Analysis, Thresholds, VisualGroups},
    catalog::Catalog,
    model::*,
    operations::{self, Plan, Report},
    pipeline::{Pipeline, Stats},
    session::Session,
};
use anyhow::Context;
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

/// All analysis profiles, including rotation, share the same evidence owner.
fn prepare_evidence_store(
    store: &mut Option<Arc<super::subjects::store::Store>>,
    root: &std::path::Path,
    persistent: bool,
    reset: &mut bool,
) -> anyhow::Result<()> {
    if *reset {
        *store = None;
    }
    if store.is_none() {
        *store = Some(Arc::new(super::subjects::store::Store::new(
            root.into(),
            persistent,
        )?));
    }
    *reset = false;
    Ok(())
}

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
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AnalysisProfile {
    Technical,
    Duplicates,
    Subjects,
    Visual,
    Mixed,
    Quality,
    Faces,
}
struct CachedAnalysis {
    technical: Analysis,
    hash: String,
    dimensions: (u32, u32),
    embedding: Option<Vec<f32>>,
    aesthetic: Option<f32>,
    faces: Option<super::faces::FaceSummary>,
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
impl Publisher {
    fn cancelled(&self, epoch: u64) -> bool {
        self.stop.load(Ordering::Relaxed) || self.generation.load(Ordering::Acquire) != epoch
    }
    fn send(&self, epoch: u64, event: Event) {
        self.send_checked(epoch, event, || false);
    }
    fn send_checked(&self, epoch: u64, event: Event, cancelled: impl Fn() -> bool) {
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
struct SimilarityJob {
    scene_groups: Arc<super::manual_groups::ManualGroups>,
    similarity_distance: f32,
    regional: Vec<super::regional::grouping::Record>,
    sources: Vec<(Media, String)>,
    profile: AnalysisProfile,
    epoch: u64,
    run: u64,
    inputs: Vec<super::near_duplicates::batch::Input>,
    store: Arc<super::subjects::store::Store>,
    stats: AnalysisStats,
}
type EvidenceReply = (
    super::pipeline::Asset,
    Option<Vec<super::faces::Face>>,
    AnalysisStats,
);
struct EvidenceRequest {
    epoch: u64,
    run: u64,
    input: super::near_duplicates::batch::Input,
    store: Arc<super::subjects::store::Store>,
    faces: bool,
    reply: SyncSender<anyhow::Result<EvidenceReply>>,
}
struct EvidenceClient<'a> {
    job: &'a SimilarityJob,
    requests: &'a SyncSender<EvidenceRequest>,
    cancelled: &'a dyn Fn() -> bool,
    stats: AnalysisStats,
}
impl EvidenceClient<'_> {
    fn request(
        &mut self,
        input: &super::near_duplicates::batch::Input,
        faces: bool,
    ) -> anyhow::Result<(super::pipeline::Asset, Option<Vec<super::faces::Face>>)> {
        let (reply, rx) = mpsc::sync_channel(1);
        let mut request = EvidenceRequest {
            epoch: self.job.epoch,
            run: self.job.run,
            input: input.clone(),
            store: self.job.store.clone(),
            faces,
            reply,
        };
        loop {
            anyhow::ensure!(!(self.cancelled)(), "Duplicate review cancelled");
            match self.requests.try_send(request) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) => {
                    request = returned;
                    thread::sleep(Duration::from_millis(2));
                }
                Err(TrySendError::Disconnected(_)) => anyhow::bail!("Image worker stopped"),
            }
        }
        loop {
            anyhow::ensure!(!(self.cancelled)(), "Duplicate review cancelled");
            match rx.recv_timeout(Duration::from_millis(15)) {
                Ok(result) => {
                    let (asset, faces, stats) = result?;
                    self.stats = stats;
                    return Ok((asset, faces));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => anyhow::bail!("Image worker stopped"),
            }
        }
    }
}
impl super::near_duplicates::batch::Evidence for EvidenceClient<'_> {
    fn image(
        &mut self,
        input: &super::near_duplicates::batch::Input,
    ) -> anyhow::Result<super::pipeline::Asset> {
        self.request(input, false).map(|(asset, _)| asset)
    }
    fn faces(
        &mut self,
        input: &super::near_duplicates::batch::Input,
    ) -> Option<Vec<super::faces::Face>> {
        self.request(input, true).ok().and_then(|(_, faces)| faces)
    }
}
struct SubjectJob {
    epoch: u64,
    run: u64,
    inputs: Vec<super::subjects::grouping::Input>,
    store: Arc<super::subjects::store::Store>,
    strictness: super::subjects::Strictness,
    stats: AnalysisStats,
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
        let io_pub = publisher.clone();
        let io_thread = thread::Builder::new()
            .name("photosort-files".into())
            .spawn(move || {
                loop {
                    // Explicit-root saves own already accepted review decisions.
                    // Drain those on shutdown even if a scan was interrupted.
                    let stopping = io_pub.stop.load(Ordering::Acquire);
                    let message = if stopping {
                        let Ok(message) = io_rx.try_recv() else {
                            break;
                        };
                        message
                    } else {
                        let Ok(message) = io_rx.recv_timeout(Duration::from_millis(100)) else {
                            continue;
                        };
                        message
                    };
                    let epoch = message.epoch;
                    // A queued session save belongs to its explicit root and is safe after navigation.
                    if io_pub.cancelled(epoch)
                        && !matches!(
                            message.work,
                            IoWork::Save { .. }
                                | IoWork::MergeSession { .. }
                                | IoWork::Checkpoint { .. }
                                | IoWork::SaveViews { .. }
                                | IoWork::SavePreferences { .. }
                        )
                    {
                        continue;
                    }
                    let result: anyhow::Result<()> = (|| {
                        match message.work {
                            IoWork::EditGroups {
                                scene,
                                cancelled,
                                run,
                                catalog,
                                automatic,
                                manual,
                                edit,
                            } => {
                                let resolved = super::manual_groups::edit(
                                    run,
                                    automatic,
                                    manual,
                                    &catalog.media,
                                    edit,
                                    || io_pub.cancelled(epoch) || cancelled.load(Ordering::Acquire),
                                );
                                if !cancelled.load(Ordering::Acquire) && !io_pub.cancelled(epoch) {
                                    let mut resolved = resolved?;
                                    if scene {
                                        resolved
                                            .groups
                                            .retain(|id, _| resolved.automatic.contains_key(id));
                                        resolved.choices = super::manual_groups::group_choices(
                                            &resolved.groups,
                                            || {
                                                io_pub.cancelled(epoch)
                                                    || cancelled.load(Ordering::Acquire)
                                            },
                                        )?;
                                    }
                                    io_pub.send(epoch, Event::GroupsEdited(resolved));
                                }
                            }
                            IoWork::Scan(root) => {
                                let root = std::fs::canonicalize(root)?;
                                let restored = operations::recovery::Store::standard()
                                    .restore(&root, &|| io_pub.cancelled(epoch))?;
                                let catalog = Catalog::scan(
                                    &root,
                                    || io_pub.cancelled(epoch),
                                    |batch| io_pub.send(epoch, Event::Batch(batch)),
                                )?;
                                let mut session = Session::load(&catalog.root)?;
                                session.reconcile_catalog(&catalog);
                                let mut views = super::views::Saved::load(&catalog.root)
                                    .unwrap_or_else(|e| {
                                        io_pub.send(
                                            epoch,
                                            Event::ViewError(format!(
                                                "Saved view could not be restored: {e}"
                                            )),
                                        );
                                        Default::default()
                                    });
                                views.reconcile_catalog(&catalog);
                                if let Some(restored) = restored {
                                    io_pub.send(epoch, Event::OperationsRestored(restored));
                                }
                                io_pub
                                    .send(epoch, Event::Opened(Arc::new(catalog), session, views));
                            }
                            IoWork::PlanTrash {
                                catalog,
                                request,
                                companions,
                                cancelled,
                            } => {
                                let result =
                                    operations::trash_plan(&catalog, &request, companions, || {
                                        io_pub.cancelled(epoch) || cancelled.load(Ordering::Acquire)
                                    });
                                if cancelled.load(Ordering::Acquire) {
                                    io_pub.send(epoch, Event::PlanningCancelled);
                                } else {
                                    io_pub.send(epoch, Event::Planned(result?));
                                }
                            }
                            IoWork::PlanOrganize {
                                cancelled,
                                directories,
                                catalog,
                                labels,
                                names,
                                output,
                                companions,
                            } => {
                                let result = operations::organize_plan_with_layout_cancellable(
                                    &catalog,
                                    &labels,
                                    &names,
                                    &output,
                                    companions,
                                    &directories,
                                    || io_pub.cancelled(epoch) || cancelled.load(Ordering::Acquire),
                                );
                                if cancelled.load(Ordering::Acquire) {
                                    io_pub.send(epoch, Event::PlanningCancelled);
                                } else {
                                    io_pub.send(epoch, Event::Planned(result?));
                                }
                            }
                            IoWork::ApplyRecoverable {
                                context,
                                plan,
                                ticket,
                                stop_requested,
                            } => {
                                let store = operations::recovery::Store::standard();
                                let result = store.execute_stoppable(
                                    &context,
                                    &plan,
                                    ticket.as_ref(),
                                    || io_pub.cancelled(epoch),
                                    || stop_requested.load(Ordering::Acquire),
                                    |done, total| {
                                        io_pub.send(
                                            epoch,
                                            Event::Progress {
                                                run: 0,
                                                done,
                                                total,
                                            },
                                        )
                                    },
                                );
                                match result {
                                    Ok(report) => io_pub.send(epoch, Event::Applied(report)),
                                    Err(error)
                                        if store.has_pending(&context.root) || ticket.is_some() =>
                                    {
                                        io_pub.send(epoch, Event::ApplyFailed(format!("{error:#}")))
                                    }
                                    Err(error) => return Err(error),
                                }
                            }
                            IoWork::AmendRecovery {
                                root,
                                ticket,
                                group,
                                folder,
                                stem,
                                cancelled,
                            } => {
                                let result = operations::recovery::Store::standard()
                                    .amend_destination(
                                        &root,
                                        &ticket,
                                        group,
                                        &folder,
                                        stem.as_deref(),
                                        &|| {
                                            io_pub.cancelled(epoch)
                                                || cancelled.load(Ordering::Acquire)
                                        },
                                    )
                                    .map_err(|e| format!("{e:#}"));
                                io_pub.send(epoch, Event::OperationsAmended(result));
                            }
                            IoWork::DiscardRecovery { root, ticket } => {
                                match operations::recovery::Store::standard()
                                    .discard(&root, &ticket)
                                {
                                    Ok(()) => io_pub.send(epoch, Event::OperationsDiscarded),
                                    Err(error) => {
                                        io_pub.send(epoch, Event::ApplyFailed(format!("{error:#}")))
                                    }
                                }
                            }
                            IoWork::Apply(plan) => {
                                let report = operations::execute(
                                    &plan,
                                    &operations::journal_dir(),
                                    || io_pub.cancelled(epoch),
                                    |done, total| {
                                        io_pub.send(
                                            epoch,
                                            Event::Progress {
                                                run: 0,
                                                done,
                                                total,
                                            },
                                        )
                                    },
                                )?;
                                io_pub.send(epoch, Event::Applied(report));
                            }
                            IoWork::SavePreferences { path, preferences } => {
                                if let Err(e) = preferences.save(&path) {
                                    io_pub.send(
                                        epoch,
                                        Event::PreferencesError(format!(
                                            "Preferences could not be saved: {e}"
                                        )),
                                    );
                                }
                            }
                            IoWork::Checkpoint {
                                root,
                                session,
                                views,
                            } => {
                                let result = session.save(&root).and_then(|()| views.save(&root));
                                io_pub.send(
                                    epoch,
                                    Event::Checkpointed(result.map_err(|e| {
                                        format!("Could not save before quitting: {e:#}")
                                    })),
                                );
                            }
                            IoWork::SaveViews { root, views } => {
                                if let Err(e) = views.save(&root) {
                                    io_pub.send(
                                        epoch,
                                        Event::ViewError(format!(
                                            "Saved view could not be written: {e}"
                                        )),
                                    );
                                }
                            }
                            IoWork::MergeSession { root, session } => {
                                let mut existing = Session::load(&root)?;
                                existing.merge(session);
                                existing.save(&root)?;
                                io_pub.send(epoch, Event::Saved);
                            }
                            IoWork::Save { root, session } => {
                                session.save(&root)?;
                                io_pub.send(epoch, Event::Saved);
                            }
                        }
                        Ok(())
                    })();
                    if let Err(e) = result {
                        io_pub.send(epoch, Event::Error(format!("{e:#}")));
                    }
                }
            })
            .expect("Start filesystem worker");
        let cache_control = super::cache::Control::default();
        let duplicate_cache = cache_control.clone();
        let (duplicate_tx, duplicate_rx) = mpsc::sync_channel::<SimilarityJob>(1);
        let (evidence_tx, evidence_rx) = mpsc::sync_channel::<EvidenceRequest>(1);
        let duplicate_pub = publisher.clone();
        let duplicate_generation = analysis.clone();
        let duplicate_thread = thread::Builder::new()
            .name("photosort-similarity".into())
            .spawn(move || {
                while !duplicate_pub.stop.load(Ordering::Relaxed) {
                    let Ok(mut job) = duplicate_rx.recv_timeout(Duration::from_millis(100)) else {
                        continue;
                    };
                    let cancelled = || {
                        duplicate_pub.cancelled(job.epoch)
                            || duplicate_generation.current() != job.run
                    };
                    if cancelled() {
                        continue;
                    }
                    let grouping = if job.profile == AnalysisProfile::Faces {
                        super::face_groups::group(&job.regional, &job.store, cancelled)
                    } else {
                        super::regional::grouping::group(
                            &job.regional,
                            &job.store,
                            job.profile == AnalysisProfile::Mixed,
                            job.similarity_distance,
                            cancelled,
                        )
                    };
                    let mut grouped = match grouping {
                        Ok(Some(grouped)) => grouped,
                        Ok(None) => continue,
                        Err(error) => {
                            duplicate_pub.send_checked(
                                job.epoch,
                                Event::Error(format!("Similarity grouping failed: {error:#}")),
                                cancelled,
                            );
                            continue;
                        }
                    };
                    let valid = job.sources.iter().all(|(media, source)| {
                        !cancelled() && Pipeline::key(media).is_ok_and(|key| key == *source)
                    });
                    if cancelled() {
                        continue;
                    }
                    if !valid {
                        duplicate_pub.send_checked(
                            job.epoch,
                            Event::Error(
                                "Photos changed during similarity grouping; analyze again".into(),
                            ),
                            cancelled,
                        );
                        continue;
                    }
                    let mut automatic = Arc::default();
                    let mut choices = Vec::new();
                    if matches!(
                        job.profile,
                        AnalysisProfile::Visual | AnalysisProfile::Duplicates
                    ) {
                        automatic = Arc::new(std::mem::take(&mut grouped.groups));
                        let resolved = super::manual_groups::resolve_scenes(
                            &automatic,
                            &job.scene_groups,
                            cancelled,
                        )
                        .and_then(|groups| {
                            let choices = super::manual_groups::group_choices(&groups, cancelled)?;
                            Ok((groups, choices))
                        });
                        match resolved {
                            Ok((groups, index)) => {
                                grouped.groups = groups;
                                choices = index;
                            }
                            Err(error) => {
                                duplicate_pub.send_checked(
                                    job.epoch,
                                    Event::Error(format!("Scene corrections failed: {error:#}")),
                                    cancelled,
                                );
                                continue;
                            }
                        }
                    }
                    for input in &mut job.inputs {
                        input.group = grouped.groups.get(&input.media.id).copied().unwrap_or(0);
                    }
                    if job.profile == AnalysisProfile::Faces {
                        duplicate_pub.send_checked(
                            job.epoch,
                            Event::FaceGroups {
                                run: job.run,
                                groups: grouped,
                            },
                            cancelled,
                        );
                    } else if job.profile == AnalysisProfile::Mixed {
                        duplicate_pub.send_checked(
                            job.epoch,
                            Event::MixedGroups {
                                run: job.run,
                                labels: grouped.folders,
                            },
                            cancelled,
                        );
                    } else {
                        duplicate_pub.send_checked(
                            job.epoch,
                            Event::GeneralGroups {
                                run: job.run,
                                automatic,
                                choices,
                                groups: grouped.groups,
                            },
                            cancelled,
                        );
                    }
                    if job.profile != AnalysisProfile::Duplicates {
                        duplicate_cache.request(false);
                        duplicate_pub.send_checked(
                            job.epoch,
                            Event::AnalysisDone {
                                run: job.run,
                                stats: job.stats,
                            },
                            cancelled,
                        );
                        continue;
                    }
                    duplicate_pub.send_checked(
                        job.epoch,
                        Event::DuplicateStarted { run: job.run },
                        cancelled,
                    );
                    let mut evidence = EvidenceClient {
                        job: &job,
                        requests: &evidence_tx,
                        cancelled: &cancelled,
                        stats: job.stats,
                    };
                    let result = super::near_duplicates::batch::review(
                        &job.inputs,
                        &job.store,
                        &mut evidence,
                        cancelled,
                        |done, total| {
                            duplicate_pub.send_checked(
                                job.epoch,
                                Event::Progress {
                                    run: job.run,
                                    done,
                                    total,
                                },
                                cancelled,
                            )
                        },
                    );
                    // Both file and sidecar identity must still match, including exact
                    // copies that deliberately bypass all pixel and face requests.
                    let valid = job.inputs.iter().all(|i| {
                        !cancelled() && Pipeline::key(&i.media).is_ok_and(|k| k == i.source)
                    });
                    if cancelled() {
                        continue;
                    }
                    match result {
                        Ok(Some(pairs)) if valid => {
                            duplicate_pub.send_checked(
                                job.epoch,
                                Event::DuplicatePairs {
                                    run: job.run,
                                    pairs,
                                },
                                cancelled,
                            );
                            duplicate_cache.request(false);
                            duplicate_pub.send_checked(
                                job.epoch,
                                Event::AnalysisDone {
                                    run: job.run,
                                    stats: evidence.stats,
                                },
                                cancelled,
                            );
                        }
                        Err(error) => duplicate_pub.send_checked(
                            job.epoch,
                            Event::Error(format!("Duplicate review failed: {error:#}")),
                            cancelled,
                        ),
                        Ok(_) if !valid => duplicate_pub.send_checked(
                            job.epoch,
                            Event::Error(
                                "Photos changed during duplicate review; analyze again".into(),
                            ),
                            cancelled,
                        ),
                        _ => {}
                    }
                }
            })
            .expect("Start general similarity policy worker");
        let subject_cache = cache_control.clone();
        let (subject_tx, subject_rx) = mpsc::sync_channel::<SubjectJob>(1);
        let subject_pub = publisher.clone();
        let subject_generation = analysis.clone();
        let subject_thread = thread::Builder::new()
            .name("photosort-subjects".into())
            .spawn(move || {
                while !subject_pub.stop.load(Ordering::Relaxed) {
                    let Ok(job) = subject_rx.recv_timeout(Duration::from_millis(100)) else {
                        continue;
                    };
                    let cancelled = || {
                        subject_pub.cancelled(job.epoch) || subject_generation.current() != job.run
                    };
                    if cancelled() {
                        continue;
                    }
                    match super::subjects::grouping::group(
                        job.inputs,
                        &job.store,
                        job.strictness,
                        cancelled,
                    ) {
                        Ok(Some(groups)) if !cancelled() => {
                            subject_pub.send_checked(
                                job.epoch,
                                Event::SubjectGroups {
                                    run: job.run,
                                    groups,
                                },
                                cancelled,
                            );
                            subject_cache.request(false);
                            subject_pub.send_checked(
                                job.epoch,
                                Event::AnalysisDone {
                                    run: job.run,
                                    stats: job.stats,
                                },
                                cancelled,
                            );
                        }
                        Err(error) if !cancelled() => subject_pub.send_checked(
                            job.epoch,
                            Event::Error(format!("Same-subject grouping failed: {error:#}")),
                            cancelled,
                        ),
                        _ => {}
                    }
                }
            })
            .expect("Start subject policy worker");
        let processing = super::processing::Control::default();
        let processing_worker = processing.clone();
        let analysis_generation = analysis.clone();
        let metadata_worker_generation = metadata_generation.clone();
        let inspection = Arc::new(Mutex::new(super::inspection::Queue::default()));
        let inspection_worker = inspection.clone();
        let cache_worker = cache_control.clone();
        let image_thread = thread::Builder::new()
            .name("photosort-images".into())
            .spawn(move || {
                let subject_root = cache.clone();
                let mut subject_store: Option<Arc<super::subjects::store::Store>> = None;
                let mut subject_inputs = Vec::new();
                let mut duplicate_inputs = Vec::new();
                let mut regional_inputs = Vec::new();
                let mut regional_sources = Vec::new();
                let mut subject_reset = false;
                let mut subject_persistent = true;
                let mut strictness = super::subjects::Strictness::default();
                let mut scene_groups = Arc::default();
                let mut similarity_distance = super::regional::DEFAULT_EPS;
                let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache);
                type AnalysisState = (
                    u64,
                    u64,
                    Arc<Vec<Media>>,
                    usize,
                    VisualGroups,
                    HashMap<String, MediaId>,
                );
                let mut pending: Option<AnalysisState> = None;
                struct MetadataIndex {
                    epoch: u64,
                    run: u64,
                    media: Arc<Vec<Media>>,
                    next: usize,
                    batch: Vec<(MediaId, Result<Metadata, String>)>,
                }
                let mut metadata_index: Option<MetadataIndex> = None;
                let mut results: HashMap<String, CachedAnalysis> = HashMap::new();
                let mut result_epoch = 0;
                let mut model_signature = String::new();
                let mut profile = AnalysisProfile::Technical;
                let mut thresholds = Thresholds::default();
                let mut technical_measurements = 0;
                let mut content_hashes = 0;

                let mut quality_measurements = 0;
                let mut models = super::models::Models::with_processing(processing_worker);
                models.configure_compiled_cache(
                    subject_root.clone(),
                    u64::from(super::cache::Limits::default().disk_mib) * super::cache::MIB,
                    false,
                );
                let mut orientation_model = None;
                while !publisher.stop.load(Ordering::Relaxed) {
                    if let Some(request) = cache_worker.take() {
                        models.configure_compiled_cache(
                            subject_root.clone(),
                            u64::from(request.limits.disk_mib) * super::cache::MIB,
                            request.clear,
                        );
                        pipeline.configure_cache(request.limits);
                        subject_persistent = request.limits.disk_mib > 0;
                        if let Some(store) = &subject_store {
                            store.persistent(subject_persistent);
                        }
                        if request.clear {
                            pipeline.clear_memory();
                            results.clear();
                            subject_reset = true;
                            if pending.is_none() {
                                subject_store = None;
                            }
                        }
                        let result = pipeline
                            .maintain_cache(request.clear, || {
                                cache_worker.cancelled(request)
                                    || publisher.stop.load(Ordering::Relaxed)
                            })
                            .map(|mut usage| {
                                usage.analysis_entries = results.len();
                                usage
                            });
                        publisher.send(
                            publisher.generation.load(Ordering::Acquire),
                            Event::CacheStats(pipeline.stats),
                        );
                        cache_worker.finish(result, request.clear);
                        (publisher.wake)();
                        continue;
                    }
                    let inspect = inspection_worker.lock().unwrap().take();
                    if let Some((epoch, request)) = inspect {
                        if !publisher.cancelled(epoch) {
                            if result_epoch != epoch {
                                results.clear();
                                subject_store = None;
                                pipeline.clear_memory();
                                result_epoch = epoch;
                            }
                            let rendered = pipeline.inspect_with_warning(&request, &|| {
                                publisher.cancelled(epoch)
                                    || !inspection_worker.lock().unwrap().accepts(epoch, &request)
                            });
                            let (result, decoder_warning) = match rendered {
                                Ok((pixels, warning)) => (Ok(pixels), warning),
                                Err(error) => (Err(format!("{error:#}")), None),
                            };
                            let accepted =
                                inspection_worker.lock().unwrap().accepts(epoch, &request);
                            if accepted {
                                publisher.send(
                                    epoch,
                                    Event::Inspected {
                                        request,
                                        result,
                                        decoder_warning,
                                        stats: pipeline.stats,
                                    },
                                );
                            }
                        }
                        continue;
                    }
                    if let Ok(request) = evidence_rx.try_recv() {
                        let cancelled = || {
                            publisher.cancelled(request.epoch)
                                || analysis_generation.current() != request.run
                        };
                        if cancelled() {
                            continue;
                        }
                        let result = (|| -> anyhow::Result<_> {
                            anyhow::ensure!(
                                Pipeline::key(&request.input.media)? == request.input.source,
                                "Photo changed during duplicate review"
                            );
                            let asset = pipeline.analysis(&request.input.media, cancelled)?;
                            let faces = if request.faces {
                                let mut features = request
                                    .store
                                    .get(&request.input.key)
                                    .context("Shared model evidence is no longer available")?;
                                let complete = features.faces.as_ref().is_some_and(|f| {
                                    f.geometry.iter().all(|f| !f.landmarks.is_empty())
                                });
                                if !complete {
                                    let faces = super::faces::cached(
                                        &mut models,
                                        &asset,
                                        &request.store,
                                        &request.input.source,
                                        features.faces.as_ref(),
                                        cancelled,
                                    )?;
                                    // Retain reusable crop identities when only the missing
                                    // landmarks were upgraded; changed crops require re-encoding.
                                    let same_crops =
                                        features.faces.as_ref().is_some_and(|old| {
                                            old.geometry.len() == faces.geometry.len()
                                                && old.geometry.iter().zip(&faces.geometry).all(
                                                    |(a, b)| a.bbox == b.bbox && a.eyes == b.eyes,
                                                )
                                        });
                                    if !same_crops {
                                        features.identities = None;
                                    }
                                    features.faces = Some(faces);
                                    request.store.put(&request.input.key, &features)?;
                                }
                                features.faces.map(|f| f.geometry)
                            } else {
                                None
                            };
                            anyhow::ensure!(
                                !cancelled()
                                    && Pipeline::key(&request.input.media)? == request.input.source,
                                "Photo changed during duplicate review"
                            );
                            Ok((
                                asset,
                                faces,
                                AnalysisStats {
                                    decodes: pipeline.stats.decodes,
                                    quality_measurements,
                                    technical_measurements,
                                    content_hashes,
                                    model_initializations: models.initializations,
                                    model_inferences: models.inferences,
                                    model_threads: models.cpu_threads(),
                                },
                            ))
                        })();
                        let _ = request.reply.try_send(result);
                        continue;
                    }
                    let message = priority_rx.try_recv().ok().or_else(|| {
                        if pending.is_some() || metadata_index.is_some() {
                            image_rx.try_recv().ok()
                        } else {
                            image_rx.recv_timeout(Duration::from_millis(15)).ok()
                        }
                    });
                    if let Some(message) = message {
                        let epoch = message.epoch;
                        if publisher.cancelled(epoch)
                            || !message
                                .work
                                .is_current(&analysis_generation, &metadata_worker_generation)
                        {
                            continue;
                        }
                        if result_epoch != epoch {
                            results.clear();
                            subject_store = None;
                            pipeline.clear_memory();
                            result_epoch = epoch;
                        }
                        match message.work {
                            ImageWork::PlanDecisions {
                                catalog,
                                decisions,
                                companions,
                                policy,
                            } => {
                                let plan = operations::decisions_plan_with_metadata(
                                    &catalog,
                                    &decisions,
                                    companions,
                                    policy,
                                    |media| {
                                        pipeline.metadata_cancellable(media, &|| {
                                            publisher.cancelled(epoch)
                                        })
                                    },
                                    || publisher.cancelled(epoch),
                                );
                                if !publisher.cancelled(epoch) {
                                    publisher.send(
                                        epoch,
                                        match plan {
                                            Ok(plan) => Event::Planned(plan),
                                            Err(error) => Event::Error(format!(
                                                "Could not plan file edits: {error:#}"
                                            )),
                                        },
                                    );
                                }
                            }
                            ImageWork::IndexMetadata { run, media } => {
                                metadata_index = Some(MetadataIndex {
                                    epoch,
                                    run,
                                    media,
                                    next: 0,
                                    batch: Vec::with_capacity(64),
                                });
                            }
                            ImageWork::Invalidate => {
                                pipeline.clear_memory();
                                results.clear();
                                subject_reset = true;
                                if pending.is_none() {
                                    subject_store = None;
                                }
                            }
                            ImageWork::Analyze {
                                run,
                                media,
                                profile: selected_profile,
                                thresholds: selected_thresholds,
                                strictness: selected_strictness,
                                similarity_distance: selected_distance,
                                scene_groups: selected_scene_groups,
                                model_config,
                            } => {
                                let cancelled = || {
                                    publisher.cancelled(epoch)
                                        || analysis_generation.current() != run
                                };
                                let issue = super::model_assets::preflight(
                                    super::model_assets::required(selected_profile),
                                );
                                if cancelled() {
                                    continue;
                                }
                                if !issue.is_empty() {
                                    publisher.send_checked(
                                        epoch,
                                        Event::ModelsNeeded { run, issue },
                                        cancelled,
                                    );
                                    continue;
                                }
                                let model_publisher = publisher.clone();
                                let model_generation = analysis_generation.clone();
                                models.begin_run(model_config, move || {
                                    model_publisher.cancelled(epoch)
                                        || model_generation.current() != run
                                });
                                let signature = super::models::analysis_signature();
                                if signature != model_signature {
                                    results.clear();
                                    subject_store = None;
                                    model_signature = signature;
                                }
                                if let Err(error) = prepare_evidence_store(
                                    &mut subject_store,
                                    &subject_root,
                                    subject_persistent,
                                    &mut subject_reset,
                                ) {
                                    publisher.send_checked(
                                        epoch,
                                        Event::Error(format!(
                                            "Cannot prepare analysis evidence: {error}"
                                        )),
                                        cancelled,
                                    );
                                    continue;
                                }
                                if cancelled() {
                                    continue;
                                }
                                subject_inputs.clear();
                                duplicate_inputs.clear();
                                regional_inputs.clear();
                                regional_sources.clear();
                                strictness = selected_strictness;
                                similarity_distance = selected_distance;
                                scene_groups = selected_scene_groups;
                                profile = selected_profile;
                                thresholds = selected_thresholds;

                                orientation_model = None;
                                pending = Some((
                                    epoch,
                                    run,
                                    media,
                                    0,
                                    VisualGroups::default(),
                                    HashMap::new(),
                                ));
                            }
                            ImageWork::Orient {
                                run,
                                media,
                                model,
                                model_config,
                            } => {
                                let cancelled = || {
                                    publisher.cancelled(epoch)
                                        || analysis_generation.current() != run
                                };
                                let mut issue = super::model_assets::preflight(&[
                                    super::model_assets::Id::Rotation,
                                ]);
                                // Explicit caller-selected orientation files are permitted.
                                issue.missing =
                                    if Fingerprint::read(&model).is_ok_and(|fp| fp.bytes > 0) {
                                        Vec::new()
                                    } else {
                                        vec![super::model_assets::Id::Rotation]
                                    };
                                if cancelled() {
                                    continue;
                                }
                                if !issue.is_empty() {
                                    publisher.send_checked(
                                        epoch,
                                        Event::ModelsNeeded { run, issue },
                                        cancelled,
                                    );
                                    continue;
                                }

                                let detector = match super::rotation::Detector::new(model) {
                                    Ok(detector) => detector,
                                    Err(error) => {
                                        publisher.send_checked(
                                            epoch,
                                            Event::Error(format!(
                                                "Cannot prepare rotation model: {error}"
                                            )),
                                            cancelled,
                                        );
                                        continue;
                                    }
                                };
                                if let Err(error) = prepare_evidence_store(
                                    &mut subject_store,
                                    &subject_root,
                                    subject_persistent,
                                    &mut subject_reset,
                                ) {
                                    publisher.send_checked(
                                        epoch,
                                        Event::Error(format!(
                                            "Cannot prepare analysis evidence: {error}"
                                        )),
                                        cancelled,
                                    );
                                    continue;
                                }
                                if cancelled() {
                                    continue;
                                }
                                let model_publisher = publisher.clone();
                                let model_generation = analysis_generation.clone();
                                models.begin_run(model_config, move || {
                                    model_publisher.cancelled(epoch)
                                        || model_generation.current() != run
                                });
                                orientation_model = Some(detector);
                                pending = Some((
                                    epoch,
                                    run,
                                    media,
                                    0,
                                    VisualGroups::default(),
                                    HashMap::new(),
                                ));
                            }
                            ImageWork::Asset(media, preview) => match pipeline
                                .load_cancellable(&media, || publisher.cancelled(epoch))
                            {
                                Ok(asset) => publisher.send(
                                    epoch,
                                    Event::Asset {
                                        id: media.id,
                                        preview,
                                        decoder_warning: asset.decoder_warning,
                                        pixels: if preview {
                                            asset.pixels
                                        } else {
                                            asset.thumbnail
                                        },
                                        metadata: asset.metadata,
                                        stats: pipeline.stats,
                                    },
                                ),
                                Err(e) => publisher.send(
                                    epoch,
                                    Event::AssetError {
                                        id: media.id,
                                        preview,
                                        error: format!("{e:#}"),
                                    },
                                ),
                            },
                        }
                        continue;
                    }
                    if let Some((epoch, run, media, index, groups, hashes)) = &mut pending {
                        if publisher.cancelled(*epoch) || analysis_generation.current() != *run {
                            pending = None;
                            continue;
                        }
                        if *index >= media.len() {
                            let stats = AnalysisStats {
                                decodes: pipeline.stats.decodes,
                                quality_measurements,
                                technical_measurements,
                                content_hashes,
                                model_initializations: models.initializations,
                                model_inferences: models.inferences,
                                model_threads: models.cpu_threads(),
                            };
                            if orientation_model.is_none()
                                && matches!(
                                    profile,
                                    AnalysisProfile::Duplicates
                                        | AnalysisProfile::Visual
                                        | AnalysisProfile::Mixed
                                        | AnalysisProfile::Faces
                                )
                            {
                                publisher.send(*epoch, Event::GeneralStarted { run: *run });
                                let job = SimilarityJob {
                                    scene_groups: scene_groups.clone(),
                                    similarity_distance,
                                    epoch: *epoch,
                                    run: *run,
                                    inputs: std::mem::take(&mut duplicate_inputs),
                                    regional: std::mem::take(&mut regional_inputs),
                                    sources: std::mem::take(&mut regional_sources),
                                    profile,
                                    store: subject_store.as_ref().unwrap().clone(),
                                    stats,
                                };
                                if duplicate_tx.try_send(job).is_err() {
                                    publisher.send(
                                        *epoch,
                                        Event::Error(
                                            "Similarity worker is busy; retry analysis".into(),
                                        ),
                                    );
                                }
                            } else if orientation_model.is_none()
                                && matches!(
                                    profile,
                                    AnalysisProfile::Subjects | AnalysisProfile::Quality
                                )
                            {
                                publisher.send(*epoch, Event::SubjectStarted { run: *run });
                                let job = SubjectJob {
                                    epoch: *epoch,
                                    run: *run,
                                    inputs: std::mem::take(&mut subject_inputs),
                                    store: subject_store.as_ref().unwrap().clone(),
                                    strictness,
                                    stats,
                                };
                                if subject_tx.try_send(job).is_err() {
                                    publisher.send(
                                        *epoch,
                                        Event::Error(
                                            "Subject worker is busy; retry analysis".into(),
                                        ),
                                    );
                                }
                            } else {
                                publisher.send(*epoch, Event::AnalysisDone { run: *run, stats });
                            }
                            pending = None;
                            let _ = pipeline
                                .maintain_cache(false, || publisher.stop.load(Ordering::Relaxed));
                            continue;
                        }
                        let item = &media[*index];
                        if item.kind != MediaKind::Video {
                            let mut item_metadata = Metadata::default();
                            let result = (|| -> anyhow::Result<()> {
                                let source_key = Pipeline::key(item)?;
                                if profile == AnalysisProfile::Faces && orientation_model.is_none()
                                {
                                    let cancelled = || {
                                        publisher.cancelled(*epoch)
                                            || analysis_generation.current() != *run
                                    };
                                    let store = subject_store
                                        .as_ref()
                                        .context("Face cache was cleared; retry analysis")?;
                                    let face_key = super::subjects::store::Store::face_key(
                                        &source_key,
                                        &super::models::face_signature(),
                                    );
                                    let recognition_key =
                                        super::subjects::store::Store::recognition_key(
                                            &face_key,
                                            &super::models::recognition_signature(),
                                        );
                                    let recognition = if let Some(recognition) =
                                        store.recognition(&recognition_key)
                                    {
                                        recognition
                                    } else {
                                        let asset = pipeline.analysis(item, cancelled)?;
                                        let old = store.get(&super::subjects::store::Store::key(
                                            &source_key,
                                            &model_signature,
                                        ));
                                        let faces = super::faces::cached(
                                            &mut models,
                                            &asset,
                                            store,
                                            &source_key,
                                            old.as_ref().and_then(|f| f.faces.as_ref()),
                                            cancelled,
                                        )?;
                                        let recognition = super::face_groups::describe(
                                            &mut models,
                                            &asset,
                                            &faces,
                                            cancelled,
                                        )?;
                                        anyhow::ensure!(
                                            !cancelled() && Pipeline::key(item)? == source_key,
                                            "Photo changed during face grouping"
                                        );
                                        store.put_recognition(&recognition_key, &recognition)?;
                                        recognition
                                    };
                                    anyhow::ensure!(
                                        !cancelled()
                                            && Pipeline::key(item)? == source_key
                                            && Fingerprint::read(&item.path)? == item.fingerprint,
                                        "Photo changed during face grouping"
                                    );
                                    regional_inputs.push(super::regional::grouping::Record {
                                        id: item.id,
                                        key: (!recognition.vectors.is_empty())
                                            .then_some(recognition_key),
                                        portrait: false,
                                        date: None,
                                    });
                                    regional_sources.push((item.clone(), source_key));
                                    return Ok(());
                                }
                                let cancelled = || {
                                    publisher.cancelled(*epoch)
                                        || analysis_generation.current() != *run
                                };
                                if let Some(detector) = &orientation_model {
                                    let store = subject_store
                                        .as_ref()
                                        .context("Rotation cache was cleared; retry detection")?;
                                    let suggestion = detector.detect(
                                        item,
                                        &mut pipeline,
                                        &mut models,
                                        store,
                                        &cancelled,
                                    )?;
                                    anyhow::ensure!(
                                        !cancelled() && Pipeline::key(item)? == source_key,
                                        "Photo changed or analysis cancelled"
                                    );
                                    publisher.send(
                                        *epoch,
                                        Event::Rotation {
                                            run: *run,
                                            id: item.id,
                                            suggestion,
                                        },
                                    );
                                    return Ok(());
                                }
                                item_metadata = pipeline.metadata_cancellable(item, &cancelled)?;
                                let mut prepared = None;
                                let mut load_asset =
                                    || -> anyhow::Result<super::pipeline::Asset> {
                                        anyhow::ensure!(!cancelled(), "Analysis cancelled");
                                        if prepared.is_none() {
                                            prepared = Some(pipeline.analysis(item, cancelled)?);
                                        }
                                        Ok(prepared.as_ref().unwrap().clone())
                                    };
                                let store = subject_store
                                    .as_ref()
                                    .context("Analysis cache was cleared; retry analysis")?;
                                let key = source_key.clone();
                                // Keep descriptors bounded independently of the decoded-image budget.
                                if results.len() >= 20_000 && !results.contains_key(&key) {
                                    results.clear();
                                }
                                let cached = match results.entry(key) {
                                    std::collections::hash_map::Entry::Occupied(entry) => {
                                        entry.into_mut()
                                    }
                                    std::collections::hash_map::Entry::Vacant(entry) => {
                                        let measurement_key =
                                            super::subjects::store::Store::measurements_key(
                                                &source_key,
                                            );
                                        let value = if let Some(value) =
                                            store.measurements(&measurement_key)
                                        {
                                            value
                                        } else {
                                            let asset = load_asset()?;
                                            let mut technical = analysis::analyze_cancellable(
                                                &asset,
                                                Thresholds::default(),
                                                cancelled,
                                            )
                                            .context("Image measurements cancelled")?;
                                            technical_measurements += 1;
                                            quality_measurements += 1;
                                            technical.issues.clear();
                                            let content_hash =
                                                operations::content_hash(&item.path, cancelled)?;
                                            content_hashes += 1;
                                            anyhow::ensure!(
                                                !cancelled() && Pipeline::key(item)? == source_key,
                                                "Photo changed or analysis cancelled"
                                            );
                                            let value = analysis::Measurements {
                                                analysis: technical,
                                                width: asset.pixels.width(),
                                                height: asset.pixels.height(),
                                                content_hash,
                                            };
                                            store.put_measurements(&measurement_key, &value)?;
                                            value
                                        };
                                        entry.insert(CachedAnalysis {
                                            technical: value.analysis,
                                            hash: value.content_hash,
                                            dimensions: (value.width, value.height),
                                            embedding: None,
                                            aesthetic: None,
                                            faces: None,
                                        })
                                    }
                                };
                                if profile != AnalysisProfile::Technical {
                                    let store = subject_store
                                        .as_ref()
                                        .context("Subject cache was cleared; retry analysis")?;
                                    let subject_key = super::subjects::store::Store::key(
                                        &Pipeline::key(item)?,
                                        &model_signature,
                                    );
                                    let previous = store.get(&subject_key);
                                    let mut changed = previous.is_none();
                                    let mut features = match previous {
                                        Some(features) => features,
                                        None => super::subjects::store::Features {
                                            dense: models.dense_embedding(&load_asset()?)?,
                                            faces: None,
                                            identities: None,
                                        },
                                    };
                                    cached.embedding = Some(features.dense.global.clone());
                                    if matches!(
                                        profile,
                                        AnalysisProfile::Quality | AnalysisProfile::Subjects
                                    ) {
                                        let complete =
                                            features.faces.as_ref().is_some_and(|f| f.complete());
                                        let faces = super::faces::cached_lazy(
                                            &mut models,
                                            store,
                                            &source_key,
                                            features.faces.as_ref(),
                                            &mut load_asset,
                                            || {
                                                publisher.cancelled(*epoch)
                                                    || analysis_generation.current() != *run
                                            },
                                        )?;
                                        if !complete {
                                            features.identities = None;
                                            features.faces = Some(faces);
                                            changed = true;
                                        }
                                    }
                                    if let Some(faces) = &features.faces {
                                        cached.faces = Some(faces.summary());
                                    }
                                    if matches!(
                                        profile,
                                        AnalysisProfile::Quality | AnalysisProfile::Subjects
                                    ) {
                                        if features.identities.is_none() {
                                            let mut identities = Vec::new();
                                            for face in &features.faces.as_ref().unwrap().geometry {
                                                anyhow::ensure!(
                                                    !publisher.cancelled(*epoch)
                                                        && analysis_generation.current() == *run,
                                                    "Analysis cancelled"
                                                );
                                                let crop = super::faces::aligned_crop(
                                                    &load_asset()?,
                                                    face,
                                                )?;
                                                let descriptor = models
                                                    .embedding(&crop)?
                                                    .into_iter()
                                                    .map(|v| half::f16::from_f32(v).to_f32())
                                                    .collect();
                                                identities.push(super::subjects::Evidence {
                                                    descriptor,
                                                    bbox: face.bbox,
                                                    area: ((face.bbox[2] - face.bbox[0])
                                                        * (face.bbox[3] - face.bbox[1]))
                                                        .max(0.0),
                                                    kind: "face".into(),
                                                });
                                            }
                                            features.identities = Some(identities);
                                            changed = true;
                                        }
                                        subject_inputs.push(super::subjects::grouping::Input {
                                            id: item.id,
                                            path: item.path.clone(),
                                            key: subject_key.clone(),
                                            global: features
                                                .dense
                                                .global
                                                .iter()
                                                .map(|v| half::f16::from_f32(*v).to_f32())
                                                .collect(),
                                            timestamp: item_metadata.captured_ms,
                                        });
                                    }
                                    if changed {
                                        store.put(&subject_key, &features)?;
                                    }
                                    if matches!(
                                        profile,
                                        AnalysisProfile::Duplicates
                                            | AnalysisProfile::Visual
                                            | AnalysisProfile::Mixed
                                    ) {
                                        let regional_key =
                                            super::subjects::store::Store::regional_key(
                                                &subject_key,
                                            );
                                        if store.regional(&regional_key).is_none() {
                                            let regions = models.similarity_regions(
                                                &load_asset()?,
                                                &features.dense.global,
                                                || {
                                                    publisher.cancelled(*epoch)
                                                        || analysis_generation.current() != *run
                                                },
                                            )?;
                                            store.put_regional(&regional_key, &regions)?;
                                        }
                                    }
                                }
                                if profile == AnalysisProfile::Quality && cached.aesthetic.is_none()
                                {
                                    let key = super::subjects::store::Store::aesthetic_key(
                                        &source_key,
                                        &super::models::aesthetic_signature(),
                                    );
                                    cached.aesthetic =
                                        Some(if let Some(value) = store.aesthetic(&key) {
                                            value
                                        } else {
                                            let value = models.aesthetic(&load_asset()?)?;
                                            anyhow::ensure!(
                                                !cancelled() && Pipeline::key(item)? == source_key,
                                                "Photo changed or analysis cancelled"
                                            );
                                            store.put_aesthetic(&key, value)?;
                                            value
                                        });
                                }
                                let mut technical = cached.technical.clone();
                                technical.reclassify(thresholds);
                                technical.faces = cached.faces;
                                if let Some(aesthetic) = cached.aesthetic {
                                    technical.aesthetic = Some(aesthetic);
                                    let sharpness = technical.quality_sharpness.expect(
                                        "Shared measurements include whole-image sharpness",
                                    );
                                    let blur = 0.35 * (1.0 - sharpness / 110.0).clamp(0.0, 1.0);
                                    let eyes = cached.faces.as_ref().map_or(0.0, |faces| {
                                        0.30 * faces.closed_eyes as f64 / faces.count.max(1) as f64
                                    });
                                    technical.score = 100.0 * (f64::from(aesthetic) - blur - eyes);
                                }
                                let hash = cached.hash.clone();
                                // Reload the shared metadata snapshot after any cold decode has
                                // refined its dimensions. Warm evidence never needs pixel IO.
                                let metadata = pipeline.metadata_cancellable(item, &cancelled)?;
                                let duplicate = hashes.get(&hash).copied();
                                hashes.entry(hash.clone()).or_insert(item.id);
                                let group = if profile != AnalysisProfile::Technical {
                                    0 // Subject and regional groups publish only after full verification.
                                } else {
                                    groups.assign(&technical)
                                };
                                if matches!(
                                    profile,
                                    AnalysisProfile::Duplicates
                                        | AnalysisProfile::Visual
                                        | AnalysisProfile::Mixed
                                ) {
                                    let source = Pipeline::key(item)?;
                                    anyhow::ensure!(
                                        source == source_key
                                            && Fingerprint::read(&item.path)? == item.fingerprint,
                                        "Photo changed during analysis; reopen the folder"
                                    );
                                    regional_inputs.push(super::regional::grouping::Record {
                                        id: item.id,
                                        key: Some(super::subjects::store::Store::regional_key(
                                            &super::subjects::store::Store::key(
                                                &source,
                                                &model_signature,
                                            ),
                                        )),
                                        portrait: super::regional::portrait(
                                            cached.dimensions.0,
                                            cached.dimensions.1,
                                        ),
                                        date: super::dates::label(item, &metadata),
                                    });
                                    regional_sources.push((item.clone(), source));
                                }
                                if profile == AnalysisProfile::Duplicates {
                                    let source = Pipeline::key(item)?;
                                    anyhow::ensure!(
                                        source == source_key
                                            && Fingerprint::read(&item.path)? == item.fingerprint,
                                        "Photo changed during analysis; reopen the folder"
                                    );
                                    duplicate_inputs.push(super::near_duplicates::batch::Input {
                                        media: item.clone(),
                                        key: super::subjects::store::Store::key(
                                            &source,
                                            &model_signature,
                                        ),
                                        source,
                                        hash,
                                        group,
                                        embedding: cached.embedding.as_ref().unwrap().clone(),
                                        sharpness: technical
                                            .sharpness
                                            .is_finite()
                                            .then_some(technical.sharpness),
                                        metadata_fields: metadata.metadata_fields,
                                    });
                                }
                                anyhow::ensure!(
                                    !cancelled()
                                        && Pipeline::key(item)? == source_key
                                        && Fingerprint::read(&item.path)? == item.fingerprint,
                                    "Photo changed or analysis cancelled"
                                );
                                publisher.send(
                                    *epoch,
                                    Event::Analyzed {
                                        run: *run,
                                        id: item.id,
                                        analysis: technical,
                                        embedding: cached
                                            .embedding
                                            .as_ref()
                                            .map(|v| Arc::new(v.clone())),
                                        group,
                                        duplicate: if profile == AnalysisProfile::Duplicates {
                                            None
                                        } else {
                                            duplicate
                                        },
                                        metadata,
                                    },
                                );
                                Ok(())
                            })();
                            // Cancellation can interrupt preparation, measurements or
                            // native models. Retire the run before publishing errors,
                            // fallback groups or progress for work the user superseded.
                            if publisher.cancelled(*epoch) || analysis_generation.current() != *run
                            {
                                pending = None;
                                continue;
                            }
                            if let Err(e) = result {
                                if matches!(
                                    profile,
                                    AnalysisProfile::Mixed | AnalysisProfile::Faces
                                ) && orientation_model.is_none()
                                {
                                    regional_inputs.push(super::regional::grouping::Record {
                                        id: item.id,
                                        key: None,
                                        portrait: false,
                                        date: super::dates::label(item, &item_metadata),
                                    });
                                }
                                publisher.send(
                                    *epoch,
                                    Event::AnalysisError {
                                        run: *run,
                                        id: item.id,

                                        error: format!("{e:#}"),
                                    },
                                );
                            }
                        }
                        *index += 1;
                        publisher.send(
                            *epoch,
                            Event::Progress {
                                run: *run,
                                done: *index,
                                total: media.len(),
                            },
                        );
                    }
                    // One metadata read per iteration. Inspection, preview requests and
                    // analysis share this actor and get a chance before the next item.
                    if let Some(index) = &mut metadata_index {
                        if publisher.cancelled(index.epoch)
                            || metadata_worker_generation.current() != index.run
                        {
                            metadata_index = None;
                            continue;
                        }
                        if let Some(media) = index.media.get(index.next) {
                            index.batch.push((
                                media.id,
                                pipeline
                                    .metadata_cancellable(media, &|| {
                                        publisher.cancelled(index.epoch)
                                            || metadata_worker_generation.current() != index.run
                                    })
                                    .map_err(|e| format!("{e:#}")),
                            ));
                            index.next += 1;
                        }
                        // A cancellation during a header read must not publish a partial
                        // snapshot into the current browser generation.
                        if publisher.cancelled(index.epoch)
                            || metadata_worker_generation.current() != index.run
                        {
                            metadata_index = None;
                            continue;
                        }
                        let complete = index.next == index.media.len();
                        if index.batch.len() >= 64 || complete {
                            publisher.send(
                                index.epoch,
                                Event::MetadataBatch {
                                    run: index.run,
                                    items: std::mem::take(&mut index.batch),
                                    done: index.next,
                                    total: index.media.len(),
                                    stats: pipeline.stats,
                                },
                            );
                        }
                        if complete {
                            metadata_index = None;
                        }
                    }
                }
            })
            .expect("Start imaging worker");
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
            self.image(ImageWork::Orient {
                run,
                media,
                model,
                model_config: self.processing.snapshot_run(),
            })
        })
    }
    pub fn shutdown(&mut self) {
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
    }
    pub fn cancel_analysis(&self) {
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
