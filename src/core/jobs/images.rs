//! Imaging actor. It alone owns decoding, cached evidence and model sessions, so
//! analysis, previews, inspection and cache maintenance interleave one step at a time.
use super::{
    AnalysisProfile, AnalysisStats, Event, ImageWork, Message, Publisher, RunGeneration,
    similarity::{EvidenceReply, EvidenceRequest, SimilarityJob},
    subjects::SubjectJob,
};
use crate::core::{
    analysis::{self, Analysis, Thresholds, VisualGroups},
    cache,
    catalog::Catalog,
    dates, face_groups, faces, inspection, manual_groups,
    model::*,
    model_assets, models, near_duplicates, operations,
    pipeline::{self, Pipeline},
    processing, regional, rotation, subjects,
};
use anyhow::Context;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::Ordering,
        mpsc::{Receiver, SyncSender},
    },
    thread,
    time::Duration,
};

/// Everything the worker thread takes over from `Runtime::with_cache`.
pub(super) struct Wiring {
    pub(super) publisher: Publisher,
    pub(super) cache: PathBuf,
    pub(super) cache_control: cache::Control,
    pub(super) processing: processing::Control,
    pub(super) raw_look: super::super::raw_look::Control,
    pub(super) inspection: Arc<Mutex<inspection::Queue>>,
    pub(super) analysis: Arc<RunGeneration>,
    pub(super) metadata: Arc<RunGeneration>,
    pub(super) images: Receiver<Message<ImageWork>>,
    pub(super) priority: Receiver<Message<ImageWork>>,
    pub(super) evidence: Receiver<EvidenceRequest>,
    pub(super) similarity: SyncSender<SimilarityJob>,
    pub(super) subjects: SyncSender<SubjectJob>,
}

pub(super) fn spawn(wiring: Wiring) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("photosort-images".into())
        .spawn(move || ImageWorker::new(wiring).run())
        .expect("Start imaging worker")
}

/// All analysis profiles, including rotation, share the same evidence owner.
fn prepare_evidence_store(
    store: &mut Option<Arc<subjects::store::Store>>,
    root: &std::path::Path,
    persistent: bool,
    reset: &mut bool,
) -> anyhow::Result<()> {
    if *reset {
        *store = None;
    }
    if store.is_none() {
        *store = Some(Arc::new(subjects::store::Store::new(
            root.into(),
            persistent,
        )?));
    }
    *reset = false;
    Ok(())
}

struct CachedAnalysis {
    technical: Analysis,
    hash: String,
    dimensions: (u32, u32),
    embedding: Option<Vec<f32>>,
    aesthetic: Option<f32>,
    faces: Option<faces::FaceSummary>,
}

/// Identity of one analysis run. Superseded by a new folder generation or run.
struct Scope {
    publisher: Publisher,
    generation: Arc<RunGeneration>,
    epoch: u64,
    run: u64,
}
impl Scope {
    fn cancelled(&self) -> bool {
        self.publisher.cancelled(self.epoch) || self.generation.current() != self.run
    }
    fn send(&self, event: Event) {
        self.publisher.send(self.epoch, event);
    }
    fn send_checked(&self, event: Event) {
        self.publisher
            .send_checked(self.epoch, event, || self.cancelled());
    }
}

/// Options of the analysis currently being collected.
struct Selection {
    profile: AnalysisProfile,
    thresholds: Thresholds,
    strictness: subjects::Strictness,
    similarity_distance: f32,
    scene_groups: Arc<manual_groups::ManualGroups>,
}

/// Evidence gathered per photo for the grouping workers.
#[derive(Default)]
struct Gathered {
    subject: Vec<subjects::grouping::Input>,
    duplicate: Vec<near_duplicates::batch::Input>,
    regional: Vec<regional::grouping::Record>,
    sources: Vec<(Media, String)>,
}
impl Gathered {
    fn clear(&mut self) {
        self.subject.clear();
        self.duplicate.clear();
        self.regional.clear();
        self.sources.clear();
    }
}

/// The analysis in progress, advanced one photo per loop iteration.
struct Pending {
    scope: Arc<Scope>,
    media: Arc<Vec<Media>>,
    index: usize,
}

struct MetadataIndex {
    epoch: u64,
    run: u64,
    media: Arc<Vec<Media>>,
    next: usize,
    batch: Vec<(MediaId, Result<Metadata, String>)>,
}

/// Decoded at most once per photo, and only when cached evidence is not enough.
#[derive(Default)]
struct Prepared(Option<pipeline::Asset>);
impl Prepared {
    fn get(
        &mut self,
        pipeline: &mut Pipeline,
        media: &Media,
        scope: &Scope,
    ) -> anyhow::Result<pipeline::Asset> {
        anyhow::ensure!(!scope.cancelled(), "Analysis cancelled");
        if self.0.is_none() {
            self.0 = Some(pipeline.analysis(media, || scope.cancelled())?);
        }
        Ok(self.0.as_ref().unwrap().clone())
    }
}

/// One photo moving through the analysis stages.
struct Item<'a> {
    scope: &'a Scope,
    media: &'a Media,
    source_key: String,
    prepared: Prepared,
}
impl Item<'_> {
    fn asset(&mut self, pipeline: &mut Pipeline) -> anyhow::Result<pipeline::Asset> {
        self.prepared.get(pipeline, self.media, self.scope)
    }
}

/// Cached measurements scored under the current thresholds.
struct Scored {
    technical: Analysis,
    hash: String,
    dimensions: (u32, u32),
    embedding: Option<Vec<f32>>,
}

struct ImageWorker {
    publisher: Publisher,
    analysis_generation: Arc<RunGeneration>,
    metadata_generation: Arc<RunGeneration>,
    cache_control: cache::Control,
    inspection: Arc<Mutex<inspection::Queue>>,
    images: Receiver<Message<ImageWork>>,
    priority: Receiver<Message<ImageWork>>,
    evidence: Receiver<EvidenceRequest>,
    similarity: SyncSender<SimilarityJob>,
    subjects: SyncSender<SubjectJob>,
    subject_root: PathBuf,
    subject_store: Option<Arc<subjects::store::Store>>,
    gathered: Gathered,
    subject_reset: bool,
    subject_persistent: bool,
    selection: Selection,
    pipeline: Pipeline,
    raw_look: super::super::raw_look::Control,
    pending: Option<Pending>,
    groups: VisualGroups,
    hashes: HashMap<String, MediaId>,
    metadata_index: Option<MetadataIndex>,
    results: HashMap<String, CachedAnalysis>,
    result_epoch: u64,
    model_signature: String,
    technical_measurements: u64,
    content_hashes: u64,
    quality_measurements: u64,
    models: models::Models,
    orientation_model: Option<rotation::Detector>,
}

impl ImageWorker {
    fn new(wiring: Wiring) -> Self {
        let Wiring {
            publisher,
            cache,
            cache_control,
            processing,
            raw_look,
            inspection,
            analysis,
            metadata,
            images,
            priority,
            evidence,
            similarity,
            subjects,
        } = wiring;
        let subject_root = cache.clone();
        let pipeline = Pipeline::new(128 * 1024 * 1024, cache);
        let mut models = models::Models::with_processing(processing);
        models.configure_compiled_cache(
            subject_root.clone(),
            u64::from(cache::Limits::default().disk_mib) * cache::MIB,
            false,
        );
        Self {
            raw_look,
            publisher,
            analysis_generation: analysis,
            metadata_generation: metadata,
            cache_control,
            inspection,
            images,
            priority,
            evidence,
            similarity,
            subjects,
            subject_root,
            subject_store: None,
            gathered: Gathered::default(),
            subject_reset: false,
            subject_persistent: true,
            selection: Selection {
                profile: AnalysisProfile::Technical,
                thresholds: Thresholds::default(),
                strictness: subjects::Strictness::default(),
                similarity_distance: regional::DEFAULT_EPS,
                scene_groups: Arc::default(),
            },
            pipeline,
            pending: None,
            groups: VisualGroups::default(),
            hashes: HashMap::new(),
            metadata_index: None,
            results: HashMap::new(),
            result_epoch: 0,
            model_signature: String::new(),
            technical_measurements: 0,
            content_hashes: 0,
            quality_measurements: 0,
            models,
            orientation_model: None,
        }
    }

    /// Each step reports whether it did work, in which case the loop starts over so
    /// cache maintenance, inspection and evidence requests keep their priority.
    fn run(&mut self) {
        // Work that panics every time (one corrupt state) must not spin; after
        // a few failures in a row this worker stops and the crash notice
        // asks for a restart.
        let mut failures = 0;
        while !self.publisher.stop.load(Ordering::Relaxed) && failures < 3 {
            let publisher = self.publisher.clone();
            // A Settings change applies to the next preview or detail request.
            self.pipeline.set_raw_look(self.raw_look.get());
            let ok = publisher.guard(super::Worker::Images, || {
                if self.handle_cache_request()
                    || self.handle_inspection()
                    || self.handle_evidence()
                    || self.handle_work()
                    || self.step_analysis()
                {
                    return;
                }
                self.step_metadata_index();
            });
            failures = if ok { 0 } else { failures + 1 };
        }
    }

    fn scope(&self, epoch: u64, run: u64) -> Scope {
        Scope {
            publisher: self.publisher.clone(),
            generation: self.analysis_generation.clone(),
            epoch,
            run,
        }
    }

    fn stats(&self) -> AnalysisStats {
        AnalysisStats {
            decodes: self.pipeline.stats.decodes,
            quality_measurements: self.quality_measurements,
            technical_measurements: self.technical_measurements,
            content_hashes: self.content_hashes,
            model_initializations: self.models.initializations,
            model_inferences: self.models.inferences,
            model_threads: self.models.cpu_threads(),
        }
    }

    /// Cached evidence belongs to one folder generation.
    fn enter_epoch(&mut self, epoch: u64) {
        if self.result_epoch != epoch {
            self.results.clear();
            self.subject_store = None;
            self.pipeline.clear_memory();
            self.result_epoch = epoch;
        }
    }

    fn handle_cache_request(&mut self) -> bool {
        let Some(request) = self.cache_control.take() else {
            return false;
        };
        self.models.configure_compiled_cache(
            self.subject_root.clone(),
            u64::from(request.limits.disk_mib) * cache::MIB,
            request.clear,
        );
        self.pipeline.configure_cache(request.limits);
        self.subject_persistent = request.limits.disk_mib > 0;
        if let Some(store) = &self.subject_store {
            store.persistent(self.subject_persistent);
        }
        if request.clear {
            self.invalidate();
        }
        let result = self
            .pipeline
            .maintain_cache(request.clear, || {
                self.cache_control.cancelled(request) || self.publisher.stop.load(Ordering::Relaxed)
            })
            .map(|mut usage| {
                usage.analysis_entries = self.results.len();
                usage
            });
        self.publisher.send(
            self.publisher.generation.load(Ordering::Acquire),
            Event::CacheStats(self.pipeline.stats),
        );
        self.cache_control.finish(result, request.clear);
        (self.publisher.wake)();
        true
    }

    /// Drop decoded images and cached evidence. A running analysis keeps its store.
    fn invalidate(&mut self) {
        self.pipeline.clear_memory();
        self.results.clear();
        self.subject_reset = true;
        if self.pending.is_none() {
            self.subject_store = None;
        }
    }

    fn handle_inspection(&mut self) -> bool {
        let inspect = self.inspection.lock().unwrap().take();
        let Some((epoch, request)) = inspect else {
            return false;
        };
        if self.publisher.cancelled(epoch) {
            return true;
        }
        self.enter_epoch(epoch);
        let rendered = self.pipeline.inspect_with_warning(&request, &|| {
            self.publisher.cancelled(epoch)
                || !self.inspection.lock().unwrap().accepts(epoch, &request)
        });
        let (result, decoder_warning) = match rendered {
            Ok((pixels, warning)) => (Ok(pixels), warning),
            Err(error) => (Err(format!("{error:#}")), None),
        };
        let accepted = self.inspection.lock().unwrap().accepts(epoch, &request);
        if accepted {
            self.publisher.send(
                epoch,
                Event::Inspected {
                    request,
                    result,
                    decoder_warning,
                    stats: self.pipeline.stats,
                },
            );
        }
        true
    }

    fn handle_evidence(&mut self) -> bool {
        let Ok(request) = self.evidence.try_recv() else {
            return false;
        };
        let scope = self.scope(request.epoch, request.run);
        if scope.cancelled() {
            return true;
        }
        let result = self.evidence_reply(&request, &scope);
        let _ = request.reply.try_send(result);
        true
    }

    fn evidence_reply(
        &mut self,
        request: &EvidenceRequest,
        scope: &Scope,
    ) -> anyhow::Result<EvidenceReply> {
        let media = &request.input.media;
        anyhow::ensure!(
            Pipeline::key(media)? == request.input.source,
            "Photo changed during duplicate review"
        );
        let asset = self.pipeline.analysis(media, || scope.cancelled())?;
        let faces = if request.faces {
            self.evidence_faces(request, &asset, scope)?
        } else {
            None
        };
        anyhow::ensure!(
            !scope.cancelled() && Pipeline::key(media)? == request.input.source,
            "Photo changed during duplicate review"
        );
        Ok((asset, faces, self.stats()))
    }

    /// Faces for duplicate review, upgrading cached geometry that lacks landmarks.
    fn evidence_faces(
        &mut self,
        request: &EvidenceRequest,
        asset: &pipeline::Asset,
        scope: &Scope,
    ) -> anyhow::Result<Option<Vec<faces::Face>>> {
        let mut features = request
            .store
            .get(&request.input.key)
            .context("Shared model evidence is no longer available")?;
        let complete = features
            .faces
            .as_ref()
            .is_some_and(|f| f.geometry.iter().all(|f| !f.landmarks.is_empty()));
        if !complete {
            let faces = faces::cached(
                &mut self.models,
                asset,
                &request.store,
                &request.input.source,
                features.faces.as_ref(),
                || scope.cancelled(),
            )?;
            // Retain reusable crop identities when only the missing
            // landmarks were upgraded; changed crops require re-encoding.
            let same_crops = features.faces.as_ref().is_some_and(|old| {
                old.geometry.len() == faces.geometry.len()
                    && old
                        .geometry
                        .iter()
                        .zip(&faces.geometry)
                        .all(|(a, b)| a.bbox == b.bbox && a.eyes == b.eyes)
            });
            if !same_crops {
                features.identities = None;
            }
            features.faces = Some(faces);
            request.store.put(&request.input.key, &features)?;
        }
        Ok(features.faces.map(|f| f.geometry))
    }

    fn handle_work(&mut self) -> bool {
        let message = self.priority.try_recv().ok().or_else(|| {
            if self.pending.is_some() || self.metadata_index.is_some() {
                self.images.try_recv().ok()
            } else {
                self.images.recv_timeout(Duration::from_millis(15)).ok()
            }
        });
        let Some(message) = message else {
            return false;
        };
        let epoch = message.epoch;
        if self.publisher.cancelled(epoch)
            || !message
                .work
                .is_current(&self.analysis_generation, &self.metadata_generation)
        {
            return true;
        }
        self.enter_epoch(epoch);
        match message.work {
            ImageWork::PlanDecisions {
                catalog,
                decisions,
                companions,
                policy,
            } => self.plan_decisions(epoch, &catalog, &decisions, companions, policy),
            ImageWork::IndexMetadata { run, media } => {
                self.metadata_index = Some(MetadataIndex {
                    epoch,
                    run,
                    media,
                    next: 0,
                    batch: Vec::with_capacity(64),
                });
            }
            ImageWork::Invalidate => self.invalidate(),
            ImageWork::Analyze {
                run,
                media,
                profile,
                thresholds,
                strictness,
                similarity_distance,
                scene_groups,
                model_config,
            } => {
                let selection = Selection {
                    profile,
                    thresholds,
                    strictness,
                    similarity_distance,
                    scene_groups,
                };
                self.analyze(epoch, run, media, selection, model_config)
            }
            ImageWork::Orient {
                run,
                media,
                model,
                model_config,
            } => self.orient(epoch, run, media, model, model_config),
            ImageWork::Asset(media, preview) => self.load_asset(epoch, &media, preview),
        }
        true
    }

    fn plan_decisions(
        &mut self,
        epoch: u64,
        catalog: &Catalog,
        decisions: &HashMap<MediaId, Decision>,
        companions: bool,
        policy: crate::core::metadata::WritePolicy,
    ) {
        let plan = operations::decisions_plan_with_metadata(
            catalog,
            decisions,
            companions,
            policy,
            |media| {
                self.pipeline
                    .metadata_cancellable(media, &|| self.publisher.cancelled(epoch))
            },
            || self.publisher.cancelled(epoch),
        );
        if !self.publisher.cancelled(epoch) {
            self.publisher.send(
                epoch,
                match plan {
                    Ok(plan) => Event::Planned(plan),
                    Err(error) => Event::Error(format!("Could not plan file edits: {error:#}")),
                },
            );
        }
    }

    fn load_asset(&mut self, epoch: u64, media: &Media, preview: bool) {
        let cancelled = || self.publisher.cancelled(epoch);
        let fast = (!preview)
            .then(|| self.pipeline.cached_thumbnail(media, cancelled))
            .flatten();
        let loaded = fast.map_or_else(
            || {
                super::super::crash::catch(|| {
                    self.pipeline
                        .load_cancellable(media, || self.publisher.cancelled(epoch))
                })
                .unwrap_or_else(|error| Err(anyhow::anyhow!(error)))
            },
            Ok,
        );
        match loaded {
            Ok(asset) => self.publisher.send(
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
                    stats: self.pipeline.stats,
                },
            ),
            Err(e) => self.publisher.send(
                epoch,
                Event::AssetError {
                    id: media.id,
                    preview,
                    error: format!("{e:#}"),
                },
            ),
        }
    }

    fn analyze(
        &mut self,
        epoch: u64,
        run: u64,
        media: Arc<Vec<Media>>,
        selection: Selection,
        model_config: processing::RunConfig,
    ) {
        let scope = Arc::new(self.scope(epoch, run));
        let issue = model_assets::preflight(model_assets::required(selection.profile));
        if self.blocked_by_models(&scope, issue) {
            return;
        }
        self.begin_models(&scope, model_config);
        let signature = models::analysis_signature();
        if signature != self.model_signature {
            self.results.clear();
            self.subject_store = None;
            self.model_signature = signature;
        }
        if self.evidence_unavailable(&scope) || scope.cancelled() {
            return;
        }
        self.gathered.clear();
        self.selection = selection;
        self.orientation_model = None;
        self.start(scope, media);
    }

    fn orient(
        &mut self,
        epoch: u64,
        run: u64,
        media: Arc<Vec<Media>>,
        model: PathBuf,
        model_config: processing::RunConfig,
    ) {
        let scope = Arc::new(self.scope(epoch, run));
        let mut issue = model_assets::preflight(&[model_assets::Id::Rotation]);
        // Explicit caller-selected orientation files are permitted.
        issue.missing = if Fingerprint::read(&model).is_ok_and(|fp| fp.bytes > 0) {
            Vec::new()
        } else {
            vec![model_assets::Id::Rotation]
        };
        if self.blocked_by_models(&scope, issue) {
            return;
        }
        let detector = match rotation::Detector::new(model) {
            Ok(detector) => detector,
            Err(error) => {
                scope.send_checked(Event::Error(format!(
                    "Cannot prepare rotation model: {error}"
                )));
                return;
            }
        };
        if self.evidence_unavailable(&scope) || scope.cancelled() {
            return;
        }
        self.begin_models(&scope, model_config);
        self.orientation_model = Some(detector);
        self.start(scope, media);
    }

    /// True when the run must not start: it was superseded or its models are missing.
    fn blocked_by_models(&self, scope: &Scope, issue: model_assets::Issue) -> bool {
        if scope.cancelled() {
            return true;
        }
        if !issue.is_empty() {
            scope.send_checked(Event::ModelsNeeded {
                run: scope.run,
                issue,
            });
            return true;
        }
        false
    }

    fn begin_models(&mut self, scope: &Arc<Scope>, config: processing::RunConfig) {
        let scope = scope.clone();
        self.models.begin_run(config, move || scope.cancelled());
    }

    /// True when the evidence store could not be prepared; the failure was reported.
    fn evidence_unavailable(&mut self, scope: &Scope) -> bool {
        match prepare_evidence_store(
            &mut self.subject_store,
            &self.subject_root,
            self.subject_persistent,
            &mut self.subject_reset,
        ) {
            Ok(()) => false,
            Err(error) => {
                scope.send_checked(Event::Error(format!(
                    "Cannot prepare analysis evidence: {error}"
                )));
                true
            }
        }
    }

    fn start(&mut self, scope: Arc<Scope>, media: Arc<Vec<Media>>) {
        self.groups = VisualGroups::default();
        self.hashes = HashMap::new();
        self.pending = Some(Pending {
            scope,
            media,
            index: 0,
        });
    }

    /// Advance the analysis by one photo. Returns true when the loop must start over.
    fn step_analysis(&mut self) -> bool {
        // Taken while a photo is processed so stages can borrow the worker freely.
        let Some(mut pending) = self.pending.take() else {
            return false;
        };
        if pending.scope.cancelled() {
            return true;
        }
        if pending.index >= pending.media.len() {
            self.finish_analysis(&pending.scope);
            return true;
        }
        let item = &pending.media[pending.index];
        if item.kind != MediaKind::Video && self.analyze_photo(&pending.scope, item) {
            return true;
        }
        pending.index += 1;
        pending.scope.send(Event::Progress {
            run: pending.scope.run,
            done: pending.index,
            total: pending.media.len(),
        });
        self.pending = Some(pending);
        false
    }

    fn finish_analysis(&mut self, scope: &Scope) {
        let stats = self.stats();
        let profile = self.selection.profile;
        if self.orientation_model.is_none()
            && matches!(
                profile,
                AnalysisProfile::Duplicates
                    | AnalysisProfile::Visual
                    | AnalysisProfile::Mixed
                    | AnalysisProfile::Faces
            )
        {
            self.start_similarity(scope, stats);
        } else if self.orientation_model.is_none()
            && matches!(
                profile,
                AnalysisProfile::Subjects | AnalysisProfile::Quality
            )
        {
            self.start_subjects(scope, stats);
        } else {
            scope.send(Event::AnalysisDone {
                run: scope.run,
                stats,
            });
        }
        let _ = self
            .pipeline
            .maintain_cache(false, || self.publisher.stop.load(Ordering::Relaxed));
    }

    fn start_similarity(&mut self, scope: &Scope, stats: AnalysisStats) {
        scope.send(Event::GeneralStarted { run: scope.run });
        let job = SimilarityJob {
            scene_groups: self.selection.scene_groups.clone(),
            similarity_distance: self.selection.similarity_distance,
            epoch: scope.epoch,
            run: scope.run,
            inputs: std::mem::take(&mut self.gathered.duplicate),
            regional: std::mem::take(&mut self.gathered.regional),
            sources: std::mem::take(&mut self.gathered.sources),
            profile: self.selection.profile,
            store: self.subject_store.as_ref().unwrap().clone(),
            stats,
        };
        if self.similarity.try_send(job).is_err() {
            scope.send(Event::Error(
                "Similarity worker is busy; retry analysis".into(),
            ));
        }
    }

    fn start_subjects(&mut self, scope: &Scope, stats: AnalysisStats) {
        scope.send(Event::SubjectStarted { run: scope.run });
        let job = SubjectJob {
            epoch: scope.epoch,
            run: scope.run,
            inputs: std::mem::take(&mut self.gathered.subject),
            store: self.subject_store.as_ref().unwrap().clone(),
            strictness: self.selection.strictness,
            stats,
        };
        if self.subjects.try_send(job).is_err() {
            scope.send(Event::Error(
                "Subject worker is busy; retry analysis".into(),
            ));
        }
    }

    /// Analyze one photo. Returns true when the run was superseded meanwhile.
    fn analyze_photo(&mut self, scope: &Scope, item: &Media) -> bool {
        let mut item_metadata = Metadata::default();
        let result =
            super::super::crash::catch(|| self.analyze_item(scope, item, &mut item_metadata))
                .unwrap_or_else(|error| Err(anyhow::anyhow!(error)));
        // Cancellation can interrupt preparation, measurements or
        // native models. Retire the run before publishing errors,
        // fallback groups or progress for work the user superseded.
        if scope.cancelled() {
            return true;
        }
        if let Err(e) = result {
            if matches!(
                self.selection.profile,
                AnalysisProfile::Mixed | AnalysisProfile::Faces
            ) && self.orientation_model.is_none()
            {
                self.gathered.regional.push(regional::grouping::Record {
                    id: item.id,
                    key: None,
                    portrait: false,
                    date: dates::label(item, &item_metadata),
                });
            }
            scope.send(Event::AnalysisError {
                run: scope.run,
                id: item.id,
                error: format!("{e:#}"),
            });
        }
        false
    }

    fn analyze_item(
        &mut self,
        scope: &Scope,
        item: &Media,
        item_metadata: &mut Metadata,
    ) -> anyhow::Result<()> {
        let source_key = Pipeline::key(item)?;
        if self.selection.profile == AnalysisProfile::Faces && self.orientation_model.is_none() {
            return self.group_faces(scope, item, source_key);
        }
        let cancelled = || scope.cancelled();
        if let Some(detector) = &self.orientation_model {
            let store = self
                .subject_store
                .as_ref()
                .context("Rotation cache was cleared; retry detection")?;
            let suggestion = detector.detect(
                item,
                &mut self.pipeline,
                &mut self.models,
                store,
                &cancelled,
            )?;
            anyhow::ensure!(
                !cancelled() && Pipeline::key(item)? == source_key,
                "Photo changed or analysis cancelled"
            );
            scope.send(Event::Rotation {
                run: scope.run,
                id: item.id,
                suggestion,
            });
            return Ok(());
        }
        *item_metadata = self.pipeline.metadata_cancellable(item, &cancelled)?;
        let mut it = Item {
            scope,
            media: item,
            source_key,
            prepared: Prepared::default(),
        };
        let store = self
            .subject_store
            .clone()
            .context("Analysis cache was cleared; retry analysis")?;
        self.measure(&mut it, &store)?;
        if self.selection.profile != AnalysisProfile::Technical {
            self.subject_evidence(&mut it, item_metadata)?;
        }
        if self.selection.profile == AnalysisProfile::Quality {
            self.aesthetic(&mut it, &store)?;
        }
        self.publish(&mut it)
    }

    /// Recognition vectors for face grouping; no per-photo analysis event is published.
    fn group_faces(
        &mut self,
        scope: &Scope,
        item: &Media,
        source_key: String,
    ) -> anyhow::Result<()> {
        let cancelled = || scope.cancelled();
        let store = self
            .subject_store
            .as_ref()
            .context("Face cache was cleared; retry analysis")?;
        let face_key = subjects::store::Store::face_key(&source_key, &models::face_signature());
        let recognition_key =
            subjects::store::Store::recognition_key(&face_key, &models::recognition_signature());
        let recognition = if let Some(recognition) = store.recognition(&recognition_key) {
            recognition
        } else {
            let asset = self.pipeline.analysis(item, cancelled)?;
            let old = store.get(&subjects::store::Store::key(
                &source_key,
                &self.model_signature,
            ));
            let faces = faces::cached(
                &mut self.models,
                &asset,
                store,
                &source_key,
                old.as_ref().and_then(|f| f.faces.as_ref()),
                cancelled,
            )?;
            let recognition = face_groups::describe(&mut self.models, &asset, &faces, cancelled)?;
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
        self.gathered.regional.push(regional::grouping::Record {
            id: item.id,
            key: (!recognition.vectors.is_empty()).then_some(recognition_key),
            portrait: false,
            date: None,
        });
        self.gathered.sources.push((item.clone(), source_key));
        Ok(())
    }

    /// The entry exists once `measure` succeeded for this photo.
    fn cached_mut(&mut self, key: &str) -> &mut CachedAnalysis {
        self.results
            .get_mut(key)
            .expect("measurements are cached before later stages")
    }

    fn measure(&mut self, it: &mut Item, store: &subjects::store::Store) -> anyhow::Result<()> {
        // Keep descriptors bounded independently of the decoded-image budget.
        if self.results.len() >= 20_000 && !self.results.contains_key(&it.source_key) {
            self.results.clear();
        }
        if self.results.contains_key(&it.source_key) {
            return Ok(());
        }
        let value = self.measurements(it, store)?;
        self.results.insert(
            it.source_key.clone(),
            CachedAnalysis {
                technical: value.analysis,
                hash: value.content_hash,
                dimensions: (value.width, value.height),
                embedding: None,
                aesthetic: None,
                faces: None,
            },
        );
        Ok(())
    }

    fn measurements(
        &mut self,
        it: &mut Item,
        store: &subjects::store::Store,
    ) -> anyhow::Result<analysis::Measurements> {
        let key = subjects::store::Store::measurements_key(&it.source_key);
        if let Some(value) = store.measurements(&key) {
            return Ok(value);
        }
        let scope = it.scope;
        let cancelled = || scope.cancelled();
        let asset = it.asset(&mut self.pipeline)?;
        let mut technical = analysis::analyze_cancellable(&asset, Thresholds::default(), cancelled)
            .context("Image measurements cancelled")?;
        self.technical_measurements += 1;
        self.quality_measurements += 1;
        technical.issues.clear();
        let content_hash = operations::content_hash(&it.media.path, cancelled)?;
        self.content_hashes += 1;
        anyhow::ensure!(
            !cancelled() && Pipeline::key(it.media)? == it.source_key,
            "Photo changed or analysis cancelled"
        );
        let value = analysis::Measurements {
            analysis: technical,
            width: asset.pixels.width(),
            height: asset.pixels.height(),
            content_hash,
        };
        store.put_measurements(&key, &value)?;
        Ok(value)
    }

    /// Dense, face and region evidence for every profile beyond plain technical checks.
    fn subject_evidence(&mut self, it: &mut Item, metadata: &Metadata) -> anyhow::Result<()> {
        let profile = self.selection.profile;
        let grouped = matches!(
            profile,
            AnalysisProfile::Quality | AnalysisProfile::Subjects
        );
        let store = self
            .subject_store
            .clone()
            .context("Subject cache was cleared; retry analysis")?;
        let subject_key =
            subjects::store::Store::key(&Pipeline::key(it.media)?, &self.model_signature);
        let previous = store.get(&subject_key);
        let mut changed = previous.is_none();
        let mut features = match previous {
            Some(features) => features,
            None => subjects::store::Features {
                dense: self
                    .models
                    .dense_embedding(&it.asset(&mut self.pipeline)?)?,
                faces: None,
                identities: None,
            },
        };
        let embedding = features.dense.global.clone();
        self.cached_mut(&it.source_key).embedding = Some(embedding);
        if grouped {
            changed |= self.complete_faces(it, &store, &mut features)?;
        }
        if let Some(faces) = &features.faces {
            self.cached_mut(&it.source_key).faces = Some(faces.summary());
        }
        if grouped {
            if features.identities.is_none() {
                self.face_identities(it, &mut features)?;
                changed = true;
            }
            self.gathered.subject.push(subjects::grouping::Input {
                id: it.media.id,
                path: it.media.path.clone(),
                key: subject_key.clone(),
                global: features
                    .dense
                    .global
                    .iter()
                    .map(|v| half::f16::from_f32(*v).to_f32())
                    .collect(),
                timestamp: metadata.captured_ms,
            });
        }
        if changed {
            store.put(&subject_key, &features)?;
        }
        if matches!(
            profile,
            AnalysisProfile::Duplicates | AnalysisProfile::Visual | AnalysisProfile::Mixed
        ) {
            self.similarity_regions(it, &store, &subject_key, &features)?;
        }
        Ok(())
    }

    /// Returns true when the face evidence was missing and has been computed.
    fn complete_faces(
        &mut self,
        it: &mut Item,
        store: &subjects::store::Store,
        features: &mut subjects::store::Features,
    ) -> anyhow::Result<bool> {
        let complete = features.faces.as_ref().is_some_and(|f| f.complete());
        let (scope, media) = (it.scope, it.media);
        let faces = faces::cached_lazy(
            &mut self.models,
            store,
            &it.source_key,
            features.faces.as_ref(),
            || it.prepared.get(&mut self.pipeline, media, scope),
            || scope.cancelled(),
        )?;
        if complete {
            return Ok(false);
        }
        features.identities = None;
        features.faces = Some(faces);
        Ok(true)
    }

    fn face_identities(
        &mut self,
        it: &mut Item,
        features: &mut subjects::store::Features,
    ) -> anyhow::Result<()> {
        let mut identities = Vec::new();
        for face in &features.faces.as_ref().unwrap().geometry {
            anyhow::ensure!(!it.scope.cancelled(), "Analysis cancelled");
            let crop = faces::aligned_crop(&it.asset(&mut self.pipeline)?, face)?;
            let descriptor = self
                .models
                .embedding(&crop)?
                .into_iter()
                .map(|v| half::f16::from_f32(v).to_f32())
                .collect();
            identities.push(subjects::Evidence {
                descriptor,
                bbox: face.bbox,
                area: ((face.bbox[2] - face.bbox[0]) * (face.bbox[3] - face.bbox[1])).max(0.0),
                kind: "face".into(),
            });
        }
        features.identities = Some(identities);
        Ok(())
    }

    fn similarity_regions(
        &mut self,
        it: &mut Item,
        store: &subjects::store::Store,
        subject_key: &str,
        features: &subjects::store::Features,
    ) -> anyhow::Result<()> {
        let regional_key = subjects::store::Store::regional_key(subject_key);
        if store.regional(&regional_key).is_none() {
            let scope = it.scope;
            let regions = self.models.similarity_regions(
                &it.asset(&mut self.pipeline)?,
                &features.dense.global,
                || scope.cancelled(),
            )?;
            store.put_regional(&regional_key, &regions)?;
        }
        Ok(())
    }

    fn aesthetic(&mut self, it: &mut Item, store: &subjects::store::Store) -> anyhow::Result<()> {
        if self.cached_mut(&it.source_key).aesthetic.is_some() {
            return Ok(());
        }
        let key =
            subjects::store::Store::aesthetic_key(&it.source_key, &models::aesthetic_signature());
        let value = if let Some(value) = store.aesthetic(&key) {
            value
        } else {
            let value = self.models.aesthetic(&it.asset(&mut self.pipeline)?)?;
            anyhow::ensure!(
                !it.scope.cancelled() && Pipeline::key(it.media)? == it.source_key,
                "Photo changed or analysis cancelled"
            );
            store.put_aesthetic(&key, value)?;
            value
        };
        self.cached_mut(&it.source_key).aesthetic = Some(value);
        Ok(())
    }

    /// Cached measurements for `key`, reclassified and scored for the current options.
    fn scored(&self, key: &str) -> Scored {
        let cached = self
            .results
            .get(key)
            .expect("measurements are cached before later stages");
        let mut technical = cached.technical.clone();
        technical.reclassify(self.selection.thresholds);
        technical.faces = cached.faces;
        if let Some(aesthetic) = cached.aesthetic {
            technical.aesthetic = Some(aesthetic);
            let sharpness = technical
                .quality_sharpness
                .expect("Shared measurements include whole-image sharpness");
            let blur = 0.35 * (1.0 - sharpness / 110.0).clamp(0.0, 1.0);
            let eyes = cached.faces.as_ref().map_or(0.0, |faces| {
                0.30 * faces.closed_eyes as f64 / faces.count.max(1) as f64
            });
            technical.score = 100.0 * (f64::from(aesthetic) - blur - eyes);
        }
        Scored {
            technical,
            hash: cached.hash.clone(),
            dimensions: cached.dimensions,
            embedding: cached.embedding.clone(),
        }
    }

    fn publish(&mut self, it: &mut Item) -> anyhow::Result<()> {
        let (scope, item) = (it.scope, it.media);
        let cancelled = || scope.cancelled();
        let profile = self.selection.profile;
        let Scored {
            technical,
            hash,
            dimensions,
            embedding,
        } = self.scored(&it.source_key);
        // Reload the shared metadata snapshot after any cold decode has
        // refined its dimensions. Warm evidence never needs pixel IO.
        let metadata = self.pipeline.metadata_cancellable(item, &cancelled)?;
        let duplicate = self.hashes.get(&hash).copied();
        self.hashes.entry(hash.clone()).or_insert(item.id);
        let group = if profile != AnalysisProfile::Technical {
            0 // Subject and regional groups publish only after full verification.
        } else {
            self.groups.assign(&technical)
        };
        if matches!(
            profile,
            AnalysisProfile::Duplicates | AnalysisProfile::Visual | AnalysisProfile::Mixed
        ) {
            self.gather_regional(it, &metadata, dimensions)?;
        }
        if profile == AnalysisProfile::Duplicates {
            let embedding = embedding.clone().unwrap();
            self.gather_duplicate(it, &metadata, &technical, hash, group, embedding)?;
        }
        anyhow::ensure!(
            !cancelled()
                && Pipeline::key(item)? == it.source_key
                && Fingerprint::read(&item.path)? == item.fingerprint,
            "Photo changed or analysis cancelled"
        );
        scope.send(Event::Analyzed {
            run: scope.run,
            id: item.id,
            analysis: technical,
            embedding: embedding.map(Arc::new),
            group,
            duplicate: if profile == AnalysisProfile::Duplicates {
                None
            } else {
                duplicate
            },
            metadata,
        });
        Ok(())
    }

    fn gather_regional(
        &mut self,
        it: &Item,
        metadata: &Metadata,
        dimensions: (u32, u32),
    ) -> anyhow::Result<()> {
        let source = Pipeline::key(it.media)?;
        anyhow::ensure!(
            source == it.source_key && Fingerprint::read(&it.media.path)? == it.media.fingerprint,
            "Photo changed during analysis; reopen the folder"
        );
        self.gathered.regional.push(regional::grouping::Record {
            id: it.media.id,
            key: Some(subjects::store::Store::regional_key(
                &subjects::store::Store::key(&source, &self.model_signature),
            )),
            portrait: regional::portrait(dimensions.0, dimensions.1),
            date: dates::label(it.media, metadata),
        });
        self.gathered.sources.push((it.media.clone(), source));
        Ok(())
    }

    fn gather_duplicate(
        &mut self,
        it: &Item,
        metadata: &Metadata,
        technical: &Analysis,
        hash: String,
        group: usize,
        embedding: Vec<f32>,
    ) -> anyhow::Result<()> {
        let source = Pipeline::key(it.media)?;
        anyhow::ensure!(
            source == it.source_key && Fingerprint::read(&it.media.path)? == it.media.fingerprint,
            "Photo changed during analysis; reopen the folder"
        );
        self.gathered.duplicate.push(near_duplicates::batch::Input {
            media: it.media.clone(),
            key: subjects::store::Store::key(&source, &self.model_signature),
            source,
            hash,
            group,
            embedding,
            sharpness: technical
                .sharpness
                .is_finite()
                .then_some(technical.sharpness),
            metadata_fields: metadata.metadata_fields,
        });
        Ok(())
    }

    /// One metadata read per iteration. Inspection, preview requests and
    /// analysis share this actor and get a chance before the next item.
    fn step_metadata_index(&mut self) {
        let Some(index) = &mut self.metadata_index else {
            return;
        };
        if self.publisher.cancelled(index.epoch) || self.metadata_generation.current() != index.run
        {
            self.metadata_index = None;
            return;
        }
        if let Some(media) = index.media.get(index.next) {
            let read = super::super::crash::catch(|| {
                self.pipeline
                    .metadata_cancellable(media, &|| {
                        self.publisher.cancelled(index.epoch)
                            || self.metadata_generation.current() != index.run
                    })
                    .map_err(|e| format!("{e:#}"))
            });
            index.batch.push((media.id, read.and_then(|result| result)));
            index.next += 1;
        }
        // A cancellation during a header read must not publish a partial
        // snapshot into the current browser generation.
        if self.publisher.cancelled(index.epoch) || self.metadata_generation.current() != index.run
        {
            self.metadata_index = None;
            return;
        }
        let complete = index.next == index.media.len();
        if index.batch.len() >= 64 || complete {
            self.publisher.send(
                index.epoch,
                Event::MetadataBatch {
                    run: index.run,
                    items: std::mem::take(&mut index.batch),
                    done: index.next,
                    total: index.media.len(),
                    stats: self.pipeline.stats,
                },
            );
        }
        if complete {
            self.metadata_index = None;
        }
    }
}
