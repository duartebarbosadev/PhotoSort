//! Worker events applied to the application state. Each variant has a named
//! handler; `poll` only drains the queue and schedules repaints.
use super::*;
use image::RgbaImage;
use std::ops::ControlFlow;

/// Failures with no success before analysis stops as a whole.
const SYSTEMIC_FAILURES: usize = 5;

/// What a photo's tile shape is drawn from.
fn shape(metadata: &Metadata) -> (u32, u32, u8) {
    (metadata.width, metadata.height, metadata.orientation)
}

/// What browsing filters and sorts by in a photo's metadata: its rating and
/// everything its capture time and day are read from.
fn browse_key(metadata: &Metadata) -> (u8, Option<i64>, &str, Option<&str>, Option<i64>) {
    (
        metadata.rating,
        metadata.captured_ms,
        &metadata.captured,
        metadata.grouping_date.as_deref(),
        metadata.created_ms,
    )
}

impl PhotoSortApp {
    /// `Break` ends the current poll without the end-of-frame repaint and save
    /// bookkeeping; only a failed session hand-off after Apply needs that.
    pub(super) fn handle_event(&mut self, ctx: &egui::Context, event: Event) -> ControlFlow<()> {
        match event {
            Event::MetadataBatch {
                run,
                items,
                done,
                total,
                stats,
            } => self.on_metadata_batch(run, items, done, total, stats),
            Event::Batch(batch) => self.on_batch(batch),
            Event::Opened(catalog, session, views) => self.on_opened(catalog, session, views),
            Event::FolderUnavailable { folder, problem } => {
                self.on_folder_unavailable(&folder, problem)
            }
            Event::Asset {
                id,
                preview,
                pixels,
                decoder_warning,
                metadata,
                stats,
            } => self.on_asset(ctx, (id, preview), pixels, decoder_warning, metadata, stats),
            Event::Inspected {
                request,
                result,
                decoder_warning,
                too_large,
                stats,
            } => self.on_inspected(ctx, request, (result, too_large), decoder_warning, stats),
            Event::AnalysisError { run, id, error } => self.on_analysis_error(run, id, error),
            Event::RunError { run, error } => {
                if self.analysis_run == Some(run) {
                    self.on_error(error);
                }
            }
            Event::AssetError { id, preview, error } => self.on_asset_error(id, preview, error),
            Event::Analyzed {
                run,
                id,
                analysis,
                embedding,
                group,
                duplicate,
                metadata,
            } => self.on_analyzed(run, id, (analysis, embedding), (group, duplicate), metadata),
            Event::Rotation {
                run,
                id,
                suggestion,
            } => self.on_rotation(run, id, suggestion),
            Event::Progress { run, done, total } => self.on_progress(run, done, total),
            Event::GeneralStarted { run } => self.on_general_started(run),
            Event::GeneralGroups {
                run,
                automatic,
                choices,
                groups,
            } => self.on_general_groups(run, automatic, choices, groups),
            Event::DuplicateStarted { run } => self.on_duplicate_started(run),
            Event::DuplicatePairs { run, pairs, blinks } => {
                self.on_duplicate_pairs(run, pairs, blinks)
            }
            Event::SubjectStarted { run } => self.on_subject_started(run),
            Event::FaceGroups { run, groups } => self.on_face_groups(run, groups),
            Event::SubjectGroups { run, groups } => self.on_subject_groups(run, groups),
            Event::MixedGroups { run, labels } => self.on_mixed_groups(run, labels),
            Event::GroupsEdited(result) => self.finish_group_edit(result),
            Event::ModelsNeeded { run, issue } => self.models_needed(run, issue),
            Event::AnalysisDone { run, .. } => self.on_analysis_done(run),
            Event::PlanningCancelled => self.on_planning_cancelled(),
            Event::Planned(plan) => self.on_planned(plan),
            Event::OperationsRestored(restored) => self.restore_operations(restored),
            Event::ApplyFailed(error) => self.on_apply_failed(error),
            Event::OperationsAmended(result) => self.on_operations_amended(result),
            Event::OperationsDiscarded => self.on_operations_discarded(),
            Event::Applied(report) => return self.on_applied(report),
            Event::CacheStats(stats) => self.stats = stats,
            Event::Saved => {}
            Event::Checkpointed(result) => self.on_checkpointed(result),
            Event::ViewError(error) | Event::PreferencesError(error) => {
                // A failed forget must not leave its success line behind.
                self.forget_message = None;
                self.error = Some(error);
            }
            Event::Error(e) => self.on_error(e),
            Event::WorkerStopped(worker) => self.on_worker_stopped(worker),
        }
        ControlFlow::Continue(())
    }

    fn on_metadata_batch(
        &mut self,
        run: u64,
        items: Vec<(MediaId, Result<Metadata, String>)>,
        done: usize,
        total: usize,
        stats: Stats,
    ) {
        if self.metadata_run == Some(run) {
            for (id, result) in items {
                if !self.index.contains_key(&id) {
                    continue;
                }
                match result {
                    Ok(metadata) => {
                        self.metadata.entry(id).or_insert(metadata);
                        self.update_browse_item(id);
                        self.metadata_failures.remove(&id);
                    }
                    Err(error) => {
                        self.metadata_failures.insert(id, error);
                    }
                }
            }
            self.stats = stats;
            self.metadata_progress = Some((done, total));
            if done == total {
                self.metadata_run = None;
                if self.metadata_organize_mode()
                    && self.organize_departure.is_some()
                    && self.plan.is_none()
                    && self.organize_metadata_ready()
                {
                    self.organize();
                }
            }
            self.filter_dirty = true;
        }
    }

    fn on_batch(&mut self, batch: Vec<Media>) {
        for item in batch {
            self.index.insert(item.id, self.media.len());
            self.media.push(item);
        }
        self.filter_dirty = true;
        self.set_status(format!("Discovered {} items…", self.media.len()));
    }

    /// Back to the welcome screen with a sentence that names the folder.
    fn on_folder_unavailable(
        &mut self,
        folder: &std::path::Path,
        problem: crate::core::catalog::Unavailable,
    ) {
        self.scanning = false;
        self.progress = None;
        self.root = None;
        self.output.clear();
        self.clear_status();
        // A disconnected drive leaves Recents until it is back; a folder
        // that needs permission still exists and stays listed.
        if problem == crate::core::catalog::Unavailable::Missing {
            self.missing_recent.insert(folder.to_path_buf());
        }
        let message = problem.message(folder);
        // A failed apply rescans and reports first; keep both.
        self.error = Some(match self.error.take() {
            Some(earlier) => format!("{earlier}\n{message}"),
            None => message,
        });
    }

    fn on_opened(
        &mut self,
        catalog: Arc<Catalog>,
        session: Session,
        views: crate::core::views::Saved,
    ) {
        // The scan worker already resolved aliases and relative paths.
        // Follow that identity only while destination still has its
        // initial value; preserve edits made during discovery.
        if self
            .root
            .as_ref()
            .is_some_and(|root| self.output == root.to_string_lossy())
        {
            self.output = catalog.root.display().to_string();
        }
        self.media = catalog.media.clone();
        self.index = self
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        self.manual_groups = session.manual_groups;
        self.scene_groups = session.scene_groups;
        self.decisions = session.decisions;
        self.delete_review.restore_suggested(session.suggested);
        self.pick_review.restore_suggested(session.pick_suggested);
        self.rotation_review
            .restore_staged(session.rotations_staged);
        if let Some(intent) = session.organize {
            self.organize_drafts = intent.drafts;
            self.file_names = intent.names;
            self.output = intent.output.to_string_lossy().into_owned();
        }
        self.active = self.media.first().map(|m| m.id);
        self.root = Some(catalog.root.clone());
        self.clear_status();
        self.report_scan_warnings(&catalog.warnings);
        self.preferences.remember(&catalog.root);
        self.missing_recent.remove(&catalog.root);
        self.save_preferences();
        self.catalog = Some(catalog);
        self.initialize_browse();
        self.scanning = false;
        // A folder chosen in place of another opens with clean views, even
        // ones it saved itself; reopening or reloading the same folder keeps
        // them. It goes back to the step it was left on, and a folder never
        // opened before starts in Organize. Its staged changes come from its
        // session either way.
        let views = if self
            .switched_from
            .take()
            .is_some_and(|previous| self.root.as_ref() != Some(&previous))
        {
            crate::core::views::Saved {
                step: Some(views.step.unwrap_or(Step::Organize)),
                ..Default::default()
            }
        } else {
            views
        };
        self.views_forgotten = false;
        self.load_bookmarks(views);
        if let Some(mut review) = self.rotation_reload.take() {
            review.retain(|id| self.index.contains_key(&id));
            // The library's session knows which files changed since.
            review.restore_staged(self.rotation_review.staged().clone());
            self.rotation_review = review;
            self.rotation_queue_only = true;
        }
        self.filter_dirty = true;
        // One review covered the moves and the staged choices: the moves are
        // applied, so plan the choices for the moved files and apply them
        // without a second review. A departure waits until they are applied.
        let staged_next = std::mem::take(&mut self.review_after_moves)
            && self.retry_plan.is_none()
            && self.decisions.values().any(|d| !d.is_empty());
        if staged_next {
            self.apply_planned = true;
            self.review_decisions();
            if !self.busy {
                self.apply_planned = false;
            }
        }
        if !self.apply_planned
            && let Some(destination) = self.after_apply.take()
        {
            self.continue_departure(destination);
        }
        if let Some(plan) = &self.retry_plan {
            self.set_status(format!(
                "{} unfinished operations · Review changes to resolve",
                plan.actions.len()
            ));
        }
        if !self.manual_groups.is_empty() {
            self.resolve_groups(
                self.automatic_groups.clone(),
                crate::core::manual_groups::Edit::Resolve,
                None,
                false,
            );
        }
        self.start_ready_analysis();
        self.build_previews();
        if self.step == Step::Cull || self.metadata_organize_mode() {
            self.start_metadata_index();
        }
    }

    fn on_asset(
        &mut self,
        ctx: &egui::Context,
        (id, preview): (MediaId, bool),
        pixels: Arc<RgbaImage>,
        decoder_warning: Option<String>,
        metadata: Metadata,
        stats: Stats,
    ) {
        self.requested.remove(&(id, preview));
        self.accept_image_warning(id, decoder_warning);
        self.failures.remove(&id);
        // Most decodes bring metadata the views already have; only a new
        // rating or capture time can change what they show or their order.
        let reorders = self
            .metadata
            .get(&id)
            .is_none_or(|known| browse_key(known) != browse_key(&metadata));
        self.replace_metadata(id, metadata);
        self.update_browse_item(id);
        self.metadata_failures.remove(&id);
        self.filter_dirty |= reorders && (self.filter == Filter::Rated || self.step == Step::Cull);
        self.stats = stats;
        let size = [pixels.width() as usize, pixels.height() as usize];
        let bytes = pixels.as_raw().len();
        self.evict(bytes);
        let handle = ctx.load_texture(
            format!("{}-{preview}-{}", id.0, self.tick),
            egui::ColorImage::from_rgba_unmultiplied(size, pixels.as_raw()),
            egui::TextureOptions::LINEAR,
        );
        self.textures.insert(
            (id, preview),
            Texture {
                handle,
                bytes,
                used: self.tick,
            },
        );
    }

    fn on_inspected(
        &mut self,
        ctx: &egui::Context,
        request: InspectionRequest,
        (result, too_large): (Result<Arc<RgbaImage>, String>, Option<TooLarge>),
        decoder_warning: Option<String>,
        stats: Stats,
    ) {
        self.detail_waiting.remove(&request.slot);
        if self.inspection_wanted.contains(&request) {
            if let Some(too_large) = too_large {
                self.on_too_large(request.media.id, too_large);
            }
            if result.is_ok() {
                self.accept_image_warning(request.media.id, decoder_warning);
            }
            self.stats = stats;
            let (texture, error) = match result {
                Ok(pixels) => (
                    Some(ctx.load_texture(
                        format!("detail-{}-{}", request.slot, self.tick),
                        egui::ColorImage::from_rgba_unmultiplied(
                            [pixels.width() as usize, pixels.height() as usize],
                            pixels.as_raw(),
                        ),
                        egui::TextureOptions::LINEAR,
                    )),
                    None,
                ),
                Err(error) => (None, Some(error)),
            };
            self.inspection_frames.insert(
                request.slot,
                InspectionFrame {
                    request,
                    texture,
                    error,
                    too_large,
                },
            );
        }
    }

    pub(super) fn on_analysis_error(&mut self, run: u64, id: MediaId, error: String) {
        if self.analysis_run != Some(run) {
            return;
        }
        // One unreadable photo must not stop the library's analysis. Record
        // it, keep going, and report the count when the step's results land.
        // Pick Best leaves out groups with an unscored member; Easy Delete
        // simply has no suggestion for it. Nothing unanalyzed is ever marked.
        tracing::warn!(?id, %error, "photo could not be analyzed");
        let name = self
            .index
            .get(&id)
            .map(|&i| self.media[i].name())
            .unwrap_or_default();
        // When nothing works at all (a broken model, a missing runtime), the
        // same error repeats for every photo: stop early and say so once.
        if self.analysis_successes == 0 && self.analysis_failures.len() + 1 >= SYSTEMIC_FAILURES {
            self.analysis_failures.insert(id, error.clone());
            self.runtime.cancel_analysis();
            self.analysis_run = None;
            self.drop_rotation_run();
            self.delete_run = None;
            self.pick_run = None;
            self.halt_organize_run();
            self.organize_preview_requested = false;
            self.progress = None;
            self.set_status("Analysis stopped · the first photos could not be analyzed".into());
            self.error = Some(format!(
                "Analysis stopped: the first {SYSTEMIC_FAILURES} photos could not be analyzed: {error}"
            ));
            return;
        }
        self.analysis_failures.insert(id, error);
        self.set_status(format!(
            "{name} could not be analyzed · {} so far · analysis continues",
            self.analysis_failures.len()
        ));
    }
    /// A worker task panicked. File work may have been interrupted, so it
    /// takes the full error path (an unfinished apply becomes a retry plan);
    /// other workers only release the runs and requests they own.
    fn on_worker_stopped(&mut self, worker: crate::core::jobs::Worker) {
        tracing::error!(?worker, "background task stopped");
        if worker == crate::core::jobs::Worker::Files {
            self.on_error(crate::core::jobs::WORKER_STOPPED.into());
            return;
        }
        self.runtime.cancel_analysis();
        self.analysis_run = None;
        self.drop_rotation_run();
        self.delete_run = None;
        self.pick_run = None;
        self.organize_run = None;
        self.organize_preview_requested = false;
        self.group_task = None;
        if worker == crate::core::jobs::Worker::Images {
            // Let visible tiles ask again once the worker is taking work.
            self.requested.clear();
        }
        if self.applying_plan.is_none() && self.apply_stop.is_none() {
            self.busy = false;
            self.progress = None;
        }
        // The crash notice explains a panic when the hook is installed.
        if crate::core::crash::background_panic().is_none() {
            self.error = Some(crate::core::jobs::WORKER_STOPPED.into());
        }
    }
    /// " · 2 photos could not be analyzed", for completion statuses.
    pub(super) fn failure_note(&self) -> String {
        match self.analysis_failures.len() {
            0 => String::new(),
            1 => " · 1 photo could not be analyzed (see Details)".into(),
            n => format!(" · {n} photos could not be analyzed (see Details)"),
        }
    }

    fn on_asset_error(&mut self, id: MediaId, preview: bool, error: String) {
        self.requested.remove(&(id, preview));
        self.failures.insert(id, error);
    }

    fn on_analyzed(
        &mut self,
        run: u64,
        id: MediaId,
        (analysis, embedding): (Analysis, Option<Arc<Vec<f32>>>),
        (group, duplicate): (usize, Option<MediaId>),
        metadata: Metadata,
    ) {
        if self.analysis_run == Some(run) {
            self.analysis_successes += 1;
            if let Some(embedding) = embedding {
                self.embeddings.insert(id, embedding);
            }
            self.analysis.insert(id, analysis);
            // A new score can change which photo of its Cull group is best.
            if let Some(&shown) = self.cull_groups.get(&id) {
                self.group_best_stale.insert(shown);
            }
            if group > 0 && self.cull_group_run != Some(run) {
                self.record_analysis_group(run, id, group);
            }
            self.replace_metadata(id, metadata);
            self.metadata_failures.remove(&id);
            if let Some(original) = duplicate {
                self.duplicates.insert(id, original);
            }
            self.update_browse_item(id);
            self.filter_dirty |= self.step == Step::Cull
                || matches!(
                    self.filter,
                    Filter::Issues | Filter::Duplicates | Filter::Rated
                );
        }
    }

    /// Keeps a decode's metadata. A known photo whose size or orientation
    /// differs advances `metadata_shapes`, so grids laid out from the old
    /// shape lay out again; a new photo already changes the map's size.
    fn replace_metadata(&mut self, id: MediaId, metadata: Metadata) {
        if let Some(known) = self.metadata.insert(id, metadata)
            && shape(&known) != shape(&self.metadata[&id])
        {
            self.metadata_shapes += 1;
        }
    }

    /// Picks again the best photo of each group whose scores changed, in
    /// one pass over the groups, by `publish_groups`'s rule: the highest
    /// score, ties to the lower id, for groups of two or more with a score.
    pub(super) fn refresh_group_best(&mut self) {
        if self.group_best_stale.is_empty() {
            return;
        }
        let stale = std::mem::take(&mut self.group_best_stale);
        let mut best: HashMap<usize, (f64, MediaId, usize)> = HashMap::new();
        for (id, group) in self.cull_groups.iter().filter(|(_, g)| stale.contains(g)) {
            let score = self.analysis.get(id).map_or(f64::NEG_INFINITY, |a| a.score);
            let entry = best.entry(*group).or_insert((score, *id, 0));
            entry.2 += 1;
            if score > entry.0 || (score == entry.0 && id.0 < entry.1.0) {
                (entry.0, entry.1) = (score, *id);
            }
        }
        for group in stale {
            match best.get(&group) {
                Some(&(score, id, members)) if members > 1 && score.is_finite() => {
                    self.group_best.insert(group, (id, members));
                }
                _ => {
                    self.group_best.remove(&group);
                }
            }
        }
    }

    fn on_rotation(
        &mut self,
        run: u64,
        id: MediaId,
        suggestion: crate::core::models::RotationSuggestion,
    ) {
        if self.analysis_run == Some(run) {
            self.analysis_successes += 1;
            self.rotations.insert(id, suggestion);
        }
    }

    fn on_progress(&mut self, run: u64, done: usize, total: usize) {
        if run == 0 || self.analysis_run == Some(run) {
            self.progress = Some((done, total));
        }
    }

    fn on_general_started(&mut self, run: u64) {
        if self.analysis_run == Some(run) {
            self.clear_status();
            self.progress = None;
        }
    }

    fn on_general_groups(
        &mut self,
        run: u64,
        automatic: Arc<HashMap<MediaId, usize>>,
        choices: Vec<(usize, usize, MediaId)>,
        groups: HashMap<MediaId, usize>,
    ) {
        if self.analysis_run == Some(run)
            && self.organize_run != Some((run, GroupMode::Mixed))
            && self.cull_group_run != Some(run)
            && self.organize_run != Some((run, GroupMode::Faces))
        {
            self.automatic_scenes = automatic;
            self.scene_choices = choices;
            self.visual_groups = Some(groups);
            self.filter_dirty = true;
        }
    }

    fn on_duplicate_started(&mut self, run: u64) {
        if self.analysis_run == Some(run) && self.delete_run == Some(run) {
            self.clear_status();
            self.progress = None;
        }
    }

    fn on_duplicate_pairs(
        &mut self,
        run: u64,
        pairs: Vec<crate::core::near_duplicates::batch::Pair>,
        blinks: Vec<crate::core::eyes::Blink>,
    ) {
        if self.analysis_run == Some(run) && self.delete_run == Some(run) {
            self.delete_pairs = pairs;
            self.delete_blinks = blinks;
        }
    }

    fn on_subject_started(&mut self, run: u64) {
        if self.analysis_run == Some(run) && self.cull_group_run == Some(run) {
            self.clear_status();
            self.progress = None;
        }
    }

    fn on_face_groups(&mut self, run: u64, groups: crate::core::regional::grouping::Output) {
        if self.analysis_run == Some(run) && self.organize_run == Some((run, GroupMode::Faces)) {
            self.face_groups = Some(groups);
            self.filter_dirty = true;
        }
    }

    fn on_subject_groups(&mut self, run: u64, groups: HashMap<MediaId, usize>) {
        if self.analysis_run == Some(run) && self.cull_group_run == Some(run) {
            let automatic = Arc::new(groups);
            if self.manual_groups.is_empty() {
                self.automatic_groups = automatic.clone();
                self.publish_groups(automatic);
            } else {
                self.resolve_groups(
                    automatic,
                    crate::core::manual_groups::Edit::Resolve,
                    Some(run),
                    false,
                );
            }
        }
    }

    fn on_mixed_groups(&mut self, run: u64, labels: HashMap<MediaId, String>) {
        if self.analysis_run == Some(run) && self.organize_run == Some((run, GroupMode::Mixed)) {
            self.mixed_labels = Some(labels);
        }
    }

    fn on_analysis_done(&mut self, run: u64) {
        if self.analysis_run == Some(run) {
            self.analysis_run = None;
            self.progress = None;
            if let Some((_, completed)) = self.organize_run.filter(|(owner, _)| *owner == run) {
                self.organize_run = None;
                let faces = (completed == GroupMode::Faces)
                    .then(|| {
                        self.face_groups
                            .as_ref()
                            .map(crate::core::face_groups::summarize)
                    })
                    .flatten();
                // The groups are on the board; only a people summary or
                // photos that failed need saying.
                let message = match completed {
                    GroupMode::Faces => self.face_group_status().unwrap_or_default(),
                    _ => String::new(),
                } + &self.failure_note();
                let message = message.trim_start_matches(" · ").to_owned();
                self.set_organize_status(completed, message);
                // Without a single recognized face every photo stays in place, so
                // a preview would move nothing; the message is the result.
                let preview = std::mem::take(&mut self.organize_preview_requested)
                    && faces.is_none_or(|summary| summary.photos > 0);
                if preview
                    && self.step == Step::Organize
                    && self.group_mode == completed
                    && self.organize_grouping_ready()
                {
                    self.organize();
                }
                if !self.organize_grouping_ready() && self.group_mode == completed {
                    self.organize_halted = Some(completed);
                }
            } else if self.rotation_run == Some(run) {
                self.finish_rotation_detection();
            } else if self.pick_run == Some(run) {
                if self.group_task.is_none() {
                    self.finish_pick_analysis();
                }
            } else if self.delete_run == Some(run) {
                self.finish_delete_analysis();
            } else {
                let note = self.failure_note();
                self.set_status(note.trim_start_matches(" · ").to_owned());
            }
            // A grouping chosen while this run was busy starts now.
            if self.step == Step::Organize && self.analysis_run.is_none() {
                self.start_ready_analysis();
            }
        }
    }

    fn on_planning_cancelled(&mut self) {
        self.review_after_moves = false;
        self.apply_planned = false;
        self.plan_stop = None;
        self.busy = false;
        self.organize_departure = None;
        self.clear_status();
    }

    fn on_planned(&mut self, mut plan: Plan) {
        if let Some(stop) = self.plan_stop.take() {
            if stop.load(std::sync::atomic::Ordering::Acquire) {
                self.busy = false;
                self.organize_departure = None;
                self.clear_status();
                return;
            }
            self.clear_status();
        }

        if self.rotation_plan {
            plan.description = "Apply confirmed rotation corrections".into();
        }
        self.busy = false;
        // The staged choices of an applied review: apply them, unreviewed again.
        if std::mem::take(&mut self.apply_planned) {
            if plan.actions.is_empty() {
                self.set_review_plan(None);
                if let Some(destination) = self.after_apply.take() {
                    self.continue_departure(destination);
                }
            } else {
                self.set_review_plan(Some(plan));
                self.apply_reviewed_plan();
            }
            return;
        }
        // No moves after all: review the staged choices straight away.
        if self.review_after_moves && plan.actions.is_empty() && self.organize_departure.is_none() {
            self.review_after_moves = false;
            self.review_decisions();
            return;
        }
        if self.organize_departure.is_some() && plan.actions.is_empty() {
            let destination = self.organize_departure.take().unwrap();
            self.set_review_plan(None);
            self.continue_departure(destination);
        } else {
            self.set_review_plan(Some(plan));
            self.plan_open = self.organize_departure.is_none();
        }
    }

    fn on_apply_failed(&mut self, error: String) {
        self.review_after_moves = false;
        self.apply_planned = false;
        self.applying_plan = None;
        self.apply_stop = None;
        self.rotation_plan = false;
        self.busy = false;
        self.after_apply = None;
        self.dirty = None;
        if let Some(root) = self.root.clone() {
            self.scan_folder(root);
        }
        self.error = Some(error);
    }

    fn on_operations_amended(
        &mut self,
        result: Result<crate::core::operations::recovery::Restored, String>,
    ) {
        self.busy = false;
        match result {
            Ok(restored) => {
                self.retry_edit = None;
                self.recovery_ticket = Some(restored.ticket);
                self.retry_plan = Some(restored.plan.clone());
                self.set_review_plan(Some(restored.plan));
                self.error = None;
                self.set_status(
                    "Destination saved · review the revised plan before retrying".into(),
                );
            }
            Err(error) => {
                if let Some(edit) = &mut self.retry_edit {
                    if edit.cancelled.load(std::sync::atomic::Ordering::Acquire) {
                        self.retry_edit = None;
                        self.clear_status();
                    } else {
                        edit.saving = false;
                        edit.error = Some(error);
                    }
                }
            }
        }
    }

    fn on_operations_discarded(&mut self) {
        self.busy = false;
        self.recovery_ticket = None;
        self.discard_remaining_operations();
    }

    fn on_applied(&mut self, report: crate::core::operations::Report) -> ControlFlow<()> {
        let recovery_ticket = report.recovery.clone();
        self.busy = false;
        // The pre-apply bookmark is already in the durable operation
        // context. Do not enqueue that old identity snapshot here.
        self.saved_views.relocate(&report.relocated);
        if let Some(root) = self.root.clone().filter(|_| !self.views_forgotten) {
            let _ = self.runtime.io(IoWork::SaveViews {
                root,
                views: self.saved_views.clone(),
            });
        }
        self.progress = None;
        self.applying_plan = None;
        self.apply_stop = None;
        let mut session = self.session_snapshot();
        if !self.rotation_plan {
            session.apply_report(&report, &self.media);
        } else {
            session.apply_verified_identities(&report.relocated);
        }
        self.decisions = session.decisions.clone();
        self.delete_review
            .restore_suggested(session.suggested.clone());
        self.pick_review
            .restore_suggested(session.pick_suggested.clone());
        self.manual_groups = session.manual_groups.clone();
        self.scene_groups = session.scene_groups.clone();
        self.rotation_review
            .complete(&report.applied_decisions(&self.media));
        self.rotation_review.relocate(&report.relocated);
        // The session's set is already remapped once; it wins.
        self.rotation_review
            .restore_staged(session.rotations_staged.clone());
        let remaining_rotations = self.rotation_review.clone();
        let retry_rotation = self.rotation_plan;
        self.rotation_plan = false;
        let incomplete =
            report.cancelled || !report.errors.is_empty() || !report.remaining.actions.is_empty();
        // The staged choices wait for the moves; they stay staged, not lost.
        let staged_kept = incomplete && self.review_after_moves;
        if incomplete {
            self.after_apply = None;
            self.review_after_moves = false;
        }
        let output = report.remaining.output_root.clone();
        let retry = (!report.remaining.actions.is_empty()).then_some(report.remaining);
        let message = format!(
            "{} changes applied{}",
            report.completed.len(),
            if staged_kept {
                " · unfinished operations retained · Trash marks, ratings and rotations are still staged"
            } else if incomplete {
                " · unfinished operations retained"
            } else {
                ""
            }
        );
        let _ = self.runtime.image(ImageWork::Invalidate);
        if let Some(root) = self.root.clone() {
            // Save before the epoch changes. These explicit-root jobs
            // survive navigation and execute before the following scan.
            if let Err(error) = self.runtime.io(IoWork::Save {
                root: root.clone(),
                session: session.clone(),
            }) {
                self.error = Some(error);
                self.retry_plan = retry;
                return ControlFlow::Break(());
            }
            if let Some(output) = output.as_ref().filter(|output| **output != root)
                && let Err(error) = self.runtime.io(IoWork::MergeSession {
                    root: output.clone(),
                    session,
                })
            {
                self.error = Some(error);
                self.retry_plan = retry;
                return ControlFlow::Break(());
            }
            self.scan_folder(if incomplete {
                root
            } else {
                output.unwrap_or(root)
            });
            self.rotation_reload = Some(remaining_rotations);
        }
        self.retry_plan = retry;
        self.recovery_ticket = recovery_ticket;
        if self.retry_plan.is_some() {
            self.rotation_plan = retry_rotation;
        }
        self.set_status(message);
        if !report.errors.is_empty() {
            self.error = Some(report.errors.join("\n"));
        }
        ControlFlow::Continue(())
    }

    fn on_error(&mut self, e: String) {
        self.plan_stop = None;
        self.apply_planned = false;
        self.apply_stop = None;
        self.group_task = None;
        self.resume_suspended = self.pending_bookmark.is_some();
        if let Some(plan) = self.applying_plan.take() {
            self.retry_plan = Some(plan);
        }
        self.runtime.cancel_analysis();
        self.analysis_run = None;
        // A worker panic is explained by the crash notice when the hook is
        // installed; show the plain message only if there is no report.
        if e != crate::core::jobs::WORKER_STOPPED
            || crate::core::crash::background_panic().is_none()
        {
            self.error = Some(e);
        }
        self.after_apply = None;
        if self.retry_plan.is_none() {
            self.rotation_plan = false;
        }
        self.drop_rotation_run();
        self.delete_run = None;
        self.pick_run = None;
        if let Some((_, mode)) = self.organize_run {
            self.set_organize_status(
                mode,
                format!(
                    "Grouping by {} failed · choose it again to retry",
                    mode.title().to_lowercase()
                ),
            );
        }
        self.halt_organize_run();
        self.organize_preview_requested = false;
        self.busy = false;
        self.scanning = false;
        self.progress = None;
    }

    fn on_checkpointed(&mut self, result: Result<(), String>) {
        if self.close_save_pending {
            self.close_save_pending = false;
            self.busy = false;
            match result {
                Ok(()) => {
                    self.dirty = None;
                    self.close_checkpointed = true;
                    self.close_authorized = true;
                }
                Err(error) => {
                    self.error = Some(error);
                    self.set_status("Save failed · your unfinished plan is still available".into());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_decoded_thumbnail_requeries_cull_only_when_its_metadata_changes_the_view() {
        let (_temp, mut app) = super::super::compact_cull_tests::fixture();
        let ctx = egui::Context::default();
        let id = app.media[1].id;
        let asset = |metadata: Metadata| Event::Asset {
            id,
            preview: false,
            pixels: Arc::new(RgbaImage::new(1, 1)),
            decoder_warning: None,
            metadata,
            stats: Stats::default(),
        };
        let known = app.metadata[&id].clone();
        app.filter_dirty = false;
        let _ = app.handle_event(&ctx, asset(known.clone()));
        assert!(!app.filter_dirty, "the same metadata re-ran the query");
        let rated = Metadata { rating: 1, ..known };
        let _ = app.handle_event(&ctx, asset(rated));
        assert!(app.filter_dirty);
        assert_eq!(app.metadata[&id].rating, 1);
        app.runtime.shutdown();
    }
    #[test]
    fn a_new_score_moves_its_cull_groups_best_photo() {
        let (_temp, mut app) = super::super::compact_cull_tests::fixture();
        let ids: Vec<_> = app.media.iter().take(3).map(|m| m.id).collect();
        app.cull_groups = Arc::new(ids.iter().map(|id| (*id, 1)).collect());
        for (id, score) in ids.iter().zip([0.9, 0.5, 0.2]) {
            app.analysis.insert(
                *id,
                Analysis {
                    score,
                    ..Default::default()
                },
            );
        }
        app.group_best = HashMap::from([(1, (ids[0], 3))]);
        app.analysis_run = Some(7);
        let scored = |app: &mut PhotoSortApp, id: MediaId, score: f64| {
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            let original = std::mem::replace(&mut app.runtime.events, receiver);
            sender
                .send(crate::core::jobs::Envelope {
                    epoch: app.runtime.epoch(),
                    event: Event::Analyzed {
                        run: 7,
                        id,
                        analysis: Analysis {
                            score,
                            ..Default::default()
                        },
                        embedding: None,
                        group: 0,
                        duplicate: None,
                        metadata: app.metadata[&id].clone(),
                    },
                })
                .unwrap();
            app.poll(&egui::Context::default());
            app.runtime.events = original;
        };
        // A re-analysis scores the second photo above the first...
        scored(&mut app, ids[1], 0.95);
        assert_eq!(app.group_best[&1], (ids[1], 3));
        // ...and the first again once that score drops.
        scored(&mut app, ids[1], 0.1);
        assert_eq!(app.group_best[&1], (ids[0], 3));
        app.runtime.shutdown();
    }
    #[test]
    fn choices_in_easy_delete_never_rescan_its_grid() {
        let (_temp, mut app) = super::super::compact_cull_tests::fixture();
        let id = app.media[1].id;
        for (step, rescans) in [(Step::EasyDelete, false), (Step::Cull, true)] {
            app.step = step;
            app.filter_dirty = false;
            app.decisions.entry(id).or_default().rejected ^= true;
            app.changed_items([id]);
            assert_eq!(app.filter_dirty, rescans, "{step:?}");
            app.filter_dirty = false;
            app.changed();
            assert_eq!(app.filter_dirty, rescans, "{step:?}");
        }
        app.runtime.shutdown();
    }
}
