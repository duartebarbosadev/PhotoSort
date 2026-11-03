//! The board itself: the shown folders' headers and photo rows, laid out
//! once and reused, with scrolling, gliding and the selection rectangle.
use super::sidebar::{preview_card, resting};
use super::*;

/// A row of the board: a folder's header, or a row of its photos.
pub(in crate::ui::workspace) enum Row {
    Header(usize),
    Tiles(usize, photo_rows::PhotoRow),
}

/// What the board's rows are laid out from.
#[derive(Clone, PartialEq)]
pub(in crate::ui::workspace) struct BoardRowsKey {
    arranged: u64,
    /// Photos' shapes come with their metadata, which arrives without
    /// rearranging the board.
    metadata: usize,
    focus: Vec<String>,
    width: f32,
    target: f32,
    gap: f32,
    spacing: f32,
}

/// The shown sections laid out as header and photo rows, with each row's
/// top, height and bottom in the scrolled board, and the board's height.
pub(in crate::ui::workspace) struct BoardRows {
    key: BoardRowsKey,
    rows: Vec<Row>,
    tops: Vec<f32>,
    heights: Vec<f32>,
    bottoms: Vec<f32>,
    total: f32,
}

/// A folder's header above its photos: name, parent path and counts.
#[allow(clippy::too_many_arguments)]
fn section_header(
    ui: &egui::Ui,
    rect: egui::Rect,
    path: Option<&str>,
    root: &str,
    counts: &str,
    hovered: bool,
    pending: Option<Pending>,
    faces: Option<&[people::FaceImage]>,
) {
    use egui_phosphor::regular as icon;
    let painter = ui.painter_at(rect);
    let y = rect.bottom() - 16.0;
    let (glyph, title, parent) = match path {
        None => (
            match pending {
                None => icon::PUSH_PIN,
                Some(Pending::Running) => icon::HOURGLASS_MEDIUM,
                Some(_) => icon::CIRCLE_DASHED,
            },
            unplaced_title(pending).to_owned(),
            String::new(),
        ),
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
    match faces {
        // A person's folder shows their face, a little larger than in the sidebar.
        Some(faces) if !faces.is_empty() => {
            use people::HEADER_FACE;
            people::paint_faces(
                &painter,
                egui::Rect::from_min_size(
                    egui::pos2(rect.left(), y - HEADER_FACE / 2.0),
                    egui::vec2(HEADER_FACE * 1.55, HEADER_FACE),
                ),
                faces,
                BG,
            )
        }
        _ => {
            painter.text(
                egui::pos2(rect.left() + 2.0, y),
                Align2::LEFT_CENTER,
                glyph,
                FontId::proportional(16.0),
                color,
            );
        }
    }
    let title = painter.layout_no_wrap(title, semibold(14.0), INK);
    let mut x = rect.left()
        + match faces.map_or(0, <[_]>::len) {
            0 => 26.0,
            1 => people::HEADER_FACE + 8.0,
            _ => people::HEADER_FACE * 1.55 + 8.0,
        };
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
    let mut job = egui::text::LayoutJob::simple_singleline(
        counts.to_owned(),
        FontId::proportional(11.5),
        MUTED,
    );
    job.wrap = egui::text::TextWrapping::truncate_at_width((rect.right() - x).max(10.0));
    let counts = painter.layout_job(job);
    painter.galley(egui::pos2(x, y - counts.size().y / 2.0), counts, MUTED);
}

impl PhotoSortApp {
    /// The shown sections as header and tile rows: justified rows, each
    /// photo as wide as its shape needs at the row's height. Laid out again
    /// only when what they depend on changes, so a still board costs
    /// nothing per frame however many photos it holds.
    fn board_rows(&mut self, key: BoardRowsKey) -> Arc<BoardRows> {
        if let Some(rows) = &self.board.rows
            && rows.key == key
        {
            return rows.clone();
        }
        // A tile is its photo at the row's height, the inset around it and
        // the caption under it.
        let tile_height = |photo: f32| photo + TILE_INSET + library::CAPTION;
        let mut rows = Vec::new();
        let mut tops = Vec::new();
        let mut heights = Vec::new();
        let mut y = 0.0;
        for (index, section) in self.board.sections.iter().enumerate() {
            if !shows(&key.focus, section.path.as_deref()) {
                continue;
            }
            rows.push(Row::Header(index));
            tops.push(y);
            heights.push(HEADER);
            y += HEADER;
            let tiles: Vec<photo_rows::Tile> = section
                .range
                .clone()
                .map(|position| {
                    let id = self.media[self.filtered[position]].id;
                    photo_rows::Tile {
                        aspect: self.photo_aspect(id, self.preview_turns(id)),
                        extra: 2.0 * TILE_INSET,
                        min_width: MIN_TILE,
                    }
                })
                .collect();
            for mut row in photo_rows::justify(&tiles, key.width, key.target, key.gap) {
                row.range =
                    row.range.start + section.range.start..row.range.end + section.range.start;
                let height = tile_height(row.height);
                rows.push(Row::Tiles(index, row));
                tops.push(y);
                heights.push(height);
                y += height + key.spacing;
            }
        }
        let bottoms = tops.iter().zip(&heights).map(|(t, h)| t + h).collect();
        let laid = Arc::new(BoardRows {
            key,
            rows,
            tops,
            heights,
            bottoms,
            total: y,
        });
        self.board.rows = Some(laid.clone());
        laid
    }
    /// The middle of Organize: photos grouped under their destination folder.
    pub(in crate::ui::workspace) fn organize_board(&mut self, ui: &mut egui::Ui) {
        if self.board.sidebar_frame != Some(ui.ctx().cumulative_frame_nr()) {
            self.organize_sidebar(ui, ui.available_width());
        } else if self.board.sections.last().map_or(0, |s| s.range.end) != self.filtered.len() {
            // The bar, drawn after the sidebar, changed what is shown.
            self.filter_dirty = true;
            self.refilter();
        }
        // The clicked photo, large, on the right, when it fits beside a
        // usable board; otherwise it opens over the board on request.
        if self.board.layout.preview {
            let widest = (ui.available_width() - BOARD_MIN - BOARD_GAP).clamp(PREVIEW_MIN, 1100.0);
            let preview_width = (ui.available_width() * 0.42).clamp(PREVIEW_MIN, widest.min(900.0));
            egui::Panel::right("organize_preview_panel")
                .resizable(true)
                .default_size(preview_width)
                .size_range(PREVIEW_MIN..=widest)
                .frame(
                    egui::Frame::new()
                        .fill(PANEL)
                        .stroke(Stroke::new(1.0, EDGE))
                        .corner_radius(R_SURFACE)
                        .inner_margin(S2 - 1.0)
                        .outer_margin(egui::Margin {
                            left: BOARD_GAP as i8,
                            right: 0,
                            top: 0,
                            bottom: 0,
                        }),
                )
                .show(ui, |ui| self.board_preview(ui));
        } else {
            let board = ui.available_rect_before_wrap();
            self.board_preview_overlay(ui.ctx(), board);
        }
        let pending = self.board_pending();
        if self.analysis_run.is_some() {
            self.analysis_banner(ui);
            ui.add_space(S2);
        }
        if pending.is_some_and(|p| p != Pending::Running) {
            self.organize_notice(ui);
            ui.add_space(S2);
        }
        self.board_summary(ui);
        let width = ui.available_width();
        let spacing = ui.spacing().item_spacing.y;
        let laid = self.board_rows(BoardRowsKey {
            arranged: self.board.arranged,
            metadata: self.metadata.len(),
            focus: self.board.focus.clone(),
            width,
            target: self.photo_size(),
            gap: ui.spacing().item_spacing.x,
            spacing,
        });
        let focus = &laid.key.focus;
        let (rows, tops, heights, bottoms) = (&laid.rows, &laid.tops, &laid.heights, &laid.bottoms);
        let total = laid.total;
        if rows.is_empty() {
            if self.filtered.is_empty()
                && (!self.query.trim().is_empty() || self.filter != Filter::All)
            {
                let clear = states::notice_card(
                    ui,
                    egui_phosphor::regular::MAGNIFYING_GLASS,
                    "No photos match",
                    "The search and filter hide every photo here.",
                    Some(("Clear search and filter", true)),
                );
                if clear {
                    self.query.clear();
                    self.filter = Filter::All;
                    self.filter_dirty = true;
                }
                return;
            }
            ui.centered_and_justified(|ui| {
                ui.label(
                    RichText::new(if focus.len() > 1 {
                        "These folders are empty. Drag photos here from another folder."
                    } else {
                        "This folder is empty. Drag photos here from another folder."
                    })
                    .color(MUTED),
                );
            });
            return;
        }
        let holds = |row: &Row, position: usize| match row {
            Row::Tiles(_, tiles) => tiles.range.contains(&position),
            Row::Header(_) => false,
        };
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(("organize_board", self.group_mode))
            .auto_shrink([false, false]);
        let saved_offset = || {
            egui::scroll_area::State::load(
                ui.ctx(),
                ui.make_persistent_id(egui::IdSalt::new(("organize_board", self.group_mode))),
            )
            .map(|state| state.offset.y)
        };
        // Rows above the view change height as photos' sizes arrive with
        // their thumbnails, or as the window is resized: the photo at the
        // top of the view stays where it was.
        // Kept per library, so another folder's photo is never the anchor.
        let anchor_key = egui::Id::new(("organize-board-anchor", self.group_mode, &self.root));
        let anchor = ui
            .data(|d| d.get_temp::<Option<(MediaId, f32)>>(anchor_key))
            .flatten()
            .and_then(|(id, shift)| {
                let position = self
                    .index
                    .get(&id)
                    .and_then(|index| self.filtered.iter().position(|i| i == index))?;
                let row = rows.iter().position(|row| holds(row, position))?;
                Some(tops[row] - shift)
            })
            .zip(saved_offset())
            .filter(|(wanted, offset)| (wanted - offset).abs() > 0.5)
            .map(|(wanted, _)| wanted.max(0.0));
        // A folder clicked in the sidebar: its header (or, without one, the
        // first folder inside it) eases up to the top of the view, or jumps
        // there when motion is reduced.
        let glide_to = self.board.glide.take().and_then(|glide| {
            let header = |row: &Row| match row {
                Row::Header(index) => self.board.sections[*index].path.as_deref(),
                Row::Tiles(..) => None,
            };
            let row =
                (0..rows.len()).find(|i| header(&rows[*i]).is_some_and(|p| within(p, &glide.0)))?;
            let end = (total - ui.available_height()).max(0.0);
            self.board.glide = Some(glide);
            Some(tops[row].min(end))
        });
        let mut glide = None;
        if let Some(target) = glide_to {
            self.library_reveal = false;
            self.board.to_top = false;
            let from = saved_offset().unwrap_or(0.0);
            let dt = ui.input(|i| i.stable_dt).min(0.1);
            let step = if ui.style().animation_time == 0.0 {
                1.0
            } else {
                1.0 - (-dt * 14.0).exp()
            };
            let next = from + (target - from) * step;
            let next = if (target - next).abs() < 1.0 {
                target
            } else {
                next
            };
            scroll = scroll.vertical_scroll_offset(next);
            glide = Some((next, target));
            ui.ctx().request_repaint();
        } else if std::mem::take(&mut self.board.to_top) {
            scroll = scroll.vertical_scroll_offset(0.0);
        } else if std::mem::take(&mut self.library_reveal)
            && let Some(position) = self
                .active
                .and_then(|id| self.index.get(&id))
                .and_then(|index| self.filtered.iter().position(|i| i == index))
            && let Some(row) = rows.iter().position(|row| holds(row, position))
        {
            let offset = anchor.or(saved_offset()).unwrap_or(0.0);
            let (top, height) = (tops[row], heights[row]);
            if top < offset {
                scroll = scroll.vertical_scroll_offset((top - HEADER).max(0.0));
            } else if top + height > offset + ui.available_height() {
                scroll =
                    scroll.vertical_scroll_offset((top + height - ui.available_height()).max(0.0));
            } else if let Some(offset) = anchor {
                scroll = scroll.vertical_scroll_offset(offset);
            }
        } else if let Some(offset) = anchor {
            scroll = scroll.vertical_scroll_offset(offset);
        }
        let faces_mode = self.board.people.signature.is_some();
        let editable = self.board_editable();
        let root = self.board_root_name();
        let dim = matches!(pending, Some(Pending::Running | Pending::Waiting));
        let mut keep: Option<Vec<MediaId>> = None;
        let mut drop: Option<(Vec<MediaId>, String)> = None;
        let mut select_section: Option<(usize, egui::Modifiers)> = None;
        let mut faceless = false;
        let active_position = self
            .active
            .and_then(|id| self.index.get(&id))
            .and_then(|index| self.filtered.iter().position(|i| i == index));
        let selecting = self.library_selection_mode() && self.selection_ready();
        let mut marquee: Option<(egui::Rect, Vec<MediaId>)> = None;
        let output = scroll.show_viewport(ui, |ui, viewport| {
            let origin = ui.max_rect().min;
            ui.set_min_size(egui::vec2(width, total));
            // Registered before the tiles, so a press on a tile still picks
            // or drags that photo; only empty space starts a rectangle.
            let background = ui.interact(
                egui::Rect::from_min_size(origin + viewport.min.to_vec2(), viewport.size()),
                egui::Id::new("organize-board-marquee"),
                if selecting {
                    Sense::drag()
                } else {
                    Sense::hover()
                },
            );
            let board_pos = |pos: egui::Pos2| pos - origin.to_vec2();
            if background.drag_started()
                && let Some(start) = ui.input(|i| i.pointer.press_origin())
            {
                let keep = ui.input(|i| i.modifiers.shift || i.modifiers.command || i.modifiers.ctrl);
                let base = if keep {
                    self.selection.ordered(&self.media, &self.filtered)
                } else {
                    Vec::new()
                };
                self.board.marquee = Some((board_pos(start), base));
            }
            if let Some((start, base)) = &self.board.marquee
                && let Some(pointer) = ui.input(|i| i.pointer.interact_pos())
            {
                let area = egui::Rect::from_two_pos(*start, board_pos(pointer));
                // Every tile the rectangle touches, drawn or not.
                let mut hit = Vec::new();
                for ((row, top), height) in rows.iter().zip(tops).zip(heights) {
                    let Row::Tiles(_, tiles) = row else {
                        continue;
                    };
                    if *top > area.max.y || top + height < area.min.y {
                        continue;
                    }
                    for (position, (left, w)) in tiles.range.clone().zip(&tiles.tiles) {
                        if *left <= area.max.x && left + w >= area.min.x {
                            hit.push(self.media[self.filtered[position]].id);
                        }
                    }
                }
                let mut ids = base.clone();
                let kept: HashSet<MediaId> = base.iter().copied().collect();
                ids.extend(hit.into_iter().filter(|id| !kept.contains(id)));
                marquee = Some((area.translate(origin.to_vec2()), ids));
                // Near or past the top or bottom edge, the board scrolls.
                let edge = 24.0;
                let shown = background.rect;
                let speed = if pointer.y < shown.top() + edge {
                    (shown.top() + edge - pointer.y).min(80.0) * 0.5
                } else if pointer.y > shown.bottom() - edge {
                    -(pointer.y - shown.bottom() + edge).min(80.0) * 0.5
                } else {
                    0.0
                };
                if speed != 0.0 {
                    ui.scroll_with_delta(egui::vec2(0.0, speed));
                    ui.ctx().request_repaint();
                }
            }
            // The folder you are looking at: the active photo's while its
            // tile is on screen, and the topmost folder shown.
            let section = |row: &Row| match row {
                Row::Header(index) | Row::Tiles(index, _) => *index,
            };
            let on_screen =
                |i: usize| tops[i] + heights[i] > viewport.min.y + 8.0 && tops[i] < viewport.max.y - 8.0;
            let looking = (
                active_position
                    .and_then(|position| (0..rows.len()).find(|i| holds(&rows[*i], position)))
                    .filter(|i| on_screen(*i))
                    .map(|i| section(&rows[i])),
                (0..rows.len()).find(|i| on_screen(*i)).map(|i| section(&rows[i])),
            );
            let first = bottoms.partition_point(|bottom| bottom + spacing < viewport.min.y);
            for ((row, top), height) in rows.iter().zip(tops).zip(heights).skip(first) {
                if *top > viewport.max.y {
                    break;
                }
                match row {
                    Row::Header(index) => {
                        let faces = faces_mode.then(|| {
                            self.board.sections[*index]
                                .path
                                .clone()
                                .map(|path| self.folder_faces(&path, ui.ctx().pixels_per_point()))
                                .unwrap_or_default()
                        });
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
                            _ if section.path.is_none() && pending.is_none() => self
                                .unplaced_phrase(photos, section.videos),
                            (p, 0) => format!("{p} {}", if p == 1 { "photo" } else { "photos" }),
                            (0, v) => format!("{v} {}", if v == 1 { "video" } else { "videos" }),
                            (p, v) => format!(
                                "{p} {} · {v} {}",
                                if p == 1 { "photo" } else { "photos" },
                                if v == 1 { "video" } else { "videos" }
                            ),
                        };
                        // Faces offers its other choice right where those
                        // photos are: one "No people" folder.
                        let offer = section.path.is_none()
                            && pending.is_none()
                            && editable
                            && self.group_mode == GroupMode::Faces
                            && self.preferences.faceless
                                == crate::core::preferences::Faceless::Stay
                            && photos > 0
                            // In a narrow board the toolbar's choice is enough.
                            && rect.width() >= 480.0;
                        // The text stops short of the offer at the right.
                        let reserve = if offer { 210.0 } else { 0.0 };
                        section_header(
                            ui,
                            egui::Rect::from_min_max(
                                rect.min,
                                egui::pos2(rect.right() - reserve, rect.bottom()),
                            ),
                            section.path.as_deref(),
                            &root,
                            &counts,
                            response.hovered(),
                            pending,
                            faces.as_deref(),
                        );
                        // Photos dropped on a folder move there; dropped on
                        // "Not moving", they stay where they are.
                        if editable && (section.path.is_some() || pending.is_none()) {
                            if response
                                .dnd_hover_payload::<grouping::PhotoDrag>()
                                .is_some()
                            {
                                ui.painter().rect_stroke(
                                    rect.shrink2(egui::vec2(0.0, 4.0)),
                                    R_CONTROL,
                                    Stroke::new(1.5, ACCENT),
                                    StrokeKind::Inside,
                                );
                            }
                            if let Some(payload) =
                                response.dnd_release_payload::<grouping::PhotoDrag>()
                            {
                                match &section.path {
                                    Some(path) => drop = Some((payload.0.clone(), path.clone())),
                                    None => keep = Some(payload.0.clone()),
                                }
                            }
                        }
                        if offer {
                            let label = format!(
                                "Put them in a “{}” folder",
                                crate::core::preferences::NO_PEOPLE
                            );
                            let size = egui::vec2(200.0, 24.0);
                            let at = egui::Rect::from_min_size(
                                egui::pos2(rect.right() - size.x, rect.bottom() - 16.0 - size.y / 2.0),
                                size,
                            );
                            if ui
                                .put(at, egui::Button::new(RichText::new(&label).small()))
                                .on_hover_text("Photos without a recognized person go to one folder. You can switch back in the toolbar.")
                                .clicked()
                            {
                                faceless = true;
                            }
                        }
                        let name = match section.path.as_deref() {
                            None if pending.is_some() => "Photos not grouped yet".to_owned(),
                            None => format!("Not moving · {counts}"),
                            Some("") => format!("Folder {root}"),
                            Some(path) => format!("Folder {path}"),
                        };
                        response.widget_info(|| {
                            egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name)
                        });
                        let tip = "Click to select every item in this folder\nCtrl/Cmd-click adds it to the selection, or removes it\nShift-click selects everything from the last click through it";
                        // A person's folder shows their faces larger on hover.
                        let path = self.board.sections[*index].path.clone();
                        let card = match &path {
                            Some(path)
                                if response.hovered()
                                    && faces.as_ref().is_some_and(|f| !f.is_empty()) =>
                            {
                                self.folder_card(path, ui.ctx().pixels_per_point())
                            }
                            _ => Vec::new(),
                        };
                        // Any other folder previews what is inside it.
                        let preview = path
                            .filter(|_| card.is_empty() && response.hovered())
                            .map(|path| self.folder_preview(&path, resting(ui)));
                        let response = match (&preview, card.is_empty()) {
                            (_, false) => response.on_hover_ui(|ui| {
                                let width = people::face_card(ui, &card);
                                ui.set_max_width(width);
                                ui.label(RichText::new(tip).small().color(MUTED));
                            }),
                            (Some(preview), true) => response.on_hover_ui(|ui| {
                                preview_card(ui, preview);
                                ui.label(RichText::new(tip).small().color(MUTED));
                            }),
                            (None, true) => response.on_hover_text(tip),
                        };
                        if response.clicked() {
                            select_section = Some((*index, ui.input(|i| i.modifiers)));
                        }
                    }
                    Row::Tiles(_, tiles) => {
                        for (position, &(left, w)) in tiles.range.clone().zip(&tiles.tiles) {
                            let rect = egui::Rect::from_min_size(
                                origin + egui::vec2(left, *top),
                                egui::vec2(w, *height),
                            );
                            let Some(&media) = self.filtered.get(position) else {
                                continue;
                            };
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .id_salt(("board", position))
                                    .max_rect(rect),
                                |ui| {
                                    // Photos wait, dimmed, for the grouping
                                    // that will place them.
                                    if dim {
                                        ui.multiply_opacity(0.45);
                                    }
                                    self.card(ui, media, w, *height, true)
                                },
                            );
                        }
                    }
                }
            }
            if let Some((area, _)) = &marquee {
                let painter = ui.painter();
                painter.rect_filled(*area, 2.0, ACCENT.gamma_multiply(0.12));
                painter.rect_stroke(*area, 2.0, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
            }
            looking
        });
        self.library_offset = output.state.offset.y;
        let looking = output.inner;
        // The photo at the top of the view once this frame's scrolling is
        // done, and how far below the top of the view its row starts.
        let offset = output.state.offset.y;
        let top_photo = rows
            .iter()
            .zip(tops)
            .skip(bottoms.partition_point(|bottom| *bottom <= offset))
            .find_map(|(row, top)| match row {
                Row::Tiles(_, tiles) => Some((
                    self.media[self.filtered[tiles.range.start]].id,
                    top - offset,
                )),
                Row::Header(_) => None,
            });
        ui.data_mut(|d| d.insert_temp(anchor_key, top_photo));
        if let Some((_, ids)) = marquee {
            self.selection.replace(ids.iter().copied());
            self.comparison.clear();
            self.comparison_focus = None;
            if !ui.input(|i| i.pointer.any_down()) {
                self.board.marquee = None;
                // The preview follows: the active photo stays when selected.
                if let Some(&first) = ids.first()
                    && !self.active.is_some_and(|a| ids.contains(&a))
                {
                    self.set_active(first);
                }
            }
        } else if !ui.input(|i| i.pointer.any_down()) {
            self.board.marquee = None;
        }
        // The board ending up anywhere but where the glide asked means you
        // scrolled it, which stops the glide; it stops on arriving too.
        if let Some((asked, target)) = glide {
            if (output.state.offset.y - asked).abs() > 0.5 {
                self.board.glide = None;
                self.board.hold = None;
            } else if asked == target {
                self.board.glide = None;
            }
        }
        let (active, top) = looking;
        let path = |index: usize| self.board.sections[index].path.clone();
        self.board_look_at(active.map(path), top.map(path), output.state.offset.y);
        if self.board.reveal_current {
            ui.ctx().request_repaint(); // The sidebar was drawn first.
        }
        if let Some((index, modifiers)) = select_section
            && self.library_selection_mode()
        {
            self.board_select_section(index, modifiers);
        }
        if let Some((ids, target)) = drop {
            self.board_move(&ids, &target);
        }
        if let Some(ids) = keep {
            self.board_keep_in_place(&ids);
        }
        if faceless {
            self.change_faceless(crate::core::preferences::Faceless::Folder);
        }
    }
}
