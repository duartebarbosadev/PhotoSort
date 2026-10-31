//! Application controls shared by every workflow and both navigation layouts.
use super::*;
use crate::core::preferences::Preferences;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Section {
    General,
    Caches,
}

impl PhotoSortApp {
    fn cache_folder_request(&self) -> Option<crate::core::desktop::Request> {
        self.runtime.cache.directory().and_then(|path| {
            crate::core::desktop::Request::for_path(path, crate::core::desktop::Action::OpenFolder)
                .ok()
        })
    }
    fn cache_folder_button(&mut self, ui: &mut egui::Ui) {
        let request = self.cache_folder_request();
        let description = request.as_ref().map_or_else(
            || "The cache location is unavailable.".into(),
            |request| request.path.display().to_string(),
        );
        if ui
            .add_enabled(
                request.is_some() && self.desktop_available(),
                egui::Button::new("Open cache folder"),
            )
            .on_hover_text(description)
            .clicked()
            && let Some(request) = request
        {
            self.dispatch_desktop(request);
        }
    }
    pub(super) fn open_settings(&mut self, section: Section) {
        if self.preferences_draft.is_none() {
            self.preferences_draft = Some(self.preferences.clone());
            self.runtime.cache.request(false);
        }
        self.preferences_section = Some(section);
        self.forget_confirm = false;
        self.forget_message = None;
    }
    /// Every folder, the open one included, opens next time as if it had never
    /// been opened. The open folder keeps its current views until then.
    pub(super) fn forget_remembered_folders(&mut self) {
        self.forget_confirm = false;
        match self
            .runtime
            .io(IoWork::ForgetViews(crate::core::session::data_dir()))
        {
            Ok(()) => {
                self.views_forgotten = true;
                self.forget_message =
                    Some("Remembered folders forgotten. Each one opens fresh next time.".into());
            }
            Err(error) => self.error = Some(error),
        }
    }
    fn clear_recent_folders(&mut self) {
        self.preferences.recent_folders.clear();
        self.missing_recent.clear();
        self.save_preferences();
        self.forget_message = Some("Recent folders cleared.".into());
    }

    /// Drops RAW thumbnails and previews so they are requested again in the
    /// new look. Other photos and all analysis results are kept.
    fn reload_raw_previews(&mut self) {
        let raw: HashSet<MediaId> = self
            .media
            .iter()
            .filter(|m| m.kind == MediaKind::Raw)
            .map(|m| m.id)
            .collect();
        self.textures.retain(|(id, _), _| !raw.contains(id));
        self.requested.retain(|(id, _)| !raw.contains(id));
        self.inspection_frames.clear();
    }
    /// Loads the saved preferences. A file that cannot be read keeps the
    /// defaults, says why, and is remembered as unreadable.
    pub(super) fn load_preferences(&mut self, path: PathBuf) {
        match Preferences::load(&path) {
            Ok(preferences) => {
                self.companions = preferences.keep_companions;
                self.intro = super::intro::Intro::at_startup(&preferences, super::intro::AUTOPLAY);
                self.preferences = preferences;
            }
            Err(e) => {
                self.preferences_unreadable = true;
                self.error = Some(format!("Preferences could not be loaded: {e}"));
            }
        }
        self.preferences_path = Some(path);
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
        next.intro_shown = self.preferences.intro_shown;
        // The size sliders own the photo size, not the dialog.
        next.photo_size = self.preferences.photo_size;
        next.normalize();
        self.runtime.cache.configure(next.cache);
        self.runtime.processing.configure(next.processing);
        self.reduce_motion_pending = Some(next.reduce_motion);
        if next.raw_look != self.preferences.raw_look {
            self.runtime.raw_look.set(next.raw_look);
            self.reload_raw_previews();
        }
        let auto_on = next.easy_delete_auto && !self.preferences.easy_delete_auto;
        let pick_auto_on = next.pick_best_auto && !self.preferences.pick_best_auto;
        let rotation_auto_on = next.rotation_auto && !self.preferences.rotation_auto;
        self.preferences = next;
        self.navigation_notice = None;
        self.save_preferences();
        if auto_on && self.step == Step::EasyDelete {
            self.stage_delete_suggestions();
        }
        if pick_auto_on && self.step == Step::PickBest {
            self.stage_pick_suggestions();
        }
        if rotation_auto_on && self.step == Step::Rotation {
            self.stage_rotation_suggestions();
        }
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
                if crate::core::playback::supported() && ui.button("Watch the intro").clicked() {
                    self.watch_intro();
                    ui.close();
                }
                if ui.button("About PhotoSort").clicked() {
                    self.about = true;
                    ui.close();
                }
                let recent: Vec<_> = self
                    .preferences
                    .recent_folders
                    .iter()
                    .filter(|p| !self.missing_recent.contains(*p))
                    .collect();
                if !recent.is_empty() {
                    ui.separator();
                    ui.label("Recent folders");
                    let mut selected = None;
                    for path in recent {
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
            let response = modal(egui::Id::new("preferences")).show(ctx, |ui| {
                ui.set_width((ctx.content_rect().width() - 64.0).clamp(280.0, 560.0));
                ui.heading("Settings");
                let error_height = if let Some(error) = self.error.clone() {
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical().id_salt("settings_error")
                            .max_height(80.0).show(ui, |ui| { ui.colored_label(RED, error); });
                        if ui.button("Dismiss error").clicked() { self.error = None; }
                    }).response.rect.height() + ui.spacing().item_spacing.y
                } else { 0.0 };
                // The highlighted tab follows the last section chosen this session.
                let tab_id = egui::Id::new("settings_tab");
                let mut current = section.unwrap_or_else(|| {
                    ctx.data(|d| d.get_temp::<Section>(tab_id)).unwrap_or(Section::General)
                });
                ui.horizontal(|ui| {
                    for (tab, label) in [(Section::General, "General"), (Section::Caches, "Caches")] {
                        if chip(ui, current == tab, label).clicked() {
                            section = Some(tab);
                            current = tab;
                        }
                    }
                });
                ctx.data_mut(|d| d.insert_temp(tab_id, current));
                egui::ScrollArea::vertical()
                    .id_salt("settings_content")
                    .max_height((ctx.content_rect().height() - 200.0 - error_height).max(80.0))
                    .show(ui, |ui| {
                        let general = settings_heading(ui, "Application updates");
                        if section == Some(Section::General) { general.scroll_to_me(Some(egui::Align::Min)); }
                        ui.add(Switch::new(&mut draft.updates.automatic, "Check for updates daily"));
                        if let Err(reason) = crate::core::updates::Source::configured() { ui.small(reason); }
                        ui.separator();
                        settings_heading(ui, "Choose your workflow");
                        settings_note(ui, "Organize and Cull always remain available.");
                        ui.horizontal_wrapped(|ui| {
                            for step in Step::ALL {
                                let visible = draft.visible(step);
                                let fixed = matches!(step, Step::Organize | Step::Cull);
                                let response = ui
                                    .add_enabled_ui(!fixed, |ui| chip(ui, visible, step.title()))
                                    .inner
                                    .on_hover_text(if fixed {
                                        "Always available"
                                    } else if visible {
                                        "Shown · click to hide this step"
                                    } else {
                                        "Hidden · click to show this step"
                                    });
                                if response.clicked() {
                                    if visible {
                                        draft.hidden_steps.insert(step);
                                    } else {
                                        draft.hidden_steps.remove(&step);
                                    }
                                }
                            }
                        });
                        ui.separator();
                        ui.add(Switch::new(&mut draft.show_shortcuts, "Show workflow shortcut hints"));
                        ui.add(Switch::new(&mut draft.reduce_motion, "Reduce motion"))
                            .on_hover_text("Panels, tabs and photos appear instantly instead of sliding and fading");
                        ui.separator();
                        settings_heading(ui, "Photo strip and file names");
                        ui.horizontal_wrapped(|ui| {
                            use crate::core::preferences::StripPosition;
                            ui.label("Photo strip in Review:");
                            ui.selectable_value(&mut draft.strip_position, StripPosition::Bottom, "Bottom");
                            ui.selectable_value(&mut draft.strip_position, StripPosition::Left, "Left");
                        });
                        ui.add(Switch::new(&mut draft.show_filenames, "Show file names under thumbnails"));
                        settings_note(ui, "Applies to every step's grid, photo strip and Review. Cull's View options change the same settings.");
                    ui.separator();
                    settings_heading(ui, "Easy Delete suggestions");
                    ui.add(Switch::new(&mut draft.easy_delete_auto, "Apply suggestions automatically"));
                    settings_note(ui, "On: what Easy Delete suggests is staged as your choice, and you change only the ones you disagree with. Off: every photo stays kept until you accept suggestions. Nothing changes on disk until you apply Review changes.");
                    ui.separator();
                    settings_heading(ui, "Pick Best suggestions");
                    ui.add(Switch::new(&mut draft.pick_best_auto, "Apply suggestions automatically"));
                    settings_note(ui, "On: the best photo of each group is kept and the others are staged for Trash, and you change only the groups you disagree with. Off: every photo stays kept until you accept suggestions.");
                    ui.separator();
                    settings_heading(ui, "Easy Delete sensitivity");
                    ui.add(egui::Slider::new(&mut draft.easy_delete.blur,10.0..=1000.0).text("Blur threshold"));
                    ui.add(egui::Slider::new(&mut draft.easy_delete.dark,0.0..=128.0).text("Near-black ceiling"));
                    ui.add(egui::Slider::new(&mut draft.easy_delete.bright,128.0..=255.0).text("Overexposed threshold"));
                    settings_note(ui, "Lower blur values flag fewer photos. Near-black suggestions still require an almost entirely black preview.");
                    settings_note(ui, "Applies to future Easy Delete analyses. Current reviews stay unchanged.");
                    if ui.button("Reset detection defaults").clicked() { draft.easy_delete = Default::default(); }
                    ui.separator();
                    settings_heading(ui, "Fix Rotation suggestions");
                    ui.add(Switch::new(&mut draft.rotation_auto, "Apply suggestions automatically"));
                    settings_note(ui, "On: photos Fix Rotation is sure about are staged turned, and you change only the ones you disagree with. Off: every photo stays as it is until you use a suggestion. Nothing changes on disk until you apply Review changes.");
                    ui.separator();
                    settings_heading(ui, "Visual similarity grouping");
                    ui.add(egui::Slider::new(&mut draft.similarity_distance,0.02..=0.20).fixed_decimals(3).text("Similarity distance"));
                    settings_note(ui, "Lower values make smaller, stricter groups. Used by the next Organize or Easy Delete analysis; current reviews stay unchanged.");
                    if ui.button("Reset visual grouping default").clicked() {draft.similarity_distance=crate::core::regional::DEFAULT_EPS;}
                    ui.separator();
                    settings_heading(ui, "Same-subject grouping");
                    ui.horizontal_wrapped(|ui| {
                        use crate::core::subjects::Strictness;
                        for (value,label) in [(Strictness::Conservative,"Conservative"),(Strictness::Standard,"Standard"),(Strictness::Broad,"Broad")] {
                            ui.selectable_value(&mut draft.subject_strictness,value,label);
                        }
                    });
                    settings_note(ui, "Used by the next Cull or Pick Best analysis. Conservative keeps uncertain matches apart; Broad accepts more matches.");
                    ui.separator();
                    settings_heading(ui, "Cull navigation");
                    ui.label("Up/Down and Previous/Next stop at:");
                    ui.horizontal_wrapped(|ui| {
                        ui.selectable_value(&mut draft.navigation.rating, None, "Any rating");
                        for rating in 0..=5 {
                            ui.selectable_value(&mut draft.navigation.rating, Some(rating), format!("{rating} ★"))
                                .on_hover_text(if rating==0 { "Jump to unrated photos".into() } else { format!("Jump to photos rated exactly {rating} stars") });
                        }
                    });
                    ui.add(Switch::new(&mut draft.navigation.skip_singletons, "Skip single-photo groups with Up/Down"));
                    ui.add(Switch::new(&mut draft.navigation.cycle_similarity, "Cycle Left/Right within similarity groups"));
                    settings_note(ui, "Rating jumps take priority. Ctrl/Cmd bypasses movement filters and includes Trash marks. Group options use Cull analysis results; photos remain visible.");
                    ui.separator();
                    settings_heading(ui, "RAW photos");
                    ui.horizontal_wrapped(|ui| {
                        use crate::core::raw_look::RawLook;
                        ui.selectable_value(&mut draft.raw_look, RawLook::Neutral, "Neutral");
                        ui.selectable_value(&mut draft.raw_look, RawLook::Enhanced, "Enhanced");
                    });
                    settings_note(ui, "Enhanced stretches contrast and adds a little colour to RAW previews, like a camera's JPEG. It only changes how photos look here: files, scores and suggestions are unchanged.");
                    ui.separator();
                    settings_heading(ui, "Saving ratings and rotation");
                    ui.horizontal_wrapped(|ui| {
                        use crate::core::metadata::WritePolicy;
                        ui.selectable_value(&mut draft.metadata_writes,WritePolicy::EmbedWhenSupported,"Embed where supported");
                        ui.selectable_value(&mut draft.metadata_writes,WritePolicy::SidecarOnly,"Sidecars only");
                        ui.selectable_value(&mut draft.metadata_writes,WritePolicy::EmbedOrRotatePixels,"Embed, or rotate pixels");
                    });
                    settings_note(ui, if draft.metadata_writes == crate::core::metadata::WritePolicy::EmbedOrRotatePixels {
                        "Like Embed where supported, but rotating a file that cannot store orientation (such as BMP) rewrites its pixels instead of using a sidecar. Pixels and colour profile are kept exactly; other data stored inside those files is not. Review lists each one before applying."
                    } else {
                        "Existing XMP sidecars remain authoritative. Unsupported formats use a sidecar. Review shows the destination before applying; photo pixels are not re-encoded."
                    });
                    ui.separator();
                    settings_heading(ui, "AI execution");
                    egui::ComboBox::from_id_salt("inference-backend")
                        .selected_text(draft.processing.backend.label())
                        .show_ui(ui, |ui| {
                            for backend in crate::core::processing::Backend::ALL {
                                ui.selectable_value(&mut draft.processing.backend, backend, backend.label());
                            }
                        });
                    settings_note(ui, "Applies to the next analysis in every workflow. Automatic chooses a platform provider and is the default on Mac. Missing or failing providers fall back to CPU per model.");
                    settings_note(ui, "Acceleration requires a compatible ONNX Runtime and driver. First model preparation can take minutes and extra memory. Native compilation finishes before cancellation can release it; cached photo results remain reusable.");
                    for status in self.runtime.processing.status() {
                        ui.label(format!("{} · {} · {}", status.model, status.backend.label(), status.detail));
                    }
                    settings_note(ui, "Session providers above can still use CPU for unsupported operations; they do not prove that every operation ran on a GPU.");
                    settings_heading(ui, "AI CPU allocation");
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
                    settings_note(ui, "Balanced uses 85% of the available thread count for AI. Applies to the next analysis, including rotation. Cached results stay available.");
                    settings_note(ui, "This controls AI calculations, not a limit on total app CPU use. Browsing and file tasks remain in the background.");
                    ui.separator();
                    let caches = settings_heading(ui, "Shared caches");
                    if section == Some(Section::Caches) { caches.scroll_to_me(Some(egui::Align::Min)); }
                    self.cache_folder_button(ui);
                    ui.add(egui::Slider::new(&mut draft.cache.previews_mib, 32..=1024).suffix(" MiB").text("Preview memory"));
                    ui.add(egui::Slider::new(&mut draft.cache.originals_mib, 64..=2048).suffix(" MiB").text("Original detail memory"));
                    ui.add(egui::Slider::new(&mut draft.cache.disk_mib, 0..=32768).suffix(" MiB").text("Disk cache"));
                    settings_note(ui, "Limits apply when settings are saved. Zero disables persistent previews, AI evidence and prepared models. Active model files can temporarily exceed the disk limit; reducing it releases this window’s prepared sessions. Displayed images and models use additional memory.");
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
                    settings_note(ui, "Clear runs immediately in the background. Current reviews, staged choices, saved positions and downloaded models are kept.");
                    ui.separator();
                    settings_heading(ui, "Remembered folders");
                    let idle = !self.busy && !self.scanning;
                    if self.forget_confirm {
                        ui.label("Forget the step and views saved for every folder?");
                        ui.horizontal(|ui| {
                            if ui.add_enabled(idle, egui::Button::new("Forget")).clicked() { self.forget_remembered_folders(); }
                            if ui.button("Keep them").clicked() { self.forget_confirm = false; }
                        });
                    } else if ui.add_enabled(idle, egui::Button::new("Forget remembered folders")).clicked() {
                        self.forget_confirm = true;
                        self.forget_message = None;
                    }
                    settings_note(ui, "Every folder opens fresh in Organize. Staged changes and preferences are kept. The open folder keeps its views until you open it again.");
                    if ui.add_enabled(!self.preferences.recent_folders.is_empty(), egui::Button::new("Clear recent folders")).clicked() { self.clear_recent_folders(); }
                    settings_note(ui, "Empties the Recent folders list. The folders themselves are not touched.");
                    if let Some(message) = &self.forget_message { ui.label(message); }

                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        action = 1;
                    }
                    if ui.add(primary("Save settings")).clicked() {
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
            let mut open = self.about;
            let mut folder = None;
            egui::Window::new("About PhotoSort")
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .default_width(380.0)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            RichText::new(egui_phosphor::regular::APERTURE)
                                .size(30.0)
                                .color(ACCENT),
                        );
                        ui.vertical(|ui| {
                            ui.label(RichText::new("PhotoSort").font(semibold(18.0)));
                            ui.label(
                                RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                                    .small()
                                    .color(MUTED),
                            );
                        });
                    });
                    ui.add_space(S2);
                    ui.label("Sort, clean up and pick the keepers from your photos. Everything runs on this computer.");
                    ui.add_space(S3);
                    ui.label(RichText::new("Built with").font(medium(12.5)));
                    let backend = self
                        .runtime
                        .processing
                        .status()
                        .first()
                        .map_or("CPU".to_owned(), |s| s.backend.label().to_owned());
                    for (label, value) in [
                        ("Interface", "egui and wgpu".to_owned()),
                        ("AI models", format!("ONNX Runtime · {backend}")),
                        (
                            "Images",
                            if cfg!(feature = "native-imaging") { "libvips" } else { "image-rs" }.to_owned(),
                        ),
                        (
                            "Metadata",
                            if cfg!(feature = "metadata-native") { "Exiv2" } else { "kamadak-exif" }.to_owned(),
                        ),
                        ("Typeface", "Inter".to_owned()),
                    ] {
                        ui.horizontal(|ui| {
                            ui.allocate_ui_with_layout(
                                egui::vec2(96.0, 18.0),
                                egui::Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.set_width(96.0);
                                    ui.label(RichText::new(label).color(MUTED));
                                },
                            );
                            ui.label(value);
                        });
                    }
                    ui.add_space(S3);
                    ui.horizontal_wrapped(|ui| {
                        let models = crate::core::models::resource_roots()
                            .into_iter()
                            .map(|root| root.join("models"))
                            .find(|path| path.is_dir());
                        if ui
                            .add_enabled(models.is_some(), egui::Button::new("Models folder"))
                            .clicked()
                        {
                            folder = models;
                        }
                        if ui.button("Crash reports").on_hover_text("Reports stay on this computer").clicked() {
                            let path = crate::core::crash::directory();
                            let _ = std::fs::create_dir_all(&path);
                            folder = Some(path);
                        }
                        if let Some(repository) = option_env!("PHOTOSORT_RELEASE_REPOSITORY")
                            && ui.button("Project page").clicked()
                        {
                            ui.ctx().open_url(egui::OpenUrl::new_tab(format!(
                                "https://github.com/{repository}"
                            )));
                        }
                    });
                    ui.add_space(S2);
                    ui.label(RichText::new("Apache-2.0 license. Fonts: SIL Open Font License 1.1.").small().color(FAINT));
                });
            self.about = open;
            if let Some(path) = folder
                && let Ok(request) = crate::core::desktop::Request::for_path(
                    &path,
                    crate::core::desktop::Action::OpenFolder,
                )
            {
                self.dispatch_desktop(request);
            }
        }
    }
}
pub(super) fn workflow_hint(step: Step) -> &'static str {
    match step {
        Step::Organize => {
            "Alt+1–5 · Grouping mode   Arrows · Browse   Shift+arrows · Compare   F2 · Rename   Hold \\ · Original   Shift+Enter · Review changes"
        }
        Step::EasyDelete => {
            "1/Left, 2/Right · Keep/Trash   Up/Down · Browse   Enter · Next   R · Undo   A · Accept all suggestions   Esc · Close photos   Shift+Enter · Review changes"
        }
        Step::Rotation => {
            "Arrows · Browse   Q/E · Rotate   R · Undo   Enter · Next   Esc · Close photos   Shift+Enter · Review changes"
        }
        Step::PickBest => {
            "1–9 · Keep/Trash   Arrows · Group   Enter · Show/Next   R · Undo   A · Accept all suggestions   Esc · Close photos   Shift+Enter · Review changes"
        }
        Step::Cull => {
            "Left/Right · Cycle   Up/Down · Browse   Ctrl/Cmd+arrows · Bypass movement filters   D · Mark   R · Rotate   Ctrl/Cmd+0–5 · Rate   Shift+Enter · Review"
        }
    }
}

#[cfg(test)]
#[path = "preferences/tests.rs"]
mod tests;

/// A settings group title with room above it; returns the label for scrolling.
fn settings_heading(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add_space(S2);
    ui.label(RichText::new(text).font(semibold(14.0)))
}
/// Secondary explanation under a setting.
fn settings_note(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).small().color(MUTED)).wrap());
}
