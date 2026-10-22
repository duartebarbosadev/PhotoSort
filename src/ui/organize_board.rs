//! Organize's main view: the proposed folders on the left, the photos and
//! videos grouped under their destination folder in the middle. Photos can be
//! dragged onto folders; folders can be created and renamed in place. Every
//! edit goes into the same folder draft as the folder editor, and nothing
//! moves on disk until the planned moves are previewed and applied.
use super::*;
use crate::core::organize::Draft;

/// What the board was arranged from; a change rearranges it.
#[derive(Clone, PartialEq)]
pub(super) struct BoardKey {
    mode: GroupMode,
    epoch: u64,
    media: usize,
    metadata: usize,
    draft: Option<(usize, usize)>,
    editor: bool,
    grouped: [bool; 3],
    layout: crate::core::geography::Layout,
}
/// One destination folder's run of photos in `filtered` order.
pub(super) struct Section {
    /// Destination relative to the output folder; `None` stays in place.
    pub(super) path: Option<String>,
    pub(super) range: std::ops::Range<usize>,
    pub(super) videos: usize,
}
/// A row of the folder tree.
pub(super) struct FolderRow {
    pub(super) path: String,
    pub(super) depth: usize,
    /// Photos and videos directly in this folder, and including subfolders.
    pub(super) direct: usize,
    pub(super) total: usize,
}
#[derive(Default)]
pub(super) struct Board {
    pub(super) key: Option<BoardKey>,
    pub(super) sections: Vec<Section>,
    pub(super) folders: Vec<FolderRow>,
    /// Photos that stay where they are (not part of any proposed folder).
    pub(super) unplaced: usize,
    /// The folder shown in the middle; `None` shows every folder.
    pub(super) focus: Option<String>,
    /// A folder being renamed or created in place: (folder, typed name, new).
    pub(super) editing: Option<(String, String, bool)>,
    pub(super) edit_focus: bool,
    /// Thumbnail width chosen with the size slider.
    pub(super) tile: Option<f32>,
}

const HEADER: f32 = 44.0;
const TILE: f32 = 160.0;

fn leaf(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
fn within(path: &str, folder: &str) -> bool {
    folder.is_empty()
        || path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

impl PhotoSortApp {
    pub(super) fn board_key(&self) -> BoardKey {
        BoardKey {
            mode: self.group_mode,
            epoch: self.board_epoch,
            media: self.media.len(),
            // Only metadata-based modes regroup as metadata arrives; elsewhere
            // arriving thumbnails and metadata must not reshuffle the tiles.
            metadata: if matches!(
                self.group_mode,
                GroupMode::Date | GroupMode::Camera | GroupMode::Location
            ) {
                self.metadata.len()
            } else {
                0
            },
            draft: self
                .organize_drafts
                .get(&self.group_mode)
                .map(|d| (d.labels().len(), d.folders().len())),
            editor: self.organize_editor.is_some(),
            grouped: [
                self.visual_groups.is_some(),
                self.mixed_labels.is_some(),
                self.face_groups.is_some(),
            ],
            layout: self.preferences.location_layout,
        }
    }
    /// Rearranges Organize when what it shows has changed.
    pub(super) fn check_board(&mut self) {
        if self.step == Step::Organize && self.board.key.as_ref() != Some(&self.board_key()) {
            self.filter_dirty = true;
        }
    }
    /// The output folder's own name, for the top of the tree.
    fn board_root_name(&self) -> String {
        std::path::Path::new(&self.output)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Destination".into())
    }
    /// Sorts `filtered` by destination folder, then capture time, and
    /// records the folder sections and tree.
    pub(super) fn arrange_board(&mut self) {
        let key = self.board_key();
        let draft = self.organize_drafts.get(&self.group_mode);
        let generated;
        let labels = match draft {
            Some(draft) => draft.labels(),
            None => {
                generated = self.organize_labels();
                &generated
            }
        };
        let mut keyed: Vec<_> = self
            .filtered
            .iter()
            .map(|&i| {
                let media = &self.media[i];
                let path = labels.get(&media.id).cloned();
                let order = crate::core::dates::capture_order(media, self.metadata.get(&media.id));
                (path.is_none(), path, order, i)
            })
            .collect();
        keyed.sort();
        self.filtered = keyed.iter().map(|k| k.3).collect();
        let mut sections: Vec<Section> = Vec::new();
        for (position, (_, path, _, index)) in keyed.iter().enumerate() {
            let video = usize::from(self.media[*index].kind == MediaKind::Video);
            match sections.last_mut() {
                Some(section) if section.path == *path => {
                    section.range.end = position + 1;
                    section.videos += video;
                }
                _ => sections.push(Section {
                    path: path.clone(),
                    range: position..position + 1,
                    videos: video,
                }),
            }
        }
        // The tree: the draft's folders (which keep empty ones), or every
        // proposed folder and its parents.
        let mut paths: std::collections::BTreeSet<String> = match draft {
            Some(draft) => draft.folders().clone(),
            None => labels.values().cloned().collect(),
        };
        for path in paths.clone() {
            let mut parent = path.as_str();
            while let Some((prefix, _)) = parent.rsplit_once('/') {
                paths.insert(prefix.to_owned());
                parent = prefix;
            }
        }
        paths.insert(String::new());
        let direct: HashMap<&str, usize> = sections
            .iter()
            .filter_map(|s| Some((s.path.as_deref()?, s.range.len())))
            .collect();
        let folders: Vec<FolderRow> = paths
            .iter()
            .map(|path| FolderRow {
                path: path.clone(),
                depth: if path.is_empty() {
                    0
                } else {
                    path.matches('/').count() + 1
                },
                direct: direct.get(path.as_str()).copied().unwrap_or(0),
                total: direct
                    .iter()
                    .filter(|(p, _)| within(p, path))
                    .map(|(_, n)| n)
                    .sum(),
            })
            .collect();
        // A generated grouping lists the folders it fills, not the library's
        // existing folders it empties; folders made on the board stay.
        let folders = if self.group_mode == GroupMode::Current {
            folders
        } else {
            let created = draft.map(|d| d.created_folders());
            folders
                .into_iter()
                .filter(|f| {
                    f.path.is_empty()
                        || f.total > 0
                        || created.is_some_and(|c| c.iter().any(|p| within(p, &f.path)))
                })
                .collect()
        };
        self.board.unplaced = sections
            .iter()
            .filter(|s| s.path.is_none())
            .map(|s| s.range.len())
            .sum();
        self.board.sections = sections;
        self.board.folders = folders;
        self.board.key = Some(key);
        if let Some(focus) = &self.board.focus
            && !self.board.folders.iter().any(|f| &f.path == focus)
        {
            self.board.focus = None;
        }
    }
    /// The folder draft to edit, created from the current proposal when the
    /// first edit is made, as the folder editor does.
    fn board_draft(&mut self) -> Option<&mut Draft> {
        if !self.can_edit_folders() || self.organize_editor.is_some() {
            return None;
        }
        if !self.organize_drafts.contains_key(&self.group_mode) {
            let draft = Draft::from_catalog(self.catalog.as_ref()?, self.organize_labels());
            self.organize_drafts.insert(self.group_mode, draft);
        }
        self.dirty = Some(Instant::now());
        self.board_epoch += 1;
        self.organize_drafts.get_mut(&self.group_mode)
    }
    /// Moves photos into a proposed folder.
    pub(super) fn board_move(&mut self, ids: &[MediaId], target: &str) {
        // Items outside the current proposal (videos and ungrouped photos in
        // the analysis modes) stay where they are and cannot be moved.
        let movable: Vec<MediaId> = {
            let generated;
            let labels = match self.organize_drafts.get(&self.group_mode) {
                Some(draft) => draft.labels(),
                None => {
                    generated = self.organize_labels();
                    &generated
                }
            };
            ids.iter()
                .copied()
                .filter(|id| labels.contains_key(id))
                .collect()
        };
        let staying = ids.len() - movable.len();
        if movable.is_empty() {
            self.navigation_notice = Some(format!(
                "{} {} where {} in this grouping and cannot be moved",
                staying,
                if staying == 1 {
                    "item stays"
                } else {
                    "items stay"
                },
                if staying == 1 { "it is" } else { "they are" }
            ));
            return;
        }
        let Some(draft) = self.board_draft() else {
            return;
        };
        match draft.move_photos(&movable, target) {
            Ok(()) => {
                let title = if target.is_empty() {
                    self.board_root_name()
                } else {
                    target.to_owned()
                };
                let mut notice = format!(
                    "Moved {} {} to {title}",
                    movable.len(),
                    if movable.len() == 1 { "item" } else { "items" }
                );
                if staying > 0 {
                    notice.push_str(&format!(" · {staying} stay where they are"));
                }
                notice.push_str(" · nothing changes on disk until you preview and apply");
                self.navigation_notice = Some(notice);
            }
            Err(error) => self.navigation_notice = Some(error.to_string()),
        }
    }
    fn board_finish_edit(&mut self) {
        let Some((path, name, new)) = self.board.editing.take() else {
            return;
        };
        let name = name.trim().to_owned();
        if name.is_empty() {
            return;
        }
        let Some(draft) = self.board_draft() else {
            return;
        };
        let result = if new {
            draft.create_folder(&path, &name)
        } else {
            draft.rename_folder(&path, &name)
        };
        match result {
            Ok(path) => {
                if !new && self.board.focus.is_some() {
                    self.board.focus = Some(path);
                }
            }
            Err(error) => self.navigation_notice = Some(error.to_string()),
        }
    }
    /// Two to nine selected photos on the board, in board order: the preview
    /// compares them side by side. Larger selections preview the active photo.
    pub(super) fn board_selection_comparison(&self) -> Option<Vec<MediaId>> {
        if self.step != Step::Organize
            || !self.library_selection_mode()
            || !(2..=9).contains(&self.selection.len())
        {
            return None;
        }
        // Called several times a frame: one pass that stops at the last
        // selected tile, without hashing every shown photo's id.
        let wanted: Vec<usize> = self
            .selection
            .snapshot()
            .0
            .iter()
            .filter_map(|id| self.index.get(id).copied())
            .collect();
        let mut shown = Vec::with_capacity(wanted.len());
        for &index in &self.filtered {
            if wanted.contains(&index) {
                shown.push(self.media[index].id);
                if shown.len() == wanted.len() {
                    break;
                }
            }
        }
        (shown.len() > 1).then_some(shown)
    }
    /// Back to the single preview: keeps only the active photo selected.
    pub(super) fn end_board_comparison(&mut self) {
        self.comparison.clear();
        if let Some(active) = self.active {
            self.selection.single(active);
        }
        self.focused_slot = 0;
    }
    /// The photos a drag or a menu acts on: the selection when `id` is in it.
    fn board_targets(&self, id: MediaId) -> Vec<MediaId> {
        if self.selection.contains(id) {
            self.selection.ordered(&self.media, &self.filtered)
        } else {
            vec![id]
        }
    }
    /// Move-to menu items for photo tiles.
    pub(super) fn board_move_menu(&mut self, ui: &mut egui::Ui, id: MediaId) {
        let ids = self.board_targets(id);
        let enabled = self.can_edit_folders() && self.organize_editor.is_none();
        ui.add_enabled_ui(enabled, |ui| {
            ui.menu_button(
                format!(
                    "Move {} to folder",
                    if ids.len() == 1 {
                        "item".to_owned()
                    } else {
                        format!("{} items", ids.len())
                    }
                ),
                |ui| {
                    let folders: Vec<String> =
                        self.board.folders.iter().map(|f| f.path.clone()).collect();
                    let root = self.board_root_name();
                    egui::ScrollArea::vertical()
                        .max_height(320.0)
                        .show(ui, |ui| {
                            for path in folders {
                                let label = if path.is_empty() {
                                    root.clone()
                                } else {
                                    path.clone()
                                };
                                if ui.button(label).clicked() {
                                    self.board_move(&ids, &path);
                                    ui.close();
                                }
                            }
                        });
                },
            );
            if ui.button("New folder from selection…").clicked() {
                let parent = self.board.focus.clone().unwrap_or_default();
                let name = (1..)
                    .map(|n| {
                        if n == 1 {
                            "New folder".to_owned()
                        } else {
                            format!("New folder {n}")
                        }
                    })
                    .find(|name| {
                        let path = if parent.is_empty() {
                            name.clone()
                        } else {
                            format!("{parent}/{name}")
                        };
                        // Hidden empty folders still take their names.
                        !self.board.folders.iter().any(|f| f.path == path)
                            && !self
                                .organize_drafts
                                .get(&self.group_mode)
                                .is_some_and(|d| d.folders().contains(&path))
                    })
                    .unwrap_or_default();
                if let Some(draft) = self.board_draft()
                    && let Ok(path) = draft.create_folder(&parent, &name)
                {
                    self.board_move(&ids, &path);
                    self.board.editing = Some((path, name, false));
                    self.board.edit_focus = true;
                }
                ui.close();
            }
        });
    }

    /// The folder panel: every proposed folder with its count.
    fn board_tree(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let editable = self.can_edit_folders() && self.organize_editor.is_none();
        ui.horizontal(|ui| {
            ui.label(caption("Folders"));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if named(
                    ui.add_enabled(editable, egui::Button::new(icon::FOLDER_PLUS).frame(false)),
                    egui::WidgetType::Button,
                    "New folder",
                )
                .on_hover_text("New folder · inside the folder shown")
                .clicked()
                {
                    let parent = self.board.focus.clone().unwrap_or_default();
                    self.board.editing = Some((parent, String::new(), true));
                    self.board.edit_focus = true;
                }
            });
        });
        ui.add_space(S1);
        let root = self.board_root_name();
        let rows = self.board.folders.len();
        // Must equal the height `folder_row` allocates.
        let row_height = 26.0;
        let mut select: Option<Option<String>> = None;
        let mut drop: Option<(Vec<MediaId>, String)> = None;
        let mut finish = false;
        egui::ScrollArea::vertical()
            .id_salt("organize_folders")
            .auto_shrink([false, false])
            .show_rows(ui, row_height, rows + 2, |ui, range| {
                for row in range {
                    if row == 0 {
                        // Every folder at once.
                        let selected = self.board.focus.is_none();
                        let total = self.filtered.len();
                        if folder_row(ui, "All folders", icon::SQUARES_FOUR, 0, total, selected)
                            .clicked()
                        {
                            select = Some(None);
                        }
                        continue;
                    }
                    if row == rows + 1 {
                        if let Some((parent, name, true)) = &mut self.board.editing {
                            let depth = if parent.is_empty() {
                                1
                            } else {
                                parent.matches('/').count() + 2
                            };
                            let parent = parent.clone();
                            ui.horizontal(|ui| {
                                ui.add_space(depth as f32 * 14.0 + 6.0);
                                let response = ui.add(
                                    egui::TextEdit::singleline(name)
                                        .hint_text(format!(
                                            "New folder in {}",
                                            if parent.is_empty() { &root } else { &parent }
                                        ))
                                        .desired_width(ui.available_width() - 8.0),
                                );
                                if std::mem::take(&mut self.board.edit_focus) {
                                    response.request_focus();
                                }
                                finish |= response.lost_focus();
                            });
                        } else if self.board.unplaced > 0 {
                            ui.label(
                                RichText::new(format!(
                                    "{} {} stay where they are",
                                    self.board.unplaced,
                                    if self.board.unplaced == 1 {
                                        "item"
                                    } else {
                                        "items"
                                    }
                                ))
                                .small()
                                .color(MUTED),
                            );
                        }
                        continue;
                    }
                    let folder = &self.board.folders[row - 1];
                    let path = folder.path.clone();
                    let (depth, direct, total) = (folder.depth, folder.direct, folder.total);
                    if let Some((editing, name, false)) = &mut self.board.editing
                        && *editing == path
                    {
                        ui.horizontal(|ui| {
                            ui.add_space(depth as f32 * 14.0 + 6.0);
                            let response = ui.add(
                                egui::TextEdit::singleline(name)
                                    .desired_width(ui.available_width() - 8.0),
                            );
                            if std::mem::take(&mut self.board.edit_focus) {
                                response.request_focus();
                            }
                            finish |= response.lost_focus();
                        });
                        continue;
                    }
                    let title = if path.is_empty() {
                        root.clone()
                    } else {
                        leaf(&path).to_owned()
                    };
                    let selected = self.board.focus.as_deref() == Some(path.as_str());
                    let response = folder_row(
                        ui,
                        &title,
                        if path.is_empty() {
                            icon::HARD_DRIVES
                        } else if total == 0 {
                            icon::FOLDER_DASHED
                        } else {
                            icon::FOLDER_SIMPLE
                        },
                        depth,
                        total,
                        selected,
                    )
                    .on_hover_text(format!(
                        "{} · {direct} here, {total} including subfolders",
                        if path.is_empty() { &root } else { &path }
                    ));
                    if editable
                        && response
                            .dnd_hover_payload::<super::grouping::PhotoDrag>()
                            .is_some()
                    {
                        ui.painter().rect_stroke(
                            response.rect,
                            R_CONTROL,
                            Stroke::new(1.5, ACCENT),
                            StrokeKind::Inside,
                        );
                    }
                    if editable
                        && let Some(payload) =
                            response.dnd_release_payload::<super::grouping::PhotoDrag>()
                    {
                        drop = Some((payload.0.clone(), path.clone()));
                    }
                    if response.clicked() {
                        select = Some(Some(path.clone()));
                    }
                    if response.double_clicked() && editable && !path.is_empty() {
                        self.board.editing = Some((path.clone(), leaf(&path).to_owned(), false));
                        self.board.edit_focus = true;
                    }
                    response.context_menu(|ui| {
                        ui.add_enabled_ui(editable, |ui| {
                            if ui.button("New folder inside").clicked() {
                                self.board.editing = Some((path.clone(), String::new(), true));
                                self.board.edit_focus = true;
                                ui.close();
                            }
                            if !path.is_empty() && ui.button("Rename · double-click").clicked() {
                                self.board.editing =
                                    Some((path.clone(), leaf(&path).to_owned(), false));
                                self.board.edit_focus = true;
                                ui.close();
                            }
                            let selected = self.selection.ordered(&self.media, &self.filtered);
                            if !selected.is_empty()
                                && ui
                                    .button(format!("Move {} selected here", selected.len()))
                                    .clicked()
                            {
                                self.board_move(&selected, &path);
                                ui.close();
                            }
                        });
                        ui.separator();
                        if ui.button("Open folder editor…").clicked() {
                            self.open_organize_editor();
                            ui.close();
                        }
                    });
                }
            });
        if let Some(focus) = select {
            self.board.focus = focus;
            self.library_reveal = false;
            ui.ctx()
                .data_mut(|d| d.insert_temp(egui::Id::new("organize-board-top"), true));
        }
        if let Some((ids, target)) = drop {
            self.board_move(&ids, &target);
        }
        // Enter or clicking elsewhere keeps the name; Escape abandons it.
        if finish {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.board.editing = None;
            } else {
                self.board_finish_edit();
            }
        }
    }

    /// The preview panel: the clicked photo (or the compared selection) at a
    /// useful size, with its name, folder and the usual mark, rating and zoom
    /// controls. Scroll to zoom, drag to pan, double-click for 1:1.
    fn board_preview(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let ids = self.comparison_ids();
        let Some(active) = self.active.filter(|id| self.index.contains_key(id)) else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Click a photo to preview it here").color(MUTED));
            });
            return;
        };
        let media = self.media[self.index[&active]].clone();
        let destination = self
            .organize_drafts
            .get(&self.group_mode)
            .and_then(|draft| draft.labels().get(&active).cloned())
            .or_else(|| {
                self.board
                    .sections
                    .iter()
                    .find(|s| {
                        self.index.get(&active).is_some_and(|i| {
                            self.filtered
                                .get(s.range.clone())
                                .is_some_and(|shown| shown.contains(i))
                        })
                    })
                    .and_then(|s| s.path.clone())
            });
        ui.horizontal(|ui| {
            ui.add(egui::Label::new(RichText::new(media.name()).font(semibold(13.0))).truncate())
                .on_hover_text(media.path.display().to_string());
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ids.len() > 1 {
                    if ui
                        .small_button("End comparison")
                        .on_hover_text("Show only this photo · Esc")
                        .clicked()
                    {
                        self.end_board_comparison();
                    }
                } else if self.library_selection_mode() && self.selection.len() > 9 {
                    ui.label(
                        RichText::new("Select 2–9 photos to compare")
                            .small()
                            .color(MUTED),
                    );
                }
            });
        });
        let root = self.board_root_name();
        let folder = match destination.as_deref() {
            None => "Stays in place".to_owned(),
            Some("") => root,
            Some(path) => format!("{root} / {}", path.replace('/', " / ")),
        };
        ui.label(
            RichText::new(format!("{}  {folder}", icon::FOLDER_SIMPLE))
                .small()
                .color(MUTED),
        );
        ui.add_space(S1);
        let controls = 34.0;
        let size = egui::vec2(
            ui.available_width(),
            (ui.available_height() - controls).max(80.0),
        );
        let ids = if ids.is_empty() { vec![active] } else { ids };
        let gap = ui.spacing().item_spacing;
        let columns = self.preview_columns(&ids, size, gap);
        let rows = ids.len().div_ceil(columns);
        let cell = egui::vec2(
            (size.x - gap.x * (columns - 1) as f32) / columns as f32,
            ((size.y - gap.y * (rows - 1) as f32) / rows as f32).max(60.0),
        );
        ui.allocate_ui(size, |ui| {
            for row in 0..rows {
                ui.horizontal(|ui| {
                    for column in 0..columns {
                        let slot = row * columns + column;
                        if let Some(id) = ids.get(slot) {
                            self.photo_canvas(ui, *id, slot as u64, cell, true);
                        }
                    }
                });
            }
        });
        ui.horizontal_centered(|ui| {
            let rejected = self.decision(active).rejected;
            if icon_text_button(
                ui,
                true,
                rejected,
                icon::TRASH,
                if rejected {
                    "Marked for Trash"
                } else {
                    "Trash"
                },
            )
            .on_hover_text(if rejected {
                "Unmark · D"
            } else {
                "Mark for Trash · D"
            })
            .clicked()
            {
                self.reject(active);
            }
            self.review_ratings(ui, active);
        });
    }
    /// Columns that show compared photos largest: their typical shape decides
    /// between side by side and stacked in the panel's shape.
    fn preview_columns(&self, ids: &[MediaId], size: Vec2, gap: Vec2) -> usize {
        let aspects: Vec<f32> = ids
            .iter()
            .filter_map(|id| self.metadata.get(id))
            .filter(|m| m.width > 0 && m.height > 0)
            .map(|m| {
                let (w, h) = if (5..=8).contains(&m.orientation) {
                    (m.height, m.width)
                } else {
                    (m.width, m.height)
                };
                w as f32 / h as f32
            })
            .collect();
        let aspect = if aspects.is_empty() {
            1.5
        } else {
            aspects.iter().sum::<f32>() / aspects.len() as f32
        };
        (1..=ids.len().max(1))
            .max_by(|a, b| {
                let width = |columns: usize| {
                    let rows = ids.len().div_ceil(columns);
                    let w = (size.x - gap.x * (columns - 1) as f32) / columns as f32;
                    let h = (size.y - gap.y * (rows - 1) as f32) / rows as f32;
                    w.min(h * aspect)
                };
                // Ties keep fewer columns.
                width(*a).total_cmp(&width(*b)).then(b.cmp(a))
            })
            .unwrap_or(1)
    }
    /// Above the photos: where you are, what it holds, and thumbnail size.
    fn board_summary(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let root = self.board_root_name();
        let focus = self.board.focus.clone();
        let (folders, items) = match &focus {
            None => (
                self.board
                    .sections
                    .iter()
                    .filter(|s| s.path.is_some())
                    .count(),
                self.filtered.len(),
            ),
            Some(focus) => {
                let shown: Vec<_> = self
                    .board
                    .sections
                    .iter()
                    .filter(|s| s.path.as_deref().is_some_and(|p| within(p, focus)))
                    .collect();
                (shown.len(), shown.iter().map(|s| s.range.len()).sum())
            }
        };
        ui.horizontal(|ui| {
            ui.set_height(28.0);
            if focus.is_some()
                && named(
                    ui.small_button(icon::ARROW_LEFT),
                    egui::WidgetType::Button,
                    "Back to all folders",
                )
                .on_hover_text("All folders")
                .clicked()
            {
                self.board.focus = None;
            }
            let title = match &focus {
                None => "All folders".to_owned(),
                Some(path) if path.is_empty() => root.clone(),
                Some(path) => format!("{root} / {}", path.replace('/', " / ")),
            };
            ui.label(RichText::new(title).font(semibold(13.0)));
            let mut summary = format!(
                "{folders} {} · {items} {}",
                if folders == 1 { "folder" } else { "folders" },
                if items == 1 { "item" } else { "items" }
            );
            if focus.is_none() && self.board.unplaced > 0 {
                summary.push_str(&format!(" · {} stay in place", self.board.unplaced));
            }
            ui.label(RichText::new(summary).color(MUTED));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(icon::IMAGE).size(16.0).color(MUTED));
                let mut tile = self.board.tile.unwrap_or(TILE);
                ui.spacing_mut().slider_width = 90.0;
                ui.spacing_mut().interact_size.y = 14.0;
                if ui
                    .add(egui::Slider::new(&mut tile, 110.0..=300.0).show_value(false))
                    .on_hover_text("Thumbnail size")
                    .changed()
                {
                    self.board.tile = Some(tile);
                }
                ui.label(RichText::new(icon::IMAGE).size(11.0).color(MUTED));
                if self.library_selection_mode() && !self.selection.is_empty() {
                    ui.add_space(S3);
                    // Only what fits beside the title: text that overflows a
                    // right-to-left row would push the board under the folders.
                    let count = self.selection.len();
                    let font = egui::TextStyle::Small.resolve(ui.style());
                    let width = |text: &str| {
                        ui.painter()
                            .layout_no_wrap(text.to_owned(), font.clone(), ACCENT)
                            .size()
                            .x
                    };
                    let full = format!("{count} selected · drag onto a folder to move");
                    let short = format!("{count} selected");
                    let room = ui.available_width();
                    let text = if width(&full) <= room {
                        Some(full)
                    } else if width(&short) <= room {
                        Some(short)
                    } else {
                        None
                    };
                    if let Some(text) = text {
                        ui.label(RichText::new(text).small().color(ACCENT))
                            .on_hover_text("Drag onto a folder to move");
                    }
                }
            });
        });
    }
    /// The middle of Organize: photos grouped under their destination folder.
    pub(super) fn organize_board(&mut self, ui: &mut egui::Ui) {
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
        egui::Panel::left("organize_folder_panel")
            .resizable(true)
            .default_size(240.0)
            .size_range(180.0..=380.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(R_SURFACE)
                    .inner_margin(S2)
                    .outer_margin(egui::Margin {
                        left: 0,
                        right: 10,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| self.board_tree(ui));
        // The clicked photo, large, on the right.
        let preview_width = (ui.available_width() * 0.42).clamp(260.0, 900.0);
        egui::Panel::right("organize_preview_panel")
            .resizable(true)
            .default_size(preview_width)
            .size_range(240.0..=1100.0)
            .frame(
                egui::Frame::new()
                    .fill(PANEL)
                    .corner_radius(R_SURFACE)
                    .inner_margin(S2)
                    .outer_margin(egui::Margin {
                        left: 10,
                        right: 0,
                        top: 0,
                        bottom: 0,
                    }),
            )
            .show(ui, |ui| self.board_preview(ui));
        self.board_summary(ui);
        let gap = ui.spacing().item_spacing.x;
        let width = ui.available_width();
        let tile = self.board.tile.unwrap_or(TILE);
        let columns = ((width + gap) / tile).floor().max(1.0) as usize;
        let cell = (width - gap * (columns.saturating_sub(1) as f32)) / columns as f32;
        let tile_height = cell * 0.75 + library::CAPTION;
        let pitch = tile_height + ui.spacing().item_spacing.y;
        // Lay out the visible sections as header and tile rows.
        enum Row {
            Header(usize),
            Tiles(usize, usize),
        }
        let focus = self.board.focus.clone();
        let mut rows = Vec::new();
        let mut tops = Vec::new();
        let mut y = 0.0;
        for (index, section) in self.board.sections.iter().enumerate() {
            let shown = match (&focus, &section.path) {
                (None, _) => true,
                (Some(focus), Some(path)) => within(path, focus),
                (Some(_), None) => false,
            };
            if !shown {
                continue;
            }
            rows.push(Row::Header(index));
            tops.push(y);
            y += HEADER;
            for start in (section.range.start..section.range.end).step_by(columns) {
                rows.push(Row::Tiles(index, start));
                tops.push(y);
                y += pitch;
            }
        }
        let total = y;
        if rows.is_empty() {
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new("This folder is empty. Drag photos here from another folder.")
                        .color(MUTED),
                );
            });
            return;
        }
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("organize_board", self.group_mode))
            .auto_shrink([false, false]);
        if ui
            .ctx()
            .data_mut(|d| d.remove_temp::<bool>(egui::Id::new("organize-board-top")))
            .is_some()
        {
            scroll = scroll.vertical_scroll_offset(0.0);
        } else if std::mem::take(&mut self.library_reveal)
            && let Some(position) = self
                .active
                .and_then(|id| self.index.get(&id))
                .and_then(|index| self.filtered.iter().position(|i| i == index))
            && let Some(row) = rows.iter().position(|row| match row {
                Row::Tiles(_, start) => (*start..start + columns).contains(&position),
                Row::Header(_) => false,
            })
        {
            let offset = egui::scroll_area::State::load(
                ui.ctx(),
                ui.make_persistent_id(egui::IdSalt::new(("organize_board", self.group_mode))),
            )
            .map_or(0.0, |state| state.offset.y);
            let top = tops[row];
            if top < offset {
                scroll = scroll.vertical_scroll_offset((top - HEADER).max(0.0));
            } else if top + tile_height > offset + ui.available_height() {
                scroll = scroll
                    .vertical_scroll_offset((top + tile_height - ui.available_height()).max(0.0));
            }
        }
        let editable = self.can_edit_folders() && self.organize_editor.is_none();
        let root = self.board_root_name();
        let mut drop: Option<(Vec<MediaId>, String)> = None;
        let mut select_section: Option<usize> = None;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            let origin = ui.max_rect().min;
            ui.set_min_size(egui::vec2(width, total));
            let first = tops.partition_point(|top| *top + pitch < viewport.min.y);
            for (row, top) in rows.iter().zip(&tops).skip(first) {
                if *top > viewport.max.y {
                    break;
                }
                match row {
                    Row::Header(index) => {
                        let section = &self.board.sections[*index];
                        let rect = egui::Rect::from_min_size(
                            origin + egui::vec2(0.0, *top),
                            egui::vec2(width, HEADER),
                        );
                        let response = ui.interact(
                            rect,
                            egui::Id::new(("organize-section", &section.path)),
                            Sense::click(),
                        );
                        let count = section.range.len();
                        let photos = count - section.videos;
                        let counts = match (photos, section.videos) {
                            (p, 0) => format!("{p} {}", if p == 1 { "photo" } else { "photos" }),
                            (0, v) => format!("{v} {}", if v == 1 { "video" } else { "videos" }),
                            (p, v) => format!(
                                "{p} {} · {v} {}",
                                if p == 1 { "photo" } else { "photos" },
                                if v == 1 { "video" } else { "videos" }
                            ),
                        };
                        section_header(
                            ui,
                            rect,
                            section.path.as_deref(),
                            &root,
                            &counts,
                            response.hovered(),
                        );
                        if let Some(path) = &section.path {
                            if editable
                                && response
                                    .dnd_hover_payload::<super::grouping::PhotoDrag>()
                                    .is_some()
                            {
                                ui.painter().rect_stroke(
                                    rect.shrink2(egui::vec2(0.0, 4.0)),
                                    R_CONTROL,
                                    Stroke::new(1.5, ACCENT),
                                    StrokeKind::Inside,
                                );
                            }
                            if editable
                                && let Some(payload) =
                                    response.dnd_release_payload::<super::grouping::PhotoDrag>()
                            {
                                drop = Some((payload.0.clone(), path.clone()));
                            }
                        }
                        let name = format!(
                            "Folder {}",
                            section.path.as_deref().map_or("stays in place", |p| {
                                if p.is_empty() { &root } else { p }
                            })
                        );
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name)
                        });
                        if response
                            .on_hover_text("Click to select every item in this folder")
                            .clicked()
                        {
                            select_section = Some(*index);
                        }
                    }
                    Row::Tiles(index, start) => {
                        let end = self.board.sections[*index].range.end.min(start + columns);
                        for (column, position) in (*start..end).enumerate() {
                            let rect = egui::Rect::from_min_size(
                                origin + egui::vec2(column as f32 * (cell + gap), *top),
                                egui::vec2(cell, tile_height),
                            );
                            let Some(&media) = self.filtered.get(position) else {
                                continue;
                            };
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .id_salt(("board", position))
                                    .max_rect(rect),
                                |ui| self.card(ui, media, cell, tile_height, true),
                            );
                        }
                    }
                }
            }
        });
        self.library_offset = output.state.offset.y;
        if let Some(index) = select_section
            && self.library_selection_mode()
        {
            let range = self.board.sections[index].range.clone();
            let ids: Vec<MediaId> = self
                .filtered
                .get(range)
                .unwrap_or_default()
                .iter()
                .map(|i| self.media[*i].id)
                .collect();
            self.selection.replace(ids.iter().copied());
            if let Some(first) = ids.first() {
                self.set_active(*first);
            }
        }
        if let Some((ids, target)) = drop {
            self.board_move(&ids, &target);
        }
    }
}

/// One row of the folder tree: indent, icon, name and count.
fn folder_row(
    ui: &mut egui::Ui,
    title: &str,
    glyph: &str,
    depth: usize,
    count: usize,
    selected: bool,
) -> egui::Response {
    let (rect, response) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), Sense::click());
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
        painter.rect_filled(rect, R_CONTROL, CARD.gamma_multiply(hover));
    }
    let x = rect.left() + 8.0 + depth.saturating_sub(1) as f32 * 14.0;
    painter.text(
        egui::pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(14.0),
        if selected { ACCENT } else { MUTED },
    );
    let count_galley = painter.layout_no_wrap(count.to_string(), FontId::proportional(10.5), MUTED);
    let count_x = rect.right() - 8.0 - count_galley.size().x;
    painter.galley(
        egui::pos2(count_x, rect.center().y - count_galley.size().y / 2.0),
        count_galley,
        MUTED,
    );
    let mut job = egui::text::LayoutJob::simple_singleline(
        title.to_owned(),
        if selected {
            medium(12.0)
        } else {
            FontId::proportional(12.0)
        },
        INK,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((count_x - x - 30.0).max(10.0));
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(x + 22.0, rect.center().y - galley.size().y / 2.0),
        galley,
        INK,
    );
    response
}

/// A folder's header above its photos: name, parent path and counts.
fn section_header(
    ui: &egui::Ui,
    rect: egui::Rect,
    path: Option<&str>,
    root: &str,
    counts: &str,
    hovered: bool,
) {
    use egui_phosphor::regular as icon;
    let painter = ui.painter_at(rect);
    let y = rect.bottom() - 16.0;
    let (glyph, title, parent) = match path {
        None => (icon::PUSH_PIN, "Stays in place".to_owned(), String::new()),
        Some("") => (icon::HARD_DRIVES, root.to_owned(), String::new()),
        Some(path) => (
            icon::FOLDER_SIMPLE,
            leaf(path).to_owned(),
            path.rsplit_once('/')
                .map(|(parent, _)| format!("{root} / {}", parent.replace('/', " / ")))
                .unwrap_or_else(|| root.to_owned()),
        ),
    };
    let color = if hovered { INK } else { ACCENT };
    painter.text(
        egui::pos2(rect.left() + 2.0, y),
        Align2::LEFT_CENTER,
        glyph,
        FontId::proportional(16.0),
        color,
    );
    let title = painter.layout_no_wrap(title, semibold(14.0), INK);
    let mut x = rect.left() + 26.0;
    let title_width = title.size().x;
    painter.galley(egui::pos2(x, y - title.size().y / 2.0), title, INK);
    x += title_width + 10.0;
    if !parent.is_empty() {
        let parent =
            painter.layout_no_wrap(format!("in {parent}"), FontId::proportional(11.5), FAINT);
        let width = parent.size().x;
        painter.galley(egui::pos2(x, y - parent.size().y / 2.0), parent, FAINT);
        x += width + 10.0;
    }
    let counts = painter.layout_no_wrap(counts.to_owned(), FontId::proportional(11.5), MUTED);
    painter.galley(egui::pos2(x, y - counts.size().y / 2.0), counts, MUTED);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn organize() -> (tempfile::TempDir, PhotoSortApp) {
        let (temp, mut app) = super::super::compact_cull_tests::fixture();
        app.step = Step::Organize;
        app.review = false;
        app.output = app.root.as_ref().unwrap().display().to_string();
        app.filter_dirty = true;
        app.refilter();
        (temp, app)
    }

    #[test]
    fn board_groups_items_by_destination_folder_in_view_order() {
        let (_temp, mut app) = organize();
        let paths: Vec<_> = app
            .board
            .sections
            .iter()
            .map(|s| s.path.clone().unwrap())
            .collect();
        assert_eq!(paths, vec!["A/Day1".to_owned(), "B".to_owned()]);
        assert_eq!(app.board.sections[0].range, 0..4);
        assert_eq!(app.board.sections[1].range, 4..9);
        // `filtered` follows the board, so arrows and the strip match it.
        for (position, &index) in app.filtered.iter().enumerate() {
            let folder = if position < 4 { "A/Day1" } else { "B" };
            assert!(app.media[index].path.parent().unwrap().ends_with(folder));
        }
        let folders: Vec<_> = app
            .board
            .folders
            .iter()
            .map(|f| (f.path.as_str(), f.direct, f.total))
            .collect();
        assert!(folders.contains(&("", 0, 9)));
        assert!(folders.contains(&("A", 0, 4)));
        assert!(folders.contains(&("A/Day1", 4, 4)));
        assert!(folders.contains(&("B", 5, 5)));
        app.runtime.shutdown();
    }

    #[test]
    fn returning_from_another_step_mid_frame_draws_the_current_board() {
        let (_temp, mut app) = organize();
        assert_eq!(app.board.sections[1].range, 4..9);
        // Pick Best leaves its two-photo comparison in `filtered`; the board
        // keeps the sections it arranged on the last Organize visit.
        app.step = Step::PickBest;
        app.filtered.truncate(2);
        app.filter_dirty = false;
        app.board.focus = Some("B".into());
        // The step tabs switch steps after this frame's refilter has run,
        // so Organize draws before `filtered` is rebuilt.
        app.set_step(Step::Organize);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let _ = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(1400.0, 900.0),
                )),
                ..Default::default()
            },
            |ui| app.organize_board(ui),
        );
        assert_eq!(app.filtered.len(), 9);
        assert_eq!(app.board.sections.last().unwrap().range.end, 9);
        app.runtime.shutdown();
    }

    #[test]
    fn moving_creating_and_renaming_folders_edit_the_shared_draft_only() {
        let (_temp, mut app) = organize();
        let moved: Vec<MediaId> = app.filtered[..2].iter().map(|i| app.media[*i].id).collect();
        app.board_move(&moved, "B");
        assert!(app.organize_drafts.contains_key(&GroupMode::Current));
        app.check_board();
        app.refilter();
        let b = app
            .board
            .sections
            .iter()
            .find(|s| s.path.as_deref() == Some("B"))
            .unwrap();
        assert_eq!(b.range.len(), 7);
        // Creating and renaming a folder in place.
        app.board.editing = Some((String::new(), "Trip".into(), true));
        app.board_finish_edit();
        app.board.editing = Some(("Trip".into(), "Holiday".into(), false));
        app.board_finish_edit();
        app.board_move(&moved, "Holiday");
        app.check_board();
        app.refilter();
        assert!(
            app.board
                .sections
                .iter()
                .any(|s| s.path.as_deref() == Some("Holiday") && s.range.len() == 2)
        );
        // Nothing has moved on disk or been staged as a decision.
        assert!(app.decisions.is_empty());
        assert!(app.media.iter().all(|m| m.path.exists()));
        app.runtime.shutdown();
    }

    fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
        vec![egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }]
    }

    #[test]
    fn shift_arrows_compare_the_selection_in_the_preview_and_escape_returns() {
        use super::super::compact_cull_tests::{frame, rendered};
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 900.0);
        app.select_library(ids[0], false, false);
        assert_eq!(app.comparison_ids(), vec![ids[0]]);
        for _ in 0..2 {
            frame(
                &mut app,
                &ctx,
                size,
                key(egui::Key::ArrowRight, egui::Modifiers::SHIFT),
            );
        }
        assert_eq!(app.selection.len(), 3);
        assert_eq!(app.comparison_ids(), ids[..3].to_vec());
        assert_eq!(app.active, Some(ids[2]));
        assert_eq!(app.focused_slot, 2);
        let nodes = rendered(&mut app, &ctx, size);
        assert!(nodes.iter().any(|(label, _)| label == "End comparison"));
        // Escape keeps the active photo and shows it alone.
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::Escape, Default::default()),
        );
        assert_eq!(app.comparison_ids(), vec![ids[2]]);
        assert_eq!(
            app.selection.ordered(&app.media, &app.filtered),
            vec![ids[2]]
        );
        assert_eq!(app.focused_slot, 0);
        let nodes = rendered(&mut app, &ctx, size);
        assert!(!nodes.iter().any(|(label, _)| label == "End comparison"));
        // A second Escape clears the selection as before.
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::Escape, Default::default()),
        );
        assert!(app.selection.is_empty() && app.active.is_none());
        app.runtime.shutdown();
    }

    #[test]
    fn the_comparison_follows_board_order_and_stops_past_nine() {
        let (_temp, mut app) = organize();
        let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
        // Clicked out of order, compared in the order the board shows them.
        app.select_library(ids[6], false, false);
        app.select_library(ids[1], true, false);
        app.select_library(ids[4], true, false);
        assert_eq!(app.comparison_ids(), vec![ids[1], ids[4], ids[6]]);
        assert_eq!(app.active, Some(ids[4]));
        assert_eq!(app.focused_slot, 1);
        app.selection.all(&app.media, &app.filtered);
        assert_eq!(app.comparison_ids(), ids);
        // A tenth selected item (one this view does not show) ends the
        // comparison; the preview shows the active photo.
        let mut selected = ids.clone();
        selected.push(MediaId(u64::MAX));
        app.selection.replace(selected);
        assert_eq!(app.comparison_ids(), vec![ids[4]]);
        // Other steps keep their own comparisons.
        app.select_library(ids[0], false, false);
        app.select_library(ids[1], true, false);
        app.step = Step::Cull;
        assert_eq!(app.comparison_ids(), vec![ids[1]]);
        app.runtime.shutdown();
    }

    #[test]
    fn generated_groupings_hide_emptied_folders_but_keep_created_ones() {
        let (_temp, mut app) = organize();
        // Making a folder creates the draft, which knows every library folder.
        app.board.editing = Some((String::new(), "Trip".into(), true));
        app.board_finish_edit();
        app.check_board();
        app.refilter();
        let paths = |app: &PhotoSortApp| -> Vec<String> {
            app.board.folders.iter().map(|f| f.path.clone()).collect()
        };
        // Current folders lists them all, empty or not.
        assert_eq!(paths(&app), ["", "A", "A/Day1", "B", "Empty", "Trip"]);
        app.group_mode = GroupMode::Camera;
        app.board.editing = Some((String::new(), "Picks".into(), true));
        app.board_finish_edit();
        app.board.editing = Some(("Picks".into(), "Best".into(), true));
        app.board_finish_edit();
        app.check_board();
        app.refilter();
        assert_eq!(
            paths(&app),
            ["", "Picks", "Picks/Best", "Unknown camera"],
            "Camera lists the folders it fills and the ones made on the board"
        );
        // A folder emptied by a move disappears unless it was made by hand.
        let ids: Vec<MediaId> = app.media.iter().map(|m| m.id).collect();
        app.board_move(&ids, "Picks/Best");
        app.check_board();
        app.refilter();
        assert_eq!(paths(&app), ["", "Picks", "Picks/Best"]);
        app.runtime.shutdown();
    }
}
