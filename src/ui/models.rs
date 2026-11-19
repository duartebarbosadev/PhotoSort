//! Model setup presentation; catalog, verification and transfers are shared services.
use super::*;
use crate::core::model_assets::{self, Health, Work};
#[derive(Clone)]
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
        if self.model_wait.is_some() && !self.analysis_waits_for_models() {
            self.model_wait = None;
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
        Some(if state.running && state.mutating {
            "Downloading AI models…"
        } else if state.running {
            "Checking AI models…"
        } else if self.model_retry_ready(state) {
            "AI models ready · retry the analysis in AI models"
        } else if self.step == Step::Organize && self.group_mode == GroupMode::Faces {
            "Grouping by faces needs the face models · open AI models"
        } else {
            "AI models needed · open AI models"
        })
    }
    /// Model files are being downloaded or changed: the analysis asked for
    /// now waits and starts when that finishes, instead of doing nothing.
    /// True when it waits.
    pub(super) fn wait_for_model_work(&mut self, state: &model_assets::State) -> bool {
        if !(state.running && state.mutating) {
            return false;
        }
        // Seen running, so its end is followed even when it ends before the
        // next frame.
        self.model_work_seen = true;
        if !self.analysis_waits_for_models() {
            self.model_wait = self.root.clone().map(|root| Retry {
                root,
                step: self.step,
                mode: self.group_mode,
                preview: false,
            });
            self.set_workflow_status(
                self.step,
                "Waiting for the AI model download · the analysis starts when it finishes".into(),
            );
        }
        true
    }
    /// Analyze or Detect rotations, pressed: while the analysis already waits
    /// for a model download, it shows that download instead.
    pub(super) fn request_analysis(&mut self) {
        if self.analysis_waits_for_models() {
            self.open_model_setup();
        } else {
            self.analyze_current_workflow();
        }
    }
    /// Organize asked for a preview: an analysis waiting for the model
    /// download shows it once grouped.
    pub(super) fn preview_after_model_wait(&mut self) {
        if self.analysis_waits_for_models()
            && let Some(wait) = &mut self.model_wait
        {
            wait.preview = true;
        }
    }
    /// An analysis of this folder and step waits for the model download.
    pub(super) fn analysis_waits_for_models(&self) -> bool {
        self.model_wait
            .as_ref()
            .is_some_and(|wait| self.retry_matches(wait))
    }
    /// Called every frame: when a download or other model change ends, the
    /// analysis waiting for it starts, and a failure the panel is not open to
    /// show is reported.
    pub(super) fn follow_model_work(&mut self, state: &model_assets::State) {
        if state.running && state.mutating {
            self.model_work_seen = true;
            return;
        }
        if state.running || !std::mem::take(&mut self.model_work_seen) {
            return;
        }
        let wait = self
            .model_wait
            .take()
            .filter(|wait| self.retry_matches(wait));
        if let Some(error) = &state.error {
            if !self.model_dialog {
                self.error = Some(format!("The AI model change did not complete.\n{error}"));
            }
            if wait.is_some() {
                self.set_workflow_status(
                    self.step,
                    "The AI models are not ready · the analysis did not start".into(),
                );
            }
            return;
        }
        let Some(wait) = wait.filter(|_| !state.cancelled) else {
            if !self.model_dialog && self.model_recovery_status(state).is_none() {
                self.set_status(if state.cancelled {
                    state.message.clone()
                } else {
                    "AI models updated".into()
                });
            }
            return;
        };
        // A paused analysis whose models are still missing stays paused; its
        // footer says what is left.
        if self.model_issue.is_some() && !self.model_retry_ready(state) {
            return;
        }
        self.model_dialog = false;
        self.clear_model_retry();
        self.resume_suspended = false;
        self.analyze_current_workflow();
        if wait.preview && self.step == Step::Organize && self.analysis_run.is_some() {
            self.organize();
        }
    }
    /// While a model download runs with its panel closed: a compact progress
    /// chip that opens the panel again.
    pub(super) fn model_work_chip(&mut self, ui: &mut egui::Ui, state: &model_assets::State) {
        if !(state.running && state.mutating) || self.model_dialog {
            return;
        }
        use egui_phosphor::regular as icon;
        let percent = download_percent(state)
            .map(|p| format!(" {p}%"))
            .unwrap_or_default();
        let next = if self.analysis_waits_for_models() {
            " · analysis next"
        } else {
            ""
        };
        let text = format!("{}  AI models{percent}{next}", icon::DOWNLOAD_SIMPLE);
        let button = ui.add(
            egui::Button::new(RichText::new(&text).small().color(ACCENT))
                .frame(false)
                .min_size(egui::vec2(0.0, 22.0)),
        );
        if named(button, egui::WidgetType::Button, "AI model download")
            .on_hover_text(format!("{} · show AI models", state.message))
            .clicked()
        {
            self.open_model_setup();
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
        self.model_retry
            .as_ref()
            .is_some_and(|retry| self.retry_matches(retry))
    }
    /// The folder, step and Organize grouping are still the ones it was for.
    fn retry_matches(&self, retry: &Retry) -> bool {
        self.root.as_ref() == Some(&retry.root)
            && self.step == retry.step
            && (self.step != Step::Organize || self.group_mode == retry.mode)
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
        let error = state.error.as_deref().filter(|_| !state.running);
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
            let reserved = 200.
                + if state.running { 50. } else { 0. }
                + if error.is_some() { 90. } else { 0. };
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - reserved).max(80.))
                .show(ui, |ui| {
                    self.model_notices(ui, state);
                    if let Some(snapshot) = state.snapshot.clone() {
                        // The same two sections every time, in the same order:
                        // the runtime and what most steps use, then what only
                        // People folders and Fix Rotation use.
                        ui.label(caption("Runtime and shared models"));
                        ui.add_space(S1);
                        self.runtime_cards(ui, state, &snapshot);
                        for optional in [false, true] {
                            if optional {
                                ui.add_space(S2);
                                ui.label(caption("For People folders and Fix Rotation"));
                                ui.add_space(S1);
                            }
                            for row in snapshot
                                .rows
                                .iter()
                                .filter(|r| r.id.asset().optional == optional)
                            {
                                ui.push_id(row.id.asset().name, |ui| {
                                    self.model_card(ui, state, row)
                                });
                                ui.add_space(S1 + 2.0);
                            }
                        }
                        if let Some((path, valid)) = &snapshot.orientation_override {
                            ui.label(
                                RichText::new(if *valid {
                                    format!("Orientation override in use: {}", crate::core::text::shown(path))
                                } else {
                                    format!("Orientation override unavailable: {} · correct or remove PHOTOSORT_ROTATION_MODEL", crate::core::text::shown(path))
                                })
                                .small()
                                .color(if *valid { MUTED } else { RED }),
                            );
                        }
                    } else if !state.running {
                        // While checking, the footer's spinner says so.
                        ui.label("Not checked yet.");
                    }
                });
            // What went wrong stays in sight above the footer, not after the
            // last card.
            if let Some(error) = error {
                ui.add_space(S1);
                egui::Frame::new()
                    .fill(RED.gamma_multiply(0.12))
                    .corner_radius(R_CONTROL + 1)
                    .inner_margin(egui::Margin::symmetric(10, 6))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        egui::ScrollArea::vertical()
                            .id_salt("model_error")
                            .max_height(64.0)
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(RichText::new(error).color(RED)).wrap());
                            });
                    });
            }
            // Footer: progress or the last result on the left, actions right.
            ui.add_space(S2);
            if state.running {
                match &state.progress {
                    Some(progress) if progress.total > 0 => {
                        ui.add(
                            egui::ProgressBar::new(progress.bytes as f32 / progress.total as f32)
                                .desired_height(8.0)
                                .text(RichText::new(format!(
                                    "{}{:.1} / {:.1} MB",
                                    progress
                                        .step
                                        .map(|(n, count)| format!("{n} of {count} · "))
                                        .unwrap_or_default(),
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
                    let (works, bytes) = state
                        .snapshot
                        .as_deref()
                        .map(missing_downloads)
                        .unwrap_or_default();
                    // One action for everything missing, while two or more are.
                    let download_all = !retry && !state.running && works.len() >= 2;
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
                    } else if download_all {
                        if ui.button("Done").clicked() {
                            close = true;
                        }
                    } else if ui.add(primary("Done")).clicked() {
                        close = true;
                    }
                    if download_all {
                        self.download_all_button(ui, works, bytes);
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
            // A paused analysis continues once the download it waits for ends.
            if state.running
                && state.mutating
                && self.model_retry_valid()
                && !self.analysis_waits_for_models()
            {
                self.model_wait = self.model_retry.clone();
            }
        }
    }
    /// Messages that apply to the whole panel.
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
    /// What the paused analysis still needs, when it is the one this panel
    /// was opened for.
    fn needed_now(&self) -> Option<&model_assets::Issue> {
        self.model_issue
            .as_ref()
            .filter(|_| self.model_retry_valid())
    }
    /// One model: name, what it does, status and the one action
    /// that fixes it; size, licence, source and file details underneath.
    fn model_card(
        &mut self,
        ui: &mut egui::Ui,
        state: &model_assets::State,
        row: &model_assets::Row,
    ) {
        let asset = row.id.asset();
        let ready = matches!(row.health, Health::Verified(_));
        let transferring = state
            .running
            .then_some(state.progress.as_ref())
            .flatten()
            .filter(|p| p.id == Some(row.id));
        let editable = !state.running && !self.busy && self.analysis_run.is_none();
        let needed = self
            .needed_now()
            .is_some_and(|issue| issue.missing.contains(&row.id));
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
            (_, None) if needed => ("Needed now".to_owned(), ACCENT),
            (Health::Missing, None) => ("Missing".to_owned(), MUTED),
            (Health::Invalid(..), None) => ("Damaged".to_owned(), RED),
        };
        let hover = match &row.health {
            Health::Verified(path) => Some(crate::core::text::shown(path)),
            Health::Invalid(path, why) => {
                Some(format!("{why}\n{}", crate::core::text::shown(path)))
            }
            Health::Missing => None,
        };
        let action = (!ready && transferring.is_none()).then(|| {
            if row.bundled.is_some() {
                ("Restore bundled copy".to_owned(), None)
            } else {
                (download_label(row), Some(asset.url))
            }
        });
        let card = Card {
            title: asset.title,
            page: asset.page,
            about: asset.about,
            meta: format!("{:.1} MB · {}", asset.bytes as f64 / 1e6, asset.license),
            // Terms worth reading before choosing to download.
            note: asset.download_note.filter(|_| !ready),
            pill: (text, color, hover),
            action: action.map(|(label, hover)| (label, hover, editable)),
        };
        let clicked = asset_card(ui, card, |ui| self.model_details(ui, state, row, editable));
        if clicked {
            self.start_model_operation(if row.bundled.is_some() {
                Work::Restore(row.id)
            } else {
                Work::Download(row.id)
            });
        }
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
        // The value wraps in its own column, beside its label: a long path
        // then never starts a line further left than the value itself. One
        // with no spaces at all (a checksum) breaks where the room ends.
        let line = |ui: &mut egui::Ui, label: &str, value: &str| {
            ui.horizontal_top(|ui| {
                ui.label(RichText::new(label).small().color(MUTED));
                let mut job = egui::text::LayoutJob::simple(
                    value.to_owned(),
                    egui::TextStyle::Small.resolve(ui.style()),
                    ui.visuals().text_color(),
                    ui.available_width(),
                );
                job.wrap.break_anywhere = !value.contains(' ');
                ui.add(egui::Label::new(job).wrap());
            });
        };
        line(ui, "Origin", asset.origin);
        line(ui, "File", asset.name);
        line(ui, "SHA-256", asset.checksum());
        if let Health::Invalid(_, why) = &row.health {
            line(ui, "Problem", why);
        }
        if !ready && row.partial > 0 {
            line(
                ui,
                "Partial download",
                &format!(
                    "{:.1} of {:.1} MB kept; the download continues from there",
                    row.partial as f64 / 1e6,
                    asset.bytes as f64 / 1e6
                ),
            );
        }
        if !ready {
            ui.add_enabled_ui(editable, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if ui.small_button("Import prepared model…").clicked() {
                        self.import_model(ui.ctx(), row.id);
                    }
                    if row.bundled.is_some() && ui.small_button(download_label(row)).clicked() {
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
                    crate::core::text::shown(copy.path())
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
    /// Download all, in the footer: the runtime and every missing model, one
    /// after another.
    fn download_all_button(&mut self, ui: &mut egui::Ui, works: Vec<Work>, bytes: u64) {
        let editable = !self.busy && self.analysis_run.is_none();
        let runtime = works
            .iter()
            .any(|work| matches!(work, Work::InstallRuntime(_)));
        let models = works.len() - usize::from(runtime);
        let label = if bytes > 0 {
            format!("Download all · {}", download_size(bytes))
        } else {
            "Restore all".to_owned()
        };
        let hover = format!(
            "{}{models} {}, downloaded and verified one after another.{}",
            if runtime { "The AI runtime and " } else { "" },
            if models == 1 { "model" } else { "models" },
            if model_assets::runtime::info(model_assets::runtime::Package::Cuda).is_some() {
                " NVIDIA GPU support is a separate download."
            } else {
                ""
            }
        );
        let bounds = ui.clip_rect().x_range();
        if hover_within(ui.add_enabled(editable, primary(label)), bounds, hover).clicked() {
            self.start_model_operation(Work::All(works));
        }
    }
    /// The AI runtime as a card like the models', and NVIDIA GPU support
    /// where it can work. Nothing downloads until the user clicks; Details
    /// says how to install the runtime by hand.
    fn runtime_cards(
        &mut self,
        ui: &mut egui::Ui,
        state: &model_assets::State,
        snapshot: &model_assets::Snapshot,
    ) {
        use model_assets::runtime::{self, Package};
        let editable = !state.running && !self.busy && self.analysis_run.is_none();
        let installing = |package: Package| {
            state
                .running
                .then_some(state.progress.as_ref())
                .flatten()
                .filter(|p| p.package == Some(package))
        };
        let pill = |installed: Option<&PathBuf>, package: Package, needed: bool| match (
            installed,
            installing(package),
        ) {
            (_, Some(p)) if p.total > 0 => (
                format!(
                    "Downloading {:.0}%",
                    p.bytes as f64 * 100.0 / p.total as f64
                ),
                ACCENT,
                None,
            ),
            (_, Some(_)) => ("Working…".to_owned(), ACCENT, None),
            (Some(path), None) => (
                "Installed".to_owned(),
                GREEN,
                Some(crate::core::text::shown(path)),
            ),
            (None, None) if needed => ("Needed now".to_owned(), ACCENT, None),
            (None, None) => ("Missing".to_owned(), MUTED, None),
        };
        let needed = self.needed_now().is_some_and(|issue| issue.runtime_missing);
        let cpu = runtime::info(Package::Cpu);
        let missing = snapshot.runtime.is_none();
        let card = Card {
            title: cpu.map_or("ONNX Runtime", |info| info.title),
            page: cpu.map_or("https://onnxruntime.ai", |info| info.page),
            about: "Runs the AI models on this computer. Every AI step needs it.",
            meta: match cpu {
                Some(info) => format!(
                    "{} · {} · {}",
                    download_size(info.download_bytes()),
                    info.publisher,
                    info.license
                ),
                None if cfg!(target_os = "macos") => {
                    "Comes with the app · restore the full app bundle".to_owned()
                }
                None => "Not offered for download on this computer".to_owned(),
            },
            note: None,
            pill: pill(snapshot.runtime.as_ref(), Package::Cpu, needed),
            action: cpu
                .filter(|_| missing && installing(Package::Cpu).is_none())
                .map(|info| {
                    (
                        format!("Download {}", download_size(info.download_bytes())),
                        Some(info.archives[0].url),
                        editable,
                    )
                }),
        };
        ui.push_id("runtime", |ui| {
            if asset_card(ui, card, |ui| manual_runtime_help(ui, snapshot)) {
                self.start_model_operation(Work::InstallRuntime(Package::Cpu));
            }
        });
        ui.add_space(S1 + 2.0);
        // NVIDIA GPU support: offered only where it can work (an NVIDIA
        // driver), since it is a large download.
        let Some(cuda) = runtime::info(Package::Cuda) else {
            return;
        };
        if snapshot.cuda.is_none() && !runtime::nvidia_driver_present() {
            return;
        }
        let card = Card {
            title: "NVIDIA GPU support",
            page: cuda.page,
            about: "Runs the AI steps on the graphics card.",
            meta: format!(
                "{} · {} · {}",
                download_size(cuda.download_bytes()),
                cuda.publisher,
                cuda.license
            ),
            note: None,
            pill: pill(snapshot.cuda.as_ref(), Package::Cuda, false),
            action: (snapshot.cuda.is_none() && installing(Package::Cuda).is_none()).then(|| {
                (
                    format!("Download {}", download_size(cuda.download_bytes())),
                    None,
                    editable,
                )
            }),
        };
        let mut remove = false;
        let installed = snapshot.cuda.is_some();
        ui.push_id("runtime-cuda", |ui| {
            if asset_card(ui, card, |ui| {
                if installed {
                    ui.add_enabled_ui(editable, |ui| {
                        remove = ui.small_button("Remove GPU support").clicked();
                    });
                } else {
                    ui.label(
                        RichText::new("Needs an NVIDIA graphics driver, already installed here.")
                            .small()
                            .color(MUTED),
                    );
                }
            }) {
                self.start_model_operation(Work::InstallRuntime(Package::Cuda));
            }
        });
        if remove {
            self.start_model_operation(Work::RemoveRuntime(Package::Cuda));
        }
        ui.add_space(S1 + 2.0);
    }
}

/// What one card in the model panel shows.
struct Card<'a> {
    title: &'a str,
    page: &'a str,
    about: &'a str,
    /// Size, publisher and licence.
    meta: String,
    /// Terms to read before downloading, in the accent colour.
    note: Option<&'a str>,
    /// Status text, its colour, and what hovering it says.
    pill: (String, Color32, Option<String>),
    /// The one action that gets it working: label, hover and whether it can
    /// be pressed now.
    action: Option<(String, Option<&'a str>, bool)>,
}

/// A card: title with its Source link, the status and its one action on the
/// right, what it does, size and licence, and Details underneath. True when
/// its action was pressed.
fn asset_card(ui: &mut egui::Ui, card: Card, details: impl FnOnce(&mut egui::Ui)) -> bool {
    use egui_phosphor::regular as icon;
    let bounds = ui.clip_rect().x_range();
    let mut clicked = false;
    egui::Frame::new()
        .fill(CARD)
        .corner_radius(R_TILE)
        .inner_margin(egui::Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(card.title).font(semibold(13.0)));
                // The upstream page; it opens in the browser only when clicked.
                let link = hover_within(
                    ui.link(RichText::new(format!("Source {}", icon::ARROW_SQUARE_OUT)).small()),
                    bounds,
                    card.page,
                );
                let name = format!("Source page for {}", card.title);
                link.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Link, true, &name));
                if link.clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(card.page));
                }
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if let Some((label, hover, enabled)) = card.action {
                        let button = ui.add_enabled(enabled, primary(label));
                        let button = match hover {
                            Some(hover) => hover_within(button, bounds, hover),
                            None => button,
                        };
                        clicked = button.clicked();
                    }
                    let (text, color, hover) = card.pill;
                    let pill = status_pill(ui, &text, color);
                    if let Some(hover) = hover {
                        hover_within(pill, bounds, hover);
                    }
                });
            });
            ui.label(RichText::new(card.about).color(INK));
            ui.add(egui::Label::new(RichText::new(card.meta).small().color(MUTED)).wrap());
            if let Some(note) = card.note {
                ui.add(egui::Label::new(RichText::new(note).small().color(ACCENT)).wrap());
            }
            egui::CollapsingHeader::new(RichText::new("Details").small().color(MUTED))
                .id_salt(("model-details", card.title))
                .show(ui, details);
        });
    clicked
}

/// How far the running download is, when it counts bytes.
pub(super) fn download_percent(state: &model_assets::State) -> Option<u64> {
    state
        .progress
        .as_ref()
        .filter(|p| state.running && p.total > 0)
        .map(|p| p.bytes.min(p.total) * 100 / p.total)
}
/// "80 MB" or "3.0 GB".
fn download_size(bytes: u64) -> String {
    if bytes >= 1_000_000_000 {
        format!("{:.1} GB", bytes as f64 / 1e9)
    } else {
        format!("{:.0} MB", bytes as f64 / 1e6)
    }
}
/// The runtime card's Details: where it is installed, and the manual route
/// for installing it by hand.
fn manual_runtime_help(ui: &mut egui::Ui, snapshot: &model_assets::Snapshot) {
    let name = if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    };
    if let Some(path) = &snapshot.runtime {
        ui.label(
            RichText::new(format!("Installed at {}", crate::core::text::shown(path)))
                .small()
                .color(MUTED),
        );
    }
    let folder = crate::core::session::data_dir().join("runtime");
    ui.add(
        egui::Label::new(
            RichText::new(format!(
                "Install it myself: copy {name} from ONNX Runtime {} into {}, or set ORT_DYLIB_PATH to its full path, then Check again.",
                crate::core::model_assets::runtime::ORT_VERSION,
                crate::core::text::shown(&folder)
            ))
            .small()
            .color(MUTED),
        )
        .wrap(),
    );
}

/// A model's download action: its size, or what is left of a download that
/// was interrupted and continues.
fn download_label(row: &model_assets::Row) -> String {
    let bytes = row.id.asset().bytes;
    if row.partial > 0 {
        format!(
            "Continue download · {:.1} MB left",
            (bytes - row.partial.min(bytes)) as f64 / 1e6
        )
    } else {
        format!("Download {:.1} MB", bytes as f64 / 1e6)
    }
}

/// What Download all fetches, in order, and the bytes left to download: the
/// AI runtime when it is missing and offered for this platform, then every
/// model that is not installed, from its bundled copy where there is one.
/// Never NVIDIA GPU support: it is optional and far larger than the rest.
fn missing_downloads(snapshot: &model_assets::Snapshot) -> (Vec<Work>, u64) {
    use model_assets::runtime::{self, Package};
    let mut works = Vec::new();
    let mut bytes = 0;
    if snapshot.runtime.is_none()
        && let Some(info) = runtime::info(Package::Cpu)
    {
        works.push(Work::InstallRuntime(Package::Cpu));
        bytes += info.download_bytes();
    }
    let overridden = snapshot
        .orientation_override
        .as_ref()
        .is_some_and(|(_, valid)| *valid);
    for row in &snapshot.rows {
        if matches!(row.health, Health::Verified(_))
            || (overridden && row.id == model_assets::Id::Rotation)
        {
            continue;
        }
        if row.bundled.is_some() {
            works.push(Work::Restore(row.id));
        } else {
            works.push(Work::Download(row.id));
            let size = row.id.asset().bytes;
            bytes += size - row.partial.min(size);
        }
    }
    (works, bytes)
}

/// Hover text that stays inside the panel, whose left and right edges are
/// `bounds`: it opens toward the side of the widget with more room and wraps
/// to fit there. A plain tooltip is as wide as its longest line allows, so a
/// path or an address ran past the panel's edge.
fn hover_within(
    response: egui::Response,
    bounds: egui::Rangef,
    text: impl Into<egui::WidgetText>,
) -> egui::Response {
    static ABOVE_START: [egui::RectAlign; 1] = [egui::RectAlign::TOP_START];
    static ABOVE_END: [egui::RectAlign; 1] = [egui::RectAlign::TOP_END];
    let (to_right, to_left) = (
        bounds.max - response.rect.left(),
        response.rect.right() - bounds.min,
    );
    let (align, above, room) = if to_right >= to_left {
        (egui::RectAlign::BOTTOM_START, &ABOVE_START, to_right)
    } else {
        (egui::RectAlign::BOTTOM_END, &ABOVE_END, to_left)
    };
    // The tooltip's own frame takes some of the room.
    let width = (room - 20.0)
        .min(response.ctx.global_style().spacing.tooltip_width)
        .max(120.0);
    let mut tooltip = egui::Tooltip::for_enabled(&response).width(width);
    tooltip.popup = tooltip.popup.align(align).align_alternatives(above);
    tooltip.show(|ui| {
        ui.set_max_width(width);
        ui.add(egui::Label::new(text).wrap());
    });
    response
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
