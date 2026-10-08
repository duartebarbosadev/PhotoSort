use super::*;

impl PhotoSortApp {
    pub(super) fn ensure_rotation_choice(&mut self, id: MediaId) {
        let (turns, uncertain) = self
            .rotations
            .get(&id)
            .map_or((0, false), |s| (s.turns, s.uncertain));
        self.rotation_review.ensure_prediction(id, turns, uncertain);
    }
    pub(super) fn select_rotation(&mut self, corrected: bool) {
        if self.busy || self.rotation_run.is_some() {
            return;
        }
        if let Some(id) = self.active {
            self.ensure_rotation_choice(id);
            self.rotation_review.select(id, corrected);
        }
    }
    pub(super) fn confirm_rotation(&mut self) {
        if self.busy || self.rotation_run.is_some() {
            return;
        }
        if let Some(id) = self.active {
            self.ensure_rotation_choice(id);
            if let Some(next) = self.rotation_review.confirm(id) {
                self.set_active(next);
            }
        }
    }
    pub(super) fn reset_rotation(&mut self, all: bool) {
        if all {
            self.rotation_review.reset_all();
        } else if let Some(id) = self.active {
            self.rotation_review.reset(id);
        }
        self.review_notice = None;
    }
    pub(super) fn review_rotations(&mut self) {
        if self.review_remaining_operations() {
            return;
        }
        if !self.allow_review_departure() {
            return;
        }
        let decisions = self.rotation_review.pending();
        if decisions.is_empty() {
            self.set_workflow_status(
                Step::Rotation,
                "No confirmed rotation changes to apply".into(),
            );
            return;
        }
        if let Some(catalog) = self.catalog.clone() {
            match self
                .runtime
                .image(crate::core::jobs::ImageWork::PlanDecisions {
                    catalog,
                    decisions,
                    companions: false,
                    policy: self.preferences.metadata_writes,
                }) {
                Ok(()) => {
                    self.busy = true;
                    self.rotation_plan = true;
                }
                Err(error) => {
                    self.after_apply = None;
                    self.error = Some(error);
                }
            }
        }
    }
    pub(super) fn detect_rotations(&mut self) {
        if !self.can_start_analysis() || !self.model_install_ready() {
            return;
        }
        if !self.allow_review_departure() {
            return;
        }
        if self.rotation_review.pending_count() > 0 {
            self.error =
                Some("Apply or discard confirmed rotations before detecting again.".into());
            return;
        }
        let Some(model) = crate::core::models::orientation_model() else {
            self.model_issue = Some(crate::core::model_assets::Issue {
                missing: vec![crate::core::model_assets::Id::Rotation],
                ..Default::default()
            });
            self.remember_model_retry();
            self.open_model_setup();
            return;
        };
        {
            match self.runtime.orient(Arc::new(self.media.clone()), model) {
                Ok(run) => {
                    self.clear_model_retry();
                    self.resume_suspended = false;
                    self.rotations.clear();
                    self.rotation_run = Some(run);
                    self.analysis_run = Some(run);
                    self.progress = Some((0, self.media.len()));
                    self.set_status("Detecting orientations locally…".into());
                }
                Err(error) => self.error = Some(error),
            }
        }
    }
    pub(super) fn finish_rotation_detection(&mut self) {
        let mut rows: Vec<_> = self
            .media
            .iter()
            .filter_map(|m| {
                self.rotations
                    .get(&m.id)
                    .filter(|s| s.turns != 0)
                    .map(|s| (m.name(), m.id, s.turns, s.uncertain))
            })
            .collect();
        rows.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| a.1.0.cmp(&b.1.0)));
        self.rotation_review.replace_predictions(
            rows.into_iter()
                .map(|(_, id, turns, uncertain)| (id, turns, uncertain))
                .collect(),
        );
        self.rotation_run = None;
        self.view_data_ready.insert(Step::Rotation);
        self.rotation_queue_only = true;
        self.filter_dirty = true;
        self.set_workflow_status(
            Step::Rotation,
            format!(
                "{} rotation suggestions · confirm choices before applying",
                self.rotation_review.ids().len()
            ),
        );
    }
    pub(super) fn rotation_controls(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(
                    self.can_start_analysis(),
                    egui::Button::new("Detect rotations…"),
                )
                .on_hover_text("Detect orientations locally · Ctrl/Cmd+S")
                .clicked()
            {
                self.analyze_current_workflow();
            }
            let mut queue_only = self.rotation_queue_only;
            if ui.checkbox(&mut queue_only, "Suggestions only").changed()
                && self.allow_review_departure()
            {
                self.rotation_queue_only = queue_only;
                self.filter_dirty = true;
            }
            let count = self.rotation_review.pending_count();
            if ui
                .add_enabled(
                    count > 0 && !self.busy,
                    egui::Button::new(format!("Apply {count} confirmed…")),
                )
                .clicked()
            {
                self.review_rotations();
            }
        });
        ui.horizontal_wrapped(|ui| {
            if let Some(id) = self.active {
                if ui.button("↶ Left (Q)").clicked() {
                    self.rotate(id, 3);
                }
                if ui.button("Right (E) ↷").clicked() {
                    self.rotate(id, 1);
                }
                let confirmed = self.rotation_review.choice(id).is_some_and(|c| c.confirmed);
                if ui
                    .button(if confirmed {
                        "Cancel confirmation"
                    } else {
                        "Confirm →"
                    })
                    .clicked()
                {
                    self.confirm_rotation();
                }
                if ui.button("Reset default").clicked() {
                    self.reset_rotation(false);
                }
                if ui.button("Reset all").clicked() {
                    self.reset_rotation(true);
                }
                if ui
                    .add_enabled(
                        !self.rotation_review.ids().is_empty(),
                        egui::Button::new("Confirm all"),
                    )
                    .clicked()
                    && self.allow_review_departure()
                {
                    self.rotation_review.confirm_all();
                }
            }
        });
        if let Some(id) = self.active {
            let text = self
                .rotation_review
                .choice(id)
                .map(|c| {
                    format!(
                        "{} · {}{} · {}°{}",
                        if c.confirmed {
                            "Confirmed"
                        } else {
                            "Choose, then confirm"
                        },
                        if c.selected {
                            "Correction selected"
                        } else {
                            "Original selected"
                        },
                        if c.manual() { " · Manual" } else { "" },
                        c.degrees(),
                        if c.confirmed { "" } else { " · Preview only" }
                    )
                })
                .unwrap_or_else(|| {
                    "Choose the original or rotate the preview, then confirm.".into()
                });
            ui.label(RichText::new(text).small().color(ACCENT));
            if self.rotation_review.choice(id).is_some_and(|c| c.uncertain) {
                ui.label(RichText::new("Uncertain orientation · rotated views disagree. Compare with the original and confirm this photo individually; Confirm all skips it.").small());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rotation_detection_routes_uncertainty_through_shared_review_and_controls() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("photo.jpg"), b"fixture").unwrap();
        let catalog = Arc::new(Catalog::scan(dir.path(), || false, |_| {}).unwrap());
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        app.media = catalog.media.clone();
        let id = app.media[0].id;
        app.index.insert(id, 0);
        app.active = Some(id);
        app.step = Step::Rotation;
        app.rotations.insert(
            id,
            crate::core::models::RotationSuggestion {
                turns: 2,
                confidence: 0.9,
                uncertain: true,
                views: 3,
            },
        );
        app.finish_rotation_detection();
        assert_eq!(app.rotation_review.ids(), &[id]);
        assert!(!app.rotation_review.choice(id).unwrap().selected);
        let ctx = egui::Context::default();
        let output = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.rotation_controls(ui));
        });
        fn has_warning(shape: &egui::Shape) -> bool {
            match shape {
                egui::Shape::Text(text) => text.galley.text().contains("Uncertain orientation"),
                egui::Shape::Vec(shapes) => shapes.iter().any(has_warning),
                _ => false,
            }
        }
        assert!(output.shapes.iter().any(|shape| has_warning(&shape.shape)));
        app.rotation_review.confirm_all();
        assert!(app.rotation_review.pending().is_empty());
        app.select_rotation(true);
        app.confirm_rotation();
        assert_eq!(app.rotation_review.pending()[&id].turns, 2);
        app.reset_rotation(false);
        assert!(!app.rotation_review.choice(id).unwrap().selected);
        assert!(app.requested.is_empty() && app.textures.is_empty());
        assert!(app.analysis_run.is_none() && app.metadata_run.is_none());
    }
}
