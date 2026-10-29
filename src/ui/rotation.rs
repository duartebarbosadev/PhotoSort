use super::review_card::{Badge, CardButton, CardEvent, CardPhoto, CardState, ReviewCard};
use super::*;

/// What a number of clockwise quarter turns does, in words.
fn turn_label(turns: u8) -> &'static str {
    match turns % 4 {
        1 => "Turn 90° right",
        2 => "Turn 180°",
        3 => "Turn 90° left",
        _ => "Keep as is",
    }
}

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
                    self.analysis_failures.clear();
                    self.analysis_successes = 0;
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
                self.rotations.get(&m.id).filter(|s| s.turns != 0).map(|s| {
                    let order = crate::core::dates::capture_order(m, self.metadata.get(&m.id));
                    (order, m.id, s.turns, s.uncertain)
                })
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
            match self.rotation_review.ids().len() {
                0 => "No rotation suggestions · every photo looks upright".to_owned(),
                1 => "1 rotation suggestion · confirm it before applying".to_owned(),
                n => format!("{n} rotation suggestions · confirm choices before applying"),
            } + &self.failure_note(),
        );
    }
    /// The review scope and the apply action. Rotating and confirming a
    /// photo live under the photos in Review and on each card in the grid;
    /// Confirm all at the right of the bar.
    pub(super) fn rotation_controls(&mut self, ui: &mut egui::Ui) {
        let mut queue_only = self.rotation_queue_only;
        if ui
            .add(Switch::new(&mut queue_only, "Suggestions only"))
            .changed()
            && self.allow_review_departure()
        {
            self.rotation_queue_only = queue_only;
            self.filter_dirty = true;
        }
        let count = self.rotation_review.pending_count();
        let apply = format!("Apply {count} confirmed…");
        let button = if count > 0 {
            primary(apply)
        } else {
            egui::Button::new(apply)
        };
        if ui
            .add_enabled(count > 0 && !self.busy, button)
            .on_hover_text("Review and write the confirmed rotations · Shift+Enter")
            .clicked()
        {
            self.review_rotations();
        }
    }
    /// Confirms every certain suggestion at once, in the bar before the
    /// view switch. Only the icon in a narrow bar.
    pub(super) fn rotation_confirm_all_button(&mut self, ui: &mut egui::Ui, compact: bool) {
        use egui_phosphor::regular as icon;
        let text = if compact {
            icon::CHECKS.to_owned()
        } else {
            format!("{}  Confirm all", icon::CHECKS)
        };
        let suggested = !self.rotation_review.ids().is_empty();
        let enabled = !self.busy && self.rotation_run.is_none() && suggested;
        let mut response = named(
            ui.add_enabled(enabled, egui::Button::new(text)),
            egui::WidgetType::Button,
            "Confirm all suggestions",
        )
        .on_hover_text("Confirm every certain suggestion; uncertain photos stay open");
        if !suggested {
            response = response.on_disabled_hover_text("No rotation suggestions to confirm");
        }
        if response.clicked() && self.allow_review_departure() {
            self.rotation_review.confirm_all();
        }
    }
    /// Rotate and confirm the current photo, under the comparison.
    pub(super) fn rotation_confirm_buttons(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let Some(id) = self.active else {
            return;
        };
        if icon_button(ui, icon::ARROW_COUNTER_CLOCKWISE, "Rotate left · Q").clicked() {
            self.rotate(id, 3);
        }
        if icon_button(ui, icon::ARROW_CLOCKWISE, "Rotate right · E").clicked() {
            self.rotate(id, 1);
        }
        let confirmed = self.rotation_review.choice(id).is_some_and(|c| c.confirmed);
        let confirm = if confirmed {
            ui.button("Cancel confirmation")
        } else {
            ui.add(primary("Confirm"))
        };
        if confirm.on_hover_text("Enter").clicked() {
            self.confirm_rotation();
        }
    }
    /// The current photo's choice in one line, and any uncertainty.
    pub(super) fn rotation_context(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let Some(id) = self.active else {
            return;
        };
        let choice = self.rotation_review.choice(id);
        let text = choice
            .map(|c| {
                format!(
                    "{} · {}{} · {}°",
                    if c.selected {
                        "Correction selected"
                    } else {
                        "Original selected"
                    },
                    if c.manual() { "manual · " } else { "" },
                    if c.confirmed {
                        "confirmed"
                    } else {
                        "not confirmed yet"
                    },
                    c.degrees(),
                )
            })
            .unwrap_or_else(|| "Pick the original or rotate the preview, then confirm.".into());
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new(text).small().color(MUTED));
            if choice.is_some_and(|c| c.uncertain) {
                ui.label(
                    RichText::new(format!("{}  Uncertain orientation", icon::WARNING))
                        .small()
                        .color(ACCENT),
                )
                .on_hover_text("The rotated views disagree. Compare with the original and confirm this photo on its own; Confirm all skips it.");
            }
        });
    }
}

impl PhotoSortApp {
    /// Fix Rotation's grid: one card per photo, the original beside the
    /// suggested correction, with the same rotate, keep original, Confirm
    /// and Reset as Review. The focused card is Review's current photo.
    pub(super) fn rotation_grid(&mut self, ui: &mut egui::Ui) {
        let ids: Vec<MediaId> = self.filtered.iter().map(|i| self.media[*i].id).collect();
        let current = self
            .active
            .and_then(|id| ids.iter().position(|other| *other == id));
        let clicked = self.card_grid(
            ui,
            "rotation",
            ids.len(),
            2,
            |app, i| Some(app.rotation_card(ids[i])),
            current,
        );
        if let Some((i, event)) = clicked {
            self.rotation_card_event(ids[i], event);
        }
    }

    /// The card for one photo: the original, the suggested correction
    /// beside it, how sure PhotoSort is, and the rotation actions.
    fn rotation_card(&self, id: MediaId) -> ReviewCard {
        use egui_phosphor::regular as icon;
        let media = self.index.get(&id).map(|i| &self.media[*i]);
        let name = media.map(Media::name).unwrap_or_default();
        let path = media
            .map(|m| m.path.display().to_string())
            .unwrap_or_default();
        let choice = self.rotation_review.choice(id);
        let turns = choice.map_or(0, |c| c.turns());
        let corrected = choice.is_some_and(|c| c.selected) && turns != 0;
        let confirmed = choice.is_some_and(|c| c.confirmed);
        let enabled = !self.busy && self.rotation_run.is_none();
        let mut photos = vec![CardPhoto {
            id,
            turns: 0,
            badge: if turns == 0 || corrected {
                Badge::Plain("Original".into())
            } else {
                Badge::Keep("Original".into())
            },
            suggested: false,
            caption: name,
            name: format!("{path} · original"),
            facts: Vec::new(),
            dim: false,
            toggle: None,
            suggested_hover: None,
            inset: None,
        }];
        if turns != 0 {
            photos.push(CardPhoto {
                id,
                turns,
                badge: if corrected {
                    Badge::Keep(turn_label(turns).into())
                } else {
                    Badge::Plain(turn_label(turns).into())
                },
                suggested: false,
                caption: "Corrected preview".into(),
                name: format!("{path} · {}", turn_label(turns).to_lowercase()),
                facts: Vec::new(),
                dim: false,
                toggle: None,
                suggested_hover: None,
                inset: None,
            });
        }
        let (note, warning) = match choice {
            None => (
                "No rotation suggested · rotate it by hand if it needs it".to_owned(),
                false,
            ),
            Some(c) if c.uncertain => (
                "Uncertain orientation · the rotated views disagree, so confirm it on its own"
                    .to_owned(),
                true,
            ),
            Some(c) => {
                let suggested = match self.rotations.get(&id) {
                    Some(s) => format!(
                        "Suggested: {} · {:.0}% confident",
                        turn_label(c.suggested).to_lowercase(),
                        s.confidence * 100.0
                    ),
                    None => format!("Suggested: {}", turn_label(c.suggested).to_lowercase()),
                };
                if c.manual() {
                    (format!("Rotated by hand · {suggested}"), false)
                } else {
                    (suggested, false)
                }
            }
        };
        ReviewCard {
            title: if choice.is_some() {
                turn_label(turns).into()
            } else {
                "No suggestion".into()
            },
            state: if confirmed {
                CardState::Confirmed
            } else if choice.is_some_and(|c| c.unfinished()) {
                CardState::Changed
            } else {
                CardState::Provisional
            },
            state_text: None,
            photos,
            note,
            warning,
            buttons: vec![
                CardButton::new(
                    icon::ARROW_COUNTER_CLOCKWISE,
                    "Rotate left · Q on the selected card",
                )
                .enabled(enabled),
                CardButton::new(
                    icon::ARROW_CLOCKWISE,
                    "Rotate right · E on the selected card",
                )
                .enabled(enabled),
                CardButton::new(
                    if corrected {
                        "Keep original"
                    } else {
                        "Use correction"
                    },
                    if corrected {
                        "Leave the photo as it is · 1 on the selected card"
                    } else {
                        "Use the corrected preview · 2 on the selected card"
                    },
                )
                .enabled(enabled && turns != 0),
                CardButton::new(
                    if confirmed {
                        "Cancel confirmation"
                    } else {
                        "Confirm"
                    },
                    "Enter on the selected card",
                )
                .primary(!confirmed)
                .enabled(enabled && choice.is_some())
                .trailing(),
                CardButton::new("Reset", "Back to the suggestion · R on the selected card")
                    .enabled(
                        enabled
                            && choice.is_some_and(|c| c.confirmed || c.unfinished() || c.manual()),
                    )
                    .trailing(),
            ],
            current: self.active == Some(id),
            title_hover: None,
            shown: false,
            undo: None,
        }
    }

    /// Applies a card's event to its photo, which first becomes the current
    /// photo through Review's guard: an unconfirmed change on another card
    /// keeps the focus, and the action, where it is.
    pub(super) fn rotation_card_event(&mut self, id: MediaId, event: CardEvent) {
        self.pending_bookmark = None;
        if self.active != Some(id) {
            self.set_active(id);
        }
        if self.active != Some(id) {
            return;
        }
        match event {
            CardEvent::Select(side) | CardEvent::Open(side) => {
                self.focused_slot = side as u64;
                if matches!(event, CardEvent::Open(_)) {
                    self.open_review();
                }
            }
            CardEvent::Button(0) => self.rotate(id, 3),
            CardEvent::Button(1) => self.rotate(id, 1),
            CardEvent::Button(2) => {
                let corrected = self.rotation_review.choice(id).is_some_and(|c| c.selected);
                self.select_rotation(!corrected);
            }
            CardEvent::Button(3) => self.confirm_rotation(),
            CardEvent::Button(_) => self.reset_rotation(false),
            // Rotation cards have neither pill switches nor Undo.
            CardEvent::Toggle(_) | CardEvent::Undo => {}
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
        theme_install(&ctx);
        let output = ctx.run_test(egui::RawInput::default(), |ui| {
            egui::CentralPanel::default().show(ui, |ui| app.rotation_context(ui));
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
    fn two_suggestions() -> PhotoSortApp {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.media = (1..=2)
            .map(|n| Media {
                id: MediaId(n),
                path: format!("/{n}.jpg").into(),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint::default(),
            })
            .collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        for (id, turns, uncertain) in [(MediaId(1), 1, false), (MediaId(2), 3, true)] {
            app.rotations.insert(
                id,
                crate::core::models::RotationSuggestion {
                    turns,
                    confidence: 0.9,
                    uncertain,
                    views: 3,
                },
            );
        }
        app.set_step(Step::Rotation);
        app.finish_rotation_detection();
        app.refilter();
        app.active = Some(MediaId(1));
        app
    }
    #[test]
    fn grid_cards_show_original_beside_the_correction_with_confidence() {
        let mut app = two_suggestions();
        let shown = super::super::review_card::render_grid(&mut app);
        for text in [
            "Turn 90° right",
            "Turn 90° left",
            "Original",
            "Corrected preview",
            "90% confident",
            "Uncertain orientation",
            "Keep original",
            "Use correction",
        ] {
            assert!(shown.contains(text), "{text} missing from:\n{shown}");
        }
        assert!(app.rotation_review.pending().is_empty());
    }
    #[test]
    fn rotation_card_actions_focus_their_photo_and_respect_pending_choices() {
        let mut app = two_suggestions();
        // Keep the first photo's original: a pending change on its card.
        app.rotation_card_event(MediaId(1), CardEvent::Button(2));
        assert!(!app.rotation_review.choice(MediaId(1)).unwrap().selected);
        assert!(app.rotation_review.unfinished(Some(MediaId(1))));
        // Another card cannot take the focus, or the action, meanwhile.
        app.rotation_card_event(MediaId(2), CardEvent::Button(1));
        assert_eq!(app.active, Some(MediaId(1)));
        assert_eq!(app.review_notice, Some(Step::Rotation));
        assert_eq!(app.rotation_review.choice(MediaId(2)).unwrap().turns(), 3);
        app.review_notice = None;
        // Rotate by hand and confirm from the card; focus moves on.
        app.rotation_card_event(MediaId(1), CardEvent::Button(1));
        assert_eq!(app.rotation_review.choice(MediaId(1)).unwrap().turns(), 1);
        app.rotation_card_event(MediaId(1), CardEvent::Button(3));
        assert_eq!(app.rotation_review.pending()[&MediaId(1)].turns, 1);
        assert_eq!(app.active, Some(MediaId(2)));
        // Reset from the card undoes the confirmation.
        app.rotation_card_event(MediaId(1), CardEvent::Button(4));
        assert!(app.rotation_review.pending().is_empty());
        app.rotation_card_event(MediaId(2), CardEvent::Open(1));
        assert!(app.review);
        assert_eq!(app.active, Some(MediaId(2)));
        assert_eq!(app.focused_slot, 1);
    }
}
