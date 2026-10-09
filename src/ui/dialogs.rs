use super::*;

impl PhotoSortApp {
    pub(super) fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(error) = self.error.clone()
            && self.preferences_draft.is_none()
            && self.organize_departure.is_none()
            && !(self.plan_open && self.retry_plan.is_some())
        {
            let mut open = true;
            egui::Window::new("Something needs attention")
                .open(&mut open)
                .resizable(true)
                .default_width(520.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(260.0)
                        .show(ui, |ui| {
                            ui.colored_label(RED, error);
                        });
                    if ui.button("Check AI models…").clicked() {
                        self.open_model_setup();
                    }
                    if ui.button("Dismiss").clicked() {
                        self.error = None;
                    }
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
            egui::Window::new("A few useful keys")
                .open(&mut self.help)
                .default_width(460.0)
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
                        ("F1 / F2", "Single photo / similar comparison"),
                        (
                            "0 / A / Double-click canvas",
                            "Fit / actual size / toggle actual size",
                        ),
                        (
                            "1-9 / F3",
                            "Focus comparison photo / toggle synchronized views",
                        ),
                        (
                            "Drag / Scroll canvas",
                            "Pan / zoom the focused photo or synchronized set",
                        ),
                        ("Shift + Enter", "Review staged changes"),
                    ];
                    for (key, action) in shortcuts {
                        ui.horizontal(|ui| {
                            ui.monospace(key);
                            ui.label(action);
                        });
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
