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

    /// Analysis or preparation running for this step.
    pub(super) fn working_state(&mut self, ui: &mut egui::Ui, title: &str, detail: &str) {
        let progress = self.progress;
        let timing = self.progress_timing();
        let cancellable = self.analysis_run.is_some();
        let mut cancel = false;
        state_frame(ui, |ui| {
            ui.add(egui::Spinner::new().size(28.0).color(ACCENT));
            ui.add_space(S3);
            ui.label(RichText::new(title).font(semibold(18.0)));
            ui.add_space(S1);
            ui.add(egui::Label::new(RichText::new(detail).color(MUTED)).wrap());
            if let Some((done, total)) = progress {
                ui.add_space(S4);
                ui.add(
                    egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                        .desired_width(320.0)
                        .desired_height(8.0)
                        .fill(ACCENT),
                );
                ui.add_space(S1);
                let mut line = format!("{done} of {total}");
                if let Some(timing) = &timing {
                    line.push_str(" · ");
                    line.push_str(timing);
                }
                ui.label(RichText::new(line).small().color(MUTED));
            }
            if cancellable {
                ui.add_space(S4);
                cancel = ui.button("Cancel analysis").clicked();
            }
        });
        if cancel {
            self.cancel_analysis();
        }
    }

    /// The folder has no photos, or this step's view is empty.
    pub(super) fn empty_state(&mut self, ui: &mut egui::Ui) {
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
                    "All suggestions are hidden"
                } else {
                    "Nothing to clean up yet"
                },
                if hidden {
                    "Turn a category back on to see its suggestions."
                } else {
                    "Easy Delete looks for exact duplicates, near-duplicates, and blurry or badly exposed shots. Analyze the library to find them."
                },
                |ui| {
                    if hidden {
                        if ui.add(primary("Show every category")).clicked() {
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
                if self.allow_review_departure() {
                    for category in Category::ALL {
                        if self
                            .delete_review
                            .entries()
                            .iter()
                            .any(|e| e.category == category)
                        {
                            self.delete_categories.insert(category);
                        }
                    }
                    self.filter_dirty = true;
                }
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
