//! Model setup presentation; catalog, verification and transfers are shared services.
use super::*;
use crate::core::model_assets::{self, Health, Work};
pub(super) struct Retry {
    root: PathBuf,
    step: Step,
    mode: GroupMode,
    preview: bool,
}
impl PhotoSortApp {
    pub(super) fn remember_model_retry(&mut self) {
        self.model_retry = self.root.clone().map(|root| Retry {
            root,
            step: self.step,
            mode: self.group_mode,
            preview: self.step == Step::Organize && self.organize_preview_requested,
        });
    }
    // A retry belongs to the failed request, never to a newly selected workflow.
    pub(super) fn clear_model_retry(&mut self) {
        let had_recovery = self.model_issue.is_some() || self.model_retry.is_some();
        self.model_issue = None;
        self.model_retry = None;
        if had_recovery && self.analysis_run.is_none() && !self.busy && !self.scanning {
            self.set_status(format!("{} items · library ready", self.media.len()));
        }
    }
    pub(super) fn discard_stale_model_retry(&mut self) {
        if self.model_retry.is_some() && !self.model_retry_valid() {
            self.clear_model_retry();
        }
    }
    pub(super) fn model_recovery_status(
        &self,
        state: &model_assets::State,
    ) -> Option<&'static str> {
        if !self.model_retry_valid()
            || self.model_issue.is_none()
            || self.analysis_run.is_some()
            || self.scanning
            || self.busy
        {
            return None;
        }
        Some(if state.running {
            "Checking model setup · your current reviews are preserved"
        } else if self.model_retry_ready(state) {
            "Models ready · open AI models to retry the paused analysis"
        } else {
            "Local AI needs model setup · your current reviews are preserved"
        })
    }
    pub(super) fn model_install_ready(&mut self) -> bool {
        let state = self.runtime.model_assets.state();
        if state.running && state.mutating {
            self.set_status("Model files are being changed · browsing remains available".into());
            false
        } else {
            true
        }
    }
    pub(super) fn open_model_setup(&mut self) {
        self.model_removal = None;
        self.model_dialog = true;
        if !self.runtime.model_assets.state().running {
            let _ = self.runtime.model_assets.start(Work::Inspect);
        }
    }
    pub(super) fn models_needed(&mut self, run: u64, issue: model_assets::Issue) {
        if self.analysis_run != Some(run) {
            return;
        }
        self.remember_model_retry();
        self.runtime.cancel_analysis();
        self.analysis_run = None;
        self.progress = None;
        self.pick_run = None;
        self.delete_run = None;
        self.rotation_run = None;
        self.organize_run = None;
        self.organize_preview_requested = false;
        self.resume_suspended = true;
        self.model_issue = Some(issue);
        self.set_status("Local AI needs model setup · your current reviews are preserved".into());
        self.open_model_setup();
    }
    fn model_retry_valid(&self) -> bool {
        self.model_retry.as_ref().is_some_and(|retry| {
            self.root.as_ref() == Some(&retry.root)
                && self.step == retry.step
                && (self.step != Step::Organize || self.group_mode == retry.mode)
        })
    }
    fn model_retry_ready(&self, state: &model_assets::State) -> bool {
        if !cfg!(feature = "onnx") || state.running || !self.model_retry_valid() {
            return false;
        }
        let Some(snapshot) = state.snapshot.as_ref() else {
            return false;
        };
        self.model_issue.as_ref().is_some_and(|issue| {
            !issue.unsupported
                && (!issue.runtime_missing || snapshot.runtime.is_some())
                && issue.missing.iter().all(|id| {
                    if *id == model_assets::Id::Rotation
                        && let Some((_, valid)) = &snapshot.orientation_override
                    {
                        return *valid;
                    }
                    snapshot
                        .rows
                        .iter()
                        .any(|row| row.id == *id && matches!(row.health, Health::Verified(_)))
                })
        })
    }
    fn retry_models(&mut self) {
        if !self.model_retry_ready(&self.runtime.model_assets.state()) {
            return;
        }
        let preview = self.model_retry.as_ref().is_some_and(|retry| retry.preview);
        self.model_dialog = false;
        self.clear_model_retry();
        self.resume_suspended = false;
        if self.step == Step::Rotation {
            self.detect_rotations();
        } else {
            self.analyze();
            if preview && self.analysis_run.is_some() {
                self.organize();
            }
        }
    }
    fn start_model_operation(&mut self, work: Work) {
        if self.busy || self.analysis_run.is_some() {
            return;
        }
        self.model_removal = None;
        if let Err(error) = self.runtime.model_assets.start(work) {
            self.error = Some(error.to_string());
        }
    }
    pub(super) fn models_dialog(&mut self, ctx: &egui::Context) {
        if !self.model_dialog {
            return;
        }
        let state = self.runtime.model_assets.state();
        self.render_model_setup(ctx, &state);
    }
    fn render_model_setup(&mut self, ctx: &egui::Context, state: &model_assets::State) {
        let mut close = false;
        egui::Modal::new(egui::Id::new("model_setup")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.).clamp(260., 580.));
            ui.heading("Local AI models");
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - if state.running {230.} else {170.}).max(80.))
                .show(ui, |ui| {
                    ui.label("Models run on this computer. Photo files are never uploaded.");
                    if !cfg!(feature = "onnx") {
                        ui.colored_label(RED, "This build has no AI inference support. Use a full PhotoSort build for AI workflows; manual review remains available.");
                    }
                    if self.model_retry_ready(state) {
                        ui.colored_label(ACCENT, "Models are ready. Retry the paused analysis when you are ready.");
                    } else if let Some(issue) = self.model_issue.as_ref().filter(|_| self.model_retry_valid()) {
                        if !issue.missing.is_empty() {
                            let needed = issue.missing.iter().map(|id| id.asset().title).collect::<Vec<_>>().join(", ");
                            ui.label(format!("Needed for this analysis: {needed}"));
                        }
                        if issue.runtime_missing && state.snapshot.as_ref().is_none_or(|snapshot| snapshot.runtime.is_none()) {
                            ui.colored_label(RED, "The native AI runtime is missing. Restore the full app bundle, then refresh. Model downloads do not install runtime libraries.");
                        }
                    }
                    if self.analysis_run.is_some() {
                        ui.horizontal_wrapped(|ui| {
                            ui.label("Stop the current analysis before changing models.");
                            if ui.button("Cancel analysis").clicked() { self.cancel_analysis(); }
                        });
                    }
                    if let Some(snapshot) = &state.snapshot {
                        let managed_bytes: u64 = snapshot.rows.iter().filter_map(|row| row.managed.as_ref()).map(|copy| copy.bytes()).sum();
                        ui.label(format!("Managed model storage: {:.1} MB", managed_bytes as f64 / 1e6));
                        ui.small("Removal affects only installed copies. Bundled models and custom model files are retained.");
                        for row in &snapshot.rows {
                            let asset = row.id.asset();
                            ui.separator();
                            ui.strong(asset.title);
                            ui.label(asset.purpose);
                            ui.small(format!("{:.1} MB · {}", asset.bytes as f64 / 1_000_000., asset.license));
                            let ready = matches!(row.health, Health::Verified(_));
                            match &row.health {
                                Health::Missing => { ui.colored_label(RED, "Not installed"); }
                                Health::Verified(path) => {
                                    ui.colored_label(ACCENT, "Verified and installed").on_hover_text(path.display().to_string());
                                }
                                Health::Invalid(path, why) => {
                                    ui.colored_label(RED, format!("Needs repair: {why}")).on_hover_text(path.display().to_string());
                                }
                            }
                            let editable = !ready && cfg!(feature = "onnx") && !state.running && !self.busy && self.analysis_run.is_none();
                            ui.add_enabled_ui(editable, |ui| {
                                ui.horizontal_wrapped(|ui| {
                                    if ui.button("Import prepared model…").clicked()
                                        && let Some(path) = rfd::FileDialog::new()
                                            .set_title(format!("Import {}", asset.name))
                                            .add_filter("ONNX", &["onnx"]).pick_file()
                                    {
                                        self.start_model_operation(Work::Import(row.id, path));
                                    }
                                    if !ready && row.bundled.is_some() && ui.button("Restore bundled copy").clicked() {
                                        self.start_model_operation(Work::Restore(row.id));
                                    }
                                    if !ready && asset.url.is_some() && cfg!(feature = "model-downloads")
                                        && ui.button(format!("Download {:.1} MB", asset.bytes as f64 / 1_000_000.)).clicked()
                                    {
                                        self.start_model_operation(Work::Download(row.id));
                                    }
                                });
                            });
                            if let Some(copy) = &row.managed {
                                ui.small(format!("Managed copy: {:.1} MB", copy.bytes() as f64 / 1e6)).on_hover_text(copy.path().display().to_string());
                                let editable = !state.running && !self.busy && self.analysis_run.is_none();
                                if self.model_removal.as_ref().is_some_and(|pending| pending.id() == row.id) {
                                    ui.label("Remove this managed copy?");
                                    if let Some(pending) = &self.model_removal {
                                        ui.small(pending.path().display().to_string());
                                    }
                                    ui.small(if row.bundled.is_some() { "The bundled copy will be checked and used if valid." }
                                        else { "New analysis using this model will require an import or download. Existing reviews are retained." });
                                    ui.horizontal_wrapped(|ui| {
                                        if ui.add_enabled(editable, egui::Button::new("Remove reviewed copy")).clicked()
                                            && let Some(pending) = self.model_removal.take() {
                                            self.start_model_operation(Work::Remove(pending));
                                        }
                                        if ui.button("Keep copy").clicked() { self.model_removal = None; }
                                    });
                                } else if ui.add_enabled(editable, egui::Button::new("Remove managed copy…")).clicked() {
                                    self.model_removal = Some(copy.clone());
                                }
                            }
                            if !ready && asset.url.is_none() {
                                ui.small("Supplied in the full app bundle. Import the prepared ONNX file from another copy of this build.");
                            }
                        }
                        ui.separator();
                        if let Some((path, valid)) = &snapshot.orientation_override {
                            ui.label(format!("Orientation override: {}", path.display()));
                            ui.small(if *valid { "The custom override takes precedence over managed orientation models." }
                                else { "The custom override is unavailable. Remove or correct PHOTOSORT_ROTATION_MODEL before retrying." });
                        }
                        ui.label(if snapshot.runtime.is_some() { "Native AI runtime found" }
                            else { "Native AI runtime not found · restore the full app bundle" });
                        if let Some(path) = snapshot.runtime.as_ref() { ui.small(path.display().to_string()); }
                    } else { ui.label(if state.running { "Checking installed model files…" } else { "Refresh to check current model files." }); }
                    if let Some(error) = &state.error { ui.colored_label(RED, error); }
                    ui.separator();
                    ui.small("Imports and downloads are verified before replacing installed models. Closing this panel leaves a transfer running; Cancel stops it.");
                });
            if state.running {
                if let Some(progress) = &state.progress {
                    if progress.total > 0 {
                        ui.add(egui::ProgressBar::new(progress.bytes as f32 / progress.total as f32)
                            .text(format!("{:.1} / {:.1} MB", progress.bytes as f64 / 1e6, progress.total as f64 / 1e6)));
                    } else { ui.horizontal(|ui| { ui.spinner(); ui.label(&state.message); }); }
                }
                ctx.request_repaint_after(Duration::from_millis(100));
            } else { ui.label(&state.message); }
            ui.horizontal_wrapped(|ui| {
                if state.running && ui.button("Cancel model operation").clicked() { self.runtime.model_assets.cancel(); }
                if ui.add_enabled(!state.running, egui::Button::new("Refresh / verify")).clicked() {
                    self.model_removal = None;
                    let _ = self.runtime.model_assets.start(Work::Inspect);
                }
                if ui.add_enabled(self.model_retry_ready(state), egui::Button::new("Retry analysis")).clicked() { self.retry_models(); }
                if ui.button("Close").clicked() { close = true; }
            });
        });
        if close {
            self.model_dialog = false;
            self.model_removal = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn app() -> (tempfile::TempDir, PhotoSortApp) {
        let dir = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        app.runtime.model_assets = crate::core::model_assets::Worker::new(
            vec![dir.path().join("assets")],
            Arc::new(|| {}),
        );
        app.root = Some(dir.path().into());
        (dir, app)
    }
    #[test]
    fn missing_models_pause_all_ai_workflows_preserve_decisions_and_ignore_stale_results() {
        for step in [
            Step::Organize,
            Step::EasyDelete,
            Step::Rotation,
            Step::PickBest,
            Step::Cull,
        ] {
            let (_dir, mut app) = app();
            app.step = step;
            app.analysis_run = Some(5);
            app.pick_run = Some(5);
            app.decisions.insert(
                MediaId(8),
                Decision {
                    rating: Some(5),
                    ..Default::default()
                },
            );
            let issue = model_assets::Issue {
                missing: vec![model_assets::Id::Scene],
                ..Default::default()
            };
            app.models_needed(4, issue.clone());
            assert!(!app.model_dialog);
            assert_eq!(app.analysis_run, Some(5));
            app.models_needed(5, issue);
            assert!(app.model_dialog && app.resume_suspended);
            assert!(app.analysis_run.is_none());
            assert!(app.pick_run.is_none());
            assert_eq!(app.decisions[&MediaId(8)].rating, Some(5));
            assert!(app.model_retry_valid());
            app.root = Some("/different".into());
            assert!(!app.model_retry_valid());
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        }
    }
    #[test]
    fn model_recovery_retains_preview_intent_only_for_its_original_context() {
        for mode in [GroupMode::Visual, GroupMode::Faces, GroupMode::Mixed] {
            let (_dir, mut app) = app();
            app.step = Step::Organize;
            app.group_mode = mode;
            app.analysis_run = Some(5);
            app.organize_run = Some((5, mode));
            app.organize_preview_requested = true;
            app.models_needed(
                5,
                model_assets::Issue {
                    missing: vec![model_assets::Id::Scene],
                    ..Default::default()
                },
            );
            assert!(app.model_retry.as_ref().unwrap().preview);
            assert!(!app.organize_preview_requested && app.organize_run.is_none());
            app.set_group_mode(GroupMode::Current);
            assert!(app.model_retry.is_none());
        }
        let (_dir, mut app) = app();
        app.step = Step::Organize;
        app.analysis_run = Some(5);
        app.models_needed(5, model_assets::Issue::default());
        assert!(
            !app.model_retry.as_ref().unwrap().preview,
            "Analyze alone must not acquire preview intent"
        );
    }
    #[test]
    fn model_dialog_fits_compact_and_wide_windows_without_loading_models_or_images() {
        let (dir, mut app) = app();
        std::fs::create_dir_all(dir.path().join("assets/models")).unwrap();
        for asset in model_assets::ALL {
            std::fs::write(
                dir.path().join("assets/models").join(asset.name),
                b"test copy",
            )
            .unwrap();
        }
        app.open_model_setup();
        let until = Instant::now() + Duration::from_secs(5);
        while app.runtime.model_assets.state().running {
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
        app.model_issue = Some(model_assets::Issue {
            missing: model_assets::ALL.iter().map(|a| a.id).collect(),
            runtime_missing: true,
            unsupported: !cfg!(feature = "onnx"),
        });
        app.model_removal = app
            .runtime
            .model_assets
            .state()
            .snapshot
            .as_ref()
            .unwrap()
            .rows[0]
            .managed
            .clone();
        assert!(app.model_removal.is_some());
        let ctx = egui::Context::default();
        crate::ui::theme::install(&ctx);
        for running in [false, true] {
            let mut state = app.runtime.model_assets.state();
            state.running = running;
            state.progress = Some(model_assets::Progress {
                id: Some(model_assets::Id::Scene),
                bytes: 10,
                total: 100,
            });
            state.error =
                Some("A transfer could not complete. Existing files remain available. ".repeat(8));
            for (width, height) in [(480., 640.), (800., 600.), (1200., 900.)] {
                for _ in 0..2 {
                    let out = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, height),
                            )),
                            ..Default::default()
                        },
                        |ctx| app.render_model_setup(ctx, &state),
                    );
                    assert!(out.shapes.len() < 500);
                    let rect = ctx
                        .memory(|memory| memory.area_rect(egui::Id::new("model_setup")))
                        .unwrap();
                    assert!(
                        rect.width() <= width && rect.height() <= height,
                        "Model panel {rect:?} must fit {width}x{height}"
                    );
                    assert!(app.analysis_run.is_none());
                }
            }
        }
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
    #[test]
    fn model_removal_uses_shared_worker_and_preserves_all_workflow_reviews() {
        for step in Step::ALL {
            let (dir, mut app) = app();
            app.step = step;
            let root = dir.path().join("assets");
            std::fs::create_dir_all(root.join("models")).unwrap();
            let path = root
                .join("models")
                .join(model_assets::Id::Detector.asset().name);
            std::fs::write(&path, b"disposable").unwrap();
            let copy = model_assets::review_managed(&root, model_assets::Id::Detector)
                .unwrap()
                .unwrap();
            app.decisions.insert(
                MediaId(8),
                Decision {
                    rating: Some(4),
                    ..Default::default()
                },
            );
            app.analysis_run = Some(3);
            app.start_model_operation(Work::Remove(copy.clone()));
            assert!(path.exists());
            assert!(app.runtime.model_assets.state().snapshot.is_none());
            app.analysis_run = None;
            app.busy = true;
            app.start_model_operation(Work::Remove(copy.clone()));
            assert!(path.exists());
            app.busy = false;
            app.start_model_operation(Work::Remove(copy));
            let deadline = Instant::now() + Duration::from_secs(5);
            while app.runtime.model_assets.state().running {
                assert!(Instant::now() < deadline);
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(app.runtime.model_assets.state().error.is_none());
            assert!(!path.exists());
            assert_eq!(app.decisions[&MediaId(8)].rating, Some(4));
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        }
    }
    #[test]
    fn retry_requires_verified_prerequisites_and_the_original_workflow_context() {
        let (_dir, mut app) = app();
        app.step = Step::Cull;
        app.remember_model_retry();
        app.model_issue = Some(model_assets::Issue {
            missing: vec![model_assets::Id::Scene],
            runtime_missing: true,
            unsupported: false,
        });
        let mut state = model_assets::State {
            snapshot: Some(Arc::new(model_assets::Snapshot {
                rows: vec![model_assets::Row {
                    id: model_assets::Id::Scene,
                    health: Health::Missing,
                    bundled: None,
                    managed: None,
                }],
                runtime: None,
                orientation_override: None,
                hash_reads: 0,
            })),
            ..Default::default()
        };
        assert!(!app.model_retry_ready(&state));
        let snapshot = Arc::make_mut(state.snapshot.as_mut().unwrap());
        snapshot.runtime = Some("runtime".into());
        snapshot.rows[0].health = Health::Verified("model".into());
        assert_eq!(app.model_retry_ready(&state), cfg!(feature = "onnx"));
        state.running = true;
        assert!(!app.model_retry_ready(&state));
        state.running = false;
        app.step = Step::Organize;
        assert!(!app.model_retry_ready(&state));
    }
    fn pending_recovery(app: &mut PhotoSortApp) {
        app.remember_model_retry();
        app.model_issue = Some(model_assets::Issue {
            missing: vec![model_assets::Id::Scene],
            runtime_missing: true,
            unsupported: false,
        });
        app.status = "Local AI needs model setup · your current reviews are preserved".into();
    }
    fn verified_setup() -> model_assets::State {
        model_assets::State {
            snapshot: Some(Arc::new(model_assets::Snapshot {
                rows: vec![model_assets::Row {
                    id: model_assets::Id::Scene,
                    health: Health::Verified("verified-model".into()),
                    bundled: None,
                    managed: None,
                }],
                runtime: Some("verified-runtime".into()),
                orientation_override: None,
                hash_reads: 0,
            })),
            ..Default::default()
        }
    }
    #[test]
    fn workflow_and_group_changes_expire_recovery_without_changing_reviews() {
        for previous in Step::ALL {
            for next in Step::ALL {
                if previous == next {
                    continue;
                }
                let (_dir, mut app) = app();
                app.step = previous;
                pending_recovery(&mut app);
                app.decisions.insert(
                    MediaId(8),
                    Decision {
                        rating: Some(4),
                        ..Default::default()
                    },
                );
                app.set_step(next);
                assert!(app.model_issue.is_none() && app.model_retry.is_none());
                assert!(!app.status.contains("model setup"));
                assert_eq!(app.decisions[&MediaId(8)].rating, Some(4));
                assert!(app.model_recovery_status(&verified_setup()).is_none());
                assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
            }
        }
        let (_dir, mut app) = app();
        app.step = Step::Organize;
        app.set_group_mode(GroupMode::Visual);
        pending_recovery(&mut app);
        app.set_group_mode(GroupMode::Visual);
        assert!(
            app.model_retry_valid(),
            "An unchanged choice preserves its retry"
        );
        app.set_group_mode(GroupMode::Current);
        assert!(app.model_issue.is_none() && app.model_retry.is_none());
        assert!(!app.status.contains("model setup"));
        app.step = Step::Cull;
        pending_recovery(&mut app);
        app.set_group_mode(GroupMode::Faces);
        assert!(
            app.model_retry_valid(),
            "Organize options do not change Cull prerequisites"
        );
    }
    #[test]
    fn new_folder_and_fresh_analysis_drop_previous_recovery() {
        let (dir, mut app) = app();
        app.media.push(Media {
            id: MediaId(8),
            path: dir.path().join("photo.jpg"),
            kind: MediaKind::Photo,
            fingerprint: Default::default(),
        });
        app.step = Step::Organize;
        pending_recovery(&mut app);
        app.analyze();
        assert!(app.analysis_run.is_some());
        assert!(app.model_issue.is_none() && app.model_retry.is_none());
        app.cancel_analysis();
        pending_recovery(&mut app);
        let next = dir.path().join("next");
        std::fs::create_dir(&next).unwrap();
        app.scan_folder(next);
        assert!(app.model_issue.is_none() && app.model_retry.is_none());
        assert!(app.scanning);
        assert!(!app.status.contains("model setup"));
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
    #[test]
    fn repaired_models_update_footer_and_dialog_without_restarting_analysis() {
        let (_dir, mut app) = app();
        app.step = Step::Cull;
        pending_recovery(&mut app);
        assert!(
            app.model_recovery_status(&model_assets::State::default())
                .unwrap()
                .contains("needs model setup")
        );
        let mut state = verified_setup();
        state.running = true;
        assert!(
            app.model_recovery_status(&state)
                .unwrap()
                .contains("Checking")
        );
        state.running = false;
        let message = app.model_recovery_status(&state).unwrap();
        assert_eq!(message.contains("Models ready"), cfg!(feature = "onnx"));
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut labels = Vec::new();
        for _ in 0..3 {
            let out = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 700.),
                    )),
                    ..Default::default()
                },
                |ctx| app.render_model_setup(ctx, &state),
            );
            labels = out
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .iter()
                .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
                .collect();
        }
        assert!(
            !labels
                .iter()
                .any(|label| label.starts_with("The native AI runtime is missing"))
        );
        if cfg!(feature = "onnx") {
            assert!(
                labels
                    .iter()
                    .any(|label| label.starts_with("Models are ready."))
            );
            assert!(
                !labels
                    .iter()
                    .any(|label| label.starts_with("Needed for this analysis"))
            );
        }
        app.set_step(Step::Organize);
        for _ in 0..3 {
            let out = ctx.run(egui::RawInput::default(), |ctx| {
                app.render_model_setup(ctx, &state)
            });
            labels = out
                .platform_output
                .accesskit_update
                .unwrap()
                .nodes
                .iter()
                .filter_map(|(_, n)| n.label().or_else(|| n.value()).map(str::to_owned))
                .collect();
        }
        assert!(
            !labels
                .iter()
                .any(|label| label.starts_with("Needed for this analysis")
                    || label.starts_with("Models are ready."))
        );
        assert!(app.analysis_run.is_none());
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
}
