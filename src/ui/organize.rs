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
    /// What the file list draws, grouped by destination, and the rows it
    /// shows in that order (arrow keys and Shift ranges follow it).
    lines: Vec<Line>,
    order: Vec<usize>,
    /// System files and other non-media files are listed only on request;
    /// they still move with their folders.
    show_other: bool,
    other: usize,
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
    /// The destination folder under the pointer, its row and since when.
    folder_hover: Option<(String, egui::Rect, f64)>,
    photo_scroll: f32,
    folder_scroll: f32,
    restore_scroll: [Option<f32>; 2],
}
/// Why an analysis-based grouping has no result yet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Pending {
    Running,
    /// Not started: another task, the folder scan or AI models come first.
    Waiting,
    Cancelled,
    Failed,
}
/// One line of the file list: a destination heading, a file (an index into
/// `rows`), or the switch that shows other files.
#[derive(Clone, Debug, PartialEq)]
enum Line {
    Group {
        folder: String,
        count: usize,
        moving: bool,
    },
    File(usize),
    Other,
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
    Parent,
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
// Tree keys are built from labels, so a `\` inside a macOS or Linux folder name
// must stay part of that name rather than become a separator.
fn folder_label(path: &std::path::Path) -> String {
    crate::core::names::relative_label(path)
        .unwrap_or_else(|| path.to_string_lossy().replace('\\', "/"))
}
/// System files (names starting with a dot) and files that are not photos or
/// videos: listed only on request, since they rarely need sorting by hand.
fn hidden_file(path: &std::path::Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().starts_with('.'))
        || crate::core::model::MediaKind::from_path(path).is_none()
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
                    .is_ok_and(|relative| {
                        crate::core::names::relative_label(relative).as_deref() == Some(original)
                    })
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
        // Counts are what the file list shows: hidden system and other
        // files count only while they are listed.
        let mut counts = HashMap::<String, usize>::new();
        for row in &mut self.rows {
            row.after = self
                .draft
                .labels()
                .get(&row.id)
                .cloned()
                .unwrap_or_default();
            if self.show_other || !hidden_file(&row.source) {
                *counts.entry(row.after.clone()).or_default() += 1;
            }
        }
        self.tree = Tree::folders(
            self.draft.folders().iter().map(|path| {
                (
                    PathBuf::from(path),
                    if path.is_empty() {
                        self.root_name()
                    } else {
                        path.to_owned()
                    },
                    counts.get(path).copied().unwrap_or_default(),
                )
            }),
            || false,
        )
        .expect("in-memory folder tree is not cancelled");
        self.project_folders();
        self.arrange_lines();
    }
    /// The opened folder's own name, for the top of the tree.
    fn root_name(&self) -> String {
        self.catalog
            .root
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| folder_title("").to_owned())
    }
    /// A folder as shown: the opened folder's name for the top.
    fn title(&self, path: &str) -> String {
        if path.is_empty() {
            self.root_name()
        } else {
            path.replace('/', " / ")
        }
    }
    /// True when a row ends up somewhere else on disk or under another name.
    fn row_changes(&self, row: &PhotoRow) -> bool {
        let renamed = self
            .names
            .get(&row.id)
            .is_some_and(|stem| !stem.trim().is_empty());
        let parent = row.source.parent().unwrap_or(std::path::Path::new(""));
        let target = if self.output.is_absolute() {
            operations::folder_destination(&self.output, &row.after)
        } else {
            operations::folder_destination(&self.catalog.root, &row.after)
        };
        renamed || target != parent
    }
    /// Groups the files under their destination, files that change first,
    /// then those that stay; system and other non-media files go last and
    /// only when asked for.
    fn arrange_lines(&mut self) {
        let other = |row: &PhotoRow| hidden_file(&row.source);
        let mut moving = std::collections::BTreeMap::<&str, Vec<usize>>::new();
        let mut staying = Vec::new();
        let mut others = Vec::new();
        for (i, row) in self.rows.iter().enumerate() {
            if other(row) {
                others.push(i);
            } else if self.row_changes(row) {
                moving.entry(row.after.as_str()).or_default().push(i);
            } else {
                staying.push(i);
            }
        }
        let mut lines = Vec::new();
        for (folder, rows) in &moving {
            lines.push(Line::Group {
                folder: (*folder).to_owned(),
                count: rows.len(),
                moving: true,
            });
            lines.extend(rows.iter().map(|&i| Line::File(i)));
        }
        if !staying.is_empty() {
            lines.push(Line::Group {
                folder: String::new(),
                count: staying.len(),
                moving: false,
            });
            lines.extend(staying.iter().map(|&i| Line::File(i)));
        }
        if !others.is_empty() {
            lines.push(Line::Other);
            if self.show_other {
                lines.extend(others.iter().map(|&i| Line::File(i)));
            }
        }
        self.other = others.len();
        self.order = lines
            .iter()
            .filter_map(|line| match line {
                Line::File(i) => Some(*i),
                _ => None,
            })
            .collect();
        self.lines = lines;
    }
    /// Shows or hides system and other files. Hidden ones leave the
    /// selection, so no move takes them along unseen.
    fn toggle_other_files(&mut self) {
        self.show_other = !self.show_other;
        self.refresh();
        if !self.show_other {
            let shown: HashSet<_> = self.order.iter().map(|&i| self.rows[i].id).collect();
            self.selected.retain(|id| shown.contains(id));
            if self.file_cursor.is_some_and(|i| !self.order.contains(&i)) {
                self.file_cursor = None;
            }
        }
    }
    /// The file cursor's place in the shown order.
    fn cursor_position(&self) -> Option<usize> {
        let cursor = self.file_cursor?;
        self.order.iter().position(|&i| i == cursor)
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
            ui.add(
                egui::Label::new(
                    RichText::new(format!("Rename {original} · the extension stays"))
                        .small()
                        .color(MUTED),
                )
                .truncate(),
            );
            ui.horizontal(|ui| {
                let buttons = 230.0;
                let mut field = egui::TextEdit::singleline(name)
                    .id_salt("organize_file_name")
                    .desired_width((ui.available_width() - buttons).max(80.0))
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
                if ui.button("Save").clicked() || enter {
                    command = Some(Command::RenameFile(*id, name.clone()));
                }
                if ui
                    .button("Original name")
                    .on_hover_text("Keep the name it has now")
                    .clicked()
                {
                    command = Some(Command::RenameFile(*id, String::new()));
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
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
            egui::UiBuilder::new().id(egui::Id::new(("organize_file_panel", &self.catalog.root))),
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
            self.order.iter().map(|&i| self.rows[i].id),
            toggle,
            extend,
        );
    }
    /// Every file shown; hidden system and other files stay unselected.
    fn select_all_files(&mut self) {
        self.selected = self.order.iter().map(|&i| self.rows[i].id).collect();
        self.file_cursor = self.file_cursor.or_else(|| self.order.first().copied());
        self.file_anchor = self.order.first().map(|&i| self.rows[i].id);
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
            if self.order.is_empty() {
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
                let row = self.file_cursor.unwrap_or(self.order[0]);
                self.select_file(row, !modifiers.shift || special, modifiers.shift);
                self.reveal_file = true;
                continue;
            }
            // Positions in the shown order, mapped back to rows.
            let last = self.order.len() - 1;
            let position = self.cursor_position();
            let target = match key {
                egui::Key::ArrowUp | egui::Key::ArrowDown => {
                    crate::core::navigation::next_position(
                        self.order.len(),
                        position,
                        if key == egui::Key::ArrowUp { -1 } else { 1 },
                        |p| {
                            special
                                || modifiers.shift
                                || !decisions
                                    .get(&self.rows[self.order[p]].id)
                                    .is_some_and(|d| d.rejected)
                        },
                    )
                }
                egui::Key::Home => Some(0),
                egui::Key::End => Some(last),
                egui::Key::PageUp => {
                    Some(position.unwrap_or(0).saturating_sub(keyboard.page_rows(ui)))
                }
                egui::Key::PageDown => {
                    Some((position.unwrap_or(0) + keyboard.page_rows(ui)).min(last))
                }
                _ => None,
            }
            .map(|p| self.order[p]);
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
        ui.horizontal(|ui| {
            // A fixed height: the right-to-left part would otherwise grow
            // to the column's height.
            ui.set_height(button_row_height(ui));
            ui.label(RichText::new("Photos").font(semibold(14.0)));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if !self.selected.is_empty() {
                    if ui.small_button("Clear").clicked() {
                        self.clear_files();
                    }
                    ui.label(
                        RichText::new(format!("{} selected", self.selected.len()))
                            .small()
                            .color(ACCENT),
                    );
                } else if ui
                    .small_button("Select all")
                    .on_hover_text("Every photo shown · Ctrl/Cmd+A")
                    .clicked()
                {
                    self.select_all_files();
                }
            });
        });
        command = self.file_name_controls(ui).or(command);
        let name_controls_height = if self.renaming.is_some() { 64.0 } else { 0.0 };
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
            && let Some(cursor) = self.file_cursor
            && let Some(line) = self.lines.iter().position(|l| *l == Line::File(cursor))
        {
            // The first file brings its heading along: Home shows the top.
            let line = if self.order.first() == Some(&cursor) {
                0
            } else {
                line
            };
            scroll = keyboard.scroll(ui, scroll, line);
        }
        let focused = keyboard.has_focus(ui);
        let mut toggle_other = false;
        // A fixed height, like the folders beside it: the dialog keeps its
        // size and place from the first frame, so rows stay under the pointer.
        let output = scroll
            .auto_shrink([false, false])
            .min_scrolled_height(max_height)
            .max_height(max_height)
            .show_rows(ui, row_height, self.lines.len(), |ui, lines| {
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Truncate);
                for line in lines {
                    #[cfg(test)]
                    {
                        let rect = egui::Rect::from_min_size(
                            ui.cursor().min,
                            egui::vec2(ui.available_width(), button_row_height(ui)),
                        );
                        ui.ctx().data_mut(|d| {
                            d.insert_temp(egui::Id::new(("test_line_rect", line)), rect)
                        });
                    }
                    match self.lines[line].clone() {
                        Line::Group { folder, count, moving } => {
                            let (rect, response) = ui.allocate_exact_size(
                                egui::vec2(ui.available_width(), row_height),
                                Sense::hover(),
                            );
                            let (glyph, title, color) = if moving {
                                (
                                    egui_phosphor::regular::ARROW_RIGHT,
                                    format!("To {}", self.title(&folder)),
                                    ACCENT,
                                )
                            } else {
                                (
                                    egui_phosphor::regular::PUSH_PIN,
                                    "Not moving".to_owned(),
                                    MUTED,
                                )
                            };
                            let y = rect.bottom() - 12.0;
                            let painter = ui.painter_at(rect);
                            painter.text(
                                egui::pos2(rect.left() + 4.0, y),
                                Align2::LEFT_CENTER,
                                glyph,
                                FontId::proportional(13.0),
                                color,
                            );
                            let mut job = egui::text::LayoutJob::simple_singleline(
                                format!("{title}  ·  {count}"),
                                semibold(12.5),
                                INK,
                            );
                            job.wrap = egui::text::TextWrapping::truncate_at_width(
                                (rect.width() - 30.0).max(10.0),
                            );
                            let galley = painter.layout_job(job);
                            painter.galley(
                                egui::pos2(rect.left() + 24.0, y - galley.size().y / 2.0),
                                galley,
                                INK,
                            );
                            response.widget_info(|| {
                                egui::WidgetInfo::labeled(
                                    egui::WidgetType::Label,
                                    true,
                                    format!("{title}, {count} files"),
                                )
                            });
                            // A heading takes files dropped on it.
                            if moving
                                && let Some(drop) = self.drop_target(ui, &response, &folder)
                            {
                                command = Some(drop);
                            }
                        }
                        Line::Other => {
                            ui.add_space((row_height - button_row_height(ui)) / 2.0);
                            let text = if self.show_other {
                                format!("Hide {} system and other files", self.other)
                            } else {
                                format!(
                                    "Show {} system and other {}",
                                    self.other,
                                    if self.other == 1 { "file" } else { "files" }
                                )
                            };
                            if ui
                                .small_button(text)
                                .on_hover_text("Files such as .DS_Store, documents and archives. They stay with their folder unless you move them.")
                                .clicked()
                            {
                                toggle_other = true;
                            }
                        }
                        Line::File(i) => {
                            if let Some(drop) = self.file_line(ui, i, focused, &keyboard, decisions) {
                                command = Some(drop);
                            }
                        }
                    }
                }
            });
        if toggle_other {
            self.toggle_other_files();
        }
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
        keyboard.finish(ui, output.inner_rect, "Photos, grouped by where they go. Arrows move focus; Space selects; Shift extends selection; Control or Command A selects all; F2 renames; D marks; Delete reviews Trash; Control or Command Shift N creates a folder.");
        self.photo_scroll = output.state.offset.y;
        self.scroll_drag(ui, &output);
        command
    }
    /// One file: its name, and where it comes from or stays.
    fn file_line(
        &mut self,
        ui: &mut egui::Ui,
        i: usize,
        focused: bool,
        keyboard: &collection_controls::Keyboard,
        decisions: &HashMap<MediaId, Decision>,
    ) -> Option<Command> {
        let mut command = None;
        let id = self.rows[i].id;
        let name = self.rows[i]
            .source
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let after_name = self
            .names
            .get(&id)
            .map(|stem| {
                crate::core::organize::renamed_file_name(
                    std::path::Path::new(&name),
                    std::ffi::OsStr::new(stem),
                )
                .to_string_lossy()
                .into_owned()
            })
            .unwrap_or_else(|| name.clone());
        let before = self.rows[i]
            .source
            .parent()
            .and_then(|p| p.strip_prefix(&self.catalog.root).ok())
            .unwrap_or_else(|| std::path::Path::new(""))
            .to_path_buf();
        let after = self.rows[i].after.clone();
        let changes = self.row_changes(&self.rows[i]);
        let selected = self.selected.contains(&id);
        ui.push_id(id.0, |ui| {
            let marked = decisions.get(&id).is_some_and(|d| d.rejected);
            let title = if marked {
                RichText::new(format!("{after_name} · Trash")).color(RED)
            } else {
                RichText::new(&after_name)
            };
            // The grow atom left-aligns the name across the full row.
            let response = ui.add_sized(
                [ui.available_width(), button_row_height(ui)],
                egui::Button::selectable(selected, (title, egui::Atom::grow()))
                    .truncate()
                    .sense(collection_controls::pointer_sense(true)),
            );
            keyboard.focus_on_click(&response);
            if focused && self.file_cursor == Some(i) {
                ui.painter().rect_stroke(
                    response.rect,
                    2.,
                    ui.visuals().selection.stroke,
                    StrokeKind::Inside,
                );
            }
            if response.clicked() {
                let m = ui.input(|i| i.modifiers);
                self.select_file(i, !m.shift || m.command || m.ctrl, m.shift);
            }
            if response.double_clicked() {
                self.start_rename(id);
            }
            response.context_menu(|ui| {
                if ui.button("Rename…  F2").clicked() {
                    self.start_rename(id);
                    ui.close();
                }
                if ui
                    .add_enabled(self.reset_enabled, egui::Button::new("Back to the suggested folder"))
                    .on_hover_text("Restore the initial grouping for this file, or the selected files when it is one of them. File names stay unchanged.")
                    .clicked()
                {
                    self.reset_request = Some(Selection {
                        files: if self.selected.contains(&id) {
                            self.selected.clone()
                        } else {
                            HashSet::from([id])
                        },
                        ..Default::default()
                    });
                    ui.close();
                }
                ui.separator();
                self.trash_menu(ui, self.file_selection(id), decisions);
                ui.separator();
                let paths = self.file_locations(&self.rows[i]);
                if let Some(request) = desktop::locations_menu(ui, &paths, self.desktop_enabled) {
                    self.desktop_pending = Some(request);
                }
            });
            #[cfg(test)]
            ui.ctx().data_mut(|d| {
                d.insert_temp(egui::Id::new(("test_file_rect", id.0)), response.rect)
            });
            if self.starts_drag(ui, &response) {
                egui::DragAndDrop::set_payload(ui.ctx(), self.file_drag(id));
            }
            self.track_completed_drag(ui, &response, Some(id), &after);
            if let Some(drop) = self.drop_target(ui, &response, &after) {
                command = Some(drop);
            }
            let from = if before.as_os_str().is_empty() {
                self.root_name()
            } else {
                before.display().to_string()
            };
            let mut line = if changes {
                format!("from {from}")
            } else {
                format!("in {from}")
            };
            if after_name != name {
                line.push_str(&format!(" · was {name}"));
            }
            let before_path = before.join(&name);
            let after_path = std::path::Path::new(&after).join(&after_name);
            // Align with the name inside the row button above,
            // without adding height: rows have a fixed pitch.
            egui::Frame::new()
                .inner_margin(egui::Margin {
                    left: ui.spacing().button_padding.x as i8,
                    ..Default::default()
                })
                .show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(line).small().color(MUTED)).truncate())
                        .on_hover_text(format!(
                            "Before: {}\nAfter: {}\nDrag selected photos onto a folder. Double-click to rename.",
                            before_path.display(),
                            after_path.display()
                        ));
                });
        });
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
            egui::UiBuilder::new().id(egui::Id::new(("organize_folder_panel", &self.catalog.root))),
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
                            Some(folder_label(path))
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
        self.folder_anchor = Some(folder_label(anchor));
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
        let mut hovered_now: Option<(String, egui::Rect)> = None;
        ui.horizontal(|ui| {
            ui.set_height(button_row_height(ui));
            ui.label(RichText::new("Folders").font(semibold(14.0)));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button(format!(
                        "{}  New folder",
                        egui_phosphor::regular::FOLDER_PLUS
                    ))
                    .on_hover_text(
                        "Inside the selected folder; selected photos go into it · Ctrl/Cmd+Shift+N",
                    )
                    .clicked()
                {
                    let folder = self.folder.clone();
                    self.begin_folder_edit(&folder, FolderEdit::Create);
                }
            });
        });
        if let Some(edit) = self.folder_edit {
            command = self.folder_name_bar(ui, edit).or(command);
        }
        let mut scroll = egui::ScrollArea::vertical().id_salt("organize_folders");
        if let Some(offset) = self.restore_scroll[1].take() {
            scroll = scroll.vertical_scroll_offset(offset);
        }
        let row_height = button_row_height(ui);
        let max_height = (height
            - if self.folder_edit.is_some() {
                96.0
            } else {
                56.0
            })
        .max(75.0);
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
                        &folder_label(&path),
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
            .min_scrolled_height(max_height)
            .max_height(max_height)
            .show_rows(ui, row_height, self.visible_folders.len(), |ui, rows| {
                for row in rows {
                    let i = self.visible_folders[row];
                    let node = &self.tree.nodes[i];
                    let Key::Folder(path) = &node.key else { unreachable!() };
                    let path = folder_label(path);
                    let (count, depth) = (node.count, node.depth);
                    ui.push_id(("folder", &path), |ui| {
                        ui.horizontal(|ui| {
                            ui.add_space((depth as f32 * 12.0).min(96.0));
                            expansion_changed |= tree_controls::expander(ui, &self.tree, i, &mut self.collapsed, &keyboard);
                            let title = if path.is_empty() {
                                self.root_name()
                            } else {
                                path.rsplit('/').next().unwrap_or_default().to_owned()
                            };
                            let title = title.as_str();
                            let response = ui.add_sized([ui.available_width(), row_height], egui::Button::selectable(
                                    self.selected_folders.contains(&path),
                                    (RichText::new(title), egui::Atom::grow(), RichText::new(count.to_string()).small().color(MUTED)),
                                )
                                .truncate().sense(tree_controls::pointer_sense(true)));
                            if response.hovered() && !owned_drag {
                                hovered_now = Some((path.clone(), response.rect));
                            }
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
                                if let Some(structure) = self.folder_menu(ui, &path) {
                                    command = Some(structure);
                                }
                                ui.separator();
                                self.trash_menu(ui, self.folder_selection(&path), decisions);
                                ui.separator();
                                if ui.add_enabled(self.reset_enabled, egui::Button::new("Files back to their suggested folders")).on_hover_text("Restore the initial grouping for files inside this folder, or the selected folders. Folder structure edits and file names stay unchanged.").clicked() {
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
        ui.label(
            RichText::new("Drag photos or folders onto a folder. Right-click a folder to rename, nest or move it.")
                .small()
                .color(MUTED),
        );
        // Keep when the pointer arrived on this folder, so the preview waits.
        let now = ui.input(|i| i.time);
        self.folder_hover = hovered_now.map(|(path, rect)| match self.folder_hover.take() {
            Some((previous, _, since)) if previous == path => (path, rect, since),
            _ => (path, rect, now),
        });
        command
    }
    /// Starts naming a folder: a new one inside `path`, `path` itself, or a
    /// new one around it.
    fn begin_folder_edit(&mut self, path: &str, edit: FolderEdit) {
        if !matches!(edit, FolderEdit::Create) && path.is_empty() {
            return;
        }
        self.folder = path.into();
        self.name = match edit {
            FolderEdit::Rename => path.rsplit('/').next().unwrap_or_default().into(),
            _ => String::new(),
        };
        self.folder_edit = Some(edit);
        self.folder_name_focus = true;
        self.compact_folders = true;
        self.error = None;
    }
    /// The one place a folder name is typed: what it is for, the name, and
    /// Save or Cancel. Enter saves; Escape cancels.
    fn folder_name_bar(&mut self, ui: &mut egui::Ui, edit: FolderEdit) -> Option<Command> {
        let mut command = None;
        let leaf = folder_title(self.folder.rsplit('/').next().unwrap_or_default()).to_owned();
        let purpose = match edit {
            FolderEdit::Create if self.selected.is_empty() => {
                format!("New folder in {}", self.title(&self.folder))
            }
            FolderEdit::Create => format!(
                "New folder in {} · the {} selected {} go into it",
                self.title(&self.folder),
                self.selected.len(),
                if self.selected.len() == 1 {
                    "photo"
                } else {
                    "photos"
                }
            ),
            FolderEdit::Rename => format!("Rename “{leaf}”"),
            FolderEdit::Parent => format!("New folder around “{leaf}”"),
        };
        ui.add(egui::Label::new(RichText::new(purpose).small().color(MUTED)).truncate());
        let mut cancel = false;
        ui.horizontal(|ui| {
            let mut field = egui::TextEdit::singleline(&mut self.name)
                .id_salt("organize_folder_name")
                .desired_width((ui.available_width() - 130.0).max(80.0))
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
            let typing = field.response.has_focus() || field.response.lost_focus();
            let enter = typing && ui.input(|i| i.key_pressed(egui::Key::Enter));
            cancel = typing && ui.input(|i| i.key_pressed(egui::Key::Escape));
            if ui.add(primary("Save")).clicked() || enter {
                command = Some(match edit {
                    FolderEdit::Create => Command::Create,
                    FolderEdit::Rename => Command::Rename,
                    FolderEdit::Parent => Command::Parent,
                });
            }
            cancel |= ui.button("Cancel").clicked();
        });
        if cancel {
            self.folder_edit = None;
            self.error = None;
        }
        command
    }
    /// Folder actions in a folder's context menu: make, rename and nest.
    fn folder_menu(&mut self, ui: &mut egui::Ui, path: &str) -> Option<Command> {
        let mut command = None;
        if ui.button("New folder inside…").clicked() {
            self.begin_folder_edit(path, FolderEdit::Create);
            ui.close();
        }
        if !path.is_empty() {
            if ui.button("Rename…  F2").clicked() {
                self.begin_folder_edit(path, FolderEdit::Rename);
                ui.close();
            }
            if ui
                .button("Put inside a new folder…")
                .on_hover_text("Makes a folder in the same place and moves this one into it")
                .clicked()
            {
                self.begin_folder_edit(path, FolderEdit::Parent);
                ui.close();
            }
            let parent = path.rsplit_once('/').map_or("", |(parent, _)| parent);
            let targets: Vec<String> = self
                .tree
                .nodes
                .iter()
                .filter_map(|node| match &node.key {
                    Key::Folder(p) => Some(folder_label(p)),
                    _ => None,
                })
                .filter(|target| {
                    target != path
                        && target != parent
                        && !target
                            .strip_prefix(path)
                            .is_some_and(|rest| rest.starts_with('/'))
                })
                .collect();
            ui.add_enabled_ui(!targets.is_empty(), |ui| {
                ui.menu_button("Move into", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for target in targets {
                                if ui.button(self.title(&target)).clicked() {
                                    self.folder = path.into();
                                    self.parent = target;
                                    command = Some(Command::Nest);
                                    ui.close();
                                }
                            }
                        });
                });
            });
            if path.contains('/') && ui.button("Move to the top level").clicked() {
                self.folder = path.into();
                command = Some(Command::Unnest);
                ui.close();
            }
        }
        if !self.selected.is_empty()
            && ui
                .button(format!(
                    "Move {} selected {} here",
                    self.selected.len(),
                    if self.selected.len() == 1 {
                        "photo"
                    } else {
                        "photos"
                    }
                ))
                .clicked()
        {
            self.folder = path.into();
            command = Some(Command::Move);
            ui.close();
        }
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
            // With files selected, the new folder receives them in one step.
            Command::Create => self
                .draft
                .create_folder(&self.folder, self.name.trim())
                .and_then(|folder| {
                    if self.selected.is_empty() {
                        return Ok(folder);
                    }
                    self.draft
                        .move_selection(
                            &Selection {
                                files: self.selected.clone(),
                                ..Default::default()
                            },
                            &folder,
                            media,
                            companions,
                        )
                        .map(|()| folder)
                }),
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
    /// Faces only: whether photos without a recognized person stay in their
    /// folders or go to one "No people" folder.
    pub(super) fn faceless_controls(&mut self, ui: &mut egui::Ui) {
        use crate::core::preferences::{Faceless, NO_PEOPLE};
        let choices = [
            (Faceless::Stay, "Keep in their folders".to_owned()),
            (Faceless::Folder, format!("Put in a “{NO_PEOPLE}” folder")),
        ];
        let selected = &choices
            .iter()
            .find(|(choice, _)| *choice == self.preferences.faceless)
            .unwrap()
            .1;
        ui.label(RichText::new("Photos without people").color(MUTED));
        ui.add_enabled_ui(
            !self.modal_active()
                && !self.scanning
                && self.analysis_run.is_none()
                && self.retry_plan.is_none(),
            |ui| {
                egui::ComboBox::from_id_salt("faceless")
                    .selected_text(selected.as_str())
                    .width(190.0)
                    .show_ui(ui, |ui| {
                        for (choice, title) in &choices {
                            if ui
                                .selectable_label(self.preferences.faceless == *choice, title)
                                .clicked()
                            {
                                self.change_faceless(*choice);
                                ui.close();
                            }
                        }
                    })
                    .response
                    .on_hover_text(
                        "Photos where no person is recognized. Nothing moves until you preview and apply.",
                    );
            },
        );
    }
    pub(super) fn change_faceless(&mut self, choice: crate::core::preferences::Faceless) {
        if self.step != Step::Organize
            || self.group_mode != GroupMode::Faces
            || self.modal_active()
            || self.scanning
            || self.analysis_run.is_some()
            || self.retry_plan.is_some()
            || self.preferences.faceless == choice
        {
            return;
        }
        if self.organize_drafts.contains_key(&GroupMode::Faces) {
            self.pending_faceless = Some(choice);
        } else {
            self.accept_faceless(choice);
        }
    }
    pub(super) fn accept_faceless(&mut self, choice: crate::core::preferences::Faceless) {
        self.preferences.faceless = choice;
        self.organize_drafts.remove(&GroupMode::Faces);
        self.organize_view = None;
        self.pending_bookmark = None;
        self.set_review_plan(None);
        self.save_preferences();
    }
    pub(super) fn faceless_dialog(&mut self, ctx: &egui::Context) {
        let Some(choice) = self.pending_faceless else {
            return;
        };
        let mut action = None;
        modal(egui::Id::new("replace_faceless")).show(ctx, |ui| {
            ui.set_max_width(400.0_f32.min((ctx.content_rect().width() - 60.0).max(180.0)));
            ui.heading("Replace your people folder edits?");
            ui.label(match choice {
                crate::core::preferences::Faceless::Folder => format!(
                    "Photos without a recognized person will go to a “{}” folder. The people folders are proposed again, replacing the folder edits you made while grouping by faces. File names and other groupings keep their edits. Nothing changes on disk until you apply.",
                    crate::core::preferences::NO_PEOPLE
                ),
                crate::core::preferences::Faceless::Stay => "Photos without a recognized person will stay in their current folders. The people folders are proposed again, replacing the folder edits you made while grouping by faces. File names and other groupings keep their edits. Nothing changes on disk until you apply.".into(),
            });
            ui.horizontal_wrapped(|ui| {
                if ui.button("Keep my edits").clicked() {
                    action = Some(false);
                }
                if ui.button("Replace folder edits").clicked() {
                    action = Some(true);
                }
            });
        });
        if let Some(replace) = action {
            self.pending_faceless = None;
            if replace {
                self.accept_faceless(choice);
            }
        }
    }
    pub(super) fn location_layout_dialog(&mut self, ctx: &egui::Context) {
        let Some(_) = self.pending_location_layout else {
            return;
        };
        let mut action = None;
        modal(egui::Id::new("replace_location_layout")).show(ctx, |ui| {
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
            .filter_map(|m| Some((m.id, self.organize_label(m)?)))
            .collect();
        if let Some(catalog) = &self.catalog {
            crate::core::organize::include_inventory(catalog, &mut labels);
        }
        labels
    }
    /// The folder the current grouping proposes for one item, before any
    /// folder edits; `None` keeps it in the folder it is in now.
    pub(super) fn organize_label(&self, m: &Media) -> Option<String> {
        let grouped = match self.group_mode {
            // Video GPS is not read, so a video without a resolved place would
            // otherwise be swept into "No GPS"; it stays where it is instead.
            GroupMode::Location => {
                m.kind != MediaKind::Video
                    || self
                        .metadata
                        .get(&m.id)
                        .is_some_and(|meta| meta.location.is_some())
            }
            GroupMode::Visual => self
                .visual_groups
                .as_ref()
                .is_some_and(|groups| groups.contains_key(&m.id)),
            // Photos without a recognized person (group 0) stay in place
            // unless the "No people" folder is chosen.
            GroupMode::Faces => match self.face_group(m.id) {
                Some(0) => {
                    return (m.kind != MediaKind::Video
                        && self.preferences.faceless
                            == crate::core::preferences::Faceless::Folder)
                        .then(|| crate::core::preferences::NO_PEOPLE.into());
                }
                group => group.is_some(),
            },
            GroupMode::Mixed => self
                .mixed_labels
                .as_ref()
                .is_some_and(|labels| labels.contains_key(&m.id)),
            _ => true,
        };
        if !grouped {
            None
        } else if self.group_mode == GroupMode::Current {
            self.current_label(m)
        } else {
            Some(self.label_for(m))
        }
    }
    fn face_group(&self, id: MediaId) -> Option<usize> {
        self.face_groups.as_ref()?.groups.get(&id).copied()
    }
    /// Why an item outside every proposed folder stays where it is, in the
    /// words of the current grouping: "without a recognized person".
    pub(super) fn unplaced_reason(&self, m: &Media) -> &'static str {
        if self.organize_pending().is_some() {
            return "not grouped yet";
        }
        match (self.group_mode, m.kind) {
            (GroupMode::Location, MediaKind::Video) => "video locations are not read",
            (GroupMode::Faces | GroupMode::Visual | GroupMode::Mixed, MediaKind::Video) => {
                "videos are not grouped this way"
            }
            (GroupMode::Faces, _) => "no recognized person",
            (GroupMode::Visual, _) => "no similar photos",
            (GroupMode::Current, _) => "its folder name cannot be read",
            _ => "not grouped",
        }
    }
    /// "58 photos without a recognized person and 2 videos": what stays in
    /// its current folders, said in the current grouping's terms.
    pub(super) fn unplaced_phrase(&self, photos: usize, videos: usize) -> String {
        let plural =
            |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
        let photos = match self.group_mode {
            GroupMode::Faces => plural(
                photos,
                "photo without a recognized person",
                "photos without a recognized person",
            ),
            GroupMode::Visual => plural(
                photos,
                "photo without similar photos",
                "photos without similar photos",
            ),
            _ => plural(photos, "photo", "photos"),
        };
        let videos = plural(videos, "video", "videos");
        match (photos.starts_with("0 "), videos.starts_with("0 ")) {
            (false, false) => format!("{photos} and {videos}"),
            (true, false) => videos,
            _ => photos,
        }
    }
    /// An analysis-based grouping is chosen, has no result or folder edits
    /// yet, and did not just fail or get cancelled.
    pub(super) fn organize_grouping_wanted(&self) -> bool {
        self.automatic_organize_mode()
            && !self.organize_grouping_ready()
            && !self.organize_drafts.contains_key(&self.group_mode)
            && self.organize_halted != Some(self.group_mode)
    }
    /// Ends the running Organize grouping without results; it waits for the
    /// user to choose it again instead of restarting into the same failure.
    pub(super) fn halt_organize_run(&mut self) {
        if let Some((_, mode)) = self.organize_run.take() {
            self.organize_halted = Some(mode);
            self.organize_cancelled = false;
        }
    }
    /// The chosen grouping is analyzing right now.
    pub(super) fn organize_running(&self) -> bool {
        self.analysis_run
            .is_some_and(|run| self.organize_run == Some((run, self.group_mode)))
    }
    /// An analysis-based grouping has no result or folder edits yet, so the
    /// board must not present its photos as staying in place.
    pub(super) fn organize_pending(&self) -> Option<Pending> {
        if self.step != Step::Organize
            || !self.automatic_organize_mode()
            || self.organize_grouping_ready()
            || self.organize_drafts.contains_key(&self.group_mode)
        {
            return None;
        }
        Some(if self.organize_running() {
            Pending::Running
        } else if self.organize_halted == Some(self.group_mode) {
            if self.organize_cancelled {
                Pending::Cancelled
            } else {
                Pending::Failed
            }
        } else {
            Pending::Waiting
        })
    }
    /// Choosing an analysis-based grouping starts it, or says why it waits.
    pub(super) fn start_organize_grouping(&mut self) {
        self.organize_halted = None;
        if self.step != Step::Organize || !self.organize_grouping_wanted() {
            return;
        }
        self.start_ready_analysis();
        let mode = self.group_mode;
        if self
            .analysis_run
            .is_some_and(|run| self.organize_run == Some((run, mode)))
            || self.scanning
            || self.media.is_empty()
        {
            return; // Started, or starts once the folder has been read.
        }
        let what = mode.title().to_lowercase();
        let message = if self.model_issue.is_some() || self.model_dialog {
            format!("Grouping by {what} needs AI models · open AI models to set them up")
        } else if self.analysis_run.is_some() || self.rotation_run.is_some() {
            format!("Grouping by {what} starts when the current analysis finishes")
        } else if self.busy || self.plan_open || self.retry_plan.is_some() {
            format!("Grouping by {what} waits for the current task · then press Preview moves")
        } else {
            return; // `analyze` already reported why it could not start.
        };
        self.set_organize_status(mode, message);
    }
    /// What a finished people grouping found; `None` before it has run.
    pub(super) fn face_group_status(&self) -> Option<String> {
        let summary = crate::core::face_groups::summarize(self.face_groups.as_ref()?);
        Some(match summary {
            crate::core::face_groups::Summary { photos: 0, .. } => {
                "No faces found in these photos".into()
            }
            crate::core::face_groups::Summary { repeated: 0, .. } => {
                "Faces found, but no person appears in more than one photo".into()
            }
            crate::core::face_groups::Summary { repeated, .. } => format!(
                "People groups ready · {repeated} {} in more than one photo",
                if repeated == 1 { "person" } else { "people" }
            ),
        })
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
            lines: Vec::new(),
            order: Vec::new(),
            show_other: false,
            other: 0,
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
            folder_hover: None,
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
        modal(egui::Id::new("organize_editor")).show(ctx, |ui| {
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
    /// After a short hover on a destination folder, up to four thumbnails of
    /// the photos going there. Only that folder's thumbnails are requested.
    fn folder_hover_preview(&mut self, ctx: &egui::Context, editor: &Editor) {
        const DELAY: f64 = 0.4;
        const THUMB_EDGE: f32 = 72.0;
        let Some((path, row, since)) = &editor.folder_hover else {
            return;
        };
        let waited = ctx.input(|i| i.time) - since;
        if waited < DELAY {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(DELAY - waited));
            return;
        }
        let going: Vec<MediaId> = editor
            .rows
            .iter()
            .filter(|r| r.after == *path)
            .map(|r| r.id)
            .collect();
        let photos: Vec<MediaId> = going
            .iter()
            .copied()
            .filter(|id| {
                self.index
                    .get(id)
                    .is_some_and(|i| self.media[*i].kind != MediaKind::Video)
            })
            .take(4)
            .collect();
        egui::Area::new(egui::Id::new("folder-hover-preview"))
            .order(egui::Order::Tooltip)
            .fixed_pos(row.right_top() + egui::vec2(S2, 0.0))
            .interactable(false)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(ELEVATED)
                    .stroke(Stroke::new(1.0, EDGE))
                    .corner_radius(R_SURFACE)
                    .inner_margin(S2)
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(format!(
                                "{} · {} {}",
                                folder_title(path),
                                going.len(),
                                if going.len() == 1 { "file" } else { "files" }
                            ))
                            .font(medium(12.5)),
                        );
                        if photos.is_empty() {
                            ui.label(RichText::new("No photos yet").small().color(MUTED));
                            return;
                        }
                        ui.horizontal(|ui| {
                            for id in photos {
                                let (rect, _) = ui.allocate_exact_size(
                                    egui::vec2(THUMB_EDGE, THUMB_EDGE),
                                    Sense::hover(),
                                );
                                ui.painter().rect_filled(rect, R_CONTROL, CARD);
                                if let Some(texture) = self.texture(id, false) {
                                    let size = texture.size_vec2();
                                    let scale = (THUMB_EDGE / size.x).min(THUMB_EDGE / size.y);
                                    let fit =
                                        egui::Rect::from_center_size(rect.center(), size * scale);
                                    ui.painter().image(
                                        texture.id(),
                                        fit,
                                        egui::Rect::from_min_max(
                                            egui::pos2(0.0, 0.0),
                                            egui::pos2(1.0, 1.0),
                                        ),
                                        Color32::WHITE,
                                    );
                                }
                            }
                        });
                    });
            });
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
        modal(egui::Id::new("organize_editor")).show(ctx, |ui| {
            let width = (ctx.content_rect().width() - 64.0).clamp(280.0, 1080.0);
            let height = (ctx.content_rect().height() - 300.0).clamp(160.0, 640.0);
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.set_height(30.0);
                ui.heading("Edit folders");
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    let more = ui.menu_button(egui_phosphor::regular::DOTS_THREE, |ui| {
                        if ui
                            .button("Start over")
                            .on_hover_text("Undo every folder and name edit in this grouping")
                            .clicked()
                        {
                            action = 4;
                            ui.close();
                        }
                        self.clear_trash_menu_item(ui);
                    })
                    .response;
                    named(more, egui::WidgetType::Button, "More").on_hover_text("More");
                    if ui
                        .add_enabled(
                            !self.organize_preview_ids(&editor).is_empty(),
                            egui::Button::new("Compare selected"),
                        )
                        .on_hover_text("Compare up to nine selected photos side by side, without accepting edits")
                        .clicked()
                    {
                        action = 5;
                    }
                });
            });
            ui.label(
                RichText::new(if width >= 680.0 {
                    "Drag photos onto a folder, or folders into each other. Nothing changes on disk until you preview and apply the moves."
                } else {
                    "Drag photos onto folders. Nothing changes until you apply."
                })
                .color(MUTED),
            )
            .on_hover_text("A photo's RAW, JPEG and edit files move and rename together when that is switched on. New empty folders are part of the reviewed plan.");
            ui.add_space(S2);
            if width >= 680.0 {
                ui.columns(2, |columns| {
                    command = editor.photos(&mut columns[0], height, &self.decisions);
                    command = editor.structure(&mut columns[1], height, &self.decisions).or_else(|| command.take());
                });
            } else {
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut editor.compact_folders, false, "Photos");
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
            ui.add_space(S2);
            ui.horizontal(|ui| {
                ui.set_height(button_row_height(ui));
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(primary("Preview moves")).on_hover_text("See every move before anything changes · Ctrl/Cmd+Enter").clicked() { action = 3; }
                    if ui.button("Done").on_hover_text("Keep these edits and go back to the board").clicked() { action = 2; }
                    if named(ui.button("Cancel"), egui::WidgetType::Button, "Cancel edits").on_hover_text("Discard folder and name edits. Trash marks stay staged for review.").clicked() { action = 1; }
                });
            });
        });
        self.folder_hover_preview(ctx, &editor);
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
