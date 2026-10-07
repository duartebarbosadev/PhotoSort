use super::*;
use crate::core::{
    browsing::tree::{Key, Tree},
    operations,
    organize::{Draft, Selection},
};

pub(super) struct Editor {
    catalog: Arc<Catalog>,
    output: PathBuf,
    desktop_enabled: bool,
    desktop_pending: Option<crate::core::desktop::Request>,
    reset_request: Option<Selection>,
    trash_request: Option<(Vec<MediaId>, bool)>,
    trash_now: Option<operations::TrashRequest>,
    reset_enabled: bool,
    names: HashMap<MediaId, String>,
    renaming: Option<(MediaId, String)>,
    rename_focus: bool,
    draft: Draft,
    rows: Vec<PhotoRow>,
    tree: Tree,
    visible_folders: Vec<usize>,
    collapsed: std::collections::BTreeSet<Key>,
    selected: HashSet<MediaId>,
    file_cursor: Option<usize>,
    file_anchor: Option<MediaId>,
    reveal_file: bool,
    selected_folders: std::collections::BTreeSet<String>,
    folder_anchor: Option<String>,
    drag_owner: Arc<()>,
    completed_drag: Option<CompletedDrag>,
    drag: pointer::DragTracker,
    hover_expand: tree_controls::HoverExpand,
    folder: String,
    name: String,
    parent: String,
    pub(super) error: Option<String>,
    compact_folders: bool,
    photo_scroll: f32,
    folder_scroll: f32,
    restore_scroll: [Option<f32>; 2],
}
struct PhotoRow {
    id: MediaId,
    source: PathBuf,
    after: String,
}
#[derive(Clone)]
struct FolderDrag {
    owner: Arc<()>,
    selection: Selection,
}
// A quick press/move/release can arrive between two repaints. egui's persistent
// drag state has already ended then; resolve that completed gesture using the
// same visible rows, selection builders and command as a normal drag.
struct CompletedDrag {
    start: egui::Pos2,
    end: egui::Pos2,
    payload: Option<FolderDrag>,
    target: Option<String>,
}
enum Command {
    Create,
    Rename,
    Parent,
    Nest,
    Unnest,
    Move,
    Drop(Selection, String),
    RenameFile(MediaId, String),
}
fn folder_title(path: &str) -> &str {
    if path.is_empty() {
        "Library root"
    } else {
        path
    }
}
impl Editor {
    fn trash_menu(
        &mut self,
        ui: &mut egui::Ui,
        selection: Selection,
        decisions: &HashMap<MediaId, Decision>,
    ) {
        let Ok(files) = self.draft.selection_files(&selection) else {
            return;
        };
        let all_marked = !files.is_empty()
            && files
                .iter()
                .all(|id| decisions.get(id).is_some_and(|d| d.rejected));
        let label = if all_marked {
            "Unmark from Trash"
        } else {
            "Mark for Trash"
        };
        if ui.add_enabled(!files.is_empty(), egui::Button::new(label))
            .on_hover_text("Mark the represented files, including files in subfolders. Marks are shared across steps. Close this editor and use Review changes to apply them.")
            .clicked() {
            let mut files: Vec<_> = files.iter().copied().collect();
            files.sort_by_key(|id| id.0);
            self.trash_request = Some((files, !all_marked));
            ui.close();
        }
        if ui.add_enabled(!files.is_empty(), egui::Button::new("Move these files to Trash…"))
            .on_hover_text("Keep your layout edits and review only these files and their companions before applying.").clicked() {
            let mut files: Vec<_> = files.iter().copied().collect();
            files.sort_by_key(|id| id.0);
            self.trash_now = Some(operations::TrashRequest { files, folders: Vec::new() });
            ui.close();
        }
        if !selection.folders.is_empty() {
            let folders: Option<Vec<_>> = selection
                .folders
                .iter()
                .map(|folder| {
                    self.folder_locations(folder)
                        .source
                        .filter(|path| path != &self.catalog.root)
                })
                .collect();
            if ui.add_enabled(folders.is_some(), egui::Button::new("Move original folders to Trash…"))
                .on_hover_text("Review whole original folders, including every file and empty subfolder. Unavailable for the library root, new destinations or merged folders.").clicked() {
                self.trash_now = Some(operations::TrashRequest { files: Vec::new(), folders: folders.unwrap() });
                ui.close();
            }
        }
    }

    fn file_locations(&self, row: &PhotoRow) -> desktop::Locations {
        let name = self
            .names
            .get(&row.id)
            .map(|stem| crate::core::organize::renamed_file_name(&row.source, stem.as_ref()))
            .unwrap_or_else(|| row.source.file_name().unwrap_or_default().to_owned());
        desktop::Locations {
            source: Some(row.source.clone()),
            relative: row
                .source
                .strip_prefix(&self.catalog.root)
                .ok()
                .map(PathBuf::from),
            proposed: self
                .output
                .is_absolute()
                .then(|| operations::folder_destination(&self.output, &row.after).join(name)),
        }
    }
    fn folder_locations(&self, path: &str) -> desktop::Locations {
        let source = self.draft.directory_source(path).and_then(|original| {
            if original.is_empty() {
                return Some(self.catalog.root.clone());
            }
            let mut candidates = self.catalog.directories.iter().filter(|folder| {
                folder
                    .strip_prefix(&self.catalog.root)
                    .is_ok_and(|relative| relative.to_string_lossy().replace('\\', "/") == original)
            });
            let first = candidates.next()?;
            candidates.next().is_none().then(|| first.clone())
        });
        desktop::Locations {
            relative: Some(PathBuf::from(if path.is_empty() { "." } else { path })),
            source,
            proposed: self
                .output
                .is_absolute()
                .then(|| operations::folder_destination(&self.output, path)),
        }
    }
    fn bookmark(&self) -> crate::core::views::EditorBookmark {
        let mut selected: Vec<_> = self.selected.iter().copied().collect();
        selected.sort_by_key(|id| id.0);
        crate::core::views::EditorBookmark {
            focused: self.file_cursor.map(|i| self.rows[i].id),
            anchor: self.file_anchor,
            selected,
            folders: self.selected_folders.iter().cloned().collect(),
            folder_anchor: self.folder_anchor.clone(),
            folder: self.folder.clone(),
            photo_scroll: self.photo_scroll,
            folder_scroll: self.folder_scroll,
            compact_folders: self.compact_folders,
            collapsed: self.collapsed.clone(),
        }
    }
    fn restore_bookmark(&mut self, view: &crate::core::views::EditorBookmark) {
        let rows: HashSet<_> = self.rows.iter().map(|row| row.id).collect();
        let folders = self.draft.folders();
        self.selected = view
            .selected
            .iter()
            .copied()
            .filter(|id| rows.contains(id))
            .collect();
        self.file_cursor = view
            .focused
            .and_then(|id| self.rows.iter().position(|row| row.id == id));
        self.file_anchor = view.anchor.filter(|id| rows.contains(id));
        self.selected_folders = view
            .folders
            .iter()
            .filter(|path| folders.contains(path.as_str()))
            .cloned()
            .collect();
        self.folder_anchor = view
            .folder_anchor
            .as_ref()
            .filter(|path| folders.contains(path.as_str()))
            .cloned();
        if folders.contains(view.folder.as_str()) {
            self.folder = view.folder.clone();
        }
        self.name = self.folder.rsplit('/').next().unwrap_or_default().into();
        self.photo_scroll = view.photo_scroll;
        self.folder_scroll = view.folder_scroll;
        self.restore_scroll = [Some(view.photo_scroll), Some(view.folder_scroll)];
        self.compact_folders = view.compact_folders;
        self.collapsed = view.collapsed.clone();
        self.project_folders();
    }
    fn begin_drag_frame(&mut self, ctx: &egui::Context) {
        self.completed_drag = None;
        if !self.drag.begin_frame(ctx, egui::Id::new("organize-drag")) {
            return;
        }
        if self.drag.cancelled {
            egui::DragAndDrop::clear_payload(ctx);
        }
        self.completed_drag = self.drag.completed().map(|motion| CompletedDrag {
            start: motion.start,
            end: motion.end,
            payload: None,
            target: None,
        });
    }

    // egui resolves a frame's hit target at the latest pointer position. When
    // press and movement arrive together, that can be a different row. Anchor
    // the payload to the actual press position and keep it until release.
    fn starts_drag(&self, ui: &egui::Ui, response: &egui::Response) -> bool {
        let Some(start) = self.drag.start() else {
            return false;
        };
        !egui::DragAndDrop::has_any_payload(ui.ctx())
            && pointer::contains_pointer(ui, response, start)
            && ui.input(|i| i.pointer.primary_down() && i.pointer.is_decidedly_dragging())
    }
    fn track_completed_drag(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        file: Option<MediaId>,
        path: &str,
    ) {
        let Some(drag) = &self.completed_drag else {
            return;
        };
        let source = pointer::contains_pointer(ui, response, drag.start);
        let target = pointer::contains_pointer(ui, response, drag.end);
        let payload = if source {
            if let Some(id) = file {
                Some(self.file_drag(id))
            } else if !path.is_empty() {
                Some(self.folder_drag(path))
            } else {
                None
            }
        } else {
            None
        };
        let drag = self.completed_drag.as_mut().unwrap();
        if payload.is_some() {
            drag.payload = payload;
        }
        if target {
            drag.target = Some(path.into());
        }
    }
    fn refresh(&mut self) {
        self.hover_expand = Default::default();
        self.folder_anchor = self
            .folder_anchor
            .take()
            .filter(|p| self.draft.folders().contains(p));
        let mut counts = HashMap::<String, usize>::new();
        for row in &mut self.rows {
            row.after = self
                .draft
                .labels()
                .get(&row.id)
                .cloned()
                .unwrap_or_default();
            *counts.entry(row.after.clone()).or_default() += 1;
        }
        self.tree = Tree::folders(
            self.draft.folders().iter().map(|path| {
                (
                    PathBuf::from(path),
                    folder_title(path).to_owned(),
                    counts.get(path).copied().unwrap_or_default(),
                )
            }),
            || false,
        )
        .expect("in-memory folder tree is not cancelled");
        self.project_folders();
    }
    fn project_folders(&mut self) {
        self.tree.retain_branches(&mut self.collapsed);
        self.visible_folders = self.tree.visible(&self.collapsed, || false).unwrap();
    }
    fn file_drag(&mut self, id: MediaId) -> FolderDrag {
        self.file_cursor = self.rows.iter().position(|row| row.id == id);
        if !self.selected.contains(&id) {
            self.file_anchor = Some(id);
            self.selected.clear();
            self.selected.insert(id);
        }
        FolderDrag {
            owner: self.drag_owner.clone(),
            selection: Selection {
                files: self.selected.clone(),
                ..Default::default()
            },
        }
    }
    fn folder_drag(&mut self, path: &str) -> FolderDrag {
        if !self.selected_folders.contains(path) {
            self.selected_folders.clear();
            self.selected_folders.insert(path.into());
            self.folder_anchor = Some(path.into());
        }
        FolderDrag {
            owner: self.drag_owner.clone(),
            selection: Selection {
                folders: self.selected_folders.clone(),
                ..Default::default()
            },
        }
    }
    fn drop_target(&self, ui: &egui::Ui, response: &egui::Response, path: &str) -> Option<Command> {
        let payload = response.dnd_hover_payload::<FolderDrag>()?;
        if !Arc::ptr_eq(&payload.owner, &self.drag_owner) {
            return None;
        }
        let valid = self.draft.validate_move(&payload.selection, path);
        ui.painter().rect_stroke(
            response.rect,
            3.0,
            Stroke::new(2.0, if valid.is_ok() { ACCENT } else { RED }),
            StrokeKind::Inside,
        );
        ui.ctx().set_cursor_icon(if valid.is_ok() {
            egui::CursorIcon::Grabbing
        } else {
            egui::CursorIcon::NotAllowed
        });
        if let Some(payload) = response.dnd_release_payload::<FolderDrag>() {
            return Some(Command::Drop(payload.selection.clone(), path.into()));
        }
        None
    }
    fn scroll_drag<R>(&self, ui: &egui::Ui, output: &egui::scroll_area::ScrollAreaOutput<R>) {
        let Some(payload) = egui::DragAndDrop::payload::<FolderDrag>(ui.ctx()) else {
            return;
        };
        if !Arc::ptr_eq(&payload.owner, &self.drag_owner) {
            return;
        }
        let Some(pointer) = ui.ctx().pointer_interact_pos() else {
            return;
        };
        let rect = output.inner_rect;
        if !rect.contains(pointer) {
            return;
        }
        let edge = 24.0_f32.min(rect.height() / 3.0);
        let direction = if pointer.y < rect.top() + edge {
            -1.0
        } else if pointer.y > rect.bottom() - edge {
            1.0
        } else {
            0.0
        };
        if direction != 0.0 {
            let mut state = output.state;
            state.offset.y = (state.offset.y
                + direction * 360.0 * ui.input(|i| i.stable_dt.min(0.05)))
            .clamp(0.0, (output.content_size.y - rect.height()).max(0.0));
            state.store(ui.ctx(), output.id);
            ui.ctx().request_repaint();
        }
    }
    pub(super) fn start_rename(&mut self, id: MediaId) {
        if let Some(row) = self.rows.iter().find(|row| row.id == id) {
            let stem = self.names.get(&id).cloned().unwrap_or_else(|| {
                row.source
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            });
            self.renaming = Some((id, stem));
            self.rename_focus = true;
            self.compact_folders = false;
            self.error = None;
        }
    }
    fn file_name_controls(&mut self, ui: &mut egui::Ui) -> Option<Command> {
        let mut command = None;
        let mut cancel = false;
        if let Some((id, name)) = &mut self.renaming {
            let original = self
                .rows
                .iter()
                .find(|row| row.id == *id)
                .map(|row| row.source.file_name().unwrap_or_default().to_string_lossy())
                .unwrap_or_default();
            ui.add(egui::Label::new(format!("Rename {original}")).truncate());
            ui.label(
                RichText::new("Base name only · extension stays unchanged")
                    .small()
                    .color(MUTED),
            );
            let mut field = egui::TextEdit::singleline(name)
                .id_salt("organize_file_name")
                .desired_width(ui.available_width())
                .hint_text("File name")
                .show(ui);
            if self.rename_focus {
                field.response.request_focus();
                field
                    .state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(name.chars().count()),
                    )));
                field.state.store(ui.ctx(), field.response.id);
                self.rename_focus = false;
            }
            let field = field.response;
            let enter = (field.has_focus() || field.lost_focus())
                && ui.input(|i| i.key_pressed(egui::Key::Enter));
            ui.horizontal_wrapped(|ui| {
                if ui.button("Stage name").clicked() || enter {
                    command = Some(Command::RenameFile(*id, name.clone()));
                }
                if ui.button("Restore original name").clicked() {
                    command = Some(Command::RenameFile(*id, String::new()));
                }
                if ui.button("Cancel name edit").clicked() {
                    cancel = true;
                }
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(self.selected.len() == 1, egui::Button::new("Rename selected file…")).clicked()
                    && let Some(id) = self.selected.iter().next().copied() {
                    self.start_rename(id);
                }
                if ui.add_enabled(self.reset_enabled && !self.selected.is_empty(), egui::Button::new("Reset destinations"))
                    .on_hover_text("Restore the initial grouping for selected files. Other edits and file names stay unchanged.").clicked() {
                    self.reset_request = Some(Selection { files: self.selected.clone(), ..Default::default() });
                }
            });
        }
        if cancel {
            self.renaming = None;
            self.error = None;
        }
        command
    }
    fn photos(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        decisions: &HashMap<MediaId, Decision>,
    ) -> Option<Command> {
        // Rendered once per frame, either in a column or the compact tab.
        // Global scope preserves egui state when that parent changes.
        ui.scope_builder(
            egui::UiBuilder::new().id(("organize_file_panel", &self.catalog.root)),
            |ui| self.photo_contents(ui, height, decisions),
        )
        .inner
    }
    fn select_file(&mut self, row: usize, toggle: bool, extend: bool) {
        if extend && self.file_anchor.is_none() {
            self.file_anchor = self.file_cursor.map(|i| self.rows[i].id);
        }
        self.file_cursor = Some(row);
        crate::core::selection::select_ordered(
            &mut self.selected,
            &mut self.file_anchor,
            self.rows[row].id,
            self.rows.iter().map(|r| r.id),
            toggle,
            extend,
        );
    }
    fn select_all_files(&mut self) {
        self.selected = self.rows.iter().map(|r| r.id).collect();
        self.file_cursor = self
            .file_cursor
            .or_else(|| (!self.rows.is_empty()).then_some(0));
        self.file_anchor = self.rows.first().map(|r| r.id);
    }
    fn clear_files(&mut self) {
        self.selected.clear();
        self.file_anchor = self.file_cursor.map(|i| self.rows[i].id);
    }
    fn file_keyboard(
        &mut self,
        ui: &egui::Ui,
        keyboard: &collection_controls::Keyboard,
        decisions: &HashMap<MediaId, Decision>,
    ) {
        let events = keyboard.events(ui, |key, m| {
            if m.alt {
                return false;
            }
            match key {
                egui::Key::ArrowUp
                | egui::Key::ArrowDown
                | egui::Key::Home
                | egui::Key::End
                | egui::Key::PageUp
                | egui::Key::PageDown
                | egui::Key::Space => true,
                egui::Key::A => m.command || m.ctrl,
                egui::Key::F2 => m.is_none(),
                _ => false,
            }
        });
        for (key, modifiers, repeat) in events {
            if repeat && matches!(key, egui::Key::Space | egui::Key::F2) {
                continue;
            }
            if self.rows.is_empty() {
                break;
            }
            let special = modifiers.command || modifiers.ctrl;
            if key == egui::Key::A {
                if modifiers.shift {
                    self.clear_files();
                } else {
                    self.select_all_files();
                }
                self.reveal_file = true;
                continue;
            }
            if key == egui::Key::F2 {
                if let Some(row) = self.file_cursor {
                    self.start_rename(self.rows[row].id);
                }
                continue;
            }
            if key == egui::Key::Space {
                let row = self.file_cursor.unwrap_or(0);
                self.select_file(row, !modifiers.shift || special, modifiers.shift);
                self.reveal_file = true;
                continue;
            }
            let target = match key {
                egui::Key::ArrowUp | egui::Key::ArrowDown => {
                    crate::core::navigation::next_position(
                        self.rows.len(),
                        self.file_cursor,
                        if key == egui::Key::ArrowUp { -1 } else { 1 },
                        |i| {
                            special
                                || modifiers.shift
                                || !decisions.get(&self.rows[i].id).is_some_and(|d| d.rejected)
                        },
                    )
                }
                egui::Key::Home => Some(0),
                egui::Key::End => Some(self.rows.len() - 1),
                egui::Key::PageUp => Some(
                    self.file_cursor
                        .unwrap_or(0)
                        .saturating_sub(keyboard.page_rows(ui)),
                ),
                egui::Key::PageDown => Some(
                    (self.file_cursor.unwrap_or(0) + keyboard.page_rows(ui))
                        .min(self.rows.len() - 1),
                ),
                _ => None,
            };
            if let Some(row) = target {
                if modifiers.shift {
                    self.select_file(row, special, true);
                } else {
                    // Checkbox-style selection: moving focus does not silently
                    // replace files already selected for a move. Space toggles.
                    self.file_cursor = Some(row);
                    self.file_anchor = Some(self.rows[row].id);
                }
                self.reveal_file = true;
            }
        }
    }
    fn photo_contents(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        decisions: &HashMap<MediaId, Decision>,
    ) -> Option<Command> {
        let mut command = None;
        ui.strong("Files · before / after");
        ui.horizontal(|ui| {
            if ui.button("Select all").clicked() {
                self.select_all_files();
            }
            if ui.button("Clear selection").clicked() {
                self.clear_files();
            }
            ui.label(format!("{} selected", self.selected.len()));
        });
        command = self.file_name_controls(ui).or(command);
        ui.separator();
        let name_controls_height = if self.renaming.is_some() { 110.0 } else { 35.0 };
        let mut scroll = egui::ScrollArea::vertical().id_salt("organize_photos");
        if let Some(offset) = self.restore_scroll[0].take() {
            scroll = scroll.vertical_scroll_offset(offset);
        }
        let row_height = button_row_height(ui)
            + ui.spacing().item_spacing.y
            + ui.text_style_height(&egui::TextStyle::Small);
        let max_height = (height - name_controls_height).max(60.0);
        let keyboard =
            collection_controls::Keyboard::new(ui, "organize_photos", max_height, row_height);
        self.file_keyboard(ui, &keyboard, decisions);
        if std::mem::take(&mut self.reveal_file)
            && let Some(row) = self.file_cursor
        {
            scroll = keyboard.scroll(ui, scroll, row);
        }
        let focused = keyboard.has_focus(ui);
        let output = scroll
            .auto_shrink([false, true])
            .max_height(max_height)
            .show_rows(ui, row_height, self.rows.len(), |ui, rows| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for i in rows {
                    let id = self.rows[i].id;
                    let name = self.rows[i].source.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let after_name = self.names.get(&id).map(|stem|
                        crate::core::organize::renamed_file_name(std::path::Path::new(&name), std::ffi::OsStr::new(stem)).to_string_lossy().into_owned())
                        .unwrap_or_else(|| name.clone());
                    let before = self.rows[i].source.parent().and_then(|p| p.strip_prefix(&self.catalog.root).ok()).unwrap_or_else(|| std::path::Path::new("")).to_path_buf();
                    let after = self.rows[i].after.clone();
                    let selected = self.selected.contains(&id);
                    ui.push_id(id.0, |ui| {
                        let marked = decisions.get(&id).is_some_and(|d| d.rejected);
                        let title = if marked { RichText::new(format!("{name} · Trash")).color(RED) } else { RichText::new(&name) };
                        let response = ui.add_sized([ui.available_width(), button_row_height(ui)],
                            egui::Button::selectable(selected, title).truncate().sense(collection_controls::pointer_sense(true)));
                        keyboard.focus_on_click(&response);
                        if focused && self.file_cursor == Some(i) {
                            ui.painter().rect_stroke(response.rect, 2., ui.visuals().selection.stroke, StrokeKind::Inside);
                        }
                        if response.clicked() {
                            let m = ui.input(|i| i.modifiers);
                            self.select_file(i, !m.shift || m.command || m.ctrl, m.shift);
                        }
                        if response.double_clicked() { self.start_rename(id); }
                        response.context_menu(|ui| {
                            let files = if self.selected.contains(&id) { self.selected.clone() } else { HashSet::from([id]) };
                            self.trash_menu(ui, Selection { files, ..Default::default() }, decisions);
                            ui.separator();
                            if ui.add_enabled(self.reset_enabled, egui::Button::new("Reset file destinations")).on_hover_text("Restore the initial grouping for the clicked file, or the selected files when it belongs to the selection. File names stay unchanged.").clicked() {
                                self.reset_request = Some(Selection { files: if self.selected.contains(&id) { self.selected.clone() } else { HashSet::from([id]) }, ..Default::default() });
                                ui.close();
                            }
                            ui.separator();
                            let paths = self.file_locations(&self.rows[i]);
                            if let Some(request) = desktop::locations_menu(ui, &paths, self.desktop_enabled) {
                                self.desktop_pending = Some(request);
                            }
                        });
                        #[cfg(test)]
                        ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test_file_rect", id.0)), response.rect));
                        if self.starts_drag(ui, &response) {
                            egui::DragAndDrop::set_payload(ui.ctx(), self.file_drag(id));
                        }
                        self.track_completed_drag(ui, &response, Some(id), &after);
                        if let Some(drop) = self.drop_target(ui, &response, &after) { command = Some(drop); }
                        let before_path = std::path::Path::new(&before).join(&name);
                        let after_path = std::path::Path::new(&after).join(&after_name);
                        ui.add(egui::Label::new(RichText::new(format!("{}  >  {}", before_path.display(), after_path.display())).small().color(MUTED)).truncate())
                            .on_hover_text(format!("Before: {}\nAfter: {}\nDrag selected files onto a destination folder. Double-click a file to rename it.", before_path.display(), after_path.display()));
                    });
                }
            });
        #[cfg(test)]
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                egui::Id::new("test_organize_photo_scroll"),
                (
                    output.inner_rect,
                    output.content_size,
                    output.state.offset.y,
                    row_height,
                ),
            )
        });
        keyboard.finish(ui, output.inner_rect, "Files. Arrows move focus; Space selects; Shift extends selection; Control or Command A selects all; F2 renames.");
        self.photo_scroll = output.state.offset.y;
        self.scroll_drag(ui, &output);
        command
    }
    fn structure(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        decisions: &HashMap<MediaId, Decision>,
    ) -> Option<Command> {
        // Rendered once per frame, either in a column or the compact tab.
        // Global scope preserves egui state when that parent changes.
        ui.scope_builder(
            egui::UiBuilder::new().id(("organize_folder_panel", &self.catalog.root)),
            |ui| self.folder_contents(ui, height, decisions),
        )
        .inner
    }
    fn select_folder(&mut self, path: &str, toggle: bool, extend: bool) {
        let position = |path: &str| {
            self.tree
                .lookup
                .get(&Key::Folder(path.into()))
                .and_then(|i| self.visible_folders.binary_search(i).ok())
        };
        let Some(target) = position(path) else {
            return;
        };
        let anchor = self
            .folder_anchor
            .as_deref()
            .or_else(|| extend.then_some(self.folder.as_str()))
            .and_then(position);
        let start = crate::core::selection::select_positions(
            &mut self.selected_folders,
            anchor,
            target,
            |range| {
                self.visible_folders[range]
                    .iter()
                    .filter_map(|&i| match &self.tree.nodes[i].key {
                        Key::Folder(path) if !path.as_os_str().is_empty() => {
                            Some(path.to_string_lossy().replace('\\', "/"))
                        }
                        _ => None,
                    })
            },
            toggle,
            extend,
        );
        let Key::Folder(anchor) = &self.tree.nodes[self.visible_folders[start]].key else {
            unreachable!()
        };
        self.folder_anchor = Some(anchor.to_string_lossy().replace('\\', "/"));
        self.folder = path.into();
        self.name = path.rsplit('/').next().unwrap_or_default().into();
    }
    fn folder_contents(
        &mut self,
        ui: &mut egui::Ui,
        height: f32,
        decisions: &HashMap<MediaId, Decision>,
    ) -> Option<Command> {
        let mut command = None;
        let mut expansion_changed = false;
        let mut hover_target = None;
        let owned_drag = egui::DragAndDrop::payload::<FolderDrag>(ui.ctx())
            .is_some_and(|payload| Arc::ptr_eq(&payload.owner, &self.drag_owner));
        let pointer = ui.ctx().pointer_interact_pos();
        ui.strong("Destination folders");
        let mut scroll = egui::ScrollArea::vertical().id_salt("organize_folders");
        if let Some(offset) = self.restore_scroll[1].take() {
            scroll = scroll.vertical_scroll_offset(offset);
        }
        let row_height = button_row_height(ui);
        let max_height = (height - 210.0).max(75.0);
        let keyboard = tree_controls::Keyboard::new(ui, "organize_folders", max_height, row_height);
        for (navigation, modifiers) in keyboard.navigation_events(ui, true) {
            let current = Key::Folder(self.folder.clone().into());
            if let Some(result) = keyboard.navigate_once(
                ui,
                &self.tree,
                Some(&current),
                navigation,
                &mut self.collapsed,
            ) {
                if result.expansion_changed {
                    self.project_folders();
                }
                // Expanding the current branch does not replace a prepared selection.
                if !(result.expansion_changed && result.key == current)
                    && let Key::Folder(path) = result.key
                {
                    self.select_folder(
                        &path.to_string_lossy().replace('\\', "/"),
                        modifiers.command || modifiers.ctrl,
                        modifiers.shift,
                    );
                }
            }
        }
        let scroll = keyboard.scroll(ui, scroll, &self.tree, &self.visible_folders, true);
        let output = scroll
            // Keep the centered dialog and drop targets still as branches open.
            .auto_shrink([false, false])
            .max_height(max_height)
            .show_rows(ui, row_height, self.visible_folders.len(), |ui, rows| {
                for row in rows {
                    let i = self.visible_folders[row];
                    let node = &self.tree.nodes[i];
                    let Key::Folder(path) = &node.key else { unreachable!() };
                    let path = path.to_string_lossy().replace('\\', "/");
                    let (count, depth) = (node.count, node.depth);
                    ui.push_id(("folder", &path), |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space((depth as f32 * 12.0).min(96.0));
                            expansion_changed |= tree_controls::expander(ui, &self.tree, i, &mut self.collapsed, &keyboard);
                            let response = ui.add_sized([ui.available_width(), row_height], egui::Button::selectable(self.selected_folders.contains(&path), format!("{} · {count}", folder_title(&path)))
                                .truncate().sense(tree_controls::pointer_sense(true)));
                            keyboard.focus_on_click(&response);
                            if self.folder == path {
                                ui.painter().rect_stroke(response.rect, 2., ui.visuals().selection.stroke, StrokeKind::Inside);
                            }
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test_folder_rect", &path)), response.rect));
                            response.context_menu(|ui| {
                                expansion_changed |= tree_controls::menu(ui, &self.tree, i, &mut self.collapsed);
                                let folders = if self.selected_folders.contains(&path) { self.selected_folders.clone() } else { std::collections::BTreeSet::from([path.clone()]) };
                                self.trash_menu(ui, Selection { folders, ..Default::default() }, decisions);
                                ui.separator();
                                if ui.add_enabled(self.reset_enabled, egui::Button::new("Reset file destinations")).on_hover_text("Restore the initial grouping for files inside this folder, or the selected folders. Folder structure edits and file names stay unchanged.").clicked() {
                                    self.reset_request = Some(Selection { folders: if self.selected_folders.contains(&path) { self.selected_folders.clone() } else { std::collections::BTreeSet::from([path.clone()]) }, ..Default::default() });
                                    ui.close();
                                }
                                ui.separator();
                                let paths = self.folder_locations(&path);
                                if let Some(request) = desktop::locations_menu(ui, &paths, self.desktop_enabled) {
                                    self.desktop_pending = Some(request);
                                }
                            });
                            if response.clicked() {
                                let modifiers = ui.input(|i| i.modifiers);
                                self.select_folder(&path, modifiers.command || modifiers.ctrl, modifiers.shift);
                            }
                            if !path.is_empty() && self.starts_drag(ui, &response) {
                                egui::DragAndDrop::set_payload(ui.ctx(), self.folder_drag(&path));
                            }
                            if owned_drag && pointer.is_some_and(|pos| pointer::contains_pointer(ui, &response, pos)) {
                                hover_target = Some(i);
                            }
                            self.track_completed_drag(ui, &response, None, &path);
                            if let Some(drop) = self.drop_target(ui, &response, &path) { command = Some(drop); }
                            response.on_hover_text("Counts include subfolders. Drop files here, or drag folders to nest them. Shift-click or Shift with Up/Down, Home/End or Page keys selects a visible range; Ctrl/Command adds to it. Pause over a collapsed folder while dragging to open it. Use the arrow or context menu to expand/collapse.");
                        });
                    });
                }
            });
        #[cfg(test)]
        ui.ctx().data_mut(|d| {
            d.insert_temp(
                egui::Id::new("test_organize_folder_scroll"),
                (
                    output.inner_rect,
                    output.content_size,
                    output.state.offset.y,
                    row_height,
                ),
            )
        });
        expansion_changed |=
            self.hover_expand
                .update(ui, &self.tree, hover_target, &mut self.collapsed);
        if expansion_changed {
            self.project_folders();
        }
        keyboard.finish(ui, output.inner_rect);
        self.folder_scroll = output.state.offset.y;
        self.scroll_drag(ui, &output);
        ui.separator();
        ui.add(egui::Label::new(format!("Selected: {}", folder_title(&self.folder))).truncate());
        if ui
            .add_enabled(
                !self.selected.is_empty(),
                egui::Button::new("Move selected files here"),
            )
            .clicked()
        {
            command = Some(Command::Move);
        }
        ui.horizontal(|ui| {
            ui.label("Name");
            ui.add(
                egui::TextEdit::singleline(&mut self.name)
                    .desired_width(ui.available_width())
                    .hint_text("Folder name"),
            );
        });
        ui.horizontal_wrapped(|ui| {
            if ui.button("Create folder").clicked() {
                command = Some(Command::Create);
            }
            if ui
                .add_enabled(!self.folder.is_empty(), egui::Button::new("Rename"))
                .clicked()
            {
                command = Some(Command::Rename);
            }
            if ui
                .add_enabled(!self.folder.is_empty(), egui::Button::new("Create parent"))
                .clicked()
            {
                command = Some(Command::Parent);
            }
        });
        egui::ComboBox::from_id_salt("organize_parent")
            .selected_text(format!("Into: {}", folder_title(&self.parent)))
            .width(ui.available_width().min(300.0))
            .show_ui(ui, |ui| {
                for node in &self.tree.nodes {
                    let Key::Folder(path) = &node.key else {
                        unreachable!()
                    };
                    ui.selectable_value(
                        &mut self.parent,
                        path.to_string_lossy().replace('\\', "/"),
                        &node.label,
                    );
                }
            });
        ui.horizontal_wrapped(|ui| {
            if ui
                .add_enabled(!self.folder.is_empty(), egui::Button::new("Nest folder"))
                .clicked()
            {
                command = Some(Command::Nest);
            }
            if ui
                .add_enabled(
                    self.folder.contains('/'),
                    egui::Button::new("Move folder to root"),
                )
                .clicked()
            {
                command = Some(Command::Unnest);
            }
        });
        command
    }
    fn apply(&mut self, command: Command, media: &[Media], companions: bool) {
        let result = match command {
            Command::RenameFile(id, name) => crate::core::organize::set_file_name(
                &self.catalog,
                &mut self.names,
                id,
                &name,
                companions,
            )
            .map(|()| {
                self.renaming = None;
                self.folder.clone()
            }),
            Command::Create => self.draft.create_folder(&self.folder, self.name.trim()),
            Command::Rename => self.draft.rename_folder(&self.folder, self.name.trim()),
            Command::Parent => self.draft.create_parent(&self.folder, self.name.trim()),
            Command::Nest => self.draft.nest_folder(&self.folder, &self.parent),
            Command::Unnest => self.draft.unnest_folder(&self.folder),
            Command::Move => self
                .draft
                .move_selection(
                    &Selection {
                        files: self.selected.clone(),
                        ..Default::default()
                    },
                    &self.folder,
                    media,
                    companions,
                )
                .map(|()| self.folder.clone()),
            Command::Drop(selection, target) => self
                .draft
                .move_selection(&selection, &target, media, companions)
                .map(|()| target),
        };
        match result {
            Ok(folder) => {
                self.folder = folder;
                self.selected_folders
                    .retain(|p| self.draft.folders().contains(p));
                if !self.draft.folders().contains(&self.parent) {
                    self.parent.clear();
                }
                self.error = None;
                self.refresh();
                self.tree.reveal(
                    &Key::Folder(self.folder.clone().into()),
                    &mut self.collapsed,
                );
                self.project_folders();
            }
            Err(error) => self.error = Some(error.to_string()),
        }
    }
}
impl PhotoSortApp {
    fn change_location_layout(&mut self, layout: crate::core::geography::Layout) {
        if self.step != Step::Organize
            || self.group_mode != GroupMode::Location
            || self.modal_active()
            || self.scanning
            || self.analysis_run.is_some()
            || self.retry_plan.is_some()
            || self.preferences.location_layout == layout
        {
            return;
        }
        if self.organize_drafts.contains_key(&GroupMode::Location) {
            self.pending_location_layout = Some(layout);
        } else {
            self.accept_location_layout(layout);
        }
    }
    fn accept_location_layout(&mut self, layout: crate::core::geography::Layout) {
        self.preferences.location_layout = layout;
        self.organize_drafts.remove(&GroupMode::Location);
        self.organize_view = None;
        self.pending_bookmark = None;
        self.set_review_plan(None);
        self.save_preferences();
    }
    pub(super) fn location_layout_controls(&mut self, ui: &mut egui::Ui) {
        use crate::core::geography::Layout;
        let choices = [
            (Layout::Country, "Country"),
            (Layout::CountryCity, "Country / City"),
            (Layout::CountryRegionCity, "Country / Region / City"),
        ];
        let selected = choices
            .iter()
            .find(|(layout, _)| *layout == self.preferences.location_layout)
            .unwrap()
            .1;
        ui.add_enabled_ui(!self.modal_active() && !self.scanning && self.analysis_run.is_none()
            && self.retry_plan.is_none(), |ui| {
            egui::ComboBox::from_id_salt("location_layout").selected_text(selected)
                .show_ui(ui, |ui| {
                    for (layout, title) in choices {
                        if ui.selectable_label(self.preferences.location_layout == layout, title).clicked() {
                            self.change_location_layout(layout);
                            ui.close();
                        }
                    }
                }).response.on_hover_text("Choose location folder levels. Reuses cached metadata; no AI analysis needed to change levels.");
        });
    }
    pub(super) fn location_layout_dialog(&mut self, ctx: &egui::Context) {
        let Some(_) = self.pending_location_layout else {
            return;
        };
        let mut action = None;
        egui::Modal::new(egui::Id::new("replace_location_layout")).show(ctx, |ui| {
            ui.set_max_width(400.0_f32.min((ctx.content_rect().width() - 60.0).max(180.0)));
            ui.heading("Replace location folder edits?");
            ui.label("The new levels will regenerate location folders and replace their custom folder edits. File names, ratings and other grouping modes keep their edits. Your photos stay unchanged until Apply.");
            ui.horizontal_wrapped(|ui| {
                if ui.button("Keep current layout").clicked() { action = Some(false); }
                if ui.button("Replace location layout").clicked() { action = Some(true); }
            });
        });
        if let Some(replace) = action {
            self.resolve_location_layout(replace);
        }
    }
    fn resolve_location_layout(&mut self, replace: bool) {
        if let Some(layout) = self.pending_location_layout.take()
            && replace
        {
            self.accept_location_layout(layout);
        }
    }
    pub(super) fn metadata_organize_mode(&self) -> bool {
        self.step == Step::Organize
            && matches!(
                self.group_mode,
                GroupMode::Date | GroupMode::Camera | GroupMode::Location
            )
    }
    pub(super) fn organize_metadata_ready(&self) -> bool {
        !self.metadata_organize_mode()
            || self.organize_drafts.contains_key(&self.group_mode)
            || self.metadata.len() == self.media.len()
    }
    pub(super) fn automatic_organize_mode(&self) -> bool {
        matches!(
            self.group_mode,
            GroupMode::Visual | GroupMode::Mixed | GroupMode::Faces
        )
    }
    pub(super) fn organize_labels(&self) -> HashMap<MediaId, String> {
        let mut labels = self
            .media
            .iter()
            .filter(|m| {
                (self.group_mode != GroupMode::Visual
                    || self
                        .visual_groups
                        .as_ref()
                        .is_some_and(|groups| groups.contains_key(&m.id)))
                    && (self.group_mode != GroupMode::Faces
                        || self
                            .face_groups
                            .as_ref()
                            .is_some_and(|groups| groups.groups.contains_key(&m.id)))
                    && (self.group_mode != GroupMode::Mixed
                        || self
                            .mixed_labels
                            .as_ref()
                            .is_some_and(|labels| labels.contains_key(&m.id)))
            })
            .map(|m| (m.id, self.label_for(m)))
            .collect();
        if let Some(catalog) = &self.catalog {
            crate::core::organize::include_inventory(catalog, &mut labels);
        }
        labels
    }
    pub(super) fn organize_grouping_ready(&self) -> bool {
        match self.group_mode {
            GroupMode::Visual => self.visual_groups.is_some(),
            GroupMode::Mixed => self.mixed_labels.is_some(),
            GroupMode::Faces => self.face_groups.is_some(),
            GroupMode::Date | GroupMode::Camera | GroupMode::Location => {
                self.metadata.len() == self.media.len()
            }
            _ => true,
        }
    }
    pub(super) fn can_edit_folders(&self) -> bool {
        !self.scanning
            && self.retry_plan.is_none()
            && !self.busy
            && self.analysis_run.is_none()
            && self.catalog.is_some()
            && (self.organize_grouping_ready()
                || self.organize_drafts.contains_key(&self.group_mode))
    }
    pub(super) fn open_organize_editor(&mut self) {
        if !self.can_edit_folders() {
            return;
        }
        let draft = self
            .organize_drafts
            .get(&self.group_mode)
            .cloned()
            .unwrap_or_else(|| {
                Draft::from_catalog(self.catalog.as_ref().unwrap(), self.organize_labels())
            });
        let rows = self
            .catalog
            .as_ref()
            .unwrap()
            .organize_files()
            .into_iter()
            .filter(|(id, _)| draft.labels().contains_key(id))
            .map(|(id, path)| PhotoRow {
                id,
                source: path.to_owned(),
                after: String::new(),
            })
            .collect();
        let folder = self
            .active
            .and_then(|id| draft.labels().get(&id))
            .cloned()
            .unwrap_or_default();
        let mut editor = Editor {
            catalog: self.catalog.as_ref().unwrap().clone(),
            output: self.output.clone().into(),
            desktop_enabled: self.desktop_available(),
            desktop_pending: None,
            reset_request: None,
            trash_request: None,
            trash_now: None,
            reset_enabled: self.organize_grouping_ready() && self.analysis_run.is_none(),
            names: self.file_names.clone(),
            renaming: None,
            rename_focus: false,
            draft,
            rows,
            tree: Tree::default(),
            visible_folders: Vec::new(),
            collapsed: Default::default(),
            selected: HashSet::new(),
            file_cursor: None,
            file_anchor: None,
            reveal_file: false,
            selected_folders: Default::default(),
            folder_anchor: None,
            drag_owner: Arc::new(()),
            completed_drag: None,
            drag: Default::default(),
            hover_expand: Default::default(),
            folder,
            name: String::new(),
            parent: String::new(),
            error: None,
            compact_folders: false,
            photo_scroll: 0.0,
            folder_scroll: 0.0,
            restore_scroll: [Some(0.0); 2],
        };
        editor.refresh();
        if let Some(view) = &self.organize_view {
            editor.restore_bookmark(view);
        }
        self.organize_editor = Some(editor);
    }
    pub(super) fn organize_dialog(&mut self, ctx: &egui::Context) {
        let Some(mut editor) = self.organize_editor.take() else {
            return;
        };
        editor.desktop_enabled = self.desktop_available();
        editor.reset_enabled = self.organize_grouping_ready() && self.analysis_run.is_none();
        editor.output = self.output.clone().into();
        editor.begin_drag_frame(ctx);
        let mut action = 0;
        let mut command = None;
        egui::Modal::new(egui::Id::new("organize_editor")).show(ctx, |ui| {
            let width = (ctx.content_rect().width() - 64.0).clamp(280.0, 1080.0);
            let height = (ctx.content_rect().height() - 360.0).clamp(160.0, 560.0);
            ui.set_width(width);
            ui.heading("Arrange files and folders");
            if width >= 680.0 {
                ui.label("Drag files onto folders, drag folders to nest them, or select a file to rename it.");
                ui.label(RichText::new("Preview moves, then Apply to change files. Photo companions move and rename together when enabled. Added empty folders are included in the reviewed plan.").small().color(MUTED));
            } else {
                ui.label("Drag files or folders to arrange them. Preview moves before applying.")
                    .on_hover_text("Photo companions move and rename together when enabled. Added empty folders are included in the reviewed plan.");
            }
            ui.separator();
            if width >= 680.0 {
                ui.columns(2, |columns| {
                    command = editor.photos(&mut columns[0], height, &self.decisions);
                    command = editor.structure(&mut columns[1], height, &self.decisions).or_else(|| command.take());
                });
            } else {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut editor.compact_folders, false, "Files");
                    let folders = ui.selectable_value(&mut editor.compact_folders, true, "Folders");
                    #[cfg(test)]
                    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("test_folders_tab"), folders.rect));
                    if folders.dnd_hover_payload::<FolderDrag>().is_some_and(|p| Arc::ptr_eq(&p.owner, &editor.drag_owner)) {
                        editor.compact_folders = true;
                    }
                });
                if editor.compact_folders { command = editor.structure(ui, height, &self.decisions); }
                else { command = editor.photos(ui, height, &self.decisions); }
            }
            if let Some(error) = &editor.error { ui.colored_label(RED, error); }
            ui.separator();
            ui.horizontal_wrapped(|ui| {
                if ui.button("Reset folder layout").clicked() { action = 4; }
                if ui.button("Cancel edits").on_hover_text("Discard folder and name edits. Shared Trash marks remain staged for review.").clicked() { action = 1; }
                if ui.button("Keep edits").clicked() { action = 2; }
                if ui.button("Preview moves >").clicked() { action = 3; }
            });
        });
        if command.is_none()
            && let Some(drag) = editor.completed_drag.take()
            && let (Some(payload), Some(target)) = (drag.payload, drag.target)
        {
            command = Some(Command::Drop(payload.selection, target));
        }
        if let Some(command) = command {
            editor.apply(command, &self.media, self.companions);
        }
        if let Some(selection) = editor.reset_request.take()
            && editor.reset_enabled
        {
            match editor.draft.reset_destinations(
                &selection,
                &self.organize_labels(),
                &self.media,
                self.companions,
            ) {
                Ok(()) => {
                    editor.error = None;
                    editor.refresh();
                }
                Err(error) => editor.error = Some(error.to_string()),
            }
        }
        if let Some((ids, marked)) = editor.trash_request.take() {
            self.stage_manual(ids, crate::core::decisions::Change::Trash(marked));
        }
        if action != 0 {
            egui::DragAndDrop::clear_payload(ctx);
        }
        if let Some(request) = editor.desktop_pending.take() {
            self.dispatch_desktop(request);
        }
        let trash_now = editor.trash_now.take();
        self.finish_organize_edit(editor, if trash_now.is_some() { 2 } else { action });
        if self.organize_editor.is_none()
            && let Some(request) = trash_now
        {
            self.review_trash(request);
        }
    }
    fn finish_organize_edit(&mut self, mut editor: Editor, action: u8) {
        // Keep/Preview accepts a currently typed name too; do not silently lose
        // the field because the user skipped the optional per-file Stage button.
        if matches!(action, 2 | 3)
            && let Some((id, name)) = editor.renaming.clone()
        {
            editor.apply(Command::RenameFile(id, name), &self.media, self.companions);
            if editor.error.is_some() {
                self.organize_editor = Some(editor);
                return;
            }
        }
        self.organize_view = Some(editor.bookmark());
        match action {
            1 => {}
            2 | 3 => {
                self.dirty = Some(Instant::now());
                self.file_names = editor.names;
                self.organize_drafts.insert(self.group_mode, editor.draft);
                if action == 3 {
                    self.organize();
                }
            }
            4 => {
                editor.draft =
                    Draft::from_catalog(self.catalog.as_ref().unwrap(), self.organize_labels());
                editor.folder.clear();
                editor.parent.clear();
                editor.name.clear();
                editor.renaming = None;
                editor.error = None;
                editor.selected_folders.clear();
                editor.folder_anchor = None;
                editor.collapsed.clear();
                editor.refresh();
                self.organize_editor = Some(editor);
            }
            _ => self.organize_editor = Some(editor),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn organize_trash_menus_share_state_for_mixed_files_and_folder_descendants() {
        for width in [640.0, 1440.0] {
            for folder in [false, true] {
                let mut app = app();
                app.index = app
                    .media
                    .iter()
                    .enumerate()
                    .map(|(i, m)| (m.id, i))
                    .collect();
                let document = PathBuf::from("/missing/trip/child/notes.txt");
                let doc = MediaId::for_path(&document);
                let catalog = Arc::get_mut(app.catalog.as_mut().unwrap()).unwrap();
                catalog.inventory.push(document);
                catalog
                    .supporting_fingerprints
                    .insert(doc, Fingerprint::default());
                catalog.directories = vec!["/missing/trip".into(), "/missing/trip/child".into()];
                app.stage_manual([MediaId(0)], crate::core::decisions::Change::Trash(true));
                app.open_organize_editor();
                let editor = app.organize_editor.as_mut().unwrap();
                editor.selected = HashSet::from([MediaId(0), doc]);
                editor.compact_folders = folder;
                editor.draft.create_folder("", "trip-1").unwrap();
                editor.draft.move_photos(&[MediaId(2)], "trip-1").unwrap();
                editor.refresh();
                let labels = editor.draft.labels().clone();
                let ctx = egui::Context::default();
                theme_install(&ctx);
                ctx.enable_accesskit();
                let frame = |app: &mut PhotoSortApp, events| {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(width, 850.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ctx| app.organize_dialog(ctx),
                    )
                };
                let click = |pos, button| {
                    vec![
                        egui::Event::PointerMoved(pos),
                        egui::Event::PointerButton {
                            pos,
                            button,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                        egui::Event::PointerButton {
                            pos,
                            button,
                            pressed: false,
                            modifiers: Default::default(),
                        },
                    ]
                };
                for desired in [true, false] {
                    for _ in 0..3 {
                        frame(&mut app, vec![]);
                    }
                    let key = if folder {
                        egui::Id::new(("test_folder_rect", "trip"))
                    } else {
                        egui::Id::new(("test_file_rect", 0_u64))
                    };
                    let pos = ctx
                        .data(|d| d.get_temp::<egui::Rect>(key))
                        .unwrap()
                        .center();
                    frame(&mut app, click(pos, egui::PointerButton::Secondary));
                    let output = frame(&mut app, vec![]);
                    let update = output.platform_output.accesskit_update.unwrap();
                    let label = if desired {
                        "Mark for Trash"
                    } else {
                        "Unmark from Trash"
                    };
                    let node = update
                        .nodes
                        .iter()
                        .find(|(_, n)| n.label() == Some(label))
                        .expect(label);
                    let bounds = node.1.bounds().unwrap();
                    assert!(
                        bounds.x0 >= 0.0 && bounds.x1 <= f64::from(width) && bounds.y1 <= 850.0
                    );
                    frame(
                        &mut app,
                        click(
                            egui::pos2(
                                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                                ((bounds.y0 + bounds.y1) / 2.0) as f32,
                            ),
                            egui::PointerButton::Primary,
                        ),
                    );
                    assert_eq!(app.decisions[&MediaId(0)].rejected, desired);
                    assert_eq!(app.decisions[&doc].rejected, desired);
                    assert!(
                        !app.decision(MediaId(2)).rejected,
                        "prefix-like sibling stays untouched"
                    );
                    assert_eq!(app.decision(MediaId(1)).rejected, folder && desired);
                    assert_eq!(
                        app.organize_editor.as_ref().unwrap().draft.labels(),
                        &labels
                    );
                }
                app.stage_manual([doc], crate::core::decisions::Change::Trash(true));
                let editor = app.organize_editor.take().unwrap();
                app.finish_organize_edit(editor, 1);
                assert!(
                    app.decisions[&doc].rejected,
                    "layout cancel preserves shared marks"
                );
                assert!(app.organize_drafts.is_empty());
                assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
                assert!(
                    app.requested.is_empty()
                        && app.analysis_run.is_none()
                        && app.metadata_run.is_none()
                );
            }
        }
    }
    #[test]
    fn folder_expansion_editor_controls_persist_without_repeating_image_work() {
        for width in [640.0, 1440.0] {
            let mut app = app();
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.draft.create_folder("", "A").unwrap();
            editor.draft.create_folder("A", "B").unwrap();
            editor.draft.create_folder("A/B", "C").unwrap();
            editor.draft.create_folder("", "A-1").unwrap();
            editor.refresh();
            editor.compact_folders = true;
            editor.folder = "A/B/C".into();
            editor.selected_folders.insert("A/B/C".into());
            editor.selected.insert(MediaId(1));
            let labels = editor.draft.labels().clone();
            let tree_address = editor.tree.nodes.as_ptr();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.enable_accesskit();
            let frame = |app: &mut PhotoSortApp, events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.0),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.organize_dialog(ctx),
                )
            };
            let click = |pos, button| {
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: true,
                        modifiers: Default::default(),
                    },
                    egui::Event::PointerButton {
                        pos,
                        button,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ]
            };
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", "A"))))
                .unwrap();
            frame(
                &mut app,
                click(rect.center(), egui::PointerButton::Secondary),
            );
            let output = frame(&mut app, vec![]);
            let update = output.platform_output.accesskit_update.unwrap();
            let bounds = update
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Collapse subtree"))
                .unwrap()
                .1
                .bounds()
                .unwrap();
            frame(
                &mut app,
                click(
                    egui::pos2(
                        ((bounds.x0 + bounds.x1) / 2.0) as f32,
                        ((bounds.y0 + bounds.y1) / 2.0) as f32,
                    ),
                    egui::PointerButton::Primary,
                ),
            );
            let editor = app.organize_editor.as_ref().unwrap();
            assert_eq!(
                editor.collapsed,
                [Key::Folder("A".into()), Key::Folder("A/B".into())].into()
            );
            assert!(
                editor
                    .visible_folders
                    .iter()
                    .any(|&i| editor.tree.nodes[i].key == Key::Folder("A-1".into()))
            );
            assert!(
                !editor
                    .visible_folders
                    .iter()
                    .any(|&i| editor.tree.nodes[i].key == Key::Folder("A/B".into()))
            );
            assert_eq!(editor.folder, "A/B/C");
            assert!(
                editor.selected_folders.contains("A/B/C") && editor.selected.contains(&MediaId(1))
            );
            assert_eq!(editor.draft.labels(), &labels);
            assert_eq!(editor.tree.nodes.as_ptr(), tree_address);
            for _ in 0..2 {
                frame(&mut app, vec![]);
            }
            let rect = ctx
                .data(|d| {
                    d.get_temp::<egui::Rect>(egui::Id::new((
                        "test_tree_expander",
                        Key::Folder("A".into()),
                    )))
                })
                .unwrap();
            frame(&mut app, click(rect.center(), egui::PointerButton::Primary));
            let mut editor = app.organize_editor.take().unwrap();
            assert_eq!(editor.collapsed, [Key::Folder("A/B".into())].into());
            let bookmark = editor.bookmark();
            let encoded = serde_json::to_string(&bookmark).unwrap();
            assert_eq!(
                serde_json::from_str::<crate::core::views::EditorBookmark>(&encoded).unwrap(),
                bookmark
            );
            // Rename is structural: reveal the destination, drop obsolete keys.
            editor.folder = "A".into();
            editor.name = "Renamed".into();
            editor.apply(Command::Rename, &app.media, false);
            assert!(editor.collapsed.is_empty());
            assert!(editor.draft.folders().contains("Renamed/B/C"));
            editor.collapsed.insert(Key::Folder("Renamed".into()));
            editor.project_folders();
            app.finish_organize_edit(editor, 2);
            app.open_organize_editor();
            let editor = app.organize_editor.take().unwrap();
            assert_eq!(editor.collapsed, [Key::Folder("Renamed".into())].into());
            assert!(editor.selected.contains(&MediaId(1)));
            app.finish_organize_edit(editor, 1);
            assert!(
                app.requested.is_empty()
                    && app.metadata_run.is_none()
                    && app.analysis_run.is_none()
                    && !app.scanning
                    && !app.busy
            );
            assert_eq!(app.stats.decodes, 0);
            assert_eq!(app.stats.metadata_reads, 0);
        }
    }
    #[test]
    fn folder_ranges_pointer_keyboard_and_resume_use_visible_order_without_image_work() {
        for width in [640., 1200.] {
            let mut app = app();
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            for path in ["A", "A/B", "A/B/C", "A-1", "B", "C"] {
                let (parent, leaf) = path.rsplit_once('/').unwrap_or(("", path));
                editor.draft.create_folder(parent, leaf).unwrap();
            }
            editor.refresh();
            editor.collapsed.insert(Key::Folder("A".into()));
            editor.project_folders();
            editor.compact_folders = true;
            editor.selected.insert(MediaId(1));
            let labels = editor.draft.labels().clone();
            let tree = editor.tree.nodes.as_ptr();
            let projection = editor.visible_folders.as_ptr();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let frame = |app: &mut PhotoSortApp, events, modifiers| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.),
                        )),
                        events,
                        modifiers,
                        ..Default::default()
                    },
                    |ctx| app.organize_dialog(ctx),
                );
            };
            let click = |app: &mut PhotoSortApp, path: &str, modifiers: egui::Modifiers| {
                let rect = ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", path))))
                    .unwrap();
                let mut events = tree_controls::tests::click(rect.center());
                for event in &mut events {
                    if let egui::Event::PointerButton { modifiers: m, .. } = event {
                        *m = modifiers;
                    }
                }
                frame(app, events, modifiers);
            };
            let key = |key, modifiers| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers,
            };
            let shift = egui::Modifiers::SHIFT;
            let command = egui::Modifiers::COMMAND;
            let additive = egui::Modifiers {
                shift: true,
                command: true,
                ..Default::default()
            };
            for _ in 0..3 {
                frame(&mut app, vec![], egui::Modifiers::NONE);
            }
            click(&mut app, "A", egui::Modifiers::NONE);
            click(&mut app, "C", shift);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected_folders,
                ["A", "A-1", "B", "C"].map(String::from).into()
            );
            assert_eq!(
                app.organize_editor
                    .as_ref()
                    .unwrap()
                    .folder_anchor
                    .as_deref(),
                Some("A")
            );
            click(&mut app, "A", egui::Modifiers::NONE);
            frame(
                &mut app,
                vec![
                    key(egui::Key::ArrowDown, shift),
                    key(egui::Key::ArrowDown, shift),
                ],
                shift,
            );
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected_folders,
                ["A", "A-1", "B"].map(String::from).into()
            );
            frame(&mut app, vec![key(egui::Key::ArrowUp, shift)], shift);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected_folders,
                ["A", "A-1"].map(String::from).into()
            );
            click(&mut app, "C", command);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected_folders,
                ["A", "A-1", "C"].map(String::from).into()
            );
            frame(&mut app, vec![key(egui::Key::ArrowUp, additive)], additive);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected_folders,
                ["A", "A-1", "B", "C"].map(String::from).into()
            );
            frame(&mut app, vec![key(egui::Key::ArrowUp, shift)], shift);
            let editor = app.organize_editor.as_ref().unwrap();
            assert_eq!(
                editor.selected_folders,
                ["A-1", "B", "C"].map(String::from).into()
            );
            assert_eq!(editor.folder_anchor.as_deref(), Some("C"));
            assert_eq!(editor.tree.nodes.as_ptr(), tree);
            assert_eq!(editor.visible_folders.as_ptr(), projection);
            assert_eq!(editor.draft.labels(), &labels);
            assert_eq!(editor.selected, [MediaId(1)].into());
            let bookmark = editor.bookmark();
            assert_eq!(
                serde_json::from_str::<crate::core::views::EditorBookmark>(
                    &serde_json::to_string(&bookmark).unwrap()
                )
                .unwrap(),
                bookmark
            );
            let editor = app.organize_editor.take().unwrap();
            app.finish_organize_edit(editor, 2);
            app.open_organize_editor();
            assert_eq!(
                app.organize_editor
                    .as_ref()
                    .unwrap()
                    .folder_anchor
                    .as_deref(),
                Some("C")
            );
            for _ in 0..3 {
                frame(&mut app, vec![], egui::Modifiers::NONE);
            }
            frame(&mut app, vec![key(egui::Key::Home, shift)], shift);
            let editor = app.organize_editor.as_ref().unwrap();
            assert_eq!(
                editor.selected_folders,
                ["A", "A-1", "B", "C"].map(String::from).into()
            );
            assert!(!editor.selected_folders.contains(""));
            assert!(!editor.selected_folders.contains("A/B"));
            assert!(
                app.requested.is_empty()
                    && app.metadata_run.is_none()
                    && app.analysis_run.is_none()
                    && !app.scanning
            );
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
            let editor = app.organize_editor.take().unwrap();
            app.finish_organize_edit(editor, 4);
            assert!(
                app.organize_editor
                    .as_ref()
                    .unwrap()
                    .folder_anchor
                    .is_none()
            );
        }
    }

    #[test]
    fn folder_ranges_reconcile_hidden_and_renamed_anchors_and_reuse_large_tree() {
        let mut app = app();
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.draft = Draft::new(
            (0..100_000)
                .map(|i| (MediaId(i), format!("folder-{i:06}")))
                .collect(),
        );
        editor.refresh();
        let tree = editor.tree.nodes.as_ptr();
        let projection = editor.visible_folders.as_ptr();
        editor.select_folder("folder-099998", false, false);
        editor.select_folder("folder-099999", false, true);
        assert_eq!(
            editor.selected_folders,
            ["folder-099998", "folder-099999"].map(String::from).into()
        );
        assert_eq!(editor.tree.nodes.as_ptr(), tree);
        assert_eq!(editor.visible_folders.as_ptr(), projection);
        // Restore drops an obsolete path; the next range starts from current focus.
        let mut bookmark = editor.bookmark();
        bookmark.folder_anchor = Some("missing".into());
        editor.restore_bookmark(&bookmark);
        assert!(editor.folder_anchor.is_none());
        editor.select_folder("folder-099998", false, true);
        assert_eq!(editor.selected_folders.len(), 2);
        // A hidden anchor is not a range through collapsed descendants.
        editor.collapsed.insert(Key::Folder("".into()));
        editor.project_folders();
        editor.select_folder("", false, true);
        assert!(editor.selected_folders.is_empty());
        assert_eq!(editor.folder_anchor.as_deref(), Some(""));
        editor.draft = Draft::new([(MediaId(1), "A/B".into())].into());
        editor.refresh();
        editor.collapsed.clear();
        editor.project_folders();
        editor.select_folder("A", false, false);
        editor.name = "Renamed".into();
        editor.apply(Command::Rename, &app.media, false);
        assert!(editor.folder_anchor.is_none());
        assert_eq!(editor.folder, "Renamed");
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }

    #[test]
    fn tree_keyboard_organize_keeps_drafts_and_file_selection_and_reveals_virtual_rows() {
        for width in [640.0, 1200.0] {
            let mut app = app();
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            for path in ["A", "A/B", "A/B/C", "A-1"] {
                let (parent, leaf) = path.rsplit_once('/').unwrap_or(("", path));
                editor.draft.create_folder(parent, leaf).unwrap();
            }
            for i in 0..2000 {
                editor.draft.create_folder("", &format!("z{i:04}")).unwrap();
            }
            editor.refresh();
            editor.compact_folders = true;
            editor.selected.insert(MediaId(1));
            let labels = editor.draft.labels().clone();
            let address = editor.tree.nodes.as_ptr();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let frame = |app: &mut PhotoSortApp, events| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.organize_dialog(ctx),
                );
            };
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", "A"))))
                .unwrap();
            frame(&mut app, tree_controls::tests::click(rect.center()));
            for (key, expected) in [
                (egui::Key::ArrowLeft, "A"),
                (egui::Key::ArrowDown, "A-1"),
                (egui::Key::ArrowUp, "A"),
                (egui::Key::ArrowRight, "A"),
                (egui::Key::ArrowRight, "A/B"),
                (egui::Key::End, "z1999"),
            ] {
                frame(&mut app, vec![tree_controls::tests::key(key)]);
                assert_eq!(
                    app.organize_editor.as_ref().unwrap().folder,
                    expected,
                    "width {width}, {key:?}"
                );
            }
            let editor = app.organize_editor.as_ref().unwrap();
            assert!(editor.folder_scroll > 10_000.);
            assert_eq!(editor.name, "z1999");
            assert_eq!(editor.selected_folders, ["z1999".into()].into());
            assert!(editor.selected.contains(&MediaId(1)));
            assert_eq!(editor.tree.nodes.as_ptr(), address);
            assert_eq!(editor.draft.labels(), &labels);
            frame(&mut app, vec![tree_controls::tests::key(egui::Key::Home)]);
            assert_eq!(app.organize_editor.as_ref().unwrap().folder, "");
            assert_eq!(app.organize_editor.as_ref().unwrap().folder_scroll, 0.);
            assert!(
                app.requested.is_empty()
                    && app.metadata_run.is_none()
                    && app.analysis_run.is_none()
                    && !app.scanning
            );
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        }
    }
    #[test]
    fn location_layout_reuses_metadata_for_editor_and_inspector_without_image_work() {
        use crate::core::geography::{Layout, Location};
        let mut app = app();
        app.group_mode = GroupMode::Location;
        for media in &app.media {
            app.metadata.insert(
                media.id,
                Metadata {
                    location: Some(Location {
                        country: "Japan".into(),
                        region: "Shizuoka".into(),
                        city: "Fuji".into(),
                    }),
                    ..Default::default()
                },
            );
        }
        for (layout, expected) in [
            (Layout::Country, "Japan"),
            (Layout::CountryCity, "Japan/Fuji"),
            (Layout::CountryRegionCity, "Japan/Shizuoka/Fuji"),
        ] {
            app.change_location_layout(layout);
            assert_eq!(app.label_for(&app.media[0]), expected);
            assert!(
                app.organize_labels()
                    .values()
                    .all(|label| label == expected)
            );
            app.open_organize_editor();
            let editor = app.organize_editor.take().unwrap();
            assert!(editor.rows.iter().all(|row| row.after == expected));
            assert!(editor.draft.folders().contains(expected));
            assert!(
                app.requested.is_empty()
                    && app.metadata_run.is_none()
                    && app.analysis_run.is_none()
                    && !app.scanning
                    && !app.busy
            );
            assert_eq!(app.stats.decodes, 0);
            assert_eq!(app.metadata.len(), 3);
        }
    }
    #[test]
    fn location_layout_requires_explicit_replacement_and_preserves_other_intent() {
        use crate::core::geography::Layout;
        let mut app = app();
        app.group_mode = GroupMode::Location;
        let draft = Draft::new([(MediaId(0), "Custom location".into())].into());
        app.organize_drafts
            .insert(GroupMode::Location, draft.clone());
        app.organize_drafts.insert(GroupMode::Camera, draft.clone());
        app.file_names.insert(MediaId(0), "Keeper".into());
        app.decisions.insert(
            MediaId(0),
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        app.change_location_layout(Layout::Country);
        assert_eq!(app.preferences.location_layout, Layout::CountryRegionCity);
        assert!(app.modal_active());
        app.resolve_location_layout(false);
        assert_eq!(app.organize_drafts[&GroupMode::Location], draft);
        assert_eq!(app.preferences.location_layout, Layout::CountryRegionCity);
        app.change_location_layout(Layout::CountryCity);
        app.resolve_location_layout(true);
        assert_eq!(app.preferences.location_layout, Layout::CountryCity);
        assert!(!app.organize_drafts.contains_key(&GroupMode::Location));
        assert_eq!(app.organize_drafts[&GroupMode::Camera], draft);
        assert_eq!(app.file_names[&MediaId(0)], "Keeper");
        assert_eq!(app.decisions[&MediaId(0)].rating, Some(4));
        assert!(app.requested.is_empty() && app.analysis_run.is_none() && app.plan.is_none());
        for state in 0..5 {
            app.step = if state == 0 {
                Step::Cull
            } else {
                Step::Organize
            };
            app.busy = state == 1;
            app.scanning = state == 2;
            app.analysis_run = (state == 3).then_some(1);
            app.plan_open = state == 4;
            app.change_location_layout(Layout::Country);
            assert_eq!(app.preferences.location_layout, Layout::CountryCity);
        }
    }
    fn metadata_fixture(count: usize) -> (PhotoSortApp, tempfile::TempDir) {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("photos");
        std::fs::create_dir(&root).unwrap();
        for i in 0..count {
            image::RgbImage::new(4, 4)
                .save(root.join(format!("photo-{i:04}.png")))
                .unwrap();
        }
        let catalog = Arc::new(Catalog::scan(&root, || false, |_| {}).unwrap());
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        app.root = Some(catalog.root.clone());
        app.output = catalog.root.display().to_string();
        app.media = catalog.media.clone();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.catalog = Some(catalog);
        (app, temp)
    }
    fn wait_metadata(app: &mut PhotoSortApp) {
        let deadline = Instant::now() + Duration::from_secs(10);
        while app.metadata_run.is_some() || app.busy {
            app.poll(&egui::Context::default());
            assert!(
                Instant::now() < deadline,
                "metadata or planning did not finish"
            );
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn metadata_organize_modes_coalesce_one_read_and_wait_for_offscreen_photos() {
        let (mut app, _temp) = metadata_fixture(129);
        let catalog = app.catalog.clone().unwrap();
        app.set_group_mode(GroupMode::Date);
        let run = app.metadata_run.unwrap();
        assert!(!app.organize_grouping_ready() && !app.can_edit_folders());
        app.open_organize_editor();
        app.organize();
        assert!(app.organize_editor.is_none() && app.plan.is_none() && !app.busy);
        for mode in [GroupMode::Location, GroupMode::Camera] {
            app.set_group_mode(mode);
            assert_eq!(app.metadata_run, Some(run));
        }
        wait_metadata(&mut app);
        assert!(app.organize_grouping_ready() && app.can_edit_folders());
        assert_eq!(app.metadata.len(), 129);
        assert_eq!(app.stats.metadata_reads, 129);
        assert_eq!(app.stats.decodes, 0);
        assert!(app.analysis_run.is_none() && app.analysis.is_empty() && app.requested.is_empty());
        assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
        for mode in [GroupMode::Date, GroupMode::Location, GroupMode::Camera] {
            app.set_group_mode(mode);
            assert!(app.metadata_run.is_none());
        }
        app.open_organize_editor();
        let editor = app.organize_editor.take().unwrap();
        assert_eq!(editor.rows.len(), 129);
        assert!(editor.rows.iter().all(|row| row.after == "Unknown camera"));
        app.organize();
        wait_metadata(&mut app);
        assert_eq!(app.plan.as_ref().unwrap().actions.len(), 129);
        assert_eq!(app.stats.metadata_reads, 129);
        assert!(app.media.iter().all(|m| m.path.is_file()));
    }
    #[test]
    fn metadata_organize_cancel_rejects_late_batches_and_resume_reuses_completed_data() {
        let (mut app, _temp) = metadata_fixture(129);
        app.set_group_mode(GroupMode::Date);
        let old = app.metadata_run.unwrap();
        app.set_group_mode(GroupMode::Current);
        assert!(app.metadata_run.is_none());
        // Drain any already-published old batch. It must not revive the cancelled run.
        for _ in 0..10 {
            app.poll(&egui::Context::default());
        }
        assert!(app.metadata.is_empty());
        app.set_group_mode(GroupMode::Camera);
        assert_ne!(app.metadata_run, Some(old));
        wait_metadata(&mut app);
        let reads = app.stats.metadata_reads;
        app.cancel_metadata_index();
        app.start_metadata_index();
        assert!(app.metadata_run.is_none());
        assert_eq!(app.metadata.len(), 129);
        assert_eq!(app.stats.metadata_reads, reads);
        assert_eq!(app.stats.decodes, 0);
        assert!(app.plan.is_none() && app.analysis.is_empty());
    }
    #[test]
    fn metadata_organize_missing_file_blocks_proposals_until_shared_retry_succeeds() {
        let (mut app, temp) = metadata_fixture(2);
        let path = app.media[0].path.clone();
        let backup = temp.path().join("temporarily-unavailable.png");
        std::fs::rename(&path, &backup).unwrap();
        app.set_group_mode(GroupMode::Location);
        wait_metadata(&mut app);
        assert_eq!(app.metadata.len(), 1);
        assert_eq!(app.metadata_failures.len(), 1);
        assert!(!app.organize_grouping_ready() && !app.can_edit_folders());
        app.open_organize_editor();
        assert!(app.organize_editor.is_none() && app.plan.is_none());
        std::fs::rename(backup, path).unwrap();
        app.start_metadata_index();
        wait_metadata(&mut app);
        assert_eq!(app.metadata.len(), 2);
        assert_eq!(app.stats.metadata_reads, 2);
        assert!(app.metadata_failures.is_empty() && app.can_edit_folders());
        assert_eq!(app.stats.decodes, 0);
    }
    #[test]
    fn metadata_organize_departure_waits_for_complete_metadata_before_reviewing_names() {
        let (mut app, _temp) = metadata_fixture(2);
        app.group_mode = GroupMode::Camera;
        app.file_names.insert(app.media[0].id, "Keeper".into());
        app.request_departure(Destination::Step(Step::Cull));
        assert!(app.organize_departure.is_some() && app.metadata_run.is_some());
        assert!(app.plan.is_none() && !app.busy && app.step == Step::Organize);
        wait_metadata(&mut app);
        assert!(app.plan.is_some() && app.organize_departure.is_some());
        assert_eq!(app.stats.metadata_reads, 2);
        assert_eq!(app.stats.decodes, 0);
        assert!(app.media.iter().all(|m| m.path.is_file()));
        app.resolve_organize_departure(super::super::transitions::OrganizeResolution::Cancel);
        assert!(app.plan.is_none() && app.file_names.len() == 1);
    }
    #[test]
    fn selective_reset_menu_preserves_other_edits_names_and_selection_at_all_sizes() {
        for size in [egui::vec2(600.0, 650.0), egui::vec2(1200.0, 850.0)] {
            for (selected_click, companions) in [(false, false), (true, false), (false, true)] {
                let mut app = app();
                app.companions = companions;
                app.open_organize_editor();
                let editor = app.organize_editor.as_mut().unwrap();
                editor.draft.create_folder("", "Custom").unwrap();
                editor
                    .draft
                    .move_photos(&[MediaId(0), MediaId(1), MediaId(2)], "Custom")
                    .unwrap();
                editor.names.insert(MediaId(1), "Keeper".into());
                editor.selected = if selected_click {
                    HashSet::from([MediaId(1), MediaId(2)])
                } else {
                    HashSet::from([MediaId(2)])
                };
                let selected = editor.selected.clone();
                editor.refresh();
                let ctx = egui::Context::default();
                theme_install(&ctx);
                ctx.enable_accesskit();
                let frame = |app: &mut PhotoSortApp, events| {
                    ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            events,
                            ..Default::default()
                        },
                        |ctx| app.organize_dialog(ctx),
                    )
                };
                for _ in 0..3 {
                    frame(&mut app, vec![]);
                }
                let pos = ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 1_u64))))
                    .unwrap()
                    .center();
                let button = |pos, button, pressed| egui::Event::PointerButton {
                    pos,
                    button,
                    pressed,
                    modifiers: Default::default(),
                };
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        button(pos, egui::PointerButton::Secondary, true),
                        button(pos, egui::PointerButton::Secondary, false),
                    ],
                );
                let tree = frame(&mut app, vec![])
                    .platform_output
                    .accesskit_update
                    .unwrap();
                let node = tree
                    .nodes
                    .iter()
                    .find(|(_, n)| n.label() == Some("Reset file destinations"))
                    .unwrap();
                let bounds = node.1.bounds().unwrap();
                assert!(
                    bounds.x0 >= 0.0
                        && bounds.y0 >= 0.0
                        && bounds.x1 <= f64::from(size.x)
                        && bounds.y1 <= f64::from(size.y)
                );
                let pos = egui::pos2(
                    ((bounds.x0 + bounds.x1) / 2.0) as f32,
                    ((bounds.y0 + bounds.y1) / 2.0) as f32,
                );
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(pos),
                        button(pos, egui::PointerButton::Primary, true),
                        button(pos, egui::PointerButton::Primary, false),
                    ],
                );
                let editor = app.organize_editor.take().unwrap();
                assert_eq!(editor.draft.labels()[&MediaId(1)], "trip");
                assert_eq!(
                    editor.draft.labels()[&MediaId(0)],
                    if companions { "trip" } else { "Custom" }
                );
                assert_eq!(
                    editor.draft.labels()[&MediaId(2)],
                    if selected_click { "trip" } else { "Custom" }
                );
                assert_eq!(editor.names[&MediaId(1)], "Keeper");
                assert_eq!(editor.selected, selected);
                assert!(editor.draft.folders().contains("Custom"));
                assert!(
                    app.requested.is_empty()
                        && app.analysis_run.is_none()
                        && app.metadata_run.is_none()
                );
                assert_eq!(app.stats.decodes, 0);
                assert_eq!(app.stats.metadata_reads, 0);
                app.finish_organize_edit(editor, 1);
                assert!(app.organize_drafts.is_empty() && app.file_names.is_empty());
            }
        }
    }
    #[test]
    fn selective_reset_waits_for_generated_metadata_and_survives_keep_reopen() {
        let mut app = app();
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.draft.create_folder("", "Custom").unwrap();
        editor.draft.move_photos(&[MediaId(0)], "Custom").unwrap();
        app.finish_organize_edit(editor, 2);
        app.open_organize_editor();
        app.organize_editor.as_mut().unwrap().reset_request = Some(Selection {
            files: HashSet::from([MediaId(0)]),
            ..Default::default()
        });
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| app.organize_dialog(ctx));
        let editor = app.organize_editor.take().unwrap();
        assert_eq!(editor.draft.labels()[&MediaId(0)], "trip");
        app.finish_organize_edit(editor, 2);
        app.open_organize_editor();
        assert_eq!(
            app.organize_editor.as_ref().unwrap().draft.labels()[&MediaId(0)],
            "trip"
        );
        app.group_mode = GroupMode::Location;
        app.metadata.clear();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.draft.move_photos(&[MediaId(0)], "Custom").unwrap();
        editor.reset_request = Some(Selection {
            files: HashSet::from([MediaId(0)]),
            ..Default::default()
        });
        let _ = ctx.run(Default::default(), |ctx| app.organize_dialog(ctx));
        assert_eq!(
            app.organize_editor.as_ref().unwrap().draft.labels()[&MediaId(0)],
            "Custom"
        );
        assert!(!app.organize_editor.as_ref().unwrap().reset_enabled);
    }
    #[test]
    fn responsive_editor_keeps_each_scroll_and_selection_without_repeating_work() {
        let mut app = app();
        let seed = |app: &mut PhotoSortApp| {
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.rows = (0..1000)
                .map(|i| PhotoRow {
                    id: MediaId(i),
                    source: format!("/missing/photo-{i:04}.jpg").into(),
                    after: String::new(),
                })
                .collect();
            editor.draft = Draft::new(
                (0..1000)
                    .map(|i| (MediaId(i), format!("folder-{i:04}")))
                    .collect(),
            );
            editor.refresh();
        };
        seed(&mut app);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let frame = |app: &mut PhotoSortApp, width, events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 900.0),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.organize_dialog(ctx),
            );
        };
        let editor = app.organize_editor.as_mut().unwrap();
        editor.restore_scroll = [Some(6500.0), Some(9000.0)];
        editor.selected.insert(MediaId(200));
        editor.selected_folders.insert("folder-0400".into());
        editor.folder = "folder-0400".into();
        let mut expected = [6500.0, 9000.0];
        for (width, folders) in [
            (1440.0, false),
            (640.0, true),
            (640.0, false),
            (1440.0, false),
            (640.0, true),
        ] {
            app.organize_editor.as_mut().unwrap().compact_folders = folders;
            for _ in 0..3 {
                frame(&mut app, width, vec![]);
            }
            let editor = app.organize_editor.as_ref().unwrap();
            for (actual, wanted) in [editor.photo_scroll, editor.folder_scroll]
                .into_iter()
                .zip(expected)
            {
                assert!(
                    (actual - wanted).abs() < 1.0,
                    "width {width}, folders {folders}: {actual} != {wanted}"
                );
            }
            assert!(
                editor.selected.contains(&MediaId(200))
                    && editor.selected_folders.contains("folder-0400")
            );
            assert_eq!(editor.folder, "folder-0400");
            let key = if folders {
                "test_organize_folder_scroll"
            } else {
                "test_organize_photo_scroll"
            };
            let viewport = ctx
                .data(|d| d.get_temp::<(egui::Rect, Vec2, f32, f32)>(egui::Id::new(key)))
                .unwrap()
                .0;
            frame(
                &mut app,
                width,
                vec![
                    egui::Event::PointerMoved(viewport.center()),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, -100.0),
                        modifiers: Default::default(),
                    },
                ],
            );
            for _ in 0..20 {
                frame(&mut app, width, vec![]);
            }
            let editor = app.organize_editor.as_ref().unwrap();
            let actual = if folders {
                editor.folder_scroll
            } else {
                editor.photo_scroll
            };
            assert!(
                actual > expected[usize::from(folders)] + 20.0,
                "Scrolling must still work after a layout change"
            );
            expected[usize::from(folders)] = actual;
        }
        // A new editor with no saved bookmark must not inherit the previous
        // editor's egui state, even when it opens the same library and layout.
        app.organize_editor = None;
        app.organize_view = None;
        seed(&mut app);
        for _ in 0..3 {
            frame(&mut app, 1440.0, vec![]);
        }
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!([editor.photo_scroll, editor.folder_scroll], [0.0, 0.0]);
        assert!(
            app.requested.is_empty() && app.metadata_run.is_none() && app.analysis_run.is_none()
        );
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }

    #[test]
    fn virtual_editor_rows_keep_exact_pitch_and_last_entries_visible_without_image_work() {
        for width in [360.0, 800.0] {
            let mut app = app();
            app.open_organize_editor();
            let mut editor = app.organize_editor.take().unwrap();
            let label = |i| format!("folder-{i:04}-{}", "long-name".repeat(25));
            editor.rows = (0..1000)
                .map(|i| PhotoRow {
                    id: MediaId(i),
                    source: format!("/missing/photo-{i:04}.jpg").into(),
                    after: label(i),
                })
                .collect();
            editor.draft = Draft::new((0..1000).map(|i| (MediaId(i), label(i))).collect());
            editor.refresh();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.enable_accesskit();
            for folders in [false, true] {
                for end in [false, true] {
                    editor.restore_scroll[usize::from(folders)] =
                        Some(if end { 1_000_000.0 } else { 0.0 });
                    for _ in 0..3 {
                        let _ = ctx.run(
                            egui::RawInput {
                                screen_rect: Some(egui::Rect::from_min_size(
                                    egui::Pos2::ZERO,
                                    egui::vec2(width, 650.0),
                                )),
                                ..Default::default()
                            },
                            |ctx| {
                                egui::CentralPanel::default().show(ctx, |ui| {
                                    if folders {
                                        editor.structure(ui, 380.0, &HashMap::new());
                                    } else {
                                        editor.photos(ui, 380.0, &HashMap::new());
                                    }
                                });
                            },
                        );
                    }
                    let key = if folders {
                        "test_organize_folder_scroll"
                    } else {
                        "test_organize_photo_scroll"
                    };
                    let (viewport, content, offset, height) = ctx
                        .data(|d| d.get_temp::<(egui::Rect, Vec2, f32, f32)>(egui::Id::new(key)))
                        .unwrap();
                    let count = if folders {
                        editor.tree.nodes.len()
                    } else {
                        editor.rows.len()
                    };
                    assert!(
                        (content.y - (count as f32 * (height + 10.0) - 10.0)).abs()
                            <= height + 10.0,
                        "{key}: content {content:?} height {height}"
                    );
                    assert!(
                        viewport.width() >= width - 30.0,
                        "Scroll target must cover the available width"
                    );
                    let indices = if end { [998, 999] } else { [0, 1] };
                    let rects: Vec<_> = indices
                        .into_iter()
                        .map(|i| {
                            let key = if folders {
                                egui::Id::new(("test_folder_rect", label(i)))
                            } else {
                                egui::Id::new(("test_file_rect", i))
                            };
                            ctx.data(|d| d.get_temp::<egui::Rect>(key)).unwrap()
                        })
                        .collect();
                    assert!(
                        (rects[1].top() - rects[0].top() - height - 10.0).abs() < 0.6,
                        "{key}: {rects:?}, height {height}, offset {offset}"
                    );
                    if end {
                        assert!(
                            rects[1].intersects(viewport),
                            "Last entry is reachable: {key}, rects {rects:?}, viewport {viewport:?}, offset {offset}"
                        );
                    }
                    assert!(
                        rects.iter().all(|r| r.right() <= viewport.right() + 1.0),
                        "Long labels must truncate"
                    );
                }
            }
            assert!(
                app.requested.is_empty()
                    && app.metadata_run.is_none()
                    && app.analysis_run.is_none()
            );
            assert_eq!(app.stats.decodes, 0);
            assert_eq!(app.stats.metadata_reads, 0);
        }
    }
    fn app() -> PhotoSortApp {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        // Nonexistent paths prove folder editing needs no rescan or image load.
        let media: Vec<_> = [
            "/missing/trip/a.jpg",
            "/missing/trip/a.ARW",
            "/missing/trip/b.jpg",
        ]
        .into_iter()
        .enumerate()
        .map(|(i, path)| Media {
            id: MediaId(i as u64),
            path: path.into(),
            kind: MediaKind::Photo,
            fingerprint: Default::default(),
        })
        .collect();
        app.catalog = Some(Arc::new(Catalog {
            root: "/missing".into(),
            media: media.clone(),
            ..Default::default()
        }));
        app.metadata = media.iter().map(|m| (m.id, Metadata::default())).collect();
        app.media = media;
        app
    }
    #[test]
    fn path_menus_keep_originals_separate_from_renamed_companions_and_new_folders() {
        let mut app = app();
        let root = PathBuf::from("/missing");
        let document = root.join("trip/notes and café.txt");
        let catalog = Arc::get_mut(app.catalog.as_mut().unwrap()).unwrap();
        catalog.inventory.push(document.clone());
        catalog.directories = vec![root.clone(), root.join("trip")];
        app.output = "/output".into();
        app.companions = true;
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.apply(
            Command::RenameFile(MediaId(0), "Keeper".into()),
            &app.media,
            true,
        );
        editor.folder = "trip".into();
        editor.name = "Selected".into();
        editor.apply(Command::Rename, &app.media, true);
        for (id, original, destination) in [
            (
                MediaId(0),
                root.join("trip/a.jpg"),
                PathBuf::from("/output/Selected/Keeper.jpg"),
            ),
            (
                MediaId(1),
                root.join("trip/a.ARW"),
                PathBuf::from("/output/Selected/Keeper.ARW"),
            ),
            (
                MediaId::for_path(&document),
                document.clone(),
                PathBuf::from("/output/Selected/notes and café.txt"),
            ),
        ] {
            let row = editor.rows.iter().find(|row| row.id == id).unwrap();
            let paths = editor.file_locations(row);
            assert_eq!(paths.source.as_deref(), Some(original.as_path()));
            assert_eq!(paths.relative.as_deref(), original.strip_prefix(&root).ok());
            assert_eq!(paths.proposed.as_deref(), Some(destination.as_path()));
        }
        let renamed = editor.folder_locations("Selected");
        assert_eq!(renamed.source, Some(root.join("trip")));
        assert_eq!(renamed.relative, Some(PathBuf::from("Selected")));
        assert_eq!(renamed.proposed, Some(PathBuf::from("/output/Selected")));
        editor.draft.create_folder("", "New folder").unwrap();
        let created = editor.folder_locations("New folder");
        assert!(created.source.is_none());
        assert_eq!(created.relative, Some(PathBuf::from("New folder")));
        assert_eq!(created.proposed, Some(PathBuf::from("/output/New folder")));
        assert_eq!(editor.folder_locations("").source, Some(root));
        assert_eq!(
            editor.folder_locations("").relative,
            Some(PathBuf::from("."))
        );
        assert_eq!(app.stats.decodes, 0);
        assert!(app.requested.is_empty());
        assert!(app.decisions.is_empty());
        app.runtime.shutdown();
    }

    #[test]
    fn editor_context_menu_copies_clicked_source_at_compact_and_wide_sizes() {
        for size in [egui::vec2(600.0, 650.0), egui::vec2(1200.0, 850.0)] {
            let mut app = app();
            app.active = Some(MediaId(0));
            app.output = "/output".into();
            app.open_organize_editor();
            app.organize_editor
                .as_mut()
                .unwrap()
                .selected
                .insert(MediaId(0));
            let ctx = egui::Context::default();
            theme_install(&ctx);
            ctx.enable_accesskit();
            let frame = |app: &mut PhotoSortApp, events| {
                ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.organize_dialog(ctx),
                )
            };
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            let pos = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 1_u64))))
                .unwrap()
                .center();
            let button = |pos, button, pressed| egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: Default::default(),
            };
            frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    button(pos, egui::PointerButton::Secondary, true),
                    button(pos, egui::PointerButton::Secondary, false),
                ],
            );
            let output = frame(&mut app, vec![]);
            let tree = output.platform_output.accesskit_update.unwrap();
            let node = tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Copy full path"))
                .expect("Organize row context menu");
            let bounds = node.1.bounds().unwrap();
            assert!(
                bounds.x0 >= 0.0
                    && bounds.y0 >= 0.0
                    && bounds.x1 <= f64::from(size.x)
                    && bounds.y1 <= f64::from(size.y)
            );
            let pos = egui::pos2(
                ((bounds.x0 + bounds.x1) / 2.0) as f32,
                ((bounds.y0 + bounds.y1) / 2.0) as f32,
            );
            let output = frame(
                &mut app,
                vec![
                    egui::Event::PointerMoved(pos),
                    button(pos, egui::PointerButton::Primary, true),
                    button(pos, egui::PointerButton::Primary, false),
                ],
            );
            assert!(output.platform_output.commands.iter().any(|c| matches!(c, egui::OutputCommand::CopyText(text) if text == "/missing/trip/a.ARW")));
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected,
                HashSet::from([MediaId(0)])
            );
            assert_eq!(app.active, Some(MediaId(0)));
            assert!(app.decisions.is_empty());
            assert!(app.requested.is_empty());
            assert_eq!(app.stats.decodes, 0);
            app.runtime.shutdown();
        }
    }

    #[test]
    fn file_rename_drafts_cancel_accept_restore_and_preserve_companions_and_decisions() {
        let mut app = app();
        app.companions = true;
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.apply(
            Command::RenameFile(MediaId(1), "Keeper".into()),
            &app.media,
            true,
        );
        assert_eq!(editor.names.len(), 2);
        assert_eq!(editor.names[&MediaId(0)], "Keeper");
        assert!(app.file_names.is_empty());
        app.finish_organize_edit(editor, 1);
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        assert!(editor.names.is_empty());
        editor.start_rename(MediaId(1));
        editor.renaming.as_mut().unwrap().1 = "Kept".into();
        app.finish_organize_edit(editor, 2);
        assert_eq!(app.file_names.len(), 2);
        assert_eq!(app.file_names[&MediaId(0)], "Kept");
        assert_eq!(app.file_names[&MediaId(1)], "Kept");
        assert!(app.organize_editor.is_none());
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.start_rename(MediaId(0));
        editor.renaming.as_mut().unwrap().1 = "bad/name".into();
        app.finish_organize_edit(editor, 2);
        let mut editor = app.organize_editor.take().unwrap();
        assert!(editor.error.is_some());
        assert_eq!(app.file_names[&MediaId(0)], "Kept");
        editor.apply(
            Command::RenameFile(MediaId(0), String::new()),
            &app.media,
            true,
        );
        app.finish_organize_edit(editor, 2);
        assert!(app.file_names.is_empty());
        assert!(app.decisions.is_empty() && app.requested.is_empty());
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
    #[test]
    fn enter_commits_a_focused_file_name_after_the_widget_releases_focus() {
        let mut app = app();
        app.companions = true;
        app.open_organize_editor();
        app.organize_editor
            .as_mut()
            .unwrap()
            .start_rename(MediaId(0));
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let mut frame = |events| {
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(800., 650.),
                    )),
                    events,
                    ..Default::default()
                },
                |ctx| app.organize_dialog(ctx),
            );
        };
        for _ in 0..3 {
            frame(vec![]);
        }
        frame(vec![egui::Event::Text("Keyboard name".into())]);
        frame(vec![egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }]);
        let editor = app.organize_editor.as_ref().unwrap();
        assert!(editor.renaming.is_none());
        assert_eq!(editor.names[&MediaId(0)], "Keyboard name");
        assert_eq!(editor.names[&MediaId(1)], "Keyboard name");
        assert!(
            app.file_names.is_empty(),
            "Enter stages inside the disposable editor draft"
        );
        assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
    }
    #[test]
    fn file_keyboard_ranges_marks_rename_and_resume_share_cached_editor_state() {
        for width in [640., 1200.] {
            let mut app = app();
            Arc::get_mut(app.catalog.as_mut().unwrap())
                .unwrap()
                .inventory
                .push("/missing/trip/notes.txt".into());
            app.open_organize_editor();
            let editor = app.organize_editor.as_ref().unwrap();
            let ids: Vec<_> = editor.rows.iter().map(|row| row.id).collect();
            assert_eq!(ids.len(), 4);
            let rows = editor.rows.as_ptr();
            let labels = editor.draft.labels().clone();
            app.decisions.insert(
                ids[1],
                Decision {
                    rejected: true,
                    ..Default::default()
                },
            );
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let frame = |app: &mut PhotoSortApp, events| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.organize_dialog(ctx),
                );
            };
            let key = |key, shift, command, repeat| egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat,
                modifiers: egui::Modifiers {
                    shift,
                    command,
                    ..Default::default()
                },
            };
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", ids[0].0))))
                .unwrap();
            frame(&mut app, tree_controls::tests::click(rect.center()));
            frame(
                &mut app,
                vec![key(egui::Key::ArrowDown, false, false, false)],
            );
            let editor = app.organize_editor.as_ref().unwrap();
            assert_eq!(editor.file_cursor, Some(2), "plain Down skips a Trash mark");
            assert_eq!(
                editor.selected,
                HashSet::from([ids[0]]),
                "focus movement retains checked files"
            );
            frame(&mut app, vec![key(egui::Key::ArrowUp, true, false, false)]);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected,
                ids[1..=2].iter().copied().collect()
            );
            frame(&mut app, vec![key(egui::Key::ArrowUp, true, false, false)]);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected,
                ids[..=2].iter().copied().collect()
            );
            frame(
                &mut app,
                vec![key(egui::Key::ArrowDown, true, false, false)],
            );
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected,
                ids[1..=2].iter().copied().collect()
            );
            frame(
                &mut app,
                vec![
                    key(egui::Key::Space, false, false, false),
                    key(egui::Key::Space, false, false, true),
                ],
            );
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected,
                HashSet::from([ids[2]])
            );
            frame(&mut app, vec![key(egui::Key::A, false, true, false)]);
            assert_eq!(app.organize_editor.as_ref().unwrap().selected.len(), 4);
            frame(&mut app, vec![key(egui::Key::A, true, true, false)]);
            assert!(app.organize_editor.as_ref().unwrap().selected.is_empty());
            frame(
                &mut app,
                vec![
                    key(egui::Key::Home, false, false, false),
                    key(egui::Key::End, true, false, false),
                ],
            );
            assert_eq!(app.organize_editor.as_ref().unwrap().selected.len(), 4);
            let editor = app.organize_editor.as_ref().unwrap();
            assert_eq!(editor.rows.as_ptr(), rows);
            assert_eq!(editor.draft.labels(), &labels);
            let bookmark = editor.bookmark();
            assert_eq!(bookmark.focused, Some(ids[3]));
            assert_eq!(bookmark.anchor, Some(ids[0]));
            let editor = app.organize_editor.take().unwrap();
            app.finish_organize_edit(editor, 1);
            app.open_organize_editor();
            let editor = app.organize_editor.as_ref().unwrap();
            assert_eq!(editor.bookmark(), bookmark);
            for _ in 0..2 {
                frame(&mut app, vec![]);
            }
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", ids[3].0))))
                .unwrap();
            frame(&mut app, tree_controls::tests::click(rect.center()));
            frame(&mut app, vec![key(egui::Key::F2, false, false, false)]);
            assert_eq!(
                app.organize_editor
                    .as_ref()
                    .unwrap()
                    .renaming
                    .as_ref()
                    .unwrap()
                    .0,
                ids[3]
            );
            frame(&mut app, vec![]);
            frame(&mut app, vec![key(egui::Key::Home, false, false, false)]);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().file_cursor,
                Some(3),
                "name field owns Home"
            );
            assert!(
                app.requested.is_empty()
                    && app.analysis_run.is_none()
                    && app.metadata_run.is_none()
                    && !app.scanning
            );
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        }
    }
    #[test]
    fn file_keyboard_large_list_reveals_end_home_and_preserves_pointer_toggle_behavior() {
        for width in [640., 1200.] {
            let mut app = app();
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.rows = (0..100_000)
                .map(|i| PhotoRow {
                    id: MediaId(i),
                    source: format!("/missing/{i:06}.txt").into(),
                    after: String::new(),
                })
                .collect();
            let rows = editor.rows.as_ptr();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let frame = |app: &mut PhotoSortApp, events| {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(width, 900.),
                        )),
                        events,
                        ..Default::default()
                    },
                    |ctx| app.organize_dialog(ctx),
                );
            };
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            let rect = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 0u64))))
                .unwrap();
            frame(&mut app, tree_controls::tests::click(rect.center()));
            frame(&mut app, vec![tree_controls::tests::key(egui::Key::End)]);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().file_cursor,
                Some(99_999)
            );
            assert!(app.organize_editor.as_ref().unwrap().photo_scroll > 1_000_000.);
            frame(&mut app, vec![tree_controls::tests::key(egui::Key::Space)]);
            assert_eq!(
                app.organize_editor.as_ref().unwrap().selected,
                HashSet::from([MediaId(0), MediaId(99_999)])
            );
            frame(&mut app, vec![tree_controls::tests::key(egui::Key::PageUp)]);
            assert!(app.organize_editor.as_ref().unwrap().file_cursor.unwrap() < 99_999);
            frame(&mut app, vec![tree_controls::tests::key(egui::Key::Home)]);
            assert_eq!(app.organize_editor.as_ref().unwrap().photo_scroll, 0.);
            assert_eq!(app.organize_editor.as_ref().unwrap().rows.as_ptr(), rows);
            assert_eq!((app.stats.decodes, app.stats.metadata_reads), (0, 0));
        }
    }
    #[test]
    fn file_name_entry_and_errors_fit_compact_and_wide_editor_windows() {
        for size in [
            egui::vec2(640., 600.),
            egui::vec2(800., 650.),
            egui::vec2(1280., 800.),
        ] {
            let mut app = app();
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.start_rename(MediaId(0));
            editor.apply(
                Command::RenameFile(MediaId(0), "bad/name".into()),
                &app.media,
                app.companions,
            );
            let ctx = egui::Context::default();
            theme_install(&ctx);
            for _ in 0..3 {
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    },
                    |ctx| app.organize_dialog(ctx),
                );
            }
            let rect = ctx
                .memory(|m| m.area_rect(egui::Id::new("organize_editor")))
                .unwrap();
            assert!(
                rect.width() <= size.x && rect.height() <= size.y,
                "{size:?}: {rect:?}"
            );
            assert!(app.organize_editor.as_ref().unwrap().renaming.is_some());
            assert_eq!(app.stats.decodes, 0);
        }
    }
    #[test]
    fn organize_trash_supporting_selection_and_marks_survive_worker_reload_and_replacement() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.jpg"), b"photo").unwrap();
        std::fs::write(dir.path().join("notes.txt"), b"document").unwrap();
        let root = dir.path().canonicalize().unwrap();
        fn settle(app: &mut PhotoSortApp) {
            let end = Instant::now() + Duration::from_secs(10);
            while app.scanning
                || app.pending_bookmark.is_some()
                || app.filter_dirty
                || app.browse_run.is_some()
            {
                app.poll(&egui::Context::default());
                app.refilter();
                app.restore_bookmark();
                assert!(Instant::now() < end, "{:?}", app.error);
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        app.scan_folder(root.clone());
        settle(&mut app);
        let id = MediaId::for_path(&root.join("notes.txt"));
        app.stage_manual([id], crate::core::decisions::Change::Trash(true));
        assert!(app.decisions[&id].rejected);
        app.save_session();
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.selected.insert(id);
        app.organize_view = Some(editor.bookmark());
        app.view_data_ready.insert(Step::Organize);
        app.checkpoint_bookmarks();
        app.runtime.shutdown();
        drop(app);
        let mut reopened =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
        reopened.scan_folder(root.clone());
        settle(&mut reopened);
        reopened.open_organize_editor();
        assert!(reopened.decisions[&id].rejected);
        assert_eq!(reopened.stats.metadata_reads, 0);
        assert!(
            reopened
                .organize_editor
                .as_ref()
                .unwrap()
                .selected
                .contains(&id)
        );
        assert_eq!(reopened.stats.decodes, 0);
        reopened.organize_editor = None;
        std::fs::remove_file(root.join("notes.txt")).unwrap();
        reopened.scan_folder(root);
        settle(&mut reopened);
        assert!(!reopened.decisions.contains_key(&id));
        reopened.open_organize_editor();
        assert!(
            !reopened
                .organize_editor
                .as_ref()
                .unwrap()
                .selected
                .contains(&id)
        );
    }

    #[test]
    fn large_editor_selection_restores_from_cached_rows() {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.rows = (0..100_000)
            .map(|i| PhotoRow {
                id: MediaId(i),
                source: PathBuf::new(),
                after: String::new(),
            })
            .collect();
        editor.draft = Draft::new(
            (0..10_000)
                .map(|i| (MediaId(i), format!("folder-{i}")))
                .collect(),
        );
        editor.refresh();
        let view = crate::core::views::EditorBookmark {
            selected: (0..100_001).map(MediaId).collect(),
            folders: (0..10_001).map(|i| format!("folder-{i}")).collect(),
            ..Default::default()
        };
        editor.restore_bookmark(&view);
        assert_eq!(editor.selected.len(), 100_000);
        assert_eq!(editor.selected_folders.len(), 10_000);
        assert_eq!(app.stats.decodes, 0);
    }

    #[test]
    fn editor_restores_presentation_without_restoring_draft_edits() {
        let mut app = app();
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.selected.insert(MediaId(1));
        editor.folder = "trip".into();
        editor.selected_folders.insert("trip".into());
        editor.photo_scroll = 120.0;
        editor.folder_scroll = 40.0;
        editor.compact_folders = true;
        let before = editor.draft.labels().clone();
        editor.name = "Changed".into();
        editor.apply(Command::Rename, &app.media, app.companions);
        assert_ne!(editor.draft.labels(), &before);
        app.organize_view = Some(editor.bookmark());
        // Closing without storing the draft is Cancel; only presentation survives.
        app.open_organize_editor();
        let reopened = app.organize_editor.as_ref().unwrap();
        assert_eq!(reopened.draft.labels(), &before);
        assert!(reopened.selected.contains(&MediaId(1)));
        assert_eq!(reopened.restore_scroll, [Some(120.0), Some(40.0)]);
        assert!(reopened.compact_folders);
        assert!(!reopened.draft.folders().contains("Changed"));
        assert!(app.decisions.is_empty() && app.requested.is_empty());
    }
    #[test]
    fn editor_moves_companions_together_without_touching_review_decisions_or_images() {
        let mut app = app();
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.folder = editor.draft.create_folder("", "keepers").unwrap();
        editor.selected.insert(MediaId(0));
        editor.apply(Command::Move, &app.media, true);
        assert_eq!(editor.draft.labels()[&MediaId(0)], "keepers");
        assert_eq!(editor.draft.labels()[&MediaId(1)], "keepers");
        assert_eq!(editor.draft.labels()[&MediaId(2)], "trip");
        assert!(app.decisions.is_empty());
        assert_eq!(app.stats.decodes, 0);
        assert!(!app.scanning && app.analysis_run.is_none());
    }
    #[test]
    fn editor_isolated_draft_can_cancel_and_modes_retain_separate_edits() {
        let mut app = app();
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.draft.rename_folder("trip", "holiday").unwrap();
        app.organize_drafts.insert(GroupMode::Current, editor.draft);
        app.open_organize_editor();
        app.organize_editor
            .as_mut()
            .unwrap()
            .draft
            .rename_folder("holiday", "discarded")
            .unwrap();
        app.organize_editor = None; // Cancel the working copy.
        app.group_mode = GroupMode::Camera;
        app.open_organize_editor();
        assert!(
            app.organize_editor
                .as_ref()
                .unwrap()
                .draft
                .folders()
                .contains("Unknown camera")
        );
        app.organize_editor = None;
        app.group_mode = GroupMode::Current;
        app.open_organize_editor();
        let editor = app.organize_editor.as_ref().unwrap();
        assert!(editor.draft.folders().contains("holiday"));
        assert!(!editor.draft.folders().contains("discarded"));
    }
    #[test]
    fn pending_analysis_blocks_editor_and_modal_blocks_workflow_keyboard() {
        let mut app = app();
        app.analysis_run = Some(8);
        app.open_organize_editor();
        assert!(app.organize_editor.is_none());
        app.analysis_run = None;
        app.open_organize_editor();
        let ctx = egui::Context::default();
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::I,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        });
        let _ = ctx.run(input, |ctx| app.keyboard(ctx));
        assert!(
            app.inspector,
            "Editor keystrokes must not toggle the underlying inspector"
        );
        app.request_departure(Destination::Step(Step::Cull));
        assert_eq!(app.step, Step::Organize);
    }
    #[test]
    fn editor_and_footer_fit_wide_and_compact_windows() {
        for size in [
            egui::vec2(640.0, 600.0),
            egui::vec2(800.0, 640.0),
            egui::vec2(1440.0, 960.0),
        ] {
            for folders in [false, true] {
                let mut app = app();
                app.open_organize_editor();
                let editor = app.organize_editor.as_mut().unwrap();
                editor.compact_folders = folders;
                editor.error = Some("A folder cannot contain itself".into());
                for n in 0..100 {
                    editor
                        .draft
                        .create_folder("", &format!("folder-{n}"))
                        .unwrap();
                }
                editor.refresh();
                // Enough rows to exercise scroll limits, not just empty content.
                for n in 3..100 {
                    editor.rows.push(PhotoRow {
                        id: MediaId(n),
                        source: PathBuf::from(format!("/missing/trip/day1/IMG_{n:04}.jpg")),

                        after: "trip/day1".into(),
                    });
                }
                let ctx = egui::Context::default();
                theme_install(&ctx);
                for _ in 0..3 {
                    let input = egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                        ..Default::default()
                    };
                    let _ = ctx.run(input, |ctx| app.organize_dialog(ctx));
                }
                let rect = ctx
                    .memory(|m| m.area_rect(egui::Id::new("organize_editor")))
                    .unwrap();
                assert!(
                    rect.height() <= size.y && rect.width() <= size.x,
                    "{size:?} folders={folders}: {rect:?}"
                );
            }
        }
    }
    #[test]
    fn editor_includes_documents_from_shared_inventory_in_subtree_edits() {
        let mut app = app();
        Arc::get_mut(app.catalog.as_mut().unwrap())
            .unwrap()
            .inventory
            .push("/missing/trip/notes.json".into());
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        assert_eq!(editor.rows.len(), 4);
        editor.draft.rename_folder("trip", "holiday").unwrap();
        editor.refresh();
        assert!(editor.rows.iter().all(|row| row.after == "holiday"));
        assert!(editor.rows.iter().any(|row| {
            row.source
                .file_name()
                .is_some_and(|name| name == "notes.json")
        }));
        assert_eq!(app.stats.decodes, 0);
        assert!(app.analysis_run.is_none());
    }
    #[test]
    fn hidden_inventory_reaches_editor_and_shared_planner_without_decoding() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join(".before/.empty")).unwrap();
        for name in [".photo.jpg", ".photo.xmp", ".notes.json"] {
            std::fs::write(temp.path().join(".before").join(name), b"unchanged").unwrap();
        }
        let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.output = catalog.root.display().to_string();
        app.media = catalog.media.clone();
        app.catalog = Some(Arc::new(catalog));
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        assert_eq!(editor.rows.len(), 2);
        assert!(editor.rows.iter().any(|row| {
            row.source
                .file_name()
                .is_some_and(|name| name == ".photo.jpg")
        }));
        assert!(editor.rows.iter().any(|row| {
            row.source
                .file_name()
                .is_some_and(|name| name == ".notes.json")
        }));
        assert!(editor.rows.iter().all(|row| {
            row.source
                .parent()
                .is_some_and(|parent| parent == editor.catalog.root.join(".before"))
        }));
        assert!(editor.draft.folders().contains(".before/.empty"));
        editor.draft.rename_folder(".before", ".after").unwrap();
        editor.refresh();
        assert!(editor.rows.iter().all(|row| row.after == ".after"));
        app.organize_drafts.insert(GroupMode::Current, editor.draft);
        app.organize();
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.busy && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.plan_open && app.error.is_none(), "{:?}", app.error);
        let plan = app.plan.as_ref().unwrap();
        use crate::core::operations::Action;
        assert_eq!(
            plan.actions
                .iter()
                .filter(|action| matches!(action, Action::Move { .. }))
                .count(),
            2
        );
        assert!(plan.actions.iter().any(|action| matches!(action, Action::CreateDirectory { path } if path.ends_with(".after/.empty"))));
        for name in [".photo.jpg", ".photo.xmp", ".notes.json"] {
            assert_eq!(
                std::fs::read(temp.path().join(".before").join(name)).unwrap(),
                b"unchanged"
            );
        }
        assert!(!temp.path().join(".after").exists());
        assert_eq!(app.stats.decodes, 0);
        assert!(app.analysis_run.is_none());
    }
    #[test]
    fn empty_folder_editor_intent_reaches_shared_io_worker_without_mutation() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("existing/empty")).unwrap();
        let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.output = catalog.root.display().to_string();
        app.catalog = Some(Arc::new(catalog));
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        assert!(editor.draft.folders().contains("existing/empty"));
        editor.draft.create_folder("", "new-empty").unwrap();
        app.organize_drafts.insert(GroupMode::Current, editor.draft);
        app.organize();
        let ctx = egui::Context::default();
        let deadline = Instant::now() + Duration::from_secs(5);
        while app.busy && Instant::now() < deadline {
            app.poll(&ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(app.plan_open && app.error.is_none());
        let plan = app.plan.as_ref().unwrap();
        assert_eq!(plan.actions.len(), 1);
        assert!(
            matches!(&plan.actions[0], crate::core::operations::Action::CreateDirectory {path} if path.ends_with("new-empty"))
        );
        assert!(!temp.path().join("new-empty").exists());
        assert_eq!(app.stats.decodes, 0);
    }
    #[test]
    fn drag_sources_preserve_multi_selection_and_replace_unselected_sources() {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.selected.extend([MediaId(0), MediaId(1)]);
        assert_eq!(editor.file_drag(MediaId(0)).selection.files.len(), 2);
        assert_eq!(
            editor.file_drag(MediaId(2)).selection.files,
            HashSet::from([MediaId(2)])
        );
        editor.draft.create_folder("", "target").unwrap();
        editor
            .selected_folders
            .extend(["trip".into(), "target".into()]);
        assert_eq!(editor.folder_drag("trip").selection.folders.len(), 2);
        editor.draft.create_folder("", "third").unwrap();
        assert_eq!(editor.folder_drag("third").selection.folders.len(), 1);
        assert_eq!(app.stats.decodes, 0);
    }
    #[test]
    fn drop_and_button_share_companion_expansion_and_failed_drop_is_unchanged() {
        let mut app = app();
        app.open_organize_editor();
        let mut button = app.organize_editor.take().unwrap();
        app.open_organize_editor();
        let mut dragged = app.organize_editor.take().unwrap();
        for editor in [&mut button, &mut dragged] {
            editor.draft.create_folder("", "target").unwrap();
            editor.selected.insert(MediaId(0));
        }
        button.folder = "target".into();
        button.apply(Command::Move, &app.media, true);
        let payload = dragged.file_drag(MediaId(0));
        dragged.apply(
            Command::Drop(payload.selection, "target".into()),
            &app.media,
            true,
        );
        assert_eq!(dragged.draft, button.draft);
        assert_eq!(dragged.draft.labels()[&MediaId(1)], "target");
        assert_eq!(dragged.rows[1].after, "target");
        let before = dragged.draft.clone();
        let payload = dragged.folder_drag("trip");
        dragged.apply(
            Command::Drop(payload.selection, "trip".into()),
            &app.media,
            true,
        );
        assert!(dragged.error.is_some());
        assert_eq!(dragged.draft, before);
        assert!(app.decisions.is_empty());
        assert_eq!(app.stats.decodes, 0);
    }
    #[test]
    fn drop_target_scopes_payload_to_editor_and_escape_cancels_drag() {
        let mut app = app();
        app.open_organize_editor();
        let mut editor = app.organize_editor.take().unwrap();
        editor.draft.create_folder("", "target").unwrap();
        let ctx = egui::Context::default();
        let mut rect = egui::Rect::NOTHING;
        let mut dropped = None;
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                rect = ui.button("target").rect;
            });
        });
        let position = rect.center();
        let payload = editor.file_drag(MediaId(0));
        for own in [false, true] {
            let payload = FolderDrag {
                owner: if own {
                    payload.owner.clone()
                } else {
                    Arc::new(())
                },
                selection: payload.selection.clone(),
            };
            egui::DragAndDrop::set_payload(&ctx, payload);
            let input = egui::RawInput {
                events: vec![
                    egui::Event::PointerMoved(position),
                    egui::Event::PointerButton {
                        pos: position,
                        button: egui::PointerButton::Primary,
                        pressed: false,
                        modifiers: Default::default(),
                    },
                ],
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let response = ui.button("target");
                    dropped = editor.drop_target(ui, &response, "target");
                });
            });
            assert_eq!(dropped.is_some(), own);
        }
        egui::DragAndDrop::set_payload(&ctx, payload);
        let mut input = egui::RawInput::default();
        input.events.push(egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        });
        let _ = ctx.run(input, |_| {});
        assert!(!egui::DragAndDrop::has_any_payload(&ctx));
        assert_eq!(editor.draft.labels()[&MediaId(0)], "trip");
        for cancellation in 0..4 {
            let _ = ctx.run(
                egui::RawInput {
                    events: vec![
                        egui::Event::PointerMoved(position),
                        egui::Event::PointerButton {
                            pos: position,
                            button: egui::PointerButton::Primary,
                            pressed: true,
                            modifiers: Default::default(),
                        },
                    ],
                    ..Default::default()
                },
                |ctx| editor.begin_drag_frame(ctx),
            );
            assert!(editor.drag.start().is_some());
            let mut input = egui::RawInput {
                focused: cancellation != 1,
                ..Default::default()
            };
            if cancellation == 0 {
                input.events.push(egui::Event::Key {
                    key: egui::Key::Escape,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                });
            }
            if cancellation == 2 {
                input.events.push(egui::Event::PointerGone);
            }
            egui::DragAndDrop::set_payload(&ctx, editor.file_drag(MediaId(0)));
            if cancellation == 3 {
                let _ = ctx.run(Default::default(), |_| {});
            }
            let _ = ctx.run(input, |ctx| editor.begin_drag_frame(ctx));
            assert!(editor.drag.start().is_none());
            assert!(editor.completed_drag.is_none());
            assert!(
                !egui::DragAndDrop::has_any_payload(&ctx),
                "cancellation {cancellation}"
            );
        }
    }
    fn hover_frame(
        ctx: &egui::Context,
        app: &mut PhotoSortApp,
        time: f64,
        width: f32,
        events: Vec<egui::Event>,
    ) {
        let _ = ctx.run(
            egui::RawInput {
                time: Some(time),
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 960.),
                )),
                events,
                ..Default::default()
            },
            |ctx| app.organize_dialog(ctx),
        );
    }
    fn hover_rect(ctx: &egui::Context, path: &str) -> egui::Pos2 {
        ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", path))))
            .expect("visible folder row")
            .center()
    }
    fn hover_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        }
    }
    #[test]
    fn drag_hover_expansion_reaches_nested_destinations_in_wide_and_compact_layouts() {
        for (width, folder_drag) in [640., 1440.]
            .into_iter()
            .flat_map(|w| [false, true].map(|folder| (w, folder)))
        {
            let mut app = app();
            app.companions = true;
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.draft.create_folder("", "target").unwrap();
            editor.draft.create_folder("target", "nested").unwrap();
            editor.draft.create_folder("target/nested", "deep").unwrap();
            editor.refresh();
            editor.collapsed.extend([
                Key::Folder("target".into()),
                Key::Folder("target/nested".into()),
            ]);
            editor.project_folders();
            editor.compact_folders = folder_drag;
            let original = editor.draft.clone();
            let nodes = editor.tree.nodes.as_ptr();
            let catalog = Arc::as_ptr(&editor.catalog);
            let ctx = egui::Context::default();
            theme_install(&ctx);
            for t in [0., 0.05, 0.1] {
                hover_frame(&ctx, &mut app, t, width, vec![]);
            }
            let source = if folder_drag {
                hover_rect(&ctx, "trip")
            } else {
                ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 0_u64))))
                    .unwrap()
                    .center()
            };
            hover_frame(
                &ctx,
                &mut app,
                0.2,
                width,
                vec![
                    egui::Event::PointerMoved(source),
                    hover_button(source, true),
                ],
            );
            hover_frame(
                &ctx,
                &mut app,
                0.3,
                width,
                vec![egui::Event::PointerMoved(source + egui::vec2(20., 0.))],
            );
            if width < 680. && !folder_drag {
                let tab = ctx
                    .data(|d| d.get_temp::<egui::Rect>(egui::Id::new("test_folders_tab")))
                    .unwrap()
                    .center();
                hover_frame(
                    &ctx,
                    &mut app,
                    0.35,
                    width,
                    vec![egui::Event::PointerMoved(tab)],
                );
                hover_frame(&ctx, &mut app, 0.36, width, vec![]);
            }
            let selected = app.organize_editor.as_ref().unwrap().selected.clone();
            let folders = app
                .organize_editor
                .as_ref()
                .unwrap()
                .selected_folders
                .clone();
            let target = hover_rect(&ctx, "target");
            hover_frame(
                &ctx,
                &mut app,
                0.4,
                width,
                vec![egui::Event::PointerMoved(target)],
            );
            hover_frame(&ctx, &mut app, 0.9, width, vec![]);
            assert!(
                app.organize_editor
                    .as_ref()
                    .unwrap()
                    .collapsed
                    .contains(&Key::Folder("target".into()))
            );
            hover_frame(&ctx, &mut app, 1.05, width, vec![]);
            hover_frame(&ctx, &mut app, 1.1, width, vec![]);
            assert!(
                !app.organize_editor
                    .as_ref()
                    .unwrap()
                    .collapsed
                    .contains(&Key::Folder("target".into()))
            );
            assert!(hover_rect(&ctx, "target").distance(target) <= 1.0);
            let nested = hover_rect(&ctx, "target/nested");
            hover_frame(
                &ctx,
                &mut app,
                1.2,
                width,
                vec![egui::Event::PointerMoved(nested)],
            );
            hover_frame(&ctx, &mut app, 1.9, width, vec![]);
            hover_frame(&ctx, &mut app, 1.95, width, vec![]);
            assert!(hover_rect(&ctx, "target").distance(target) <= 1.0);
            assert!(hover_rect(&ctx, "target/nested").distance(nested) <= 1.0);
            let editor = app.organize_editor.as_ref().unwrap();
            assert!(
                !editor
                    .collapsed
                    .contains(&Key::Folder("target/nested".into())),
                "width={width} folder_drag={folder_drag} initial={nested:?} current={:?} pointer={:?}",
                hover_rect(&ctx, "target/nested"),
                ctx.pointer_interact_pos()
            );
            assert_eq!(editor.draft, original);
            assert_eq!(editor.selected, selected);
            assert_eq!(editor.selected_folders, folders);
            assert_eq!(editor.tree.nodes.as_ptr(), nodes);
            assert_eq!(Arc::as_ptr(&editor.catalog), catalog);
            let destination = hover_rect(&ctx, "target/nested/deep");
            hover_frame(
                &ctx,
                &mut app,
                2.,
                width,
                vec![
                    egui::Event::PointerMoved(destination),
                    hover_button(destination, false),
                ],
            );
            let editor = app.organize_editor.as_ref().unwrap();
            assert!(editor.error.is_none(), "{:?}", editor.error);
            let expected = if folder_drag {
                "target/nested/deep/trip"
            } else {
                "target/nested/deep"
            };
            assert_eq!(editor.draft.labels()[&MediaId(0)], expected);
            assert_eq!(editor.draft.labels()[&MediaId(1)], expected);
            assert_eq!(app.stats.decodes, 0);
            assert_eq!(app.stats.metadata_reads, 0);
        }
    }
    #[test]
    fn drag_hover_expansion_ignores_foreign_and_cancelled_gestures() {
        for interruption in 0..4 {
            let mut app = app();
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.draft.create_folder("", "target").unwrap();
            editor.draft.create_folder("target", "nested").unwrap();
            editor.refresh();
            editor.collapsed.insert(Key::Folder("target".into()));
            editor.project_folders();
            let original = editor.draft.clone();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            for t in [0., 0.05, 0.1] {
                hover_frame(&ctx, &mut app, t, 1440., vec![]);
            }
            let source = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 0_u64))))
                .unwrap()
                .center();
            hover_frame(
                &ctx,
                &mut app,
                0.2,
                1440.,
                vec![
                    egui::Event::PointerMoved(source),
                    hover_button(source, true),
                ],
            );
            hover_frame(
                &ctx,
                &mut app,
                0.3,
                1440.,
                vec![egui::Event::PointerMoved(source + egui::vec2(20., 0.))],
            );
            let target = hover_rect(&ctx, "target");
            hover_frame(
                &ctx,
                &mut app,
                0.4,
                1440.,
                vec![egui::Event::PointerMoved(target)],
            );
            let events = match interruption {
                0 => vec![tree_controls::tests::key(egui::Key::Escape)],
                1 => vec![
                    egui::Event::WindowFocused(false),
                    egui::Event::WindowFocused(true),
                ],
                2 => vec![egui::Event::PointerGone],
                _ => {
                    let mut payload = app.organize_editor.as_mut().unwrap().file_drag(MediaId(0));
                    payload.owner = Arc::new(());
                    egui::DragAndDrop::set_payload(&ctx, payload);
                    vec![]
                }
            };
            hover_frame(&ctx, &mut app, 0.5, 1440., events);
            hover_frame(
                &ctx,
                &mut app,
                1.5,
                1440.,
                vec![egui::Event::PointerMoved(target)],
            );
            let editor = app.organize_editor.as_ref().unwrap();
            assert!(
                editor.collapsed.contains(&Key::Folder("target".into())),
                "interruption {interruption}"
            );
            assert_eq!(editor.draft, original);
            assert_eq!(app.stats.decodes, 0);
        }
    }
    #[test]
    fn pointer_drags_in_scrollable_modal_reach_shared_draft() {
        for (folder, timing) in [false, true]
            .into_iter()
            .flat_map(|folder| (0..4).map(move |timing| (folder, timing)))
        {
            let mut app = app();
            app.open_organize_editor();
            let editor = app.organize_editor.as_mut().unwrap();
            editor.draft.create_folder("", "target").unwrap();
            editor.refresh();
            let ctx = egui::Context::default();
            theme_install(&ctx);
            let frame = |app: &mut PhotoSortApp, events: Vec<egui::Event>| {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(1440.0, 960.0),
                    )),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| app.organize_dialog(ctx));
            };
            for _ in 0..3 {
                frame(&mut app, vec![]);
            }
            let source = if folder {
                egui::Id::new(("test_folder_rect", "trip"))
            } else {
                egui::Id::new(("test_file_rect", 0_u64))
            };
            let source = ctx
                .data(|d| d.get_temp::<egui::Rect>(source))
                .unwrap()
                .center();
            let target = ctx
                .data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_folder_rect", "target"))))
                .unwrap()
                .center();
            let button = |pos, pressed| egui::Event::PointerButton {
                pos,
                pressed,
                button: egui::PointerButton::Primary,
                modifiers: Default::default(),
            };
            if timing == 1 {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(source),
                        button(source, true),
                        egui::Event::PointerMoved(target),
                        button(target, false),
                    ],
                );
            } else if timing == 2 {
                frame(
                    &mut app,
                    vec![egui::Event::PointerMoved(source), button(source, true)],
                );
                frame(
                    &mut app,
                    vec![egui::Event::PointerMoved(target), button(target, false)],
                );
            } else if timing == 3 {
                frame(
                    &mut app,
                    vec![
                        egui::Event::PointerMoved(source),
                        button(source, true),
                        egui::Event::PointerMoved(target),
                    ],
                );
                frame(&mut app, vec![button(target, false)]);
            } else {
                frame(
                    &mut app,
                    vec![egui::Event::PointerMoved(source), button(source, true)],
                );
                frame(
                    &mut app,
                    vec![egui::Event::PointerMoved(source + egui::vec2(20.0, 0.0))],
                );
                assert!(
                    egui::DragAndDrop::has_payload_of_type::<FolderDrag>(&ctx),
                    "Drag did not start: folder={folder}, source={source:?}, current={:?}, scroll={:?}, gesture={:?}",
                    ctx.data(|d| d.get_temp::<egui::Rect>(egui::Id::new(("test_file_rect", 0_u64)))),
                    ctx.data(|d| d.get_temp::<(egui::Rect, Vec2, f32, f32)>(egui::Id::new("test_organize_photo_scroll"))),
                    app.organize_editor.as_ref().unwrap().drag.start()
                );
                frame(&mut app, vec![egui::Event::PointerMoved(target)]);
                frame(&mut app, vec![button(target, false)]);
            }
            let editor = app.organize_editor.as_ref().unwrap();
            assert!(editor.error.is_none(), "{:?}", editor.error);
            assert_eq!(
                editor.draft.labels()[&MediaId(0)],
                if folder { "target/trip" } else { "target" }
            );
            assert_eq!(
                editor.draft.labels()[&MediaId(1)],
                if folder { "target/trip" } else { "target" }
            );
            assert_eq!(app.stats.decodes, 0);
        }
    }
    #[test]
    fn compact_drag_can_switch_tabs_without_losing_its_selection() {
        let mut app = app();
        app.open_organize_editor();
        let editor = app.organize_editor.as_mut().unwrap();
        editor.draft.create_folder("", "target").unwrap();
        editor.refresh();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let frame = |app: &mut PhotoSortApp, events: Vec<egui::Event>| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 600.0),
                )),
                events,
                ..Default::default()
            };
            let _ = ctx.run(input, |ctx| app.organize_dialog(ctx));
        };
        for _ in 0..3 {
            frame(&mut app, vec![]);
        }
        let rect = |id| ctx.data(|d| d.get_temp::<egui::Rect>(id)).unwrap();
        let source = rect(egui::Id::new(("test_file_rect", 0_u64))).center();
        let tab = rect(egui::Id::new("test_folders_tab")).center();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            pressed,
            button: egui::PointerButton::Primary,
            modifiers: Default::default(),
        };
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(source), button(source, true)],
        );
        frame(
            &mut app,
            vec![egui::Event::PointerMoved(source + egui::vec2(20.0, 0.0))],
        );
        frame(&mut app, vec![egui::Event::PointerMoved(tab)]);
        assert!(app.organize_editor.as_ref().unwrap().compact_folders);
        frame(&mut app, vec![]);
        let target = rect(egui::Id::new(("test_folder_rect", "target"))).center();
        frame(&mut app, vec![egui::Event::PointerMoved(target)]);
        frame(&mut app, vec![button(target, false)]);
        let editor = app.organize_editor.as_ref().unwrap();
        assert_eq!(editor.draft.labels()[&MediaId(0)], "target");
        assert_eq!(editor.draft.labels()[&MediaId(1)], "target");
        assert_eq!(app.stats.decodes, 0);
    }
}
