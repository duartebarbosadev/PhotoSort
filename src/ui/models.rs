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
        // The recovery message has nothing left to say.
        if had_recovery && self.analysis_run.is_none() && !self.busy && !self.scanning {
            self.set_status(String::new());
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
            "Checking AI models…"
        } else if self.model_retry_ready(state) {
            "AI models ready · retry the analysis in AI models"
        } else if self.step == Step::Organize && self.group_mode == GroupMode::Faces {
            "Grouping by faces needs the face models · open AI models"
        } else {
            "AI models needed · open AI models"
        })
    }
    pub(super) fn model_install_ready(&mut self) -> bool {
        let state = self.runtime.model_assets.state();
        if state.running && state.mutating {
            self.set_status("Model files are being changed · try again when done".into());
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
        let grouping = self.organize_run.map(|(_, mode)| mode);
        self.remember_model_retry();
        self.runtime.cancel_analysis();
        self.analysis_run = None;
        self.progress = None;
        self.pick_run = None;
        self.delete_run = None;
        self.drop_rotation_run();
        self.organize_run = None;
        self.organize_preview_requested = false;
        self.resume_suspended = true;
        self.model_issue = Some(issue);
        match grouping {
            Some(mode) => self.set_organize_status(
                mode,
                format!(
                    "Grouping by {} needs AI models · download them in AI models",
                    mode.title().to_lowercase()
                ),
            ),
            None => self.set_status("AI models needed · open AI models".into()),
        }
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
        if state.running || !self.model_retry_valid() {
            return false;
        }
        let Some(snapshot) = state.snapshot.as_ref() else {
            return false;
        };
        self.model_issue.as_ref().is_some_and(|issue| {
            (!issue.runtime_missing || snapshot.runtime.is_some())
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
            ui.heading("AI models");
            ui.label(
                RichText::new(format!(
                    "{}  Models run on this computer. Photos are never uploaded.",
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
                        // then People folders and Fix Rotation.
                        let group = |row: &model_assets::Row| {
                            if needed.contains(&row.id) {
                                0
                            } else if row.id.asset().optional {
                                2
                            } else {
                                1
                            }
                        };
                        // Without the runtime no model can run, so its offer comes first.
                        let runtime_first = snapshot.runtime.is_none();
                        if runtime_first {
                            ui.add_space(S2);
                            ui.label(caption("Runtime"));
                            ui.add_space(S1);
                            self.runtime_section(ui, state, &snapshot);
                        }
                        let titles = [
                            "Needed for the paused analysis",
                            "Used by Easy Delete, Pick Best, Cull and Organize",
                            "Used by People folders and Fix Rotation",
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
                        if !runtime_first {
                            ui.add_space(S2);
                            ui.label(caption("Runtime"));
                            ui.add_space(S1);
                            self.runtime_section(ui, state, &snapshot);
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
                    } else if !state.running {
                        // While checking, the footer's spinner says so.
                        ui.label("Not checked yet.");
                    }
                    if let Some(error) = &state.error {
                        ui.add_space(S1);
                        ui.colored_label(RED, error);
                    }
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
                        if ui.button("Cancel").clicked() {
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
    /// Messages that apply to the whole panel. A missing runtime is said
    /// once, in the Runtime section, with its download.
    fn model_notices(&mut self, ui: &mut egui::Ui, state: &model_assets::State) {
        if self.model_retry_ready(state) {
            notice(
                ui,
                ACCENT,
                "The models are ready. Retry analysis to continue.",
            );
        }
        if self.analysis_run.is_some() {
            ui.horizontal_wrapped(|ui| {
                ui.label("Stop the current analysis before changing models.");
                if ui.button("Cancel analysis").clicked() {
                    self.cancel_analysis();
                }
            });
        }
    }
    /// One model: name, what it does, status and the one action
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
        let editable = !state.running && !self.busy && self.analysis_run.is_none();
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
                                } else if ui
                                    .add(primary(format!(
                                        "Download {:.1} MB",
                                        asset.bytes as f64 / 1e6
                                    )))
                                    .on_hover_text(asset.url)
                                    .clicked()
                                {
                                    self.start_model_operation(Work::Download(row.id));
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
                            "{:.1} MB · {}",
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
        if let Health::Invalid(_, why) = &row.health {
            line(ui, "Problem", why);
        }
        if !ready {
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Import prepared model…").clicked() {
                        self.import_model(ui.ctx(), row.id);
                    }
                    if row.bundled.is_some()
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
            if self
                .model_removal
                .as_ref()
                .is_some_and(|pending| pending.id() == row.id)
            {
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
                    if ui
                        .add_enabled(editable, egui::Button::new("Remove reviewed copy"))
                        .clicked()
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
                .clicked()
            {
                self.model_removal = Some(copy.clone());
            }
        }
    }
    fn import_model(&mut self, ctx: &egui::Context, id: model_assets::Id) {
        let dialog = rfd::FileDialog::new()
            .set_title(format!("Import {}", id.asset().name))
            .add_filter("ONNX", &["onnx"]);
        if let Some(path) = native_dialog::pick(ctx, dialog, rfd::FileDialog::pick_file) {
            self.start_model_operation(Work::Import(id, path));
        }
    }
}

impl PhotoSortApp {
    /// The AI runtime: found, or offered for download with its size and source,
    /// and NVIDIA GPU support where a driver is present. Nothing downloads until
    /// the user clicks; "Install it myself" explains the manual route instead.
    fn runtime_section(
        &mut self,
        ui: &mut egui::Ui,
        state: &model_assets::State,
        snapshot: &model_assets::Snapshot,
    ) {
        use egui_phosphor::regular as icon;
        use model_assets::runtime::{self, Package};
        let editable = !state.running && !self.busy && self.analysis_run.is_none();
        let installing = state
            .progress
            .as_ref()
            .and_then(|p| p.package.map(|k| (k, p)));
        let progress_text = |package: Package| {
            installing.filter(|(k, _)| *k == package).map(|(_, p)| {
                if p.total > 0 {
                    format!(
                        "Downloading {:.0}% of {}",
                        p.bytes as f64 * 100.0 / p.total as f64,
                        download_size(p.total)
                    )
                } else {
                    "Working…".to_owned()
                }
            })
        };
        match snapshot.runtime.as_ref() {
            Some(path) => {
                ui.label(
                    RichText::new(format!("{}  AI runtime found", icon::CHECK_CIRCLE))
                        .color(GREEN)
                        .small(),
                )
                .on_hover_text(path.display().to_string());
            }
            None => match runtime::info(Package::Cpu) {
                Some(info) => {
                    notice(
                        ui,
                        ACCENT,
                        &format!(
                            "The AI steps need {} from {} ({:.0} MB).",
                            info.title,
                            info.publisher,
                            info.download_bytes() as f64 / 1e6
                        ),
                    );
                    ui.horizontal_wrapped(|ui| {
                        if let Some(text) = progress_text(Package::Cpu) {
                            ui.label(RichText::new(text).color(ACCENT).small());
                        } else {
                            ui.add_enabled_ui(editable, |ui| {
                                if ui
                                    .add(primary(format!("Download {}", info.title)))
                                    .on_hover_text(info.archives[0].url)
                                    .clicked()
                                {
                                    self.start_model_operation(Work::InstallRuntime(Package::Cpu));
                                }
                            });
                        }
                        manual_runtime_help(ui);
                    });
                }
                _ => {
                    ui.label(
                        RichText::new(if cfg!(target_os = "macos") {
                            "AI runtime not found · restore the full app bundle"
                        } else {
                            "AI runtime not found"
                        })
                        .color(RED)
                        .small(),
                    );
                    manual_runtime_help(ui);
                }
            },
        }
        // NVIDIA GPU support: offered only where it can work (an NVIDIA driver),
        // since it is a large download.
        let Some(cuda) = runtime::info(Package::Cuda) else {
            return;
        };
        ui.add_space(S1);
        if let Some(path) = snapshot.cuda.as_ref() {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(format!(
                        "{}  NVIDIA GPU support installed",
                        icon::CHECK_CIRCLE
                    ))
                    .color(GREEN)
                    .small(),
                )
                .on_hover_text(path.display().to_string());
                ui.add_enabled_ui(editable, |ui| {
                    if ui.small_button("Remove").clicked() {
                        self.start_model_operation(Work::RemoveRuntime(Package::Cuda));
                    }
                });
            });
        } else if runtime::nvidia_driver_present() {
            ui.label(RichText::new("NVIDIA GPU").font(semibold(13.0)));
            ui.label(
                RichText::new(format!(
                    "{} from {} runs the AI steps on the graphics card.",
                    cuda.title, cuda.publisher
                ))
                .small()
                .color(MUTED),
            );
            ui.horizontal_wrapped(|ui| {
                if let Some(text) = progress_text(Package::Cuda) {
                    ui.label(RichText::new(text).color(ACCENT).small());
                } else {
                    ui.add_enabled_ui(editable, |ui| {
                        if ui
                            .button(format!(
                                "Download GPU support · {:.1} GB",
                                cuda.download_bytes() as f64 / 1e9
                            ))
                            .clicked()
                        {
                            self.start_model_operation(Work::InstallRuntime(Package::Cuda));
                        }
                    });
                }
            });
        }
    }
}

/// "80 MB" or "3.0 GB".
fn download_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}
/// The manual route: where the runtime library goes when installed by hand.
fn manual_runtime_help(ui: &mut egui::Ui) {
    let name = if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    };
    let folder = crate::core::session::data_dir().join("runtime");
    ui.label(RichText::new("Install it myself").small().color(MUTED))
        .on_hover_text(format!(
            "Copy {name} from ONNX Runtime {} into {}, or set ORT_DYLIB_PATH to its full path, then check again.",
            crate::core::model_assets::runtime::ORT_VERSION,
            folder.display()
        ));
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
