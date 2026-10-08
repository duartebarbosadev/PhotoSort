//! Application controls shared by every workflow and both navigation layouts.
use super::*;
use crate::core::preferences::Preferences;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    General,
    Caches,
}

impl PhotoSortApp {
    pub(super) fn open_settings(&mut self, section: Section) {
        if self.preferences_draft.is_none() {
            self.preferences_draft = Some(self.preferences.clone());
            self.runtime.cache.request(false);
        }
        self.preferences_section = Some(section);
    }

    pub(super) fn save_preferences(&mut self) {
        if let Some(path) = &self.preferences_path
            && let Err(error) = self.runtime.io(IoWork::SavePreferences {
                path: path.clone(),
                preferences: self.preferences.clone(),
            })
        {
            self.error = Some(error);
        }
    }
    pub(super) fn apply_preferences(&mut self, mut next: Preferences) {
        // A dialog does not own history; a folder may finish loading while it is open.
        next.recent_folders = self.preferences.recent_folders.clone();
        next.updates.last_attempt = self.preferences.updates.last_attempt;
        next.updates.repository = self.preferences.updates.repository.clone();
        next.normalize();
        self.runtime.cache.configure(next.cache);
        self.runtime.processing.configure(next.processing);
        self.preferences = next;
        self.navigation_notice = None;
        self.save_preferences();
        if !self.preferences.visible(self.step)
            && let Some(next) = self.preferences.neighbor(self.step, true)
        {
            self.switch_step(next);
        }
    }
    pub(super) fn navigation_steps(&self) -> Vec<Step> {
        // A cancelled departure leaves the current step reachable until the user leaves it.
        Step::ALL
            .into_iter()
            .filter(|s| self.preferences.visible(*s) || *s == self.step)
            .collect()
    }
    pub(super) fn workflow_shortcut(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        if !(modifiers.command || modifiers.ctrl) || !modifiers.alt || modifiers.shift {
            return false;
        }
        let Some(index) = [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
            egui::Key::Num5,
        ]
        .iter()
        .position(|k| *k == key) else {
            return false;
        };
        let step = Step::ALL[index];
        if self.preferences.visible(step) {
            self.switch_step(step);
        } else {
            self.set_status(format!(
                "{} is hidden · enable it in Settings",
                step.title()
            ));
        }
        true
    }
    pub(super) fn app_menu(&mut self, ui: &mut egui::Ui) {
        let available = matches!(
            self.runtime.updates.state(),
            crate::core::updates::State::Available(_)
        );
        ui.menu_button(
            if available {
                "Menu · Update available"
            } else {
                "Menu"
            },
            |ui| {
                if ui
                    .button(if available {
                        "Update available…"
                    } else {
                        "Check for updates…"
                    })
                    .clicked()
                {
                    self.update_dialog = true;
                    if !available {
                        self.start_update_check();
                    }
                    ui.close();
                }
                ui.separator();
                self.clear_trash_menu_item(ui);
                ui.separator();
                if ui.button("Settings…").clicked() {
                    self.open_settings(Section::General);
                    ui.close();
                }
                if ui.button("Cache usage…").clicked() {
                    self.open_settings(Section::Caches);
                    ui.close();
                }
                if ui.button("AI models…").clicked() {
                    self.open_model_setup();
                    ui.close();
                }
                if ui.button("Keyboard guide").clicked() {
                    self.help = true;
                    ui.close();
                }
                if ui.button("About PhotoSort").clicked() {
                    self.about = true;
                    ui.close();
                }
                if !self.preferences.recent_folders.is_empty() {
                    ui.separator();
                    ui.label("Recent folders");
                    let mut selected = None;
                    for path in &self.preferences.recent_folders {
                        if ui
                            .add_enabled(
                                !self.busy,
                                egui::Button::new(
                                    path.file_name().unwrap_or_default().to_string_lossy(),
                                ),
                            )
                            .on_hover_text(path.display().to_string())
                            .clicked()
                        {
                            selected = Some(path.clone());
                            ui.close();
                        }
                    }
                    if let Some(path) = selected {
                        self.open(path);
                    }
                }
            },
        );
    }
    pub(super) fn preferences_dialog(&mut self, ctx: &egui::Context) {
        if let Some(mut draft) = self.preferences_draft.take() {
            let mut action = 0;
            let mut section = self.preferences_section.take();
            let response = egui::Modal::new(egui::Id::new("preferences")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 64.0).clamp(280.0, 480.0));
                ui.heading("Settings");
                ui.horizontal(|ui| {
                    if ui.button("General").clicked() { section = Some(Section::General); }
                    if ui.button("Caches").clicked() { section = Some(Section::Caches); }
                });
                egui::ScrollArea::vertical()
                    .id_salt("settings_content")
                    .max_height((ctx.content_rect().height() - 200.0).max(160.0))
                    .show(ui, |ui| {
                        let general = ui.label("Application updates");
                        if section == Some(Section::General) { general.scroll_to_me(Some(egui::Align::Min)); }
                        ui.checkbox(&mut draft.updates.automatic, "Check for updates daily");
                        if let Err(reason) = crate::core::updates::Source::configured() { ui.small(reason); }
                        ui.separator();
                        ui.label("Choose your workflow");
                        ui.label("Organize and Cull always remain available.");
                        for step in Step::ALL {
                            let mut visible = draft.visible(step);
                            if ui
                                .add_enabled(
                                    !matches!(step, Step::Organize | Step::Cull),
                                    egui::Checkbox::new(&mut visible, step.title()),
                                )
                                .changed()
                            {
                                if visible {
                                    draft.hidden_steps.remove(&step);
                                } else {
                                    draft.hidden_steps.insert(step);
                                }
                            }
                        }
                        ui.separator();
                        ui.checkbox(&mut draft.show_shortcuts, "Show workflow shortcut hints");
                    ui.separator();
                    ui.label("Easy Delete sensitivity");
                    ui.add(egui::Slider::new(&mut draft.easy_delete.blur,10.0..=1000.0).text("Blur threshold"));
                    ui.add(egui::Slider::new(&mut draft.easy_delete.dark,0.0..=128.0).text("Near-black ceiling"));
                    ui.add(egui::Slider::new(&mut draft.easy_delete.bright,128.0..=255.0).text("Overexposed threshold"));
                    ui.label("Lower blur values flag fewer photos. Near-black suggestions still require an almost entirely black preview.");
                    ui.label("Applies to future Easy Delete analyses. Current reviews stay unchanged.");
                    if ui.button("Reset detection defaults").clicked() { draft.easy_delete = Default::default(); }
                    ui.separator();
                    ui.label("Visual similarity grouping");
                    ui.add(egui::Slider::new(&mut draft.similarity_distance,0.02..=0.20).fixed_decimals(3).text("Similarity distance"));
                    ui.label("Lower values make smaller, stricter groups. Used by the next Organize or Easy Delete analysis; current reviews stay unchanged.");
                    if ui.button("Reset visual grouping default").clicked() {draft.similarity_distance=crate::core::regional::DEFAULT_EPS;}
                    ui.separator();
                    ui.label("Same-subject grouping");
                    ui.horizontal_wrapped(|ui| {
                        use crate::core::subjects::Strictness;
                        for (value,label) in [(Strictness::Conservative,"Conservative"),(Strictness::Standard,"Standard"),(Strictness::Broad,"Broad")] {
                            ui.selectable_value(&mut draft.subject_strictness,value,label);
                        }
                    });
                    ui.label("Used by the next Cull or Pick Best analysis. Conservative keeps uncertain matches apart; Broad accepts more matches.");
                    ui.separator();
                    ui.label("Cull navigation");
                    ui.label("Up/Down and Previous/Next stop at:");
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(&mut draft.navigation.rating, None, "Any rating");
                        for rating in 0..=5 {
                            ui.selectable_value(&mut draft.navigation.rating, Some(rating), format!("{rating} ★"))
                                .on_hover_text(if rating==0 { "Jump to unrated photos".into() } else { format!("Jump to photos rated exactly {rating} stars") });
                        }
                    });
                    ui.checkbox(&mut draft.navigation.skip_singletons, "Skip single-photo groups with Up/Down");
                    ui.checkbox(&mut draft.navigation.cycle_similarity, "Cycle Left/Right within similarity groups");
                    ui.label("Rating jumps take priority. Ctrl/Cmd bypasses movement filters and includes Trash marks. Group options use Cull analysis results; photos remain visible.");
                    ui.separator();
                    ui.label("Saving ratings and rotation");
                    ui.horizontal_wrapped(|ui| {
                        use crate::core::metadata::WritePolicy;
                        ui.selectable_value(&mut draft.metadata_writes,WritePolicy::EmbedWhenSupported,"Embed where supported");
                        ui.selectable_value(&mut draft.metadata_writes,WritePolicy::SidecarOnly,"Sidecars only");
                    });
                    ui.label("Existing XMP sidecars remain authoritative. Unsupported formats use a sidecar. Review shows the destination before applying; photo pixels are never re-encoded.");
                    ui.separator();
                    ui.label("AI execution");
                    egui::ComboBox::from_id_salt("inference-backend")
                        .selected_text(draft.processing.backend.label())
                        .show_ui(ui, |ui| {
                            for backend in crate::core::processing::Backend::ALL {
                                ui.selectable_value(&mut draft.processing.backend, backend, backend.label());
                            }
                        });
                    ui.label("Applies to the next analysis in every workflow. Automatic chooses a platform provider. Missing or failing providers fall back to CPU per model.");
                    ui.label("Acceleration requires a compatible ONNX Runtime and driver. First model preparation can take minutes and extra memory. Native compilation finishes before cancellation can release it; cached photo results remain reusable.");
                    for status in self.runtime.processing.status() {
                        ui.label(format!("{} · {} · {}", status.model, status.backend.label(), status.detail));
                    }
                    ui.label("Session providers above can still use CPU for unsupported operations; they do not prove that every operation ran on a GPU.");
                    ui.label("AI CPU allocation");
                    ui.horizontal_wrapped(|ui| {
                        use crate::core::processing::Mode;
                        for (mode,label) in [(Mode::Balanced,"Balanced"),(Mode::Maximum,"Maximum"),(Mode::Custom,"Custom")] {
                            ui.selectable_value(&mut draft.processing.mode,mode,label);
                        }
                    });
                    let available=self.runtime.processing.available();
                    if draft.processing.mode==crate::core::processing::Mode::Custom {
                        ui.add(egui::Slider::new(&mut draft.processing.custom_threads,1..=available).text("AI threads"));
                    }
                    ui.label(format!("{} AI threads · {} available",draft.processing.threads(available),available));
                    ui.label("Balanced uses 85% of the available thread count for AI. Applies to the next analysis, including rotation. Cached results stay available.");
                    ui.label("This controls AI calculations, not a limit on total app CPU use. Browsing and file tasks remain in the background.");
                    ui.separator();
                    let caches = ui.label("Shared caches");
                    if section == Some(Section::Caches) { caches.scroll_to_me(Some(egui::Align::Min)); }
                    ui.add(egui::Slider::new(&mut draft.cache.previews_mib, 32..=1024).suffix(" MiB").text("Preview memory"));
                    ui.add(egui::Slider::new(&mut draft.cache.originals_mib, 64..=2048).suffix(" MiB").text("Original detail memory"));
                    ui.add(egui::Slider::new(&mut draft.cache.disk_mib, 0..=32768).suffix(" MiB").text("Disk cache"));
                    ui.label("Limits apply when settings are saved. Zero disables persistent previews, AI evidence and prepared models. Active model files can temporarily exceed the disk limit; reducing it releases this window’s prepared sessions. Displayed images and models use additional memory.");
                    let cache = self.runtime.cache.status();
                    if let Some(usage) = cache.usage {
                        ui.label(format!("Disk: {:.1} MiB · {} files", usage.disk_bytes as f64 / 1048576.0, usage.files));
                        if usage.pinned_disk_bytes > 0 { ui.label(format!("{:.1} MiB held by active model sessions", usage.pinned_disk_bytes as f64 / 1048576.0)); }
                        ui.label(format!("Cached pixels: {:.1} MiB previews · {:.1} MiB originals", usage.preview_bytes as f64 / 1048576.0, usage.original_bytes as f64 / 1048576.0));
                        ui.label(format!("{} metadata entries · {} analysis entries", usage.metadata_entries, usage.analysis_entries));
                    }
                    if let Some(message) = cache.message { ui.label(message); }
                    if cache.busy {
                        ui.label("Updating caches…");
                        if ui.button("Cancel cache task").clicked() { self.runtime.cache.cancel(); }
                    } else {
                        if ui.button("Refresh cache usage").clicked() { self.runtime.cache.request(false); }
                        if ui.add_enabled(!self.busy && !self.scanning, egui::Button::new("Clear reusable caches")).clicked() { self.runtime.cache.request(true); }
                    }
                    ui.label("Clear runs immediately in the background. Current reviews, staged choices, saved positions and downloaded models are kept.");

                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        action = 1;
                    }
                    if ui.button("Save settings").clicked() {
                        action = 2;
                    }
                });
            });
            if action == 0 && response.should_close() {
                action = 1;
            }
            match action {
                1 => {}
                2 => self.apply_preferences(draft),
                _ => self.preferences_draft = Some(draft),
            }
        }
        if self.about {
            egui::Window::new("About PhotoSort")
                .open(&mut self.about)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.heading("PhotoSort Rust");
                    ui.label(format!(
                        "Version {} · native preview",
                        env!("CARGO_PKG_VERSION")
                    ));
                    ui.label("Local photo organization and review. Built with Rust and egui.");
                    ui.label("Apache-2.0 license");
                });
        }
    }
    pub(super) fn workflow_footer(&mut self, ctx: &egui::Context) {
        if self.root.is_none() {
            return;
        }
        egui::TopBottomPanel::bottom("workflow_navigation").show(ctx, |ui| {
            ui.horizontal_wrapped(|ui| {
                let previous = self.preferences.neighbor(self.step, false);
                let next = self.preferences.neighbor(self.step, true);
                if ui
                    .add_enabled(
                        previous.is_some() && !self.busy,
                        egui::Button::new(
                            previous.map_or("First step".into(), |s| format!("< {}", s.title())),
                        ),
                    )
                    .clicked()
                {
                    self.switch_step(previous.unwrap());
                }
                if ui
                    .add_enabled(
                        next.is_some() && !self.busy,
                        egui::Button::new(
                            next.map_or("Final step".into(), |s| format!("{} >", s.title())),
                        ),
                    )
                    .clicked()
                {
                    self.switch_step(next.unwrap());
                }
                if self.preferences.show_shortcuts {
                    ui.label(
                        RichText::new("Ctrl/Cmd+Alt+1–5 · Switch step")
                            .small()
                            .color(MUTED),
                    );
                }
            });
            if self.preferences.show_shortcuts {
                ui.label(RichText::new(workflow_hint(self.step)).small().color(MUTED));
            }
        });
    }
}
pub(super) fn workflow_hint(step: Step) -> &'static str {
    match step {
        Step::Organize => {
            "Arrows · Browse   Shift+arrows · Compare   I · Details   Shift+Enter · Review changes"
        }
        Step::EasyDelete => {
            "1/Left, 2/Right · Keep/Trash   Up/Down · Browse   Enter · Confirm   R · Reset   Shift+R · Reset all   A · Accept defaults"
        }
        Step::Rotation => {
            "1/2 · Original/Correction   Q/E · Rotate   Enter · Confirm   R · Reset   Shift+R · Reset all   Shift+Enter · Apply confirmed"
        }
        Step::PickBest => {
            "1/2 · Keep/Trash   Left/Right · Cluster   Up/Down · Comparison   C · Focus   K · Keep all   Enter · Confirm   R · Reset"
        }
        Step::Cull => {
            "Left/Right · Cycle   Up/Down · Browse   Ctrl/Cmd+arrows · Bypass movement filters   D · Mark   R · Rotate   Ctrl/Cmd+0–5 · Rate   Shift+Enter · Review"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
        let dir = tempfile::tempdir().unwrap();
        let app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        (dir, app)
    }
    #[test]
    fn hiding_current_step_uses_departure_guard_and_keeps_it_reachable_until_resolved() {
        let (_dir, mut app) = fixture();
        app.step = Step::Rotation;
        app.decisions.insert(
            MediaId(1),
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        let mut p = Preferences::default();
        p.hidden_steps.extend([Step::Rotation, Step::PickBest]);
        app.apply_preferences(p);
        assert_eq!(app.step, Step::Rotation);
        assert!(matches!(app.departure, Some(Destination::Step(Step::Cull))));
        assert!(app.navigation_steps().contains(&Step::Rotation));
        assert!(app.decisions.contains_key(&MediaId(1)));
        let destination = app.departure.take().unwrap();
        app.discard_staged_choices();
        app.continue_departure(destination);
        assert_eq!(app.step, Step::Cull);
        assert!(!app.navigation_steps().contains(&Step::Rotation));
    }
    #[test]
    fn hidden_saved_workflow_falls_forward_without_losing_its_bookmark_or_running_models() {
        let (_dir, mut app) = fixture();
        app.preferences
            .hidden_steps
            .extend([Step::PickBest, Step::EasyDelete, Step::Rotation]);
        let saved = crate::core::views::Saved {
            step: Some(Step::PickBest),
            bookmarks: [(Step::PickBest, Default::default())].into_iter().collect(),
            ..Default::default()
        };
        app.load_bookmarks(saved);
        assert_eq!(app.step, Step::Cull);
        assert!(app.saved_views.bookmarks.contains_key(&Step::PickBest));
        assert!(app.analysis_run.is_none() && app.requested.is_empty() && app.textures.is_empty());
        assert!(app.workflow_shortcut(
            egui::Key::Num4,
            egui::Modifiers {
                ctrl: true,
                alt: true,
                ..Default::default()
            }
        ));
        assert_eq!(app.step, Step::Cull);
        assert!(!app.workflow_shortcut(
            egui::Key::Num4,
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            }
        ));
        assert!(app.workflow_shortcut(
            egui::Key::Num1,
            egui::Modifiers {
                command: true,
                alt: true,
                ..Default::default()
            }
        ));
        assert_eq!(app.step, Step::Organize);
        // A new folder has no bookmark, but must not reopen a hidden step retained
        // by a previously cancelled departure.
        app.step = Step::Rotation;
        app.load_bookmarks(Default::default());
        assert_eq!(app.step, Step::Cull);
    }
    #[test]
    fn changing_sensitivity_preserves_current_reviews_decisions_and_cached_results() {
        let (_dir, mut app) = fixture();
        app.step = Step::EasyDelete;
        app.analysis.insert(
            MediaId(1),
            Analysis {
                issues: vec!["Possibly soft".into()],
                ..Default::default()
            },
        );
        app.decisions.insert(
            MediaId(1),
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        let mut next = app.preferences.clone();
        next.easy_delete.blur = 10.0;
        app.apply_preferences(next);
        assert_eq!(app.preferences.easy_delete.blur, 10.0);
        assert_eq!(app.analysis[&MediaId(1)].issues, vec!["Possibly soft"]);
        assert_eq!(app.decisions[&MediaId(1)].rating, Some(4));
        assert!(app.analysis_run.is_none() && app.requested.is_empty());
    }
    #[test]
    fn saving_cpu_allocation_updates_shared_runtime_without_restarting_current_review() {
        use crate::core::processing::{Mode, Preferences as Cpu};
        let (_dir, mut app) = fixture();
        app.analysis_run = Some(77);
        app.step = Step::PickBest;
        app.decisions.insert(
            MediaId(1),
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        let mut next = app.preferences.clone();
        next.processing = Cpu {
            backend: crate::core::processing::Backend::CoreMl,
            mode: Mode::Custom,
            custom_threads: 1,
        };
        app.apply_preferences(next);
        assert_eq!(app.runtime.processing.snapshot(), 1);
        assert_eq!(
            app.runtime.processing.snapshot_run().backend,
            crate::core::processing::Backend::CoreMl
        );
        assert_eq!(app.analysis_run, Some(77));
        assert_eq!(app.step, Step::PickBest);
        assert_eq!(app.decisions[&MediaId(1)].rating, Some(4));
        assert!(app.departure.is_none());
    }
    #[test]
    fn settings_preserve_live_history_and_render_compactly_without_image_or_analysis_work() {
        let (dir, mut app) = fixture();
        let draft = app.preferences.clone();
        app.preferences.remember(dir.path());
        app.apply_preferences(draft);
        assert_eq!(
            app.preferences.recent_folders,
            vec![dir.path().to_path_buf()]
        );
        app.preferences_draft = Some(app.preferences.clone());
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 600.0),
                )),
                ..Default::default()
            },
            |ctx| app.preferences_dialog(ctx),
        );
        assert!(!output.shapes.is_empty());
        assert!(
            app.analysis_run.is_none() && app.metadata_run.is_none() && app.requested.is_empty()
        );
        assert!(app.preferences_draft.is_some());
    }
    #[test]
    fn clearing_shared_caches_preserves_every_workflows_review_and_bookmark_state() {
        let (_dir, mut app) = fixture();
        for step in Step::ALL {
            app.step = step;
            app.saved_views.bookmarks.insert(step, Default::default());
            app.decisions.insert(
                MediaId(1),
                Decision {
                    rating: Some(4),
                    rejected: true,
                    ..Default::default()
                },
            );
            app.analysis.insert(
                MediaId(1),
                Analysis {
                    issues: vec!["Possibly soft".into()],
                    ..Default::default()
                },
            );
            let saved = serde_json::to_vec(&app.saved_views).unwrap();
            app.stats.resident_bytes = 123;
            app.stats.detail_bytes = 456;
            assert!(app.runtime.cache.request(true));
            let deadline = Instant::now() + std::time::Duration::from_secs(5);
            while app.runtime.cache.status().busy {
                assert!(Instant::now() < deadline);
                app.poll(&egui::Context::default());
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            app.poll(&egui::Context::default());
            assert_eq!((app.stats.resident_bytes, app.stats.detail_bytes), (0, 0));
            assert_eq!(app.step, step);
            assert_eq!(serde_json::to_vec(&app.saved_views).unwrap(), saved);
            assert_eq!(app.decisions[&MediaId(1)].rating, Some(4));
            assert!(app.decisions[&MediaId(1)].rejected);
            assert_eq!(app.analysis[&MediaId(1)].issues, vec!["Possibly soft"]);
            assert!(app.analysis_run.is_none() && app.metadata_run.is_none() && !app.scanning);
        }
    }
    #[test]
    fn escape_cancels_settings_without_applying_draft_navigation() {
        let (_dir, mut app) = fixture();
        let mut draft = app.preferences.clone();
        draft.navigation.rating = Some(5);
        app.preferences_draft = Some(draft);
        let ctx = egui::Context::default();
        let raw = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(640.0, 600.0),
            )),
            ..Default::default()
        };
        for _ in 0..2 {
            let _ = ctx.run(raw(), |ctx| app.preferences_dialog(ctx));
        }
        let mut input = raw();
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(input, |ctx| app.preferences_dialog(ctx));
        assert!(app.preferences_draft.is_none());
        assert_eq!(app.preferences.navigation.rating, None);
        assert!(app.analysis_run.is_none() && app.metadata_run.is_none());
    }
    #[test]
    fn settings_access_reuses_draft_and_requests_usage_once_without_clearing() {
        let (_dir, mut app) = fixture();
        app.runtime.shutdown();
        app.runtime.cache.take();
        app.runtime.cache.finish(Ok(Default::default()), false);
        app.open_settings(Section::Caches);
        let request = app.runtime.cache.take().expect("one usage request");
        assert!(!request.clear);
        app.runtime.cache.finish(Ok(Default::default()), false);
        app.preferences_draft.as_mut().unwrap().navigation.rating = Some(5);
        app.open_settings(Section::General);
        app.open_settings(Section::Caches);
        assert!(
            app.runtime.cache.take().is_none(),
            "section navigation repeated cache work"
        );
        assert_eq!(
            app.preferences_draft.as_ref().unwrap().navigation.rating,
            Some(5)
        );
        assert_eq!(app.preferences.navigation.rating, None);
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
    #[test]
    fn settings_access_cache_and_general_anchors_keep_footer_visible_at_small_sizes() {
        for size in [egui::vec2(600., 500.), egui::vec2(1440., 900.)] {
            let (_dir, mut app) = fixture();
            let ctx = egui::Context::default();
            crate::ui::theme::install(&ctx);
            ctx.enable_accesskit();
            let mut time = 0.;
            for (section, label) in [
                (Section::Caches, "Shared caches"),
                (Section::General, "Application updates"),
                (Section::Caches, "Shared caches"),
            ] {
                app.open_settings(section);
                let mut output = None;
                for _ in 0..8 {
                    time += 0.1;
                    output = Some(ctx.run(
                        egui::RawInput {
                            time: Some(time),
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ctx| app.preferences_dialog(ctx),
                    ));
                }
                let tree = output.unwrap().platform_output.accesskit_update.unwrap();
                for label in [label, "General", "Caches", "Cancel", "Save settings"] {
                    let node = tree
                        .nodes
                        .iter()
                        .find(|(_, n)| n.label() == Some(label) || n.value() == Some(label))
                        .unwrap_or_else(|| {
                            panic!(
                                "missing {label} at {size:?}: {:?}",
                                tree.nodes
                                    .iter()
                                    .filter_map(|(_, n)| n.label())
                                    .collect::<Vec<_>>()
                            )
                        });
                    let bounds = node.1.bounds().expect("visible control bounds");
                    assert!(
                        bounds.x0 >= 0.
                            && bounds.x1 <= f64::from(size.x)
                            && bounds.y0 >= 0.
                            && bounds.y1 <= f64::from(size.y),
                        "{label}: {bounds:?}, {size:?}"
                    );
                }
            }
            assert!(
                app.analysis_run.is_none()
                    && app.metadata_run.is_none()
                    && app.requested.is_empty()
            );
            app.runtime.shutdown();
        }
    }
}
