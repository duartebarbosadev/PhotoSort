use super::*;

/// The keyboard-shortcuts window, centred like the other dialogs.
pub(super) const HELP_DIALOG: &str = "help_dialog";

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
        self.organize_dialog(ctx);
        self.organize_departure_dialog(ctx);
        self.review_dialogs(ctx);
        self.preferences_dialog(ctx);
        self.updates_dialog(ctx);
        self.models_dialog(ctx);
        if self.help {
            let width = (ctx.content_rect().width() - 64.0).clamp(280.0, 460.0);
            egui::Window::new("A few useful keys")
                .id(egui::Id::new(HELP_DIALOG))
                .open(&mut self.help)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                .default_width(width)
                .max_width(width)
                .max_height((ctx.content_rect().height() - 80.0).max(120.0))
                .vscroll(true)
                .show(ctx, |ui| {
                    let shortcuts = [
                        ("Ctrl/Cmd + O", "Open a library"),
                        ("Ctrl/Cmd + F", "Focus library search (except Pick Best)"),
                        ("Ctrl/Cmd + S", "Analyze this step; detect in Fix Rotation"),
                        ("Alt + 1 / 2 / 3", "Cull list / icons / grid"),
                        (
                            "Organize: Alt + 1–5",
                            "Current / date + similarity / visual / faces / location",
                        ),
                        ("F / S", "Cull folder / similarity-group navigator"),
                        ("F9", "Open cache usage and controls"),
                        ("F10 / Ctrl/Cmd + ,", "Open Settings"),
                        ("F12", "About PhotoSort"),
                        (
                            "Organize files: arrows / Space",
                            "Move focus / toggle selection; Ctrl/Cmd+arrows include Trash marks",
                        ),
                        (
                            "Organize files: Shift + click/arrows",
                            "Select a range; Ctrl/Cmd+Shift adds to the selection",
                        ),
                        (
                            "Organize files: Ctrl/Cmd+A",
                            "Select all; add Shift to clear selection",
                        ),
                        (
                            "Organize: F2",
                            "Rename the focused file or folder; Enter stages the name",
                        ),
                        (
                            "Organize: Ctrl/Cmd+Shift+N",
                            "Name a new folder inside the focused destination",
                        ),
                        (
                            "Organize: D / Delete",
                            "Toggle selection marks / review selected files for Trash",
                        ),
                        (
                            "Organize: Shift+Enter or Ctrl/Cmd+Enter",
                            "Preview moves; applying still requires review",
                        ),
                        (
                            "Tree: Up / Down",
                            "Select the previous / next visible folder, date or group",
                        ),
                        (
                            "Tree: Left / Right",
                            "Collapse or select parent / expand or select child",
                        ),
                        (
                            "Tree: Home / End / Page keys",
                            "Jump or page through the focused tree",
                        ),
                        (
                            "Left / Right",
                            "Cull cycles through visible photos or the current similarity group",
                        ),
                        (
                            "Up / Down",
                            "Cull follows rating and group navigation preferences",
                        ),
                        (
                            "Ctrl or Cmd + arrows",
                            "Include Trash marks and bypass Cull movement filters",
                        ),
                        (
                            "Shift + arrows / Shift + click",
                            "Select a range in Cull browser; extend comparison in Review",
                        ),
                        (
                            "Ctrl/Cmd + click / Ctrl/Cmd + A",
                            "Toggle browser selection / select all visible",
                        ),
                        ("D / Alt + D", "Toggle mark / clear library Trash marks"),
                        (
                            "Ctrl or Cmd + 0-5",
                            "Rate selection; viewer stars rate the focused photo",
                        ),
                        (
                            "R / Shift + R / Alt + R",
                            "Rotate right / left / 180 degrees",
                        ),
                        ("I", "Toggle photo details"),
                        (
                            "F1 / F2",
                            "Viewer: single photo, press again to return / similar comparison",
                        ),
                        (
                            "F2 / Ctrl or Cmd + Shift + N",
                            "Organize grid: rename / new folder",
                        ),
                        (
                            "0 / A / Double-click canvas",
                            "Fit / actual size / toggle actual size",
                        ),
                        (
                            "1-9 / F3",
                            "Focus comparison photo (in single view: nth photo of its group) / toggle synchronized views",
                        ),
                        (
                            "Drag / Scroll canvas",
                            "Pan / zoom toward the pointer for the focused photo or synchronized set",
                        ),
                        ("Shift + Enter", "Review staged changes"),
                    ];
                    for (key, action) in shortcuts {
                        let row = |ui: &mut egui::Ui| {
                            ui.monospace(key);
                            ui.add(egui::Label::new(action).wrap());
                        };
                        // Narrow windows put the action under its key.
                        if width < 400.0 {
                            ui.vertical(row);
                        } else {
                            ui.horizontal(row);
                        }
                    }
                    ui.separator();
                    ui.label("Ctrl/Cmd+Alt+1–5 switches to a visible workflow step.");
                    for step in Step::ALL {
                        ui.label(format!(
                            "{}: {}",
                            step.title(),
                            preferences::workflow_hint(step)
                        ));
                    }
                    ui.label("All analysis runs on your computer. Nothing is uploaded.");
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
                "PhotoSort closed unexpectedly"
            } else {
                "Part of PhotoSort stopped working"
            });
            ui.add_space(S2);
            ui.label(if previous {
                "It happened during your last session. A report was saved on this computer; nothing was sent anywhere."
            } else {
                "Restart PhotoSort to continue. A report was saved on this computer; nothing was sent anywhere."
            });
            ui.add_space(S1);
            ui.label(
                RichText::new(
                    "Your photos only change when you apply a reviewed plan. An interrupted apply can be finished from Review changes.",
                )
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
