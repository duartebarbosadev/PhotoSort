//! Full-panel states: work in progress, and empty views with the obvious
//! next action, so no step ever ends on a bare sentence.
use super::*;
use crate::core::delete_review::Category;
use egui_phosphor::regular as icon;

impl PhotoSortApp {
    /// Keeps the start time of the current progress run for elapsed/estimate.
    pub(super) fn track_progress_clock(&mut self) {
        match (self.progress, self.progress_started) {
            (Some(_), None) => self.progress_started = Some(Instant::now()),
            (None, Some(_)) => self.progress_started = None,
            _ => {}
        }
    }

    /// "1:05 elapsed · about 2 min left", once there is enough to estimate.
    pub(super) fn progress_timing(&self) -> Option<String> {
        let (done, total) = self.progress?;
        let elapsed = self.progress_started?.elapsed();
        let mut text = format!("{} elapsed", clock(elapsed));
        if done >= 3 && done < total && elapsed >= Duration::from_secs(3) {
            let left = elapsed.mul_f64((total - done) as f64 / done as f64);
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
            preparing: self
                .preparing_model()
                .map(|status| format!("{} · {}", status.model, status.backend.label())),
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
                    "No rotation suggestions left"
                } else {
                    "Find sideways photos"
                },
                if detected {
                    "Every suggested photo has been reviewed. You can still turn any photo by hand."
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
                if self.allow_review_departure() {
                    self.rotation_queue_only = false;
                    self.filter_dirty = true;
                }
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
    /// The model being prepared and where, while preparation runs.
    preparing: Option<String>,
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
    let preparing = |ui: &mut egui::Ui, model: &str| {
        ui.add(
            egui::Label::new(
                RichText::new(format!(
                    "{}  Preparing AI models · first run only, can take a few minutes",
                    icon::GEAR
                ))
                .small()
                .color(ACCENT),
            )
            .wrap(),
        )
        .on_hover_text(model);
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
}
