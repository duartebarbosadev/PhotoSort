use super::*;

/// The Keyboard Shortcuts window, centred like the other dialogs.
pub(super) const HELP_DIALOG: &str = "help_dialog";

/// Keys by where they work. In a key, `+` joins the keys of one combination
/// and ` / ` separates alternatives; `Ctrl` reads Cmd on macOS.
const SHORTCUTS: &[(&str, &[(&str, &str)])] = &[
    (
        "Everywhere",
        &[
            ("Ctrl+O", "Open a folder"),
            ("Ctrl+F", "Search"),
            ("Ctrl+S", "Analyze"),
            ("Ctrl+Alt+1–6", "Switch step"),
            ("Shift+Enter", "Go to Review"),
            ("Ctrl+C", "Copy selected photos"),
            ("F10 / Ctrl+,", "Settings"),
            ("F12", "About PhotoSort"),
        ],
    ),
    (
        "Organize",
        &[
            ("Alt+1–5", "Grouping mode"),
            ("F2", "Rename photo"),
            ("Ctrl+Shift+N", "New folder"),
            ("\\", "Hold to show original folders"),
            ("Up / Down", "Previous / next in the tree"),
            ("Left / Right", "Collapse / expand"),
        ],
    ),
    (
        "Easy Delete",
        &[
            ("Left / Right", "Keep / Trash"),
            ("1–9", "Switch that photo"),
            ("Enter", "Next"),
            ("R", "Undo"),
            ("A / Shift+R", "Accept all suggestions"),
        ],
    ),
    (
        "Fix Rotation",
        &[
            ("Q / E", "Rotate left / right"),
            ("Enter", "Next"),
            ("R", "Undo"),
            ("A", "Accept all suggestions"),
        ],
    ),
    (
        "Pick Best",
        &[
            ("1–9", "Keep or trash"),
            ("Enter", "Show or next"),
            ("R", "Undo"),
            ("A / Shift+R", "Accept all suggestions"),
        ],
    ),
    (
        "Cull",
        &[
            ("Left / Right", "Previous / next photo"),
            ("Up / Down", "Previous / next group"),
            ("D", "Mark for Trash"),
            ("Alt+D", "Clear all marks"),
            ("Delete / Backspace", "Mark for Trash"),
            ("Ctrl+0–5", "Rate"),
            ("R / Shift+R", "Rotate right / left"),
            ("Ctrl+A", "Select all"),
            ("F", "Folder navigator"),
            ("I", "Photo details"),
        ],
    ),
    ("Review", &[("Alt+D", "Clear all Trash marks")]),
    (
        "Viewer",
        &[
            ("F1", "Single photo"),
            ("F2", "Compare similar"),
            ("F3", "Sync views"),
            ("1–9", "Focus a compared photo"),
            ("0 / A", "Fit / actual size"),
            ("C", "Swipe compare"),
        ],
    ),
];

/// What a key cap shows: arrows as glyphs, modifiers in the platform's words.
fn cap_text(key: &str) -> &str {
    use egui_phosphor::regular as icon;
    let mac = cfg!(target_os = "macos");
    match key {
        "Left" => icon::ARROW_LEFT,
        "Right" => icon::ARROW_RIGHT,
        "Up" => icon::ARROW_UP,
        "Down" => icon::ARROW_DOWN,
        "Ctrl" if mac => "Cmd",
        "Alt" if mac => "Option",
        other => other,
    }
}

/// One key cap.
fn key_cap(ui: &mut egui::Ui, key: &str) {
    egui::Frame::new()
        .fill(SELECTED)
        .stroke(Stroke::new(1.0, LINE))
        .corner_radius(R_CONTROL)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(cap_text(key)).font(medium(11.5)).color(INK));
        });
}

/// The caps for `keys`: combinations side by side, alternatives apart.
fn key_caps(ui: &mut egui::Ui, keys: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for (index, combo) in keys.split(" / ").enumerate() {
            if index > 0 {
                ui.label(RichText::new("/").small().color(FAINT));
            }
            // "Ctrl+," splits into "Ctrl" and ",".
            let mut parts = combo.split('+').peekable();
            while let Some(part) = parts.next() {
                if part.is_empty() && parts.peek().is_some_and(|p| p.is_empty()) {
                    parts.next();
                    key_cap(ui, "+");
                } else if !part.is_empty() {
                    key_cap(ui, part);
                }
            }
        }
    });
}

/// The width of the key column, so every action starts at the same place.
const KEY_COLUMN: f32 = 120.0;

impl PhotoSortApp {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        self.crash_dialog(ctx);
        if let Some(error) = self.error.clone()
            && self.preferences_draft.is_none()
            && self.organize_departure.is_none()
            && !(self.plan_open && self.retry_plan.is_some())
        {
            let mut open = true;
            let (headline, details) = error_parts(&error);
            let models = error.to_ascii_lowercase().contains("model");
            egui::Window::new("Something needs attention")
                .id(egui::Id::new(ERROR_DIALOG))
                .open(&mut open)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .default_width((ctx.content_rect().width() - 64.0).clamp(280.0, 460.0))
                .show(ctx, |ui| {
                    ui.add(egui::Label::new(RichText::new(headline).color(INK)).wrap());
                    if let Some(details) = details {
                        egui::CollapsingHeader::new(RichText::new("Details").small().color(MUTED))
                            .id_salt("error_details")
                            .show(ui, |ui| {
                                egui::ScrollArea::vertical()
                                    .max_height(200.0)
                                    .show(ui, |ui| {
                                        ui.add(
                                            egui::Label::new(
                                                RichText::new(details).small().color(MUTED),
                                            )
                                            .wrap(),
                                        );
                                    });
                            });
                    }
                    ui.add_space(S2);
                    ui.horizontal(|ui| {
                        if ui.add(primary("Dismiss")).clicked() {
                            self.error = None;
                        }
                        if models && ui.button("Check AI models…").clicked() {
                            self.open_model_setup();
                        }
                    });
                });
            if !open {
                self.error = None;
            }
        }
        self.location_layout_dialog(ctx);
        self.faceless_dialog(ctx);
        self.raw_question_dialog(ctx);
        self.organize_reset_dialog(ctx);
        self.organize_departure_dialog(ctx);
        self.review_dialogs(ctx);
        self.preferences_dialog(ctx);
        self.updates_dialog(ctx);
        self.models_dialog(ctx);
        if self.help {
            let width = (ctx.content_rect().width() - 64.0).clamp(280.0, 400.0);
            let height = (ctx.content_rect().height() - 80.0).max(120.0);
            egui::Window::new("Keyboard Shortcuts")
                .id(egui::Id::new(HELP_DIALOG))
                .open(&mut self.help)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .default_width(width)
                .max_width(width)
                .default_height(height)
                .max_height(height)
                .vscroll(true)
                .show(ctx, |ui| {
                    ui.set_width(width - 16.0);
                    for (index, (title, keys)) in SHORTCUTS.iter().enumerate() {
                        if index > 0 {
                            ui.add_space(S3);
                        }
                        ui.label(caption(title));
                        ui.add_space(S1);
                        egui::Grid::new(("shortcuts", index))
                            .num_columns(2)
                            .min_col_width(KEY_COLUMN.min(width / 2.0))
                            .spacing(egui::vec2(S2, 5.0))
                            .show(ui, |ui| {
                                for (key, action) in *keys {
                                    key_caps(ui, key);
                                    ui.label(RichText::new(*action).color(INK));
                                    ui.end_row();
                                }
                            });
                    }
                });
        }
        self.plan_review_dialog(ctx);
    }
}

impl PhotoSortApp {
    /// A readable notice for a crash report. The report itself stays on disk
    /// for anyone who wants to send it; the notice never shows its contents.
    fn crash_dialog(&mut self, ctx: &egui::Context) {
        if !self.crash_notice_shown
            && let Some(path) = crate::core::crash::background_panic()
        {
            self.crash_notice = Some((path, false));
            self.crash_notice_shown = true;
        }
        let Some((path, previous)) = self.crash_notice.clone() else {
            return;
        };
        let mut reveal = false;
        let mut dismiss = false;
        modal(egui::Id::new("crash_notice")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(280.0, 460.0));
            ui.heading(if previous {
                "PhotoSort closed unexpectedly last time"
            } else {
                "Part of PhotoSort stopped working"
            });
            ui.add_space(S2);
            ui.label(if previous {
                "A report was saved on this computer. Nothing was sent."
            } else {
                "Restart PhotoSort to continue. A report was saved on this computer. Nothing was sent."
            });
            ui.add_space(S1);
            ui.label(
                RichText::new("An interrupted apply can be finished from the Review step.")
                    .small()
                    .color(MUTED),
            );
            ui.add_space(S3);
            ui.horizontal(|ui| {
                if ui.add(primary("Dismiss")).clicked() {
                    dismiss = true;
                }
                if ui
                    .button(crate::core::desktop::Action::Reveal.label())
                    .on_hover_text(path.display().to_string())
                    .clicked()
                {
                    reveal = true;
                }
            });
        });
        if reveal || dismiss {
            // Seen either way: a background report must not reappear next
            // launch as if the app had closed.
            let shown = crate::core::crash::mark_seen(&crate::core::crash::directory(), &path);
            if reveal
                && let Ok(request) = crate::core::desktop::Request::for_path(
                    &shown,
                    crate::core::desktop::Action::Reveal,
                )
            {
                self.dispatch_desktop(request);
            }
            self.crash_notice = None;
        }
    }
}

/// The "Something needs attention" window, centred like the other dialogs.
pub(super) const ERROR_DIALOG: &str = "error_dialog";

/// Splits "What failed: low-level cause" into a readable headline and the
/// technical remainder, which stays available under Details.
fn error_parts(error: &str) -> (String, Option<String>) {
    let first = error.lines().next().unwrap_or(error).trim();
    if let Some((what, _)) = first.split_once(": ")
        && (8..=90).contains(&what.chars().count())
    {
        return (
            format!("{}.", what.trim_end_matches('.')),
            Some(error.to_owned()),
        );
    }
    if first.chars().count() <= 240 && !error.contains('\n') {
        return (error.to_owned(), None);
    }
    let short: String = first.chars().take(140).collect();
    (format!("{short}…"), Some(error.to_owned()))
}

#[cfg(test)]
mod error_tests {
    use super::error_parts;

    #[test]
    fn errors_lead_with_what_failed_and_keep_the_cause_as_details() {
        let (headline, details) =
            error_parts("Preferences could not be loaded: expected value at line 1 column 1");
        assert_eq!(headline, "Preferences could not be loaded.");
        assert!(details.unwrap().contains("line 1 column 1"));
        assert_eq!(
            error_parts("Folder is no longer available").0,
            "Folder is no longer available"
        );
        assert!(error_parts("Folder is no longer available").1.is_none());
        // A folder message with a long path is still shown whole.
        let folder = std::path::PathBuf::from(format!("/Volumes/{}", "y".repeat(200)));
        for problem in [
            crate::core::catalog::Unavailable::Missing,
            crate::core::catalog::Unavailable::NoPermission,
        ] {
            let message = problem.message(&folder);
            assert_eq!(error_parts(&message), (message, None));
        }
        let long = "x".repeat(400);
        let (headline, details) = error_parts(&long);
        assert!(headline.chars().count() <= 141 && details.is_some());
    }
}

/// Hooks for the headless screenshot gallery (`examples/ui_gallery.rs`); not
/// part of the application API.
#[doc(hidden)]
impl PhotoSortApp {
    pub fn gallery_shortcuts(&mut self) {
        self.help = true;
    }
}

#[cfg(test)]
mod help_tests {
    use super::*;

    #[test]
    fn shortcuts_open_in_a_centred_window_that_fits_narrow_screens() {
        let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
        for size in [egui::vec2(1200.0, 800.0), egui::vec2(360.0, 640.0)] {
            let mut app = PhotoSortApp::with_runtime(
                Runtime::with_cache(|| {}, dir.path().join("cache")),
                None,
            );
            app.help = true;
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(screen),
                    ..Default::default()
                };
                let _ = ctx.run_test(input, |ui| app.update_workspace(ui, false));
            }
            let dialog = ctx
                .memory(|m| m.area_rect(egui::Id::new(HELP_DIALOG)))
                .expect("the shortcuts window is shown");
            assert!(
                (dialog.center() - screen.center()).length() < 2.0,
                "{dialog:?} is not centred in {screen:?}"
            );
            assert!(
                screen.contains_rect(dialog),
                "{dialog:?} overflows {screen:?}"
            );
            assert!(dialog.width() <= 480.0, "{dialog:?}");
        }
    }
}
