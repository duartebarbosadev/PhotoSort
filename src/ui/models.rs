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
        use egui_phosphor::regular as icon;
        let mut close = false;
        modal(egui::Id::new("model_setup")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.).clamp(260., 640.));
            // Header: what this is, how much is installed, and privacy.
            ui.heading("Local AI models");
            if let Some(snapshot) = &state.snapshot {
                let installed = snapshot
                    .rows
                    .iter()
                    .filter(|row| matches!(row.health, Health::Verified(_)))
                    .count();
                let managed: u64 = snapshot
                    .rows
                    .iter()
                    .filter_map(|row| row.managed.as_ref())
                    .map(|copy| copy.bytes())
                    .sum();
                ui.label(
                    RichText::new(format!(
                        "{installed} of {} models installed · {:.1} MB downloaded or imported",
                        snapshot.rows.len(),
                        managed as f64 / 1e6
                    ))
                    .color(INK),
                );
            }
            ui.label(
                RichText::new(format!(
                    "{}  Models run on this computer. Photo files are never uploaded.",
                    icon::LOCK_SIMPLE
                ))
                .small()
                .color(MUTED),
            );
            ui.add_space(S2);
            let reserved = if state.running { 250. } else { 200. };
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - reserved).max(80.))
                .show(ui, |ui| {
                    self.model_notices(ui, state);
                    if let Some(snapshot) = state.snapshot.clone() {
                        let needed: Vec<model_assets::Id> = self
                            .model_issue
                            .as_ref()
                            .filter(|_| self.model_retry_valid())
                            .map(|issue| issue.missing.clone())
                            .unwrap_or_default();
                        // Group by what depends on the model: what the paused
                        // analysis needs first, then the shared analysis,
                        // then optional features.
                        let group = |row: &model_assets::Row| {
                            if needed.contains(&row.id) {
                                0
                            } else if row.id.asset().optional {
                                2
                            } else {
                                1
                            }
                        };
                        let titles = [
                            "Needed for the paused analysis",
                            "Used by Easy Delete, Pick Best, Cull and Organize",
                            "Optional features",
                        ];
                        for (index, title) in titles.iter().enumerate() {
                            let rows: Vec<_> =
                                snapshot.rows.iter().filter(|r| group(r) == index).collect();
                            if rows.is_empty() {
                                continue;
                            }
                            ui.add_space(S2);
                            ui.label(caption(title));
                            ui.add_space(S1);
                            for row in rows {
                                ui.push_id(row.id.asset().name, |ui| {
                                    self.model_card(ui, state, row)
                                });
                                ui.add_space(S1 + 2.0);
                            }
                        }
                        ui.add_space(S2);
                        ui.label(caption("Runtime"));
                        ui.add_space(S1);
                        let runtime = match snapshot.runtime.as_ref() {
                            Some(path) => {
                                ui.label(
                                    RichText::new(format!("{}  Native AI runtime found", icon::CHECK_CIRCLE))
                                        .color(GREEN)
                                        .small(),
                                )
                                .on_hover_text(path.display().to_string());
                                None
                            }
                            None => Some("Native AI runtime not found · restore the full app bundle"),
                        };
                        if let Some(text) = runtime {
                            ui.label(RichText::new(text).color(RED).small());
                        }
                        if let Some((path, valid)) = &snapshot.orientation_override {
                            ui.label(
                                RichText::new(if *valid {
                                    format!("Orientation override in use: {}", path.display())
                                } else {
                                    format!("Orientation override unavailable: {} · correct or remove PHOTOSORT_ROTATION_MODEL", path.display())
                                })
                                .small()
                                .color(if *valid { MUTED } else { RED }),
                            );
                        }
                    } else {
                        ui.label(if state.running {
                            "Checking installed model files…"
                        } else {
                            "Click Check again to check the current model files."
                        });
                    }
                    if let Some(error) = &state.error {
                        ui.add_space(S1);
                        ui.colored_label(RED, error);
                    }
                    ui.add_space(S1);
                    ui.label(
                        RichText::new("Imports and downloads are verified (size and SHA-256) before they replace an installed model. Nothing downloads unless you ask. Closing this panel leaves a transfer running; Cancel stops it.")
                            .small()
                            .color(FAINT),
                    );
                });
            // Footer: progress or the last result on the left, actions right.
            ui.add_space(S2);
            if state.running {
                match &state.progress {
                    Some(progress) if progress.total > 0 => {
                        ui.add(
                            egui::ProgressBar::new(progress.bytes as f32 / progress.total as f32)
                                .desired_height(8.0)
                                .text(RichText::new(format!(
                                    "{:.1} / {:.1} MB",
                                    progress.bytes as f64 / 1e6,
                                    progress.total as f64 / 1e6
                                )).small()),
                        );
                    }
                    _ => {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(RichText::new(&state.message).small().color(MUTED));
                        });
                    }
                }
                ctx.request_repaint_after(Duration::from_millis(100));
            }
            ui.horizontal(|ui| {
                if !state.running {
                    ui.add(egui::Label::new(RichText::new(&state.message).small().color(MUTED)).truncate());
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let retry = self.model_retry_ready(state);
                    if retry {
                        if ui
                            .add(primary("Retry analysis"))
                            .on_hover_text("Continue the analysis that was paused for these models")
                            .clicked()
                        {
                            self.retry_models();
                        }
                        if ui.button("Close").clicked() {
                            close = true;
                        }
                    } else if ui.add(primary("Done")).clicked() {
                        close = true;
                    }
                    if state.running {
                        if ui.button("Cancel").on_hover_text("Cancel model operation").clicked() {
                            self.runtime.model_assets.cancel();
                        }
                    } else if ui
                        .button(format!("{}  Check again", icon::ARROW_CLOCKWISE))
                        .on_hover_text("Verify every model file again")
                        .clicked()
                    {
                        self.model_removal = None;
                        let _ = self.runtime.model_assets.start(Work::Inspect);
                    }
                });
            });
        });
        if close {
            self.model_dialog = false;
            self.model_removal = None;
        }
    }
    /// Messages that apply to the whole panel: the build, the paused
    /// analysis, the runtime and a build without downloads, each said once.
    fn model_notices(&mut self, ui: &mut egui::Ui, state: &model_assets::State) {
        if !cfg!(feature = "onnx") {
            notice(
                ui,
                RED,
                "This build has no AI inference support. Use a full PhotoSort build for AI workflows; manual review remains available.",
            );
        }
        if self.model_retry_ready(state) {
            notice(
                ui,
                ACCENT,
                "The models this analysis needs are ready. Retry analysis to continue where it paused.",
            );
        } else if let Some(issue) = self
            .model_issue
            .as_ref()
            .filter(|_| self.model_retry_valid())
        {
            if !issue.missing.is_empty() {
                let needed = issue
                    .missing
                    .iter()
                    .map(|id| id.asset().title)
                    .collect::<Vec<_>>()
                    .join(", ");
                notice(
                    ui,
                    ACCENT,
                    &format!("Needed for this analysis: {needed}. Install them below, then retry."),
                );
            }
            if issue.runtime_missing && state.snapshot.as_ref().is_none_or(|s| s.runtime.is_none())
            {
                notice(
                    ui,
                    RED,
                    "The native AI runtime is missing. Restore the full app bundle, then check again. Model downloads do not install runtime libraries.",
                );
            }
        }
        if self.analysis_run.is_some() {
            ui.horizontal_wrapped(|ui| {
                ui.label("Stop the current analysis before changing models.");
                if ui.button("Cancel analysis").clicked() {
                    self.cancel_analysis();
                }
            });
        }
        let import_only = cfg!(feature = "onnx")
            && !cfg!(feature = "model-downloads")
            && state.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot
                    .rows
                    .iter()
                    .any(|row| !matches!(row.health, Health::Verified(_)) && row.bundled.is_none())
            });
        if import_only {
            notice(
                ui,
                MUTED,
                "This build cannot download models. Import the prepared .onnx file from another copy of this build.",
            );
        }
    }
    /// One model: name, plain-language purpose, status and the one action
    /// that fixes it; size, licence, source and file details underneath.
    fn model_card(
        &mut self,
        ui: &mut egui::Ui,
        state: &model_assets::State,
        row: &model_assets::Row,
    ) {
        use egui_phosphor::regular as icon;
        let asset = row.id.asset();
        let ready = matches!(row.health, Health::Verified(_));
        let transferring = state
            .running
            .then_some(state.progress.as_ref())
            .flatten()
            .filter(|p| p.id == Some(row.id));
        let editable =
            cfg!(feature = "onnx") && !state.running && !self.busy && self.analysis_run.is_none();
        egui::Frame::new()
            .fill(CARD)
            .corner_radius(R_TILE)
            .inner_margin(egui::Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(asset.title).font(semibold(13.0)));
                    // The upstream page; it opens in the browser only when clicked.
                    let link = ui
                        .link(RichText::new(format!("Source {}", icon::ARROW_SQUARE_OUT)).small())
                        .on_hover_text(asset.page);
                    let name = format!("Source page for {}", asset.title);
                    link.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::WidgetType::Link, true, &name)
                    });
                    if link.clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(asset.page));
                    }
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        // The one action that gets this model working.
                        if !ready && transferring.is_none() {
                            ui.add_enabled_ui(editable, |ui| {
                                if row.bundled.is_some() {
                                    if ui.add(primary("Restore bundled copy")).clicked() {
                                        self.start_model_operation(Work::Restore(row.id));
                                    }
                                } else if cfg!(feature = "model-downloads") {
                                    if ui
                                        .add(primary(format!(
                                            "Download {:.1} MB",
                                            asset.bytes as f64 / 1e6
                                        )))
                                        .on_hover_text(asset.url)
                                        .clicked()
                                    {
                                        self.start_model_operation(Work::Download(row.id));
                                    }
                                } else if ui.button("Import prepared model…").clicked() {
                                    self.import_model(row.id);
                                }
                            });
                        }
                        let (text, color) = match (&row.health, transferring) {
                            (_, Some(p)) if p.total > 0 => (
                                format!(
                                    "Downloading {:.0}%",
                                    p.bytes as f64 * 100.0 / p.total as f64
                                ),
                                ACCENT,
                            ),
                            (_, Some(_)) => ("Working…".to_owned(), ACCENT),
                            (Health::Verified(_), None) => ("Installed".to_owned(), GREEN),
                            (Health::Missing, None) => ("Missing".to_owned(), MUTED),
                            (Health::Invalid(..), None) => ("Damaged".to_owned(), RED),
                        };
                        let pill = status_pill(ui, &text, color);
                        match &row.health {
                            Health::Verified(path) => {
                                pill.on_hover_text(path.display().to_string());
                            }
                            Health::Invalid(path, why) => {
                                pill.on_hover_text(format!("{why}\n{}", path.display()));
                            }
                            Health::Missing => {}
                        }
                    });
                });
                ui.label(RichText::new(asset.about).color(INK));
                ui.add(
                    egui::Label::new(
                        RichText::new(format!(
                            "{} · {:.1} MB · {}",
                            asset.purpose,
                            asset.bytes as f64 / 1e6,
                            asset.license
                        ))
                        .small()
                        .color(MUTED),
                    )
                    .wrap(),
                );
                // Terms worth reading before choosing to download.
                if let Some(note) = asset.download_note
                    && !ready
                    && cfg!(feature = "model-downloads")
                {
                    ui.add(egui::Label::new(RichText::new(note).small().color(ACCENT)).wrap());
                }
                egui::CollapsingHeader::new(RichText::new("Details").small().color(MUTED))
                    .id_salt(("model-details", asset.name))
                    .show(ui, |ui| self.model_details(ui, state, row, editable));
            });
    }
    fn model_details(
        &mut self,
        ui: &mut egui::Ui,
        state: &model_assets::State,
        row: &model_assets::Row,
        editable: bool,
    ) {
        let asset = row.id.asset();
        let ready = matches!(row.health, Health::Verified(_));
        let line = |ui: &mut egui::Ui, label: &str, value: &str| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new(label).small().color(MUTED));
                ui.label(RichText::new(value).small());
            });
        };
        line(ui, "Origin", asset.origin);
        line(ui, "File", asset.name);
        line(ui, "SHA-256", asset.checksum());
        let downloadable = cfg!(feature = "model-downloads");
        line(
            ui,
            "Download",
            if downloadable {
                "Public pinned file, verified after download"
            } else {
                "Not included in this build · import a prepared copy"
            },
        );
        if let Health::Invalid(_, why) = &row.health {
            line(ui, "Problem", why);
        }
        if !ready {
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal_wrapped(|ui| {
                    // The card's main action is Import when nothing else
                    // can restore this model; do not offer it twice.
                    let import_is_primary = row.bundled.is_none() && !downloadable;
                    if !import_is_primary && ui.small_button("Import prepared model…").clicked() {
                        self.import_model(row.id);
                    }
                    if row.bundled.is_some()
                        && downloadable
                        && ui
                            .small_button(format!("Download {:.1} MB", asset.bytes as f64 / 1e6))
                            .clicked()
                    {
                        self.start_model_operation(Work::Download(row.id));
                    }
                });
            });
        }
        if let Some(copy) = &row.managed {
            line(
                ui,
                "Managed copy",
                &format!(
                    "{:.1} MB · {}",
                    copy.bytes() as f64 / 1e6,
                    copy.path().display()
                ),
            );
            let editable = !state.running && !self.busy && self.analysis_run.is_none();
            if self.model_removal.as_ref().is_some_and(|pending| pending.id() == row.id) {
                ui.label(RichText::new("Remove this managed copy?").color(INK));
                ui.label(
                    RichText::new(if row.bundled.is_some() {
                        "The bundled copy will be checked and used if valid."
                    } else {
                        "New analysis using this model will need an import or download. Existing reviews are kept."
                    })
                    .small()
                    .color(MUTED),
                );
                ui.horizontal_wrapped(|ui| {
                    if ui.add_enabled(editable, egui::Button::new("Remove reviewed copy")).clicked()
                        && let Some(pending) = self.model_removal.take()
                    {
                        self.start_model_operation(Work::Remove(pending));
                    }
                    if ui.button("Keep copy").clicked() {
                        self.model_removal = None;
                    }
                });
            } else if ui
                .add_enabled(editable, egui::Button::new("Remove managed copy…").small())
                .on_hover_text("Removal affects only this installed copy. Bundled models and custom model files are kept.")
                .clicked()
            {
                self.model_removal = Some(copy.clone());
            }
        }
    }
    fn import_model(&mut self, id: model_assets::Id) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title(format!("Import {}", id.asset().name))
            .add_filter("ONNX", &["onnx"])
            .pick_file()
        {
            self.start_model_operation(Work::Import(id, path));
        }
    }
}

/// A rounded message line in the model panel.
fn notice(ui: &mut egui::Ui, color: Color32, text: &str) {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.12))
        .corner_radius(R_CONTROL + 1)
        .inner_margin(egui::Margin::symmetric(10, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.add(
                egui::Label::new(RichText::new(text).color(if color == MUTED {
                    INK
                } else {
                    color
                }))
                .wrap(),
            );
        });
    ui.add_space(S1);
}
/// A small status label on a tinted pill.
fn status_pill(ui: &mut egui::Ui, text: &str, color: Color32) -> egui::Response {
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_owned(), medium(11.0), color);
    let size = galley.size() + egui::vec2(16.0, 6.0);
    let (rect, response) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter()
        .rect_filled(rect, size.y / 2.0, color.gamma_multiply(0.16));
    ui.painter()
        .galley(rect.min + egui::vec2(8.0, 3.0), galley, color);
    let label = text.to_owned();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &label));
    response
}

#[cfg(test)]
#[path = "models/tests.rs"]
mod tests;
