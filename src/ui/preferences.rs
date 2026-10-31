//! Application controls shared by every workflow and both navigation layouts.
use super::*;
use crate::core::preferences::Preferences;
#[path = "preferences/pages.rs"]
mod pages;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Section {
    General,
    Appearance,
    Steps,
    Performance,
    Storage,
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
            let requested = self.preferences_section.take();
            let screen = ctx.content_rect();
            let width = (screen.width() - 88.0).clamp(280.0, 840.0);
            let compact = width < pages::COMPACT_WIDTH;
            let response = modal(egui::Id::new("preferences")).show(ctx, |ui| {
                ui.set_width(width);
                ui.horizontal(|ui| {
                    ui.heading("Settings");
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if icon_button(ui, egui_phosphor::regular::X, "Close without saving")
                            .clicked()
                        {
                            action = 1;
                        }
                    });
                });
                let error_height = if let Some(error) = self.error.clone() {
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("settings_error")
                            .max_height(80.0)
                            .show(ui, |ui| {
                                ui.colored_label(RED, error);
                            });
                        if ui.button("Dismiss error").clicked() {
                            self.error = None;
                        }
                    })
                    .response
                    .rect
                    .height()
                        + ui.spacing().item_spacing.y
                } else {
                    0.0
                };
                ui.add_space(S2);
                // The highlighted page follows the last one chosen this session.
                let page_id = egui::Id::new("settings_tab");
                let mut current = requested.unwrap_or_else(|| {
                    ctx.data(|d| d.get_temp::<Section>(page_id))
                        .unwrap_or(Section::General)
                });
                let body = (screen.height() - 220.0 - error_height).clamp(120.0, 600.0);
                ui.horizontal_top(|ui| {
                    if let Some(chosen) =
                        pages::sidebar(ui, current, compact, &draft, &self.preferences)
                    {
                        current = chosen;
                    }
                    ui.add_space(if compact { S2 } else { S4 });
                    ui.vertical(|ui| {
                        egui::ScrollArea::vertical()
                            .id_salt(("settings_page", current))
                            .max_height(body)
                            .auto_shrink([false, false])
                            .show(ui, |ui| self.settings_page(ui, current, &mut draft));
                    });
                });
                ctx.data_mut(|d| d.insert_temp(page_id, current));
                ui.add_space(S2);
                ui.separator();
                let pending = pages::has_pending(&draft, &self.preferences);
                pages::fixed_width(ui, |ui| {
                    ui.horizontal(|ui| {
                        if pending {
                            let (dot, _) =
                                ui.allocate_exact_size(egui::vec2(8.0, 8.0), egui::Sense::hover());
                            ui.painter().circle_filled(dot.center(), 3.5, ACCENT);
                            ui.label(RichText::new("Unsaved changes").color(ACCENT));
                        } else {
                            ui.label(RichText::new("Changes apply when you save.").color(FAINT));
                        }
                        pages::trailing(ui, egui::Id::new("settings_footer"), |ui| {
                            if ui.button("Cancel").clicked() {
                                action = 1;
                            }
                            if ui.add_enabled(pending, primary("Save settings")).clicked() {
                                action = 2;
                            }
                        });
                    })
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
                        crate::ui::brand::app_logo(ui, 44.0);
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
            "Alt+1–5 · Grouping mode   Arrows · Browse   Shift+arrows · Compare   F2 · Rename   Hold \\ · Original   Ctrl/Cmd+C · Copy   Shift+Enter · Review changes"
        }
        Step::EasyDelete => {
            "1/Left, 2/Right · Keep/Trash   Up/Down · Browse   Enter · Next   R · Undo   A · Accept all suggestions   Esc · Close photos   Ctrl/Cmd+C · Copy   Shift+Enter · Review changes"
        }
        Step::Rotation => {
            "Arrows · Browse   Q/E · Rotate   R · Undo   Enter · Next   Esc · Close photos   Ctrl/Cmd+C · Copy   Shift+Enter · Review changes"
        }
        Step::PickBest => {
            "1–9 · Keep/Trash   Arrows · Group   Enter · Show/Next   R · Undo   A · Accept all suggestions   Esc · Close photos   Ctrl/Cmd+C · Copy   Shift+Enter · Review changes"
        }
        Step::Cull => {
            "Left/Right · Cycle   Up/Down · Browse   Ctrl/Cmd+arrows · Bypass movement filters   D · Mark   R · Rotate   Ctrl/Cmd+0–5 · Rate   Ctrl/Cmd+C · Copy   Shift+Enter · Review"
        }
    }
}

#[cfg(test)]
#[path = "preferences/tests.rs"]
mod tests;
