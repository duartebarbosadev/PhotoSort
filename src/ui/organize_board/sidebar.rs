//! The folder sidebar: the tree, each folder's menu and hover preview,
//! and the popover it folds into in a narrow window.
use super::*;

/// One row of the folder tree: indent, icon, name and count. With `faces` (grouping by faces) every row keeps a slot
/// wide enough for two faces, so names line up; a person's folder shows their
/// face there, others the folder icon. `hint` shows on hover when it fits.
#[allow(clippy::too_many_arguments)]
pub(in crate::ui::workspace) fn folder_row(
    ui: &mut egui::Ui,
    height: f32,
    title: &str,
    glyph: &str,
    depth: usize,
    count: usize,
    selected: bool,
    draggable: bool,
    faces: Option<&[people::FaceImage]>,
    hint: Option<&str>,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), height),
        if draggable {
            Sense::click_and_drag()
        } else {
            Sense::click()
        },
    );
    let name = format!("{title}  ·  {count}");
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, true, selected, &name)
    });
    let hover = ui
        .ctx()
        .animate_bool_responsive(response.id.with("hover"), response.hovered());
    let painter = ui.painter();
    if selected {
        painter.rect_filled(rect, R_CONTROL, SELECTED);
    } else if hover > 0.0 {
        painter.rect_filled(rect, R_CONTROL, CARD_HOVER.gamma_multiply(hover));
    }
    let x = rect.left() + 8.0 + depth as f32 * 14.0;
    match faces {
        Some(faces) if !faces.is_empty() => {
            use people::{ROW_FACE, ROW_SLOT};
            let slot = egui::Rect::from_min_size(
                egui::pos2(x - 2.0, rect.center().y - ROW_FACE / 2.0),
                egui::vec2(ROW_SLOT, ROW_FACE),
            );
            let behind = if selected {
                SELECTED
            } else {
                lerp_color(SIDEBAR, CARD_HOVER, hover)
            };
            people::paint_faces(painter, slot, faces, behind);
        }
        _ => {
            painter.text(
                egui::pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                glyph,
                FontId::proportional(14.0),
                if selected { ACCENT } else { MUTED },
            );
        }
    }
    let text_x = x + if faces.is_some() {
        people::ROW_SLOT + 4.0
    } else {
        22.0
    };
    let count_galley = painter.layout_no_wrap(count.to_string(), FontId::proportional(10.5), MUTED);
    let mut count_x = rect.right() - 8.0 - count_galley.size().x;
    painter.galley(
        egui::pos2(count_x, rect.center().y - count_galley.size().y / 2.0),
        count_galley,
        MUTED,
    );
    if let Some(hint) = hint.filter(|_| hover > 0.5) {
        let galley = painter.layout_no_wrap(hint.to_owned(), FontId::proportional(10.5), ACCENT);
        let left = count_x - 8.0 - galley.size().x;
        // Only beside a name that still reads in full.
        let title_width = painter
            .layout_no_wrap(title.to_owned(), FontId::proportional(12.0), INK)
            .size()
            .x;
        if left - 8.0 >= text_x + title_width {
            painter.galley(
                egui::pos2(left, rect.center().y - galley.size().y / 2.0),
                galley,
                ACCENT,
            );
            count_x = left;
        }
    }
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_owned(),
        if selected {
            medium(12.0)
        } else {
            FontId::proportional(12.0)
        },
        INK,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((count_x - text_x - 8.0).max(10.0));
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(text_x, rect.center().y - galley.size().y / 2.0),
        galley,
        INK,
    );
    response
}

/// Photos shown in a folder's hover preview.
pub(super) const PREVIEW_THUMBNAILS: usize = 6;

const PREVIEW_TILE: f32 = 56.0;

/// A folder's contents for its hover preview.
#[derive(Default)]
pub(in crate::ui::workspace) struct FolderPreview {
    /// The first photos' thumbnails; `None` while one is loading.
    pub(in crate::ui::workspace) thumbnails: Vec<Option<(egui::TextureId, Vec2)>>,
    pub(in crate::ui::workspace) photos: usize,
    pub(in crate::ui::workspace) videos: usize,
    pub(in crate::ui::workspace) folders: usize,
}

impl FolderPreview {
    /// "12 photos · 2 videos · 3 folders", or "Empty".
    pub(in crate::ui::workspace) fn counts(&self) -> String {
        let parts: Vec<String> = [
            (self.photos, "photo", "photos"),
            (self.videos, "video", "videos"),
            (self.folders, "folder", "folders"),
        ]
        .into_iter()
        .filter(|(n, _, _)| *n > 0)
        .map(|(n, one, many)| format!("{n} {}", if n == 1 { one } else { many }))
        .collect();
        if parts.is_empty() {
            "Empty".into()
        } else {
            parts.join(" · ")
        }
    }
}

/// The pointer has stopped on what it hovers, rather than passing over it:
/// only then does a hover preview ask for thumbnails.
pub(super) fn resting(ui: &egui::Ui) -> bool {
    let still = ui.input(|i| i.pointer.time_since_last_movement()) >= REST_SECONDS;
    if !still {
        // Look again once it has had time to stop.
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f32(REST_SECONDS));
    }
    still
}

const REST_SECONDS: f32 = 0.12;

/// A folder's hover preview: its first photos as small square tiles, three
/// to a row, and what it holds.
pub(super) fn preview_card(ui: &mut egui::Ui, preview: &FolderPreview) {
    let gap = S1;
    for row in preview.thumbnails.chunks(3) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for thumbnail in row {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(PREVIEW_TILE), Sense::hover());
                match thumbnail {
                    Some((texture, size)) => {
                        // The middle square of the photo.
                        let side = size.x.min(size.y).max(1.0);
                        let uv = egui::Rect::from_center_size(
                            egui::pos2(0.5, 0.5),
                            egui::vec2(side / size.x.max(1.0), side / size.y.max(1.0)),
                        );
                        ui.painter().image(*texture, rect, uv, Color32::WHITE);
                    }
                    // Still loading.
                    None => {
                        ui.painter().rect_filled(rect, R_CONTROL, CARD);
                    }
                }
            }
        });
    }
    ui.label(RichText::new(preview.counts()).font(medium(12.0)));
}

impl PhotoSortApp {
    /// The folder panel: every proposed folder with its count. Folders are
    /// made, renamed, moved and removed right here, by right-click and drag.
    fn board_tree(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let editable = self.board_editable();
        // Grouped by faces, rows leave room for a person's face.
        let faces_mode = self.board.people.signature.is_some();
        let mut action: Option<FolderAction> = None;
        ui.horizontal(|ui| {
            ui.label(caption("Folders"));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if named(
                    ui.add_enabled(editable, egui::Button::new(icon::FOLDER_PLUS).frame(false)),
                    egui::WidgetType::Button,
                    "New folder",
                )
                .on_hover_text("New folder · inside the folder you clicked · Ctrl/Cmd+Shift+N")
                .clicked()
                {
                    action = Some(FolderAction::NewInside(self.board_focus_folder()));
                }
            });
        });
        ui.add_space(S1);
        let root = self.board_root_name();
        let rows = self.board.folders.len();
        // Must equal the height `folder_row` allocates.
        let row_height = if faces_mode { people::ROW_HEIGHT } else { 26.0 };
        let mut select: Option<(String, egui::Modifiers)> = None;
        let mut drop: Option<(Vec<MediaId>, String)> = None;
        let mut finish = false;
        // What stays where it is, under the folders; the folders get the rest.
        let note = self.board_note().map(|note| {
            ui.painter().layout(
                note,
                egui::TextStyle::Small.resolve(ui.style()),
                MUTED,
                ui.available_width(),
            )
        });
        let reserve = note.as_ref().map_or(0.0, |g| g.size().y + S3);
        let highlighted = self.board_highlight();
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("organize_folders")
            .auto_shrink([false, false])
            .max_height((ui.available_height() - reserve).max(row_height));
        // Scroll a folder into view once: the one being renamed (a new
        // folder can be anywhere in a long tree), else the highlighted one.
        let editing = self.board.editing.as_ref().map(|(path, _)| path.clone());
        let reveal = if self.board.reveal_editing && editing.is_some() {
            editing.clone()
        } else if self.board.reveal_current {
            self.board.reveal_current = false;
            highlighted.clone()
        } else {
            None
        };
        if let Some(index) =
            reveal.and_then(|target| self.board.folders.iter().position(|f| f.path == target))
        {
            self.board.reveal_editing = false;
            let pitch = row_height + ui.spacing().item_spacing.y;
            let (offset, height) = self.board.tree_view;
            let top = index as f32 * pitch;
            if top < offset {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top + pitch > offset + height {
                scroll = scroll.vertical_scroll_offset(top + pitch - height);
            }
        }
        let output = scroll.show_rows(ui, row_height, rows, |ui, range| {
            for row in range {
                let folder = &self.board.folders[row];
                let path = folder.path.clone();
                let (depth, direct, total) = (folder.depth, folder.direct, folder.total);
                if let Some((editing, name)) = &mut self.board.editing
                    && *editing == path
                {
                    ui.horizontal(|ui| {
                        ui.set_height(row_height);
                        ui.add_space(depth as f32 * 14.0 + 6.0);
                        let mut field = egui::TextEdit::singleline(name)
                            .id_salt(("organize-folder-name", &path))
                            .desired_width(ui.available_width() - 8.0)
                            .show(ui);
                        if std::mem::take(&mut self.board.edit_focus) {
                            field.response.request_focus();
                            field.state.cursor.set_char_range(Some(
                                egui::text::CCursorRange::two(
                                    egui::text::CCursor::new(0),
                                    egui::text::CCursor::new(name.chars().count()),
                                ),
                            ));
                            field.state.store(ui.ctx(), field.response.id);
                        }
                        finish |= field.response.lost_focus();
                        self.board.typing |=
                            field.response.has_focus() || field.response.lost_focus();
                    });
                    continue;
                }
                let title = if path.is_empty() {
                    root.clone()
                } else {
                    leaf(&path).to_owned()
                };
                // The library folder is the top row and shows every
                // item, those staying in place included.
                let top = path.is_empty();
                let selected = self.board_highlighted(&path, highlighted.as_deref());
                let faces = faces_mode.then(|| self.folder_faces(&path, ui.ctx().pixels_per_point()));
                let unnamed = editable && self.unnamed_person(&path);
                let response = folder_row(
                    ui,
                    row_height,
                    &title,
                    if top {
                        icon::HARD_DRIVES
                    } else if total == 0 {
                        icon::FOLDER_DASHED
                    } else {
                        icon::FOLDER_SIMPLE
                    },
                    depth,
                    if top { self.filtered.len() } else { total },
                    selected,
                    editable && !top,
                    faces.as_deref(),
                    unnamed.then_some("Add a name"),
                );
                // A person's folder shows their faces larger on hover.
                let card = if response.hovered() && faces.as_ref().is_some_and(|f| !f.is_empty()) {
                    self.folder_card(&path, ui.ctx().pixels_per_point())
                } else {
                    Vec::new()
                };
                let tip = if top {
                    format!("{root} · every item · {direct} directly in it, {total} in its folders")
                } else if unnamed {
                    format!(
                        "{path} · {total} photos · double-click to add a name; it is used for every folder with this person"
                    )
                } else {
                    format!(
                        "{path} · {direct} here, {total} including subfolders · click to scroll to it · Ctrl/Cmd-click to show only the folders you choose · drag onto another folder to move it"
                    )
                };
                // Any other folder previews what is inside it.
                let preview = (card.is_empty() && response.hovered())
                    .then(|| self.folder_preview(&path, resting(ui)));
                let response = match (&preview, card.is_empty()) {
                    (_, false) => response.on_hover_ui(|ui| {
                        let width = people::face_card(ui, &card);
                        ui.set_max_width(width);
                        ui.label(RichText::new(&tip).small().color(MUTED));
                    }),
                    (Some(preview), true) => response.on_hover_ui(|ui| {
                        preview_card(ui, preview);
                        ui.label(RichText::new(&tip).small().color(MUTED));
                    }),
                    (None, true) => response.on_hover_text(tip),
                };
                if editable && !top && response.drag_started() {
                    response.dnd_set_drag_payload(FolderDrag(path.clone()));
                }
                let photos = response
                    .dnd_hover_payload::<grouping::PhotoDrag>()
                    .is_some();
                // A person's folder dropped on another person's: the same person.
                let same_person = |app: &Self, source: &str| {
                    app.folder_person(source)
                        .zip(app.folder_person(&path))
                        .is_some_and(|(a, b)| a != b)
                };
                let folder_here = response
                    .dnd_hover_payload::<FolderDrag>()
                    .is_some_and(|source| can_nest(&source.0, &path) || same_person(self, &source.0));
                if editable && (photos || folder_here) {
                    ui.painter().rect_stroke(
                        response.rect,
                        R_CONTROL,
                        Stroke::new(1.5, ACCENT),
                        StrokeKind::Inside,
                    );
                }
                // Taking a payload of the other type would drop it.
                if editable
                    && photos
                    && let Some(payload) =
                        response.dnd_release_payload::<grouping::PhotoDrag>()
                {
                    drop = Some((payload.0.clone(), path.clone()));
                }
                if editable
                    && egui::DragAndDrop::has_payload_of_type::<FolderDrag>(ui.ctx())
                    && let Some(source) = response.dnd_release_payload::<FolderDrag>()
                {
                    if same_person(self, &source.0) {
                        action = Some(FolderAction::SamePerson(source.0.clone(), path.clone()));
                    } else if can_nest(&source.0, &path) {
                        action = Some(FolderAction::Nest(source.0.clone(), path.clone()));
                    }
                }
                if response.clicked() {
                    select = Some((path.clone(), ui.input(|i| i.modifiers)));
                    // Arrow keys then walk this folder's photos instead
                    // of moving between folder rows.
                    ui.memory_mut(|m| m.surrender_focus(response.id));
                }
                if response.double_clicked() && editable && !top {
                    action = Some(FolderAction::Rename(path.clone()));
                }
                response.context_menu(|ui| {
                    if let Some(chosen) = self.board_folder_menu(ui, &path) {
                        action = Some(chosen);
                    }
                });
            }
        });
        self.board.tree_view = (output.state.offset.y, output.inner_rect.height());
        if let Some(note) = note {
            ui.add_space(S2);
            ui.add(egui::Label::new(RichText::new(note.text()).small().color(MUTED)).wrap());
        }
        if let Some((path, modifiers)) = select {
            self.board_choose_folder(&path, modifiers);
        }
        if let Some((ids, target)) = drop {
            self.board_move(&ids, &target);
        }
        // Enter or clicking elsewhere keeps the name; Escape keeps the old one.
        if finish {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.board.editing = None;
            } else {
                self.board_finish_edit();
            }
        }
        if let Some(action) = action {
            self.board_folder_action(action);
        }
    }
    /// A folder's right-click menu.
    fn board_folder_menu(&mut self, ui: &mut egui::Ui, path: &str) -> Option<FolderAction> {
        let mut action = None;
        let editable = self.board_editable();
        let top = path.is_empty();
        let mut choose = |ui: &mut egui::Ui, enabled: bool, label: &str, chosen: FolderAction| {
            if ui.add_enabled(enabled, egui::Button::new(label)).clicked() {
                action = Some(chosen);
                ui.close();
            }
        };
        let only = !top && self.board.focus != [path];
        if only {
            choose(
                ui,
                true,
                "Show only this folder",
                FolderAction::ShowOnly(Some(path.to_owned())),
            );
        }
        if !self.board.focus.is_empty() {
            choose(ui, true, "Show every folder", FolderAction::ShowOnly(None));
        }
        if only || !self.board.focus.is_empty() {
            ui.separator();
        }
        choose(
            ui,
            editable,
            "New folder inside",
            FolderAction::NewInside(path.to_owned()),
        );
        if !top {
            choose(
                ui,
                editable,
                "Rename · double-click",
                FolderAction::Rename(path.to_owned()),
            );
            // Two people the grouping told apart may be one.
            let same = self.same_person_choices(path);
            if !same.is_empty() {
                ui.add_enabled_ui(editable, |ui| {
                    ui.menu_button("Same person as…", |ui| {
                        for other in same {
                            choose(
                                ui,
                                true,
                                leaf(&other),
                                FolderAction::SamePerson(path.to_owned(), other.clone()),
                            );
                        }
                    })
                    .response
                    .on_hover_text("Combine their folders · or drag this folder onto the other");
                });
            }
        }
        let selected = self.selection.len();
        if selected > 0 && self.library_selection_mode() {
            choose(
                ui,
                editable,
                &format!("Move {selected} selected here"),
                FolderAction::MoveSelected(path.to_owned()),
            );
        }
        if !top {
            let targets: Vec<String> = self
                .board
                .folders
                .iter()
                .map(|f| f.path.clone())
                .filter(|target| !target.is_empty() && can_nest(path, target))
                .collect();
            ui.add_enabled_ui(editable && !targets.is_empty(), |ui| {
                ui.menu_button("Move into", |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for target in targets {
                                if ui.button(target.replace('/', " / ")).clicked() {
                                    action = Some(FolderAction::Nest(path.to_owned(), target));
                                    ui.close();
                                }
                            }
                        });
                });
            });
            if path.contains('/')
                && ui
                    .add_enabled(editable, egui::Button::new("Move to the top level"))
                    .clicked()
            {
                action = Some(FolderAction::Nest(path.to_owned(), String::new()));
                ui.close();
            }
            if ui
                .add_enabled(editable, egui::Button::new("Put inside a new folder"))
                .on_hover_text("Makes a folder in the same place and moves this one into it")
                .clicked()
            {
                action = Some(FolderAction::Wrap(path.to_owned()));
                ui.close();
            }
        }
        if let Some(folder) = self.board_folder_on_disk(path) {
            ui.separator();
            let label = crate::core::desktop::Action::Reveal.label();
            if ui
                .add_enabled(self.desktop_available(), egui::Button::new(label))
                .clicked()
            {
                action = Some(FolderAction::Reveal(folder));
                ui.close();
            }
        }
        let ids = self.board_folder_items(path);
        let new = self
            .organize_drafts
            .get(&self.group_mode)
            .is_some_and(|d| d.is_new_folder(path));
        let library = self.board_library_folder(path);
        if !ids.is_empty() || new || library.is_some() {
            ui.separator();
        }
        if !ids.is_empty() {
            let marked = ids.iter().all(|id| self.decision(*id).rejected);
            let count = ids.len();
            if ui
                .add_enabled(
                    !self.busy,
                    egui::Button::new(if marked {
                        format!("Unmark {count} for Trash")
                    } else {
                        format!("Mark {count} for Trash")
                    }),
                )
                .on_hover_text(
                    "Marks every photo and video in this folder and its subfolders. Nothing goes to Trash until you review the changes.",
                )
                .clicked()
            {
                action = Some(FolderAction::Trash(path.to_owned()));
                ui.close();
            }
        }
        if let Some(folder) = library
            && ui
                .add_enabled(
                    !self.busy && !self.scanning,
                    egui::Button::new("Move folder to Trash…"),
                )
                .on_hover_text(
                    "Shows the whole folder, every file and empty folder in it, for review before anything goes to Trash",
                )
                .clicked()
        {
            action = Some(FolderAction::TrashFolder(folder));
            ui.close();
        }
        if new
            && ui
                .add_enabled(editable, egui::Button::new("Remove folder"))
                .on_hover_text("Removes this new folder from the plan. Its photos go back to where they were going.")
                .clicked()
        {
            action = Some(FolderAction::Remove(path.to_owned()));
            ui.close();
        }
        if top && self.organize_drafts.contains_key(&self.group_mode) {
            ui.separator();
            if ui
                .add_enabled(editable, egui::Button::new("Start over"))
                .on_hover_text("Undo every folder edit in this grouping")
                .clicked()
            {
                action = Some(FolderAction::StartOver);
                ui.close();
            }
        }
        action
    }
    /// What a folder holds, for its hover preview: its first photos and the
    /// counts of photos, videos and folders in it. With `load`, thumbnails
    /// not loaded yet are asked for through the usual thumbnail queue, which
    /// keeps only a few in flight, so sweeping the pointer over the tree
    /// cannot flood the decoder; until they arrive they are placeholders, and
    /// once loaded they stay cached for the next hover.
    pub(in crate::ui::workspace) fn folder_preview(
        &mut self,
        path: &str,
        load: bool,
    ) -> FolderPreview {
        let mut preview = FolderPreview::default();
        let mut first = Vec::new();
        for section in &self.board.sections {
            if !in_folder(path, section.path.as_deref()) {
                continue;
            }
            preview.videos += section.videos;
            preview.photos += section.range.len() - section.videos;
            first.extend(
                self.filtered[section.range.clone()]
                    .iter()
                    .map(|&index| &self.media[index])
                    .filter(|m| m.kind != MediaKind::Video)
                    .map(|m| m.id)
                    .take(PREVIEW_THUMBNAILS - first.len()),
            );
        }
        preview.thumbnails = first
            .into_iter()
            .map(|id| {
                let texture = if load {
                    self.texture(id, false)
                } else {
                    self.textures.get(&(id, false)).map(|t| t.handle.clone())
                };
                texture.map(|t| (t.id(), t.size_vec2()))
            })
            .collect();
        let depth = if path.is_empty() {
            0
        } else {
            path.matches('/').count() + 1
        };
        preview.folders = self
            .board
            .folders
            .iter()
            .filter(|f| f.depth == depth + 1 && within(&f.path, path))
            .count();
        preview
    }
    /// The board header's button for the folders when the sidebar does not
    /// fit: they open in a popover below it, as Cull's navigator does.
    pub(super) fn board_folders_button(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let response = named(
            ui.add(
                egui::Button::new(RichText::new(icon::FOLDERS).color(INK))
                    .selected(self.board.folders_open)
                    .min_size(egui::vec2(28.0, 0.0)),
            ),
            egui::WidgetType::Button,
            "Folders",
        )
        .on_hover_text("Folders · drag photos onto one to move them");
        let mut open = self.board.folders_open;
        if response.clicked() {
            open = !open;
        }
        let width = 280.0_f32.min(ui.ctx().content_rect().width() - 32.0);
        let height = (ui.ctx().content_rect().height() - 200.0).clamp(160.0, 480.0);
        egui::Popup::from_response(&response)
            .open_bool(&mut open)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .width(width)
            .show(|ui| {
                ui.set_width(width);
                ui.set_height(height);
                if self.busy {
                    ui.disable();
                }
                self.board_tree(ui);
            });
        self.board.folders_open = open;
    }
    /// Organize's folder sidebar, flush with the window's left edge from the
    /// top of the window to the page bar, on its own warm grey like a desktop app's
    /// sidebar. Drawn before the middle of the window; the board draws it
    /// itself when it was not. `room` is the width for the sidebar, board and
    /// preview together: when the sidebar does not fit beside a usable board,
    /// the board's header offers the folders in a popover instead.
    pub(in crate::ui::workspace) fn organize_sidebar(&mut self, ui: &mut egui::Ui, room: f32) {
        // Holding "Show original" or backslash shows the folders on disk now.
        let key = ui.input(|i| i.focused && i.key_down(egui::Key::Backslash))
            && !ui.ctx().egui_wants_keyboard_input()
            && !self.modal_active();
        // Nothing to compare with while the plan is the folders on disk.
        self.show_original((key || self.board.original_button) && self.organize_planned());
        self.board.typing = false;
        // A step switch earlier in this frame (the step tabs, a shortcut)
        // leaves `filtered` holding the previous step's photos, and the
        // sections from the last Organize visit, until the next refilter.
        // Sections always cover `filtered` exactly; rebuild both before drawing.
        if self.filter_dirty
            || self.board.key.is_none()
            || self.board.sections.last().map_or(0, |s| s.range.end) != self.filtered.len()
        {
            self.filter_dirty = true;
            self.refilter();
        }
        self.update_people(ui.ctx());
        self.board.sidebar_frame = Some(ui.ctx().cumulative_frame_nr());
        let sidebar = egui::PanelState::load(ui.ctx(), egui::Id::new("organize_folder_panel"))
            .map_or(240.0, |panel| panel.size().x);
        self.board.layout = board_layout(room, sidebar);
        if self.board.layout.preview {
            self.board.preview_open = false;
        }
        if !self.board.layout.sidebar {
            return;
        }
        self.board.folders_open = false;
        egui::Panel::left("organize_folder_panel")
            .resizable(true)
            .show_separator_line(false)
            .default_size(240.0)
            .size_range(180.0..=380.0)
            .frame(egui::Frame::new().fill(SIDEBAR).inner_margin(egui::Margin {
                left: S2 as i8,
                right: S2 as i8,
                top: S3 as i8,
                bottom: S2 as i8,
            }))
            .show(ui, |ui| {
                self.sidebar_title_band(ui, S3);
                if self.busy {
                    ui.disable();
                }
                self.board_tree(ui);
            });
    }
}
