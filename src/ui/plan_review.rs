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
}
impl Rows {
    fn replace(&mut self, plan: Option<&Plan>) {
        self.generation = self.generation.wrapping_add(1);
        self.ends.clear();
        self.movable.clear();
        let mut end = 0;
        if let Some(plan) = plan {
            for (group, action) in plan.actions.iter().enumerate() {
                if matches!(action, Action::Move { files } if !files.is_empty()) {
                    self.movable.push(group);
                }
                end += match action {
                    Action::Move { files } | Action::FinishMove { files } => files.len().max(1),
                    Action::Trash { files } => files.len().max(1),
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

fn path_label(ui: &mut egui::Ui, prefix: &str, path: &Path, root: Option<&Path>) {
    let relative = root
        .and_then(|root| path.strip_prefix(root).ok())
        .unwrap_or(path);
    let display = if relative.as_os_str().is_empty() {
        ".".into()
    } else {
        relative.display().to_string()
    };
    // Extend inside the two-axis scroll area: long paths can be read without hover.
    ui.add(egui::Label::new(RichText::new(format!("{prefix}{display}")).monospace()).extend())
        .on_hover_text(path.display().to_string());
}

fn row_ui(
    ui: &mut egui::Ui,
    action: &Action,
    group: usize,
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
            ui.small(format!(
                "Group {} · {kind} · file {} of {}",
                group + 1,
                file + 1,
                files.len()
            ));
            path_label(ui, "From  ", &source.path, root);
            path_label(ui, "To    ", to, output.or(root));
        }
        Action::Trash { files } if !files.is_empty() => {
            ui.small(format!(
                "Group {} · Trash · file {} of {}",
                group + 1,
                file + 1,
                files.len()
            ));
            path_label(ui, "From  ", &files[file].path, root);
            ui.add(egui::Label::new("To    System Trash").extend());
        }
        Action::RenameFile { source, to } => {
            ui.small(format!("Group {} · Rename file", group + 1));
            path_label(ui, "From  ", &source.path, root);
            path_label(ui, "To    ", to, output.or(root));
        }
        Action::RenameDirectory(rename) => {
            ui.small(format!("Group {} · Rename folder", group + 1));
            path_label(ui, "From  ", &rename.from, root);
            path_label(ui, "To    ", &rename.to, root);
        }
        Action::Metadata { source, .. } => {
            ui.small(format!("Group {} · Update metadata", group + 1));
            path_label(ui, "File  ", &source.path, root);
            ui.add(egui::Label::new(action.label()).extend());
        }
        Action::CreateDirectory { path } | Action::RemoveEmptyDirectory { path, .. } => {
            ui.small(format!(
                "Group {} · {}",
                group + 1,
                if matches!(action, Action::CreateDirectory { .. }) {
                    "Create folder"
                } else {
                    "Remove folder only if empty"
                }
            ));
            path_label(ui, "Folder  ", path, output.or(root));
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
        .show_rows(ui, 68.0, index.len(), |ui, rows| {
            let rendered = rows.len();
            for row in rows {
                let (group, file) = index.at(row);
                ui.push_id(row, |ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(ui.available_width(), 68.0),
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
                ui.label(RichText::new(format!("{} operation groups · {} file/folder entries", plan.actions.len(), self.plan_rows.len())).color(ACCENT));
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
                            ui.label(if self.rotation_plan { "Only confirmed corrections are included. Metadata rows identify embedded updates or XMP sidecars; pixels are not re-encoded." } else { "All listed companion files are included. Metadata rows identify embedded updates or XMP sidecars; pixels are not re-encoded. Trash entries go to your system's Trash. Moves never overwrite existing files." });
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
                        egui::Button::new(RichText::new(if retry { "Retry unfinished changes" } else { "Apply these changes" }).color(BG)).fill(ACCENT)).clicked() { apply = true; }
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
                if ui.add_enabled(!edit.saving, egui::Button::new("Save destination")).clicked() { save = true; }
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
mod tests {
    use super::*;
    use crate::core::operations::Source;

    fn source(name: &str) -> Source {
        Source {
            path: Path::new("/library").join(name),
            fingerprint: Fingerprint::default(),
        }
    }
    fn plan() -> Plan {
        Plan {
            actions: vec![
                Action::Move {
                    files: ["ARW", "JPG", "xmp"]
                        .into_iter()
                        .map(|ext| {
                            (
                                source(&format!("sample.{ext}")),
                                Path::new("/library").join(format!("keepers.{ext}")),
                            )
                        })
                        .collect(),
                },
                Action::Move {
                    files: vec![(source("notes.txt"), "/library/Trip notes.txt".into())],
                },
                Action::Trash {
                    files: vec![source("rejected.jpg"), source("rejected.xmp")],
                },
                Action::CreateDirectory {
                    path: "/library/Empty".into(),
                },
            ],
            output_root: Some("/library".into()),
            ..Default::default()
        }
    }

    #[test]
    fn review_rows_cover_every_companion_and_refresh_on_plan_replacement() {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        app.set_review_plan(Some(plan()));
        assert_eq!(app.plan_rows.len(), 7);
        assert_eq!(
            (0..7).map(|i| app.plan_rows.at(i)).collect::<Vec<_>>(),
            [(0, 0), (0, 1), (0, 2), (1, 0), (2, 0), (2, 1), (3, 0)]
        );
        let generation = app.plan_rows.generation;
        // Same group count but different cohort sizes must never reuse stale offsets.
        let mut changed = plan();
        changed.actions[0] = Action::Trash {
            files: vec![source("only-one.jpg")],
        };
        app.set_review_plan(Some(changed));
        assert_eq!(app.plan_rows.len(), 5);
        assert_ne!(app.plan_rows.generation, generation);
        assert_eq!(app.plan_rows.at(1), (1, 0));
        app.set_review_plan(None);
        assert_eq!(app.plan_rows.len(), 0);
        app.runtime.shutdown();
    }

    #[test]
    fn review_virtualization_reaches_last_file_without_materializing_other_rows() {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut plan = plan();
        plan.actions = vec![Action::Move {
            files: (0..100_000)
                .map(|i| {
                    (
                        source(&format!("before-{i}.jpg")),
                        PathBuf::from(format!("/library/after-{i}.jpg")),
                    )
                })
                .collect(),
        }];
        let mut index = Rows::default();
        index.replace(Some(&plan));
        assert_eq!(index.ends.len(), 1);
        let mut scroll = None;
        for at_end in [false, true] {
            if at_end {
                let (id, mut state): (egui::Id, egui::scroll_area::State) = scroll.take().unwrap();
                state.offset.y = 7_200_000.0;
                state.store(&ctx, id);
            }
            let mut output = None;
            for _ in 0..3 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(640.0, 500.0),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            let shown =
                                file_rows(ui, &plan, &index, Some(Path::new("/library")), 220.0);
                            assert!(shown.inner <= 6, "{} rendered rows", shown.inner);
                            scroll = Some((shown.id, shown.state));
                        });
                    },
                ));
            }
            let tree = output.unwrap().platform_output.accesskit_update.unwrap();
            let expected = if at_end {
                "To    after-99999.jpg"
            } else {
                "To    after-0.jpg"
            };
            assert!(
                tree.nodes.iter().any(|(_, n)| n.value() == Some(expected)),
                "{expected}"
            );
            assert!(
                tree.nodes.len() < 100,
                "virtualization emitted {} nodes",
                tree.nodes.len()
            );
        }
    }

    #[test]
    fn shared_review_exposes_names_and_keeps_apply_visible_in_compact_and_retry_views() {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        app.root = Some("/library".into());
        app.set_review_plan(Some(plan()));
        app.plan_open = true;
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for step in Step::ALL {
            app.step = step;
            for retry in [false, true] {
                app.retry_plan = retry.then(plan);
                let mut reviewed = plan();
                if retry {
                    reviewed.actions[0] = Action::FinishMove {
                        files: vec![(source("sample.ARW"), "/library/keepers.ARW".into())],
                    };
                }
                app.set_review_plan(Some(reviewed));
                app.recovery_ticket = retry.then(|| crate::core::operations::recovery::Ticket {
                    journal: "test".into(),
                    fingerprint: Default::default(),
                });
                app.after_apply = retry.then_some(Destination::Close);
                app.error = retry.then(|| "A file changed since review. ".repeat(500));
                for size in [
                    egui::vec2(600.0, 500.0),
                    egui::vec2(640.0, 600.0),
                    egui::vec2(1280.0, 800.0),
                ] {
                    let mut output = None;
                    for _ in 0..3 {
                        output = Some(ctx.run(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    size,
                                )),
                                ..Default::default()
                            },
                            |ctx| app.plan_review_dialog(ctx),
                        ));
                    }
                    let tree = output.unwrap().platform_output.accesskit_update.unwrap();
                    if retry {
                        assert!(tree.nodes.iter().any(|(_, n)| {
                            n.value()
                                .is_some_and(|text| text.contains("Finish staged move"))
                        }));
                    }
                    for name in ["From  sample.ARW", "To    keepers.ARW"] {
                        assert!(
                            tree.nodes.iter().any(|(_, n)| n.value() == Some(name)),
                            "missing visible {name}"
                        );
                    }
                    for label in [
                        if retry {
                            "Save plan and quit"
                        } else {
                            "Keep reviewing"
                        },
                        if retry {
                            "Retry unfinished changes"
                        } else {
                            "Apply these changes"
                        },
                    ] {
                        let n = tree
                            .nodes
                            .iter()
                            .find(|(_, n)| n.label() == Some(label))
                            .expect(label);
                        let bounds = n.1.bounds().unwrap();
                        assert!(
                            bounds.y0 >= 0.0
                                && bounds.y1 <= f64::from(size.y)
                                && bounds.x0 >= 0.0
                                && bounds.x1 <= f64::from(size.x),
                            "{step:?} {size:?} retry={retry}: {label} {bounds:?}"
                        );
                    }
                    assert!(
                        app.requested.is_empty()
                            && app.analysis_run.is_none()
                            && app.rotation_run.is_none()
                    );
                    assert_eq!(app.stats.decodes, 0);
                    assert!(app.decisions.is_empty());
                }
            }
        }
        app.runtime.shutdown();
    }
    #[test]
    fn destination_editor_virtualizes_large_plans_and_keeps_compact_controls_visible() {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        app.root = Some("/library".into());
        let mut plan = plan();
        plan.actions = (0..100_000)
            .map(|i| Action::Move {
                files: vec![(
                    source(&format!("{i}.jpg")),
                    Path::new("/library/Sorted").join(format!("{i}.jpg")),
                )],
            })
            .collect();
        app.retry_plan = Some(plan.clone());
        app.recovery_ticket = Some(crate::core::operations::recovery::Ticket {
            journal: "test".into(),
            fingerprint: Default::default(),
        });
        app.set_review_plan(Some(plan));
        app.plan_open = true;
        app.begin_destination_edit();
        app.retry_edit.as_mut().unwrap().error = Some("A source changed. ".repeat(100));
        app.retry_edit.as_mut().unwrap().stem = "Reviewed name".into();
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        for size in [egui::vec2(600., 500.), egui::vec2(1280., 800.)] {
            let mut output = None;
            for _ in 0..3 {
                output = Some(ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| app.plan_review_dialog(ctx),
                ));
            }
            let tree = output.unwrap().platform_output.accesskit_update.unwrap();
            for label in ["Cancel", "Save destination"] {
                let bounds = tree
                    .nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some(label))
                    .unwrap()
                    .1
                    .bounds()
                    .unwrap();
                assert!(
                    bounds.y0 >= 0.
                        && bounds.y1 <= f64::from(size.y)
                        && bounds.x0 >= 0.
                        && bounds.x1 <= f64::from(size.x),
                    "{label}: {bounds:?}"
                );
            }
            assert!(tree.nodes.len() < 100);
        }
        assert_eq!(app.stats.decodes, 0);
        app.set_review_plan(None);
        assert!(app.retry_edit.is_none() && app.plan_rows.movable.is_empty());
        app.runtime.shutdown();
    }
}
