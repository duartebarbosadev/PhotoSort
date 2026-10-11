use super::*;
use crate::core::{
    browsing::tree::{Key, Tree},
    operations,
    organize::{Draft, Selection},
};

// Temporary comparison presentation; the folder draft remains untouched.
pub(super) struct Preview {
    pub(super) ids: Vec<MediaId>,
    pub(super) active_before: Option<MediaId>,
    pub(super) view_before: InspectionView,
    pub(super) independent_before: HashMap<u64, InspectionView>,
    pub(super) sync_before: bool,
    focused_before: u64,
}
pub(super) struct Editor {
    preview: Option<Preview>,
    catalog: Arc<Catalog>,
    output: PathBuf,
    desktop_enabled: bool,
    desktop_pending: Option<crate::core::desktop::Request>,
    reset_request: Option<Selection>,
    mark_requests: Vec<MarkRequest>,
    preview_moves_requested: bool,
    folder_edit: Option<FolderEdit>,
    folder_name_focus: bool,
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
enum MarkRequest {
    Toggle(Vec<MediaId>),
    Clear,
}
#[derive(Clone, Copy)]
enum FolderEdit {
    Create,
    Rename,
}
#[derive(Clone, Copy)]
enum EditAction {
    Mark,
    ClearMarks,
    TrashFiles,
    Rename,
    NewFolder,
    PreviewMoves,
}
impl EditAction {
    fn from_key(key: egui::Key, m: egui::Modifiers) -> Option<Self> {
        if super::selection::clear_trash_shortcut(key, m) {
            return Some(Self::ClearMarks);
        }
        let command = m.command || m.ctrl;
        match key {
            egui::Key::D if m.is_none() => Some(Self::Mark),
            egui::Key::Delete | egui::Key::Backspace if m.is_none() => Some(Self::TrashFiles),
            egui::Key::F2 if m.is_none() => Some(Self::Rename),
            egui::Key::N if command && m.shift && !m.alt => Some(Self::NewFolder),
            egui::Key::Enter if !m.alt && (m.shift != command) => Some(Self::PreviewMoves),
            _ => None,
        }
    }
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
    fn file_selection(&self, id: MediaId) -> Selection {
        Selection {
            files: if self.selected.contains(&id) {
                self.selected.clone()
            } else {
                HashSet::from([id])
            },
            ..Default::default()
        }
    }
    fn folder_selection(&self, path: &str) -> Selection {
        Selection {
            folders: if self.selected_folders.contains(path) {
                self.selected_folders.clone()
            } else {
                std::collections::BTreeSet::from([path.to_owned()])
            },
            ..Default::default()
        }
    }
    fn queue_marks(&mut self, files: HashSet<MediaId>) {
        if files.is_empty() {
            return;
        }
        let mut files: Vec<_> = files.into_iter().collect();
        files.sort_by_key(|id| id.0);
        self.mark_requests.push(MarkRequest::Toggle(files));
    }
    fn queue_file_trash(&mut self, files: HashSet<MediaId>) {
        if files.is_empty() {
            return;
        }
        let mut files: Vec<_> = files.into_iter().collect();
        files.sort_by_key(|id| id.0);
        self.trash_now = Some(operations::TrashRequest {
            files,
            folders: Vec::new(),
        });
    }
    // Returns true when subsequent collection keys belong to a new editor/review context.
    fn edit_action(&mut self, action: EditAction, folders: bool) -> bool {
        match action {
            EditAction::ClearMarks => self.mark_requests.push(MarkRequest::Clear),
            EditAction::Mark | EditAction::TrashFiles => {
                let selection = if folders {
                    Some(self.folder_selection(&self.folder))
                } else {
                    self.file_cursor
                        .map(|i| self.file_selection(self.rows[i].id))
                };
                if let Some(selection) = selection
                    && let Ok(files) = self.draft.selection_files(&selection)
                {
                    if matches!(action, EditAction::Mark) {
                        self.queue_marks(files);
                    } else {
                        self.queue_file_trash(files);
                        return self.trash_now.is_some();
                    }
                }
            }
            EditAction::Rename if !folders => {
                if let Some(row) = self.file_cursor {
                    self.start_rename(self.rows[row].id);
                    return true;
                }
            }
            EditAction::Rename if !self.folder.is_empty() => {
                self.name = self.folder.rsplit('/').next().unwrap_or_default().into();
                self.folder_edit = Some(FolderEdit::Rename);
                self.folder_name_focus = true;
                self.compact_folders = true;
                self.error = None;
                return true;
            }
            EditAction::Rename => {}
            EditAction::NewFolder => {
                if !folders && let Some(row) = self.file_cursor {
                    self.folder = self.rows[row].after.clone();
                }
                self.name.clear();
                self.folder_edit = Some(FolderEdit::Create);
                self.folder_name_focus = true;
                self.compact_folders = true;
                self.error = None;
                return true;
            }
            EditAction::PreviewMoves => {
                self.preview_moves_requested = true;
                return true;
            }
        }
        false
    }
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
            self.queue_marks(files.clone());
            ui.close();
        }
        if ui.add_enabled(!files.is_empty(), egui::Button::new("Move these files to Trash…"))
            .on_hover_text("Keep your layout edits and review only these files and their companions before applying.").clicked() {
            self.queue_file_trash(files.clone());
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
            if EditAction::from_key(key, m).is_some() {
                return true;
            }
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
                _ => false,
            }
        });
        for (key, modifiers, repeat) in events {
            if let Some(action) = EditAction::from_key(key, modifiers) {
                if !repeat && self.edit_action(action, false) {
                    break;
                }
                continue;
            }
            if repeat && key == egui::Key::Space {
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
        ui.label(RichText::new("Files").font(semibold(14.0)));
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
                        // The grow atom left-aligns the name across the full row.
                        let response = ui.add_sized([ui.available_width(), button_row_height(ui)],
                            egui::Button::selectable(selected, (title, egui::Atom::grow())).truncate().sense(collection_controls::pointer_sense(true)));
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
                            self.trash_menu(ui, self.file_selection(id), decisions);
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
                        let line = if before_path == after_path {
                            RichText::new(format!("Stays in {}", if before.as_os_str().is_empty() { "Library root".into() } else { before.display().to_string() })).small().color(FAINT)
                        } else {
                            RichText::new(format!("{}  →  {}", before_path.display(), after_path.display())).small().color(ACCENT)
                        };
                        // Align with the name inside the row button above,
                        // without adding height: rows have a fixed pitch.
                        egui::Frame::new()
                            .inner_margin(egui::Margin { left: ui.spacing().button_padding.x as i8, ..Default::default() })
                            .show(ui, |ui| {
                                ui.add(egui::Label::new(line).truncate())
                                .on_hover_text(format!("Before: {}\nAfter: {}\nDrag selected files onto a destination folder. Double-click a file to rename it.", before_path.display(), after_path.display()));
                            });
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
        keyboard.finish(ui, output.inner_rect, "Files. Arrows move focus; Space selects; Shift extends selection; Control or Command A selects all; F2 renames; D marks; Delete reviews Trash; Control or Command Shift N creates a folder.");
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
        self.folder_edit = None;
        self.folder_name_focus = false;
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
        ui.label(RichText::new("Destination folders").font(semibold(14.0)));
        let mut scroll = egui::ScrollArea::vertical().id_salt("organize_folders");
        if let Some(offset) = self.restore_scroll[1].take() {
            scroll = scroll.vertical_scroll_offset(offset);
        }
        let row_height = button_row_height(ui);
        let max_height = (height - 210.0).max(75.0);
        let keyboard = tree_controls::Keyboard::new(ui, "organize_folders", max_height, row_height);
        let events = keyboard.collection().events(ui, |key, modifiers| {
            EditAction::from_key(key, modifiers).is_some()
                || tree_controls::navigation_key(key, modifiers, true)
        });
        for (key, modifiers, repeat) in events {
            if let Some(action) = EditAction::from_key(key, modifiers) {
                if !repeat && self.edit_action(action, true) {
                    break;
                }
                continue;
            }
            let Some(navigation) = keyboard.navigation_command(ui, key) else {
                continue;
            };
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
                            let title = folder_title(&path);
                            let response = ui.add_sized([ui.available_width(), row_height], egui::Button::selectable(
                                    self.selected_folders.contains(&path),
                                    (RichText::new(title), egui::Atom::grow(), RichText::new(count.to_string()).small().color(MUTED)),
                                )
                                .truncate().sense(tree_controls::pointer_sense(true)));
                            let spoken = format!("{title}, {count} files");
                            let selected = self.selected_folders.contains(&path);
                            response.widget_info(|| egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &spoken));
                            keyboard.focus_on_click(&response);
                            if self.folder == path {
                                ui.painter().rect_stroke(response.rect, 2., ui.visuals().selection.stroke, StrokeKind::Inside);
                            }
                            #[cfg(test)]
                            ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new(("test_folder_rect", &path)), response.rect));
                            response.context_menu(|ui| {
                                expansion_changed |= tree_controls::menu(ui, &self.tree, i, &mut self.collapsed);
                                self.trash_menu(ui, self.folder_selection(&path), decisions);
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
            ui.label(match self.folder_edit {
                Some(FolderEdit::Create) => "New folder",
                Some(FolderEdit::Rename) => "Rename folder",
                None => "Name",
            });
            let mut field = egui::TextEdit::singleline(&mut self.name)
                .id_salt("organize_folder_name")
                .desired_width(ui.available_width())
                .hint_text("Folder name")
                .show(ui);
            if std::mem::take(&mut self.folder_name_focus) {
                field.response.request_focus();
                field
                    .state
                    .cursor
                    .set_char_range(Some(egui::text::CCursorRange::two(
                        egui::text::CCursor::new(0),
                        egui::text::CCursor::new(self.name.chars().count()),
                    )));
                field.state.store(ui.ctx(), field.response.id);
            }
            if (field.response.has_focus() || field.response.lost_focus())
                && ui.input(|i| i.key_pressed(egui::Key::Enter))
            {
                command = self.folder_edit.map(|edit| match edit {
                    FolderEdit::Create => Command::Create,
                    FolderEdit::Rename => Command::Rename,
                });
            }
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
                self.folder_edit = None;
                self.folder_name_focus = false;
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
    /// F2 and Ctrl/Cmd+Shift+N in the Organize grid open the folder editor on
    /// the active photo and start renaming it or naming a new folder.
    pub(super) fn organize_shortcut(&mut self, key: egui::Key, modifiers: egui::Modifiers) -> bool {
        let Some(action @ (EditAction::Rename | EditAction::NewFolder)) =
            EditAction::from_key(key, modifiers)
        else {
            return false;
        };
        if self.step != Step::Organize
            || self.review
            || self.organize_editor.is_some()
            || !self.can_edit_folders()
        {
            return false;
        }
        let Some(active) = self.active else {
            return false;
        };
        self.open_organize_editor();
        if let Some(editor) = self.organize_editor.as_mut() {
            if let Some(row) = editor.rows.iter().position(|r| r.id == active) {
                editor.file_cursor = Some(row);
                editor.file_anchor = Some(active);
                editor.reveal_file = true;
            }
            editor.edit_action(action, false);
        }
        true
    }

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
                // Video GPS is not read, so a video without a resolved place would
                // otherwise be swept into "No GPS"; it stays where it is instead.
                !(self.group_mode == GroupMode::Location
                    && m.kind == MediaKind::Video
                    && self
                        .metadata
                        .get(&m.id)
                        .is_none_or(|meta| meta.location.is_none()))
                    && (self.group_mode != GroupMode::Visual
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
        let mut rows: Vec<PhotoRow> = self
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
        // Folders alphabetically, photos within a folder in the order taken.
        rows.sort_by_cached_key(|row| {
            let order = self.index.get(&row.id).map(|&i| {
                crate::core::dates::capture_order(&self.media[i], self.metadata.get(&row.id))
            });
            (row.source.parent().map(std::path::Path::to_path_buf), order)
        });
        let folder = self
            .active
            .and_then(|id| draft.labels().get(&id))
            .cloned()
            .unwrap_or_default();
        let mut editor = Editor {
            preview: None,
            catalog: self.catalog.as_ref().unwrap().clone(),
            output: self.output.clone().into(),
            desktop_enabled: self.desktop_available(),
            desktop_pending: None,
            reset_request: None,
            mark_requests: Vec::new(),
            preview_moves_requested: false,
            folder_edit: None,
            folder_name_focus: false,
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
    pub(super) fn organize_preview(&self) -> Option<&Preview> {
        self.organize_editor.as_ref()?.preview.as_ref()
    }
    fn organize_preview_ids(&self, editor: &Editor) -> Vec<MediaId> {
        // Bound work even for very large folder-editor selections. Documents
        // remain selected but are not sent to the shared media pipeline.
        if editor.selected.len() > 9 {
            return Vec::new();
        }
        let mut ids: Vec<_> = editor
            .selected
            .iter()
            .copied()
            .filter(|id| self.index.contains_key(id))
            .collect();
        ids.sort_by_key(|id| self.index[id]);
        ids
    }
    fn begin_organize_preview(&mut self) {
        let Some(editor) = self.organize_editor.as_ref() else {
            return;
        };
        if editor.preview.is_some() || self.step != Step::Organize || self.busy || self.scanning {
            return;
        }
        let ids = self.organize_preview_ids(editor);
        if ids.is_empty() {
            return;
        }
        let preview = Preview {
            active_before: self.active,
            view_before: self.inspection_view,
            independent_before: std::mem::take(&mut self.independent_views),
            sync_before: self.sync_views,
            focused_before: self.focused_slot,
            ids,
        };
        self.active = preview.ids.first().copied();
        self.focused_slot = 0;
        self.inspection_view = InspectionView::default();
        self.organize_editor.as_mut().unwrap().preview = Some(preview);
    }
    fn end_organize_preview(&mut self) {
        let Some(preview) = self.organize_editor.as_mut().and_then(|e| e.preview.take()) else {
            return;
        };
        self.active = preview.active_before;
        self.focused_slot = preview.focused_before;
        self.inspection_view = preview.view_before;
        self.independent_views = preview.independent_before;
        self.sync_views = preview.sync_before;
        self.inspection_wanted.clear();
    }
    fn organize_preview_dialog(&mut self, ctx: &egui::Context) {
        let mut back = false;
        egui::Modal::new(egui::Id::new("organize_editor")).show(ctx, |ui| {
            ui.set_width((ctx.content_rect().width() - 64.0).clamp(280.0, 1080.0));
            ui.horizontal_wrapped(|ui| {
                back |= ui.button("< Back to arranging").clicked();
                ui.heading("Selection preview");
            });
            ui.label("Your folder and name edits stay pending while you compare.");
            ui.separator();
            let height = (ctx.content_rect().height() - 210.0).clamp(140.0, 720.0);
            ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
                self.review_canvas(ui)
            });
        });
        if !self.other_modal_active() && !egui::Popup::is_any_open(ctx) {
            self.video_keyboard(ctx);
            back |=
                ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
        }
        if back {
            self.end_organize_preview();
        }
    }
    pub(super) fn organize_dialog(&mut self, ctx: &egui::Context) {
        if self.organize_preview().is_some() {
            self.organize_preview_dialog(ctx);
            return;
        }
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
            let height = (ctx.content_rect().height() - 380.0).clamp(160.0, 560.0);
            ui.set_width(width);
            ui.horizontal_wrapped(|ui| {
                ui.heading("Arrange files and folders");
                if ui.add_enabled(!self.organize_preview_ids(&editor).is_empty(), egui::Button::new("Preview selection"))
                    .on_hover_text("Compare up to nine selected items without accepting edits. Supporting documents are omitted.").clicked() { action = 5; }
                ui.menu_button("Actions", |ui| self.clear_trash_menu_item(ui));
            });
            if width >= 680.0 {
                ui.label(RichText::new("Drag files onto folders and folders into each other; double-click a file to rename it. Nothing changes on disk until you preview and apply the moves.").color(MUTED))
                    .on_hover_text("Photo companions move and rename together when enabled. Added empty folders are included in the reviewed plan.");
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
                if ui.add(primary("Preview moves")).clicked() { action = 3; }
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
        for request in std::mem::take(&mut editor.mark_requests) {
            match request {
                MarkRequest::Toggle(ids) => {
                    let marked = !ids.iter().all(|id| self.decision(*id).rejected);
                    self.stage_manual(ids, crate::core::decisions::Change::Trash(marked));
                }
                MarkRequest::Clear => self.clear_all_trash_marks(),
            }
        }
        if std::mem::take(&mut editor.preview_moves_requested) && action == 0 {
            action = 3;
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
            5 => {
                self.organize_editor = Some(editor);
                self.begin_organize_preview();
            }
            4 => {
                editor.draft =
                    Draft::from_catalog(self.catalog.as_ref().unwrap(), self.organize_labels());
                editor.folder.clear();
                editor.parent.clear();
                editor.name.clear();
                editor.folder_edit = None;
                editor.folder_name_focus = false;
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
#[path = "organize/tests.rs"]
mod tests;
