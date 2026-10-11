//! Shared presentation of immutable operation plans. Rendering never rescans files.
use super::*;
use crate::core::operations::Action;
use std::path::Path;

#[derive(Default)]
pub(super) struct Rows {
    // One cumulative offset per operation group, not a second copy of file paths.
    ends: Vec<usize>,
    movable: Vec<usize>,
    generation: u64,
    summary: Summary,
}
/// Plain counts for the plan headline, by what happens to the files.
#[derive(Default)]
struct Summary {
    trash: usize,
    moves: usize,
    renames: usize,
    metadata: usize,
    folders: usize,
}
impl Summary {
    fn parts(&self) -> Vec<(String, Color32)> {
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        [
            (
                self.trash,
                plural(self.trash, "file to Trash", "files to Trash"),
                RED,
            ),
            (self.moves, plural(self.moves, "move", "moves"), ACCENT),
            (self.renames, plural(self.renames, "rename", "renames"), INK),
            (
                self.metadata,
                plural(self.metadata, "metadata update", "metadata updates"),
                INK,
            ),
            (
                self.folders,
                plural(self.folders, "folder change", "folder changes"),
                MUTED,
            ),
        ]
        .into_iter()
        .filter(|(n, ..)| *n > 0)
        .map(|(_, text, color)| (text, color))
        .collect()
    }
}
impl Rows {
    fn replace(&mut self, plan: Option<&Plan>) {
        self.generation = self.generation.wrapping_add(1);
        self.ends.clear();
        self.movable.clear();
        self.summary = Summary::default();
        let mut end = 0;
        if let Some(plan) = plan {
            for (group, action) in plan.actions.iter().enumerate() {
                if matches!(action, Action::Move { files } if !files.is_empty()) {
                    self.movable.push(group);
                }
                let summary = &mut self.summary;
                match action {
                    Action::Move { files } | Action::FinishMove { files } => {
                        for (source, to) in files {
                            if source.path.parent() == to.parent() {
                                summary.renames += 1;
                            } else {
                                summary.moves += 1;
                            }
                        }
                    }
                    Action::Trash { files } => summary.trash += files.len(),
                    Action::TrashDirectory(folder) => {
                        summary.trash += folder.inventory.files.len().max(1)
                    }
                    Action::RenameFile { .. } => summary.renames += 1,
                    Action::RenameDirectory(_) => summary.folders += 1,
                    Action::Metadata { .. } => summary.metadata += 1,
                    Action::CreateDirectory { .. } | Action::RemoveEmptyDirectory { .. } => {
                        summary.folders += 1
                    }
                }
                end += match action {
                    Action::Move { files } | Action::FinishMove { files } => files.len().max(1),
                    Action::Trash { files } => files.len().max(1),
                    Action::TrashDirectory(folder) => 1 + folder.inventory.files.len(),
                    _ => 1,
                };
                self.ends.push(end);
            }
        }
    }
    fn len(&self) -> usize {
        self.ends.last().copied().unwrap_or(0)
    }
    fn at(&self, row: usize) -> (usize, usize) {
        let group = self.ends.partition_point(|end| *end <= row);
        let start = group.checked_sub(1).map_or(0, |i| self.ends[i]);
        (group, row - start)
    }
}

const ROW_HEIGHT: f32 = 46.0;

/// One path line. `spoken` prefixes the accessible text ("From  ", "To    ")
/// so screen readers keep the direction the arrow shows visually.
fn path_line(
    ui: &mut egui::Ui,
    spoken: &str,
    shown: &str,
    path: &Path,
    root: Option<&Path>,
    color: Color32,
) {
    let relative = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let display = if relative.as_os_str().is_empty() {
        ".".into()
    } else {
        relative.display().to_string()
    };
    // Extend inside the two-axis scroll area: long paths can be read without hover.
    let response = ui
        .add(egui::Label::new(RichText::new(format!("{shown}{display}")).color(color)).extend())
        .on_hover_text(path.display().to_string());
    let spoken = format!("{spoken}{display}");
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &spoken));
}
fn path_label(ui: &mut egui::Ui, prefix: &str, path: &Path, root: Option<&Path>) {
    let to = prefix.trim() == "To";
    path_line(
        ui,
        prefix,
        if to { "→  " } else { "" },
        path,
        root,
        if to { MUTED } else { INK },
    );
}
/// The first line of a row: what happens, then the file it happens to.
fn source_line(
    ui: &mut egui::Ui,
    kind: &str,
    color: Color32,
    prefix: &str,
    path: &Path,
    root: Option<&Path>,
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = S2;
        ui.label(RichText::new(kind).small().color(color));
        path_line(ui, prefix, "", path, root, INK);
    });
}
fn note_line(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(RichText::new(text).color(MUTED)).extend());
}

fn row_ui(
    ui: &mut egui::Ui,
    action: &Action,
    _group: usize,
    file: usize,
    root: Option<&Path>,
    output: Option<&Path>,
) {
    match action {
        Action::Move { files } | Action::FinishMove { files } if !files.is_empty() => {
            let (source, to) = &files[file];
            let kind = if matches!(action, Action::FinishMove { .. }) {
                "Finish staged move (remove verified source link)"
            } else if source.path.parent() == to.parent() {
                "Rename"
            } else if source.path.file_name() == to.file_name() {
                "Move"
            } else {
                "Move and rename"
            };
            source_line(ui, kind, ACCENT, "From  ", &source.path, root);
            path_label(ui, "To    ", to, output.or(root));
        }
        Action::Trash { files } if !files.is_empty() => {
            source_line(ui, "Trash", RED, "From  ", &files[file].path, root);
            note_line(ui, "→  System Trash");
        }
        Action::TrashDirectory(folder) => {
            if file == 0 {
                source_line(
                    ui,
                    &format!(
                        "Trash folder · {} files, {} subfolders",
                        folder.inventory.files.len(),
                        folder.inventory.directory_count().saturating_sub(1)
                    ),
                    RED,
                    "From  ",
                    &folder.path,
                    root,
                );
            } else {
                source_line(
                    ui,
                    &format!("In folder · {} of {}", file, folder.inventory.files.len()),
                    RED,
                    "File  ",
                    &folder.inventory.files[file - 1].path,
                    root,
                );
            }
            note_line(ui, "→  System Trash, with the whole folder");
        }
        Action::RenameFile { source, to } => {
            source_line(ui, "Rename", INK, "From  ", &source.path, root);
            path_label(ui, "To    ", to, output.or(root));
        }
        Action::RenameDirectory(rename) => {
            source_line(ui, "Rename folder", INK, "From  ", &rename.from, root);
            path_label(ui, "To    ", &rename.to, root);
        }
        Action::Metadata { source, .. } => {
            source_line(ui, "Metadata", INK, "File  ", &source.path, root);
            note_line(ui, &action.label());
        }
        Action::CreateDirectory { path } | Action::RemoveEmptyDirectory { path, .. } => {
            let create = matches!(action, Action::CreateDirectory { .. });
            source_line(
                ui,
                if create {
                    "Create folder"
                } else {
                    "Remove folder if empty"
                },
                if create { GREEN } else { MUTED },
                "Folder  ",
                path,
                output.or(root),
            );
        }
        _ => {
            ui.label("Empty file operation");
        }
    }
}

fn file_rows(
    ui: &mut egui::Ui,
    plan: &Plan,
    index: &Rows,
    root: Option<&Path>,
    height: f32,
) -> egui::scroll_area::ScrollAreaOutput<usize> {
    egui::ScrollArea::both()
        .id_salt(("plan_files", index.generation))
        .max_height(height)
        .auto_shrink([false, true])
        .show_rows(ui, ROW_HEIGHT, index.len(), |ui, rows| {
            let rendered = rows.len();
            for row in rows {
                let (group, file) = index.at(row);
                ui.push_id(row, |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), ROW_HEIGHT),
                        egui::Layout::top_down(egui::Align::Min),
                        |ui| {
                            row_ui(
                                ui,
                                &plan.actions[group],
                                group,
                                file,
                                root,
                                plan.output_root.as_deref(),
                            );
                        },
                    );
                });
            }
            rendered
        })
}

impl PhotoSortApp {
    pub(super) fn set_review_plan(&mut self, plan: Option<Plan>) {
        self.retry_edit = None;
        self.plan_rows.replace(plan.as_ref());
        self.plan = plan;
    }

    pub(super) fn plan_review_dialog(&mut self, ctx: &egui::Context) {
        if !self.plan_open {
            return;
        }
        if self.retry_edit.is_some() {
            self.destination_editor(ctx);
            return;
        }
        let mut edit = false;
        let mut apply = false;
        let mut cancel = false;
        let mut discard = false;
        let mut save_quit = false;
        egui::Modal::new(egui::Id::new("review_changes")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(240.0, 720.0));
            let top = ui.cursor().top();
            if let Some(plan) = &self.plan {
                let retry = self.retry_plan.is_some();
                ui.heading(if retry { "Unfinished file operations" } else { "Review your changes" });
                ui.horizontal_wrapped(|ui| {
                    for (part, color) in self.plan_rows.summary.parts() {
                        ui.label(RichText::new(part).font(medium(13.5)).color(color));
                    }
                });
                if let Some(root) = &self.root {
                    ui.add(egui::Label::new(format!("Library: {}", root.display())).truncate()).on_hover_text(root.display().to_string());
                }
                if let Some(output) = &plan.output_root && self.root.as_ref() != Some(output) {
                    ui.add(egui::Label::new(format!("Destination: {}", output.display())).truncate()).on_hover_text(output.display().to_string());
                }
                if retry && self.error.is_some() { ui.colored_label(RED, "Some operations need attention. Open details before retrying."); }
                egui::CollapsingHeader::new("Operation details").show(ui, |ui| {
                    egui::ScrollArea::vertical().id_salt("plan_explanation").max_height(70.0).show(ui, |ui| {
                        ui.label(&plan.description);
                        if retry {
                            ui.label("Successful operations are already applied. Finish staged move rechecks that the destination is the same file before removing its source link. Edit destination saves a revised folder or base name for a pending move and all its companions. Retry uses the saved destinations and source checks. Resolve remaining conflicts first. Discard abandons only unfinished operations; the audit record is retained.");
                            if let Some(error) = &self.error { ui.colored_label(RED, error); }
                        } else {
                            ui.label(if self.rotation_plan { "Only confirmed corrections are included. Metadata rows identify embedded updates, XMP sidecars or lossless pixel rotations." } else { "All listed companion files are included. Metadata rows identify embedded updates, XMP sidecars or lossless pixel rotations. Trash entries go to your system's Trash. Moves never overwrite existing files." });
                        }
                    });
                });
                ui.separator();
                // Account for actual wrapped heading/notice heights and reserve the footer.
                let footer = if retry && matches!(self.after_apply, Some(Destination::Close)) { 230.0 } else { 130.0 };
                let height = (ctx.content_rect().height() - footer - (ui.cursor().top() - top)).clamp(40.0, 400.0);
                file_rows(ui, plan, &self.plan_rows, self.root.as_deref(), height);
                ui.separator();
                if self.close_save_pending { ui.label("Saving before quitting…"); }
                if retry && matches!(self.after_apply, Some(Destination::Close)) && self.recovery_ticket.is_some() {
                    ui.label("Save your plan and review progress to resume next time. No remaining file operations will run.");
                    if self.has_unsaved_plan_extras() { ui.label("Keep reviewing to apply or reset other unsaved organization or confirmed rotation choices first."); }
                }
                ui.add_enabled_ui(!self.busy, |ui| { ui.horizontal_wrapped(|ui| {
                    if retry && self.recovery_ticket.is_some() && matches!(self.after_apply, Some(Destination::Close))
                        && ui.add_enabled(!self.has_unsaved_plan_extras(), egui::Button::new("Save plan and quit")).clicked() { save_quit = true; }
                    if ui.button("Keep reviewing").clicked() { cancel = true; }
                    if retry && ui.add_enabled(!self.busy && self.recovery_ticket.is_some() && !self.plan_rows.movable.is_empty(), egui::Button::new("Edit destination…")).clicked() { edit = true; }
                    if retry && ui.button("Discard unfinished plan").clicked() { discard = true; }
                    if ui.add_enabled(!plan.actions.is_empty() && !self.busy,
                        primary(if retry { "Retry unfinished changes" } else { "Apply these changes" })).clicked() { apply = true; }
                }); });
            }
        });
        if save_quit {
            self.save_plan_and_quit();
        }
        if edit {
            self.begin_destination_edit();
        }
        if cancel {
            self.plan_open = false;
            self.after_apply = None;
            if self.retry_plan.is_none() {
                self.rotation_plan = false;
            }
        }
        if discard {
            self.discard_remaining_operations();
        }
        if apply {
            self.apply_reviewed_plan();
        }
    }
}

pub(super) struct Edit {
    pub(super) group: usize,
    pub(super) folder: String,
    pub(super) stem: String,
    pub saving: bool,
    pub cancelled: Arc<std::sync::atomic::AtomicBool>,
    pub error: Option<String>,
}
impl PhotoSortApp {
    pub(super) fn begin_destination_edit(&mut self) {
        if self.busy || self.scanning || self.recovery_ticket.is_none() {
            return;
        }
        let Some(&group) = self.plan_rows.movable.first() else {
            return;
        };
        self.select_destination_group(group);
    }
    fn select_destination_group(&mut self, group: usize) {
        let Some(plan) = &self.plan else {
            return;
        };
        let Some(Action::Move { files }) = plan.actions.get(group) else {
            return;
        };
        let Some((_, destination)) = files.first() else {
            return;
        };
        let base = plan.output_root.as_deref().or(self.root.as_deref());
        let folder = destination
            .parent()
            .and_then(|p| base.and_then(|b| p.strip_prefix(b).ok()))
            .unwrap_or(Path::new(""));
        self.retry_edit = Some(Edit {
            group,
            folder: folder.display().to_string(),
            stem: String::new(),
            saving: false,
            cancelled: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            error: None,
        });
    }
    pub(super) fn save_destination_edit(&mut self) {
        if self.busy || self.scanning {
            return;
        }
        let (Some(root), Some(ticket), Some(edit)) =
            (&self.root, &self.recovery_ticket, &mut self.retry_edit)
        else {
            return;
        };
        edit.cancelled = Arc::new(std::sync::atomic::AtomicBool::new(false));
        match self.runtime.io(IoWork::AmendRecovery {
            root: root.clone(),
            ticket: ticket.clone(),
            group: edit.group,
            folder: PathBuf::from(&edit.folder),
            stem: (!edit.stem.is_empty()).then(|| edit.stem.clone()),
            cancelled: edit.cancelled.clone(),
        }) {
            Ok(()) => {
                edit.saving = true;
                edit.error = None;
                self.busy = true;
            }
            Err(error) => edit.error = Some(error),
        }
    }
    fn destination_editor(&mut self, ctx: &egui::Context) {
        let mut select = None;
        let mut save = false;
        let mut cancel = false;
        let Some(edit) = &mut self.retry_edit else {
            return;
        };
        let Some(plan) = &self.plan else {
            return;
        };
        egui::Modal::new(egui::Id::new("edit_pending_destination")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.).clamp(240., 720.));
            ui.heading("Edit pending destination");
            ui.label("Choose a move. Photo companions stay together; extensions keep their original spelling.");
            ui.add_enabled_ui(!edit.saving, |ui| {
                egui::ScrollArea::vertical().id_salt("pending_move_groups")
                    .max_height((ctx.content_rect().height() * 0.22).clamp(50., 150.))
                    .show_rows(ui, button_row_height(ui), self.plan_rows.movable.len(), |ui, rows| {
                        for row in rows {
                            let group = self.plan_rows.movable[row];
                            if let Action::Move { files } = &plan.actions[group] {
                                let name = files[0].1.file_name().unwrap_or_default().to_string_lossy();
                                if ui.add(egui::Button::new(format!("Group {} · {} · {} file(s)", group + 1, name, files.len())).selected(edit.group == group).truncate()).clicked() { select = Some(group); }
                            }
                        }
                    });
                let base = plan.output_root.as_deref().or(self.root.as_deref()).unwrap_or(Path::new(""));
                ui.add(egui::Label::new(format!("Folder within {}", base.display())).truncate()).on_hover_text(base.display().to_string());
                ui.add(egui::TextEdit::singleline(&mut edit.folder).hint_text("Leave empty for the destination root").desired_width(f32::INFINITY));
                ui.label("New base name (optional)");
                ui.add(egui::TextEdit::singleline(&mut edit.stem).hint_text("Leave empty to keep reviewed names").desired_width(f32::INFINITY));
                if let Some(Action::Move { files }) = plan.actions.get(edit.group) {
                    egui::ScrollArea::both().id_salt("pending_names_preview").max_height(48.).show_rows(ui, ui.text_style_height(&egui::TextStyle::Monospace), files.len(), |ui, rows| {
                        for row in rows {
                            let old = &files[row].1;
                            let name = if edit.stem.is_empty() { old.file_name().unwrap_or_default().to_owned() }
                                else { crate::core::organize::renamed_file_name(old, edit.stem.as_ref()) };
                            let to = base.join(&edit.folder).join(name);
                            path_label(ui, "To  ", &to, Some(base));
                        }
                    });
                }
            });
            if let Some(error) = &edit.error {
                egui::ScrollArea::vertical().id_salt("destination_edit_error").max_height(55.).show(ui, |ui| { ui.colored_label(RED, error); });
            }
            ui.small("Saving updates the plan only. Existing files stay untouched until you review and retry.");
            if edit.saving { ui.label("Checking sources and saving…"); }
            ui.horizontal_wrapped(|ui| {
                if ui.button(if edit.saving { "Cancel saving" } else { "Cancel" }).clicked() { cancel = true; }
                if ui.add_enabled(!edit.saving, primary("Save destination")).clicked() { save = true; }
            });
        });
        if cancel {
            if edit.saving {
                edit.cancelled
                    .store(true, std::sync::atomic::Ordering::Release);
            } else {
                self.retry_edit = None;
            }
        } else if let Some(group) = select {
            self.select_destination_group(group);
        } else if save {
            self.save_destination_edit();
        }
    }
}

#[cfg(test)]
#[path = "plan_review/tests.rs"]
mod tests;
