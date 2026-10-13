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
        modal(egui::Id::new("model_setup")).show(ctx, |ui| {
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
#[path = "models/tests.rs"]
mod tests;
