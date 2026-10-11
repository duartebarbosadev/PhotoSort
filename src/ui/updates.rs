use super::*;
use crate::core::updates::{self, Source, State};
impl PhotoSortApp {
    pub(super) fn start_update_check(&mut self) {
        let source = match Source::configured() {
            Ok(source) => source,
            Err(reason) => {
                self.runtime.updates.unavailable(reason);
                return;
            }
        };
        match self
            .runtime
            .updates
            .start(source.clone(), env!("CARGO_PKG_VERSION").into())
        {
            Ok(true) => {
                self.preferences.updates.attempted(&source, updates::now());
                self.save_preferences();
            }
            Ok(false) => {}
            Err(error) => self.runtime.updates.unavailable(error.to_string()),
        }
    }
    pub(super) fn check_updates_at_startup(&mut self) {
        if self.update_startup_checked {
            return;
        }
        self.update_startup_checked = true;
        if let Ok(source) = Source::configured()
            && self
                .preferences
                .updates
                .due(&source, env!("CARGO_PKG_VERSION"), updates::now())
        {
            self.start_update_check();
        }
    }
    pub(super) fn updates_dialog(&mut self, ctx: &egui::Context) {
        if !self.update_dialog {
            return;
        }
        self.render_updates_dialog(ctx, self.runtime.updates.state());
    }
    fn render_updates_dialog(&mut self, ctx: &egui::Context, state: State) {
        let mut close = false;
        let response = egui::Modal::new(egui::Id::new("updates")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(240.0, 480.0));
            ui.heading("PhotoSort updates");
            ui.label(format!("Installed version {}", env!("CARGO_PKG_VERSION")));
            egui::ScrollArea::vertical()
                .max_height((ctx.content_rect().height() - 220.0).max(80.0))
                .show(ui, |ui| {
                    match &state {
                        State::Checking => {
                            ui.spinner();
                            ui.label("Checking for a new stable release…");
                        }
                        State::Available(release) => {
                            ui.heading(format!("Version {} is available", release.version));
                            if !release.published_at.is_empty() {
                                ui.small(&release.published_at);
                            }
                            // Release text stays plain text: no remote images or executable HTML.
                            ui.label(if release.notes.is_empty() {
                                "No release notes supplied."
                            } else {
                                &release.notes
                            });
                            if release.notes_truncated {
                                ui.small("Preview shortened. Full notes are on the release page.");
                            }
                        }
                        State::Current { latest } => {
                            ui.label(format!(
                                "No newer stable release is available. Latest published: {latest}."
                            ));
                        }
                        State::Failed(error) => {
                            ui.colored_label(RED, "Could not check for updates.");
                            ui.label(error);
                        }
                        State::Unavailable(reason) => {
                            ui.label(reason);
                        }
                        State::Cancelled => {
                            ui.label("Update check cancelled.");
                        }
                        State::Idle => {
                            ui.label("Check for a new stable release.");
                        }
                    }
                });
            // Keep the primary action visible even for long, scrolling notes.
            if let State::Available(release) = &state {
                ui.hyperlink_to("View release and downloads", &release.url);
            }
            let mut automatic = self.preferences.updates.automatic;
            if ui
                .checkbox(&mut automatic, "Check for updates daily")
                .changed()
            {
                self.preferences.updates.automatic = automatic;
                self.save_preferences();
            }
            ui.horizontal_wrapped(|ui| {
                if self.runtime.updates.running() {
                    if ui.button("Cancel check").clicked() {
                        self.runtime.updates.cancel();
                    }
                } else if Source::configured().is_ok() && ui.button("Check again").clicked() {
                    self.start_update_check();
                }
                if ui.button("Close").clicked() {
                    close = true;
                }
            });
        });
        if close || response.should_close() {
            self.runtime.updates.cancel();
            self.update_dialog = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn long_release_preview_keeps_footer_actions_visible_in_compact_windows() {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        let source = Source::new("example/rust").unwrap();
        let body = serde_json::to_vec(&serde_json::json!({"tag_name":"v1.0.0","draft":false,
            "prerelease":false,"body":"Photo release notes.\n".repeat(50_000)}))
        .unwrap();
        let state = updates::evaluate(&source, "0.1.0", &body).unwrap();
        app.update_dialog = true;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        for size in [egui::vec2(600.0, 500.0), egui::vec2(1440.0, 900.0)] {
            let mut output = None;
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                };
                output = Some(ctx.run_test(input, |ui| {
                    let ctx = &ui.ctx().clone();
                    app.render_updates_dialog(ctx, state.clone())
                }));
            }
            let tree = output.unwrap().platform_output.accesskit_update.unwrap();
            for label in [
                "View release and downloads",
                "Check for updates daily",
                "Close",
            ] {
                let node = tree
                    .nodes
                    .iter()
                    .find(|(_, node)| node.label() == Some(label))
                    .expect(label);
                let bounds = node.1.bounds().expect("visible control bounds");
                assert!(
                    bounds.y0 >= 0.0 && bounds.y1 <= f64::from(size.y),
                    "{label}: {bounds:?}"
                );
            }
            assert!(app.requested.is_empty() && app.analysis_run.is_none());
        }
    }
    #[test]
    fn all_workflows_share_update_dialog_and_preserve_photo_state_at_small_sizes() {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        for step in Step::ALL {
            app.step = step;
            app.runtime
                .updates
                .unavailable("No Rust release source configured.".into());
            app.update_dialog = true;
            assert!(app.modal_active());
            let before = app.stats;
            for size in [egui::vec2(600.0, 500.0), egui::vec2(1440.0, 900.0)] {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                };
                let _ = ctx.run_test(input, |ui| {
                    let ctx = &ui.ctx().clone();
                    {
                        app.keyboard(ctx);
                        app.updates_dialog(ctx);
                    }
                });
                assert_eq!(app.step, step);
                assert!(app.decisions.is_empty() && app.requested.is_empty());
                assert!(app.analysis_run.is_none() && app.rotation_run.is_none());
                assert_eq!(app.stats.decodes, before.decodes);
            }
            app.update_dialog = false;
        }
        // Settings opened before a check must not roll its throttle timestamp back.
        let draft = app.preferences.clone();
        app.preferences
            .updates
            .attempted(&Source::new("example/rust").unwrap(), 12345);
        app.apply_preferences(draft);
        assert_eq!(app.preferences.updates.last_attempt, 12345);
        app.runtime.shutdown();
    }
}
