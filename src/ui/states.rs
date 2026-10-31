//! Full-panel states: work in progress, and empty views with the obvious
//! next action, so no step ever ends on a bare sentence.
use super::*;
use crate::core::delete_review::Category;
use egui_phosphor::regular as icon;

impl PhotoSortApp {
    /// Keeps the start time of the current progress run for elapsed/estimate.
    pub(super) fn track_progress_clock(&mut self) {
        match (self.progress, self.progress_started) {
            (Some(_), None) => {
                self.progress_started = Some(Instant::now());
                self.progress_prep = Default::default();
            }
            (None, Some(_)) => self.progress_started = None,
            _ => {}
        }
        match (self.preparing_model().is_some(), self.progress_prep.1) {
            (true, None) => self.progress_prep.1 = Some(Instant::now()),
            (false, Some(since)) => {
                self.progress_prep.0 += since.elapsed();
                self.progress_prep.1 = None;
            }
            _ => {}
        }
    }

    /// "1:05 elapsed · about 2 min left", once there is enough to estimate.
    /// Model preparation is not photo work: it is left out of the estimate,
    /// and no estimate shows while a model prepares.
    pub(super) fn progress_timing(&self) -> Option<String> {
        let (done, total) = self.progress?;
        let elapsed = self.progress_started?.elapsed();
        let working = elapsed.saturating_sub(self.progress_prep.0);
        let mut text = format!("{} elapsed", clock(elapsed));
        if self.progress_prep.1.is_none()
            && done >= 3
            && done < total
            && working >= Duration::from_secs(3)
        {
            let left = working.mul_f64((total - done) as f64 / done as f64);
            text.push_str(&format!(" · about {} left", rough(left)));
        }
        Some(text)
    }

    /// A model the running analysis is still preparing (Core ML compiles a
    /// model the first time it is used).
    pub(super) fn preparing_model(&self) -> Option<crate::core::processing::ModelStatus> {
        if self.analysis_run.is_none() && self.rotation_run.is_none() {
            return None;
        }
        self.runtime
            .processing
            .status()
            .into_iter()
            .find(|s| s.preparing)
    }

    /// What preparing the current model is doing, for the card and status bar.
    pub(super) fn prep_note(&self) -> Option<PrepNote> {
        let status = self.preparing_model()?;
        let ready = self
            .runtime
            .processing
            .status()
            .iter()
            .filter(|s| !s.preparing && !s.detail.starts_with("Initialization stopped"))
            .count();
        Some(prep_note(
            &status,
            ready,
            self.runtime.processing.snapshot_run().backend,
            Instant::now(),
        ))
    }

    /// What the running analysis is for, in the words of the step that
    /// started it: a title and one sentence.
    pub(super) fn analysis_owner(&self) -> Option<(&'static str, &'static str)> {
        let run = self.analysis_run?;
        Some(match self.organize_run {
            Some((owner, mode)) if owner == run => (
                organize_activity(mode),
                "Previews stay available. Folders appear when grouping finishes.",
            ),
            _ if self.rotation_run == Some(run) => (
                "Checking orientation",
                "Looking at each photo to find the ones that are sideways or upside down.",
            ),
            _ if self.delete_run == Some(run) => (
                "Looking for photos to clean up",
                "Checking for duplicates, near-duplicates, and blurry or badly exposed shots.",
            ),
            _ if self.pick_run == Some(run) => (
                "Finding similar photos",
                "Grouping bursts and near-identical frames, then scoring each photo.",
            ),
            _ if self.cull_group_run == Some(run) => (
                "Analyzing the library",
                "Finding similar photos and subjects. You can keep culling meanwhile.",
            ),
            _ => ("Analyzing photos", "Previews stay available."),
        })
    }

    /// The progress card's contents for the current run.
    fn activity(&self, title: &str, detail: &str) -> Activity {
        let analyzing = self.analysis_run.is_some();
        let organize_faces = self
            .analysis_run
            .is_some_and(|run| self.organize_run == Some((run, GroupMode::Faces)));
        Activity {
            title: title.to_owned(),
            detail: detail.to_owned(),
            count: self.progress.map(|(done, total)| {
                let mut line = if analyzing {
                    format!(
                        "{done} of {total} {}",
                        if total == 1 { "photo" } else { "photos" }
                    )
                } else {
                    format!("{done} of {total}")
                };
                if let Some(timing) = self.progress_timing() {
                    line.push_str(" · ");
                    line.push_str(&timing);
                }
                line
            }),
            fraction: self
                .progress
                .map(|(done, total)| done as f32 / total.max(1) as f32),
            // Some passes end with a step that has no count of its own.
            phase: (analyzing && self.progress.is_none()).then(|| {
                if organize_faces {
                    "Grouping the faces found into people…".to_owned()
                } else {
                    "Comparing the analyzed photos…".to_owned()
                }
            }),
            preparing: self.prep_note(),
            cancellable: analyzing,
        }
    }

    /// Analysis or preparation running for this step, filling the panel.
    pub(super) fn working_state(&mut self, ui: &mut egui::Ui, title: &str, detail: &str) {
        let activity = self.activity(title, detail);
        if progress_card(ui, &activity, true) {
            self.cancel_analysis();
        }
    }

    /// The running analysis as a card above content that stays usable.
    pub(super) fn analysis_banner(&mut self, ui: &mut egui::Ui) {
        let Some((title, detail)) = self.analysis_owner() else {
            return;
        };
        let activity = self.activity(title, detail);
        if progress_card(ui, &activity, false) {
            self.cancel_analysis();
        }
    }

    /// The folder has no photos, or this step's view is empty.
    pub(super) fn empty_state(&mut self, ui: &mut egui::Ui) {
        // Organize's search sits on its board, so the board stays when the
        // search or filter hides every photo; it says so itself.
        if self.step == Step::Organize && !self.media.is_empty() {
            return self.library(ui);
        }
        let next = self.preferences.neighbor(self.step, true);
        let can_analyze = self.can_start_analysis();
        let mut action = None;
        if self.media.is_empty() {
            state_card(
                ui,
                icon::IMAGES,
                "No photos in this folder",
                "PhotoSort found no photos or videos it can open here, including in subfolders.",
                |ui| {
                    if ui.add(primary("Open another folder")).clicked() {
                        action = Some(Empty::OpenFolder);
                    }
                },
            );
        } else if self.step == Step::Rotation {
            let detected = !self.rotations.is_empty() || !self.rotation_review.ids().is_empty();
            state_card(
                ui,
                icon::ARROW_CLOCKWISE,
                if detected {
                    "Every photo looks upright"
                } else {
                    "Find sideways photos"
                },
                if detected {
                    "PhotoSort found no photo to turn. Show all photos to turn any of them by hand."
                } else {
                    "Detect rotations checks each photo's orientation on this computer and lists the ones that look sideways or upside down."
                },
                |ui| {
                    if !detected
                        && ui
                            .add_enabled(can_analyze, primary("Detect rotations"))
                            .clicked()
                    {
                        action = Some(Empty::Analyze);
                    }
                    if self.rotation_queue_only && ui.button("Show all photos").clicked() {
                        action = Some(Empty::ShowAllRotations);
                    }
                    next_button(ui, next, &mut action);
                },
            );
        } else if self.step == Step::PickBest {
            state_card(
                ui,
                icon::STACK,
                "No similar photos to compare",
                "Pick Best compares bursts and near-identical frames. Analyze the library to find them; if none turn up, every photo is already one of a kind.",
                |ui| {
                    if ui
                        .add_enabled(can_analyze, primary("Analyze library"))
                        .clicked()
                    {
                        action = Some(Empty::Analyze);
                    }
                    next_button(ui, next, &mut action);
                },
            );
        } else if self.step == Step::EasyDelete {
            let hidden = !self.delete_review.entries().is_empty();
            state_card(
                ui,
                icon::BROOM,
                if hidden {
                    "No suggestions match"
                } else {
                    "Nothing to clean up yet"
                },
                if hidden {
                    "The search or the selected group hides every suggestion."
                } else {
                    "Easy Delete looks for exact duplicates, near-duplicates, and blurry or badly exposed shots. Analyze the library to find them."
                },
                |ui| {
                    if hidden {
                        if ui.add(primary("Show all suggestions")).clicked() {
                            action = Some(Empty::ShowAllCategories);
                        }
                    } else if ui
                        .add_enabled(can_analyze, primary("Analyze library"))
                        .clicked()
                    {
                        action = Some(Empty::Analyze);
                    }
                    next_button(ui, next, &mut action);
                },
            );
        } else {
            state_card(
                ui,
                icon::MAGNIFYING_GLASS,
                "No photos match",
                "The search and filters hide every photo in this folder.",
                |ui| {
                    if ui.add(primary("Clear search and filters")).clicked() {
                        action = Some(Empty::ClearFilters);
                    }
                },
            );
        }
        match action {
            Some(Empty::OpenFolder) => self.select_folder(),
            Some(Empty::Analyze) => self.analyze_current_workflow(),
            Some(Empty::ShowAllRotations) => {
                self.rotation_queue_only = false;
                self.filter_dirty = true;
            }
            Some(Empty::ShowAllCategories) => {
                self.delete_categories = Category::ALL.into_iter().collect();
                self.query.clear();
                self.filter_dirty = true;
            }
            Some(Empty::ClearFilters) => self.clear_browse_filters(),
            Some(Empty::Continue(step)) => self.switch_step(step),
            None => {}
        }
    }
}

enum Empty {
    OpenFolder,
    Analyze,
    ShowAllRotations,
    ShowAllCategories,
    ClearFilters,
    Continue(Step),
}

fn next_button(ui: &mut egui::Ui, next: Option<Step>, action: &mut Option<Empty>) {
    if let Some(step) = next
        && ui
            .add(egui::Button::new(format!("Continue to {}", step.title())).frame(false))
            .clicked()
    {
        *action = Some(Empty::Continue(step));
    }
}

/// Centred column used by every full-panel state.
fn state_frame(ui: &mut egui::Ui, body: impl FnOnce(&mut egui::Ui)) {
    let top = (ui.available_height() * 0.22).max(S4);
    ui.vertical_centered(|ui| {
        ui.add_space(top);
        ui.set_max_width(440.0);
        body(ui);
    });
}

fn state_card(
    ui: &mut egui::Ui,
    glyph: &str,
    title: &str,
    body: &str,
    actions: impl FnOnce(&mut egui::Ui),
) {
    state_frame(ui, |ui| {
        ui.label(RichText::new(glyph).size(40.0).color(FAINT));
        ui.add_space(S3);
        ui.label(RichText::new(title).font(semibold(18.0)));
        ui.add_space(S1);
        ui.add(egui::Label::new(RichText::new(body).color(MUTED)).wrap());
        ui.add_space(S4);
        ui.spacing_mut().item_spacing.y = S2;
        actions(ui);
    });
}

/// What a progress card shows; built by [`PhotoSortApp::activity`].
struct Activity {
    title: String,
    detail: String,
    /// "27 of 502 photos · 0:40 elapsed · about 12 min left".
    count: Option<String>,
    fraction: Option<f32>,
    /// What a pass without a count is doing.
    phase: Option<String>,
    /// The model being prepared and its current step.
    preparing: Option<PrepNote>,
    cancellable: bool,
}

/// What an Organize grouping is doing, as the progress card's title.
pub(super) fn organize_activity(mode: GroupMode) -> &'static str {
    match mode {
        GroupMode::Faces => "Finding faces",
        GroupMode::Mixed => "Grouping by date and similarity",
        _ => "Grouping by visual similarity",
    }
}

/// The one progress card every step uses: centred in an empty panel, or
/// as a banner above content that stays usable. Returns whether Cancel
/// was pressed.
fn progress_card(ui: &mut egui::Ui, activity: &Activity, centred: bool) -> bool {
    let mut cancel = false;
    let preparing = |ui: &mut egui::Ui, note: &PrepNote| {
        ui.add(
            egui::Label::new(
                RichText::new(match &note.ready {
                    Some(ready) => format!("{}  {} · {ready}", icon::GEAR, note.headline),
                    None => format!("{}  {}", icon::GEAR, note.headline),
                })
                .small()
                .color(ACCENT),
            )
            .wrap(),
        )
        .on_hover_text(&note.hover);
        if let Some(step) = &note.step {
            ui.add(
                egui::Label::new(RichText::new(step).small().color(if note.slow {
                    ACCENT
                } else {
                    MUTED
                }))
                .wrap(),
            );
        }
        if let Some(advice) = note.advice {
            ui.add(egui::Label::new(RichText::new(advice).small().color(MUTED)).wrap());
        }
    };
    if centred {
        state_frame(ui, |ui| {
            ui.add(egui::Spinner::new().size(28.0).color(ACCENT));
            ui.add_space(S3);
            ui.label(RichText::new(&activity.title).font(semibold(18.0)));
            ui.add_space(S1);
            ui.add(egui::Label::new(RichText::new(&activity.detail).color(MUTED)).wrap());
            if let Some(fraction) = activity.fraction {
                ui.add_space(S4);
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(320.0)
                        .desired_height(8.0)
                        .fill(ACCENT),
                );
            }
            if let Some(line) = activity.count.as_ref().or(activity.phase.as_ref()) {
                ui.add_space(S1);
                ui.label(RichText::new(line).small().color(MUTED));
            }
            if let Some(model) = &activity.preparing {
                ui.add_space(S2);
                preparing(ui, model);
            }
            if activity.cancellable {
                ui.add_space(S4);
                cancel = ui
                    .button("Cancel")
                    .on_hover_text("Cancel analysis · completed results are kept")
                    .clicked();
            }
        });
        return cancel;
    }
    egui::Frame::new()
        .fill(ELEVATED)
        .corner_radius(R_SURFACE)
        .inner_margin(S3)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = S1;
            ui.horizontal(|ui| {
                ui.set_height(26.0);
                // The button first, so a long title truncates instead of
                // running under it.
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if activity.cancellable {
                        cancel = icon_text_button(ui, true, false, icon::STOP, "Cancel")
                            .on_hover_text("Cancel analysis · completed results are kept")
                            .clicked();
                    }
                    ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                        ui.add(egui::Spinner::new().size(16.0).color(ACCENT));
                        ui.add(
                            egui::Label::new(RichText::new(&activity.title).font(semibold(14.0)))
                                .truncate(),
                        )
                        .on_hover_text(&activity.detail);
                    });
                });
            });
            if let Some(fraction) = activity.fraction {
                ui.add(
                    egui::ProgressBar::new(fraction)
                        .desired_width(ui.available_width())
                        .desired_height(6.0)
                        .fill(ACCENT),
                );
            }
            if let Some(line) = activity.count.as_ref().or(activity.phase.as_ref()) {
                ui.add(egui::Label::new(RichText::new(line).small().color(MUTED)).wrap());
            }
            if let Some(model) = &activity.preparing {
                preparing(ui, model);
            }
        });
    cancel
}

/// A card above content saying why something has not happened yet, with
/// the action that starts it. Returns whether the action was pressed.
pub(super) fn notice_card(
    ui: &mut egui::Ui,
    glyph: &str,
    title: &str,
    body: &str,
    action: Option<(&str, bool)>,
) -> bool {
    let mut clicked = false;
    egui::Frame::new()
        .fill(ELEVATED)
        .corner_radius(R_SURFACE)
        .inner_margin(S3)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = S1;
            ui.horizontal(|ui| {
                ui.set_height(26.0);
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if let Some((label, enabled)) = action {
                        clicked = ui.add_enabled(enabled, egui::Button::new(label)).clicked();
                    }
                    ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                        ui.label(RichText::new(glyph).size(16.0).color(MUTED));
                        ui.add(
                            egui::Label::new(RichText::new(title).font(semibold(14.0))).truncate(),
                        );
                    });
                });
            });
            ui.add(egui::Label::new(RichText::new(body).small().color(MUTED)).wrap());
        });
    clicked
}

/// What the card and status bar say about one model being prepared.
#[derive(Debug)]
pub(super) struct PrepNote {
    /// "Preparing the aesthetic scoring model".
    pub headline: String,
    /// "2 other models ready", for the card only.
    pub ready: Option<String>,
    /// "Optimizing for this computer · 1:12 · about 40 s left".
    pub step: Option<String>,
    /// Past the usual time for this step.
    pub slow: bool,
    /// What the user can do once the step is slow.
    pub advice: Option<&'static str>,
    /// The model's file name and where it runs.
    pub hover: String,
}

/// The model in the words the models dialog uses.
fn model_name(kind: &str) -> &str {
    match kind {
        "dinov2" => "similarity",
        "aesthetic" => "aesthetic scoring",
        "face_detector" => "face detection",
        "face_landmarks" => "face landmarks",
        "face_blendshapes" => "eye openness",
        "sface" => "person recognition",
        "orientation" => "orientation",
        other => other,
    }
}

pub(super) fn prep_note(
    status: &crate::core::processing::ModelStatus,
    ready: usize,
    requested: crate::core::processing::Backend,
    now: Instant,
) -> PrepNote {
    use crate::core::processing::{Backend, PrepStep};
    let headline = format!("Preparing the {} model", model_name(status.model));
    let ready = (ready > 0).then(|| {
        format!(
            "{ready} other {} ready",
            if ready == 1 { "model" } else { "models" }
        )
    });
    let hover = format!("{} · {}", status.model, status.backend.label());
    let Some(prep) = status.prep else {
        return PrepNote {
            headline,
            ready,
            step: Some("Starting…".into()),
            slow: false,
            advice: None,
            hover,
        };
    };
    let elapsed = prep.elapsed(now);
    let label = if prep.step == PrepStep::Loading && requested != Backend::Cpu {
        "Accelerator unavailable, loading on the CPU instead"
    } else {
        prep.step.label()
    };
    let mut step = format!("{label} · {}", clock(elapsed));
    let (slow, stalled) = (prep.slow(now), prep.stalled(now));
    if stalled {
        step.push_str(" · this may be stuck");
    } else if slow && prep.usual.is_some() {
        step.push_str(" · taking longer than usual");
    } else if slow {
        step.push_str(" · taking longer than expected");
    } else if let Some(usual) = prep.usual.filter(|u| *u >= Duration::from_secs(5)) {
        if usual > elapsed {
            step.push_str(&format!(" · about {} left", rough(usual - elapsed)));
        } else {
            step.push_str(" · almost done");
        }
    }
    // Native compilation is one call that cannot be interrupted; hashing can.
    let native = matches!(
        prep.step,
        PrepStep::Compiling | PrepStep::Reusing | PrepStep::Loading
    );
    let advice = match (slow, stalled, native) {
        (_, true, true) if status.backend != Backend::Cpu => Some(
            "Closing waits for this step. If it never ends, force quit and choose CPU in Settings › AI execution.",
        ),
        (_, true, true) => {
            Some("Closing waits for this step. If it never ends, force quit PhotoSort.")
        }
        (true, _, true) if status.backend != Backend::Cpu => Some(
            "This step can't be interrupted; Cancel takes effect when it ends. CPU in Settings › AI execution skips it.",
        ),
        (true, _, true) => {
            Some("This step can't be interrupted; Cancel takes effect when it ends.")
        }
        (true, _, false) => {
            Some("Large models check slowly on a busy disk. Cancel stops right away.")
        }
        _ => None,
    };
    PrepNote {
        headline,
        ready,
        step: Some(step),
        slow,
        advice,
        hover,
    }
}

fn clock(duration: Duration) -> String {
    let seconds = duration.as_secs();
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

fn rough(duration: Duration) -> String {
    let seconds = duration.as_secs();
    if seconds < 60 {
        format!("{} s", seconds.max(5).div_ceil(5) * 5)
    } else {
        format!("{} min", seconds.div_ceil(60))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_reads_as_elapsed_clock_and_rounded_estimate() {
        assert_eq!(clock(Duration::from_secs(65)), "1:05");
        assert_eq!(rough(Duration::from_secs(3)), "5 s");
        assert_eq!(rough(Duration::from_secs(42)), "45 s");
        assert_eq!(rough(Duration::from_secs(61)), "2 min");
    }

    #[test]
    fn model_preparation_estimates_then_says_slow_then_possibly_stuck() {
        use crate::core::processing::{Backend, ModelStatus, Prep, PrepStep};
        let start = Instant::now();
        let status = |usual| ModelStatus {
            model: "aesthetic",
            backend: Backend::CoreMl,
            detail: String::new(),
            preparing: true,
            prep: Some(Prep {
                step: PrepStep::Compiling,
                started: start,
                step_started: start,
                usual,
            }),
        };
        let at = |secs| start + Duration::from_secs(secs);
        let known = status(Some(Duration::from_secs(120)));
        let note = prep_note(&known, 2, Backend::Auto, at(80));
        assert_eq!(note.headline, "Preparing the aesthetic scoring model");
        assert_eq!(note.ready.as_deref(), Some("2 other models ready"));
        assert_eq!(
            note.step.as_deref(),
            Some("Optimizing for this computer · 1:20 · about 40 s left")
        );
        assert!(!note.slow && note.advice.is_none());
        let note = prep_note(&known, 0, Backend::Auto, at(200));
        assert!(note.slow && note.step.unwrap().ends_with("taking longer than usual"));
        assert!(
            note.advice
                .unwrap()
                .contains("Cancel takes effect when it ends")
        );
        // With no earlier timing, a first compile gets five minutes before
        // it is called slow, and fifteen before it may be stuck.
        let first = status(None);
        assert!(!prep_note(&first, 0, Backend::Auto, at(290)).slow);
        let note = prep_note(&first, 0, Backend::Auto, at(310));
        assert!(note.step.unwrap().ends_with("taking longer than expected"));
        let note = prep_note(&first, 0, Backend::Auto, at(901));
        assert!(note.step.unwrap().ends_with("this may be stuck"));
        assert!(note.advice.unwrap().contains("force quit"));
    }
}
