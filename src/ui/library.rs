use super::*;

impl PhotoSortApp {
    fn browse_group_label(&self, id: MediaId) -> Option<String> {
        (self.step == Step::Cull
            && self.browse.group_sort != crate::core::browsing::GroupSort::Flat)
            .then(|| {
                self.cull_groups.get(&id).map_or_else(
                    || "Ungrouped".into(),
                    |g| {
                        format!(
                            "Group {g}{}",
                            if self.manual_groups.contains(id) {
                                " · manual"
                            } else {
                                ""
                            }
                        )
                    },
                )
            })
    }

    pub(super) fn library(&mut self, ui: &mut egui::Ui) {
        if self.step == Step::Organize {
            self.organize_board(ui);
            return;
        }
        // Guided steps show one review card per suggestion or group.
        match self.step {
            Step::EasyDelete => return self.delete_grid(ui),
            Step::Rotation => return self.rotation_grid(ui),
            Step::PickBest => return self.pick_grid(ui),
            _ => {}
        }
        self.grid(ui);
    }
    /// Cull's photos break into similarity sections, each under a header.
    pub(super) fn grouped_view(&self) -> bool {
        self.step == Step::Cull
            && self.browse.group_sort != crate::core::browsing::GroupSort::Flat
            && !self.group_best.is_empty()
    }
    /// The lines of a grid or strip of `columns`: rows of photos, and with
    /// [`Self::grouped_view`] a header before each similar set and before
    /// the photos between them.
    pub(super) fn photo_lines(&self, columns: usize, row: f32, gap: f32, header: f32) -> Lines {
        let mut lines = Lines {
            columns: columns.max(1),
            row,
            gap,
            count: self.filtered.len(),
            sections: None,
        };
        if !self.grouped_view() {
            return lines;
        }
        let set = |position: usize| {
            let id = self.media[self.filtered[position]].id;
            self.similar_group(id).map(|(group, ..)| group)
        };
        let mut sections = Vec::new();
        let mut top = 0.0;
        let mut start = 0;
        while start < self.filtered.len() {
            let group = set(start);
            let mut end = start + 1;
            while end < self.filtered.len() && set(end) == group {
                end += 1;
            }
            sections.push(Line {
                top,
                kind: LineKind::Header { group, start, end },
            });
            top += header;
            for first in (start..end).step_by(lines.columns) {
                sections.push(Line {
                    top,
                    kind: LineKind::Cells {
                        start: first,
                        end: (first + lines.columns).min(end),
                    },
                });
                top += row + gap;
            }
            start = end;
        }
        lines.sections = Some((sections, top));
        lines
    }
    /// A Cull grid or strip's scroll area, scrolled to what it must show
    /// once: a saved position, a folder chosen in the navigator, or the
    /// active photo when it changed off screen.
    pub(super) fn photos_scroll(
        &mut self,
        ui: &egui::Ui,
        lines: &Lines,
        id: egui::Id,
        restore: bool,
    ) -> egui::ScrollArea {
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(id)
            .auto_shrink([false, false]);
        if restore && let Some(offset) = self.library_restore_offset.take() {
            return scroll.vertical_scroll_offset(offset);
        }
        if self.step == Step::Cull
            && let Some(position) = self.take_folder_scroll()
        {
            self.library_reveal = false;
            return scroll.vertical_scroll_offset(lines.section_top(position));
        }
        if self.step == Step::Cull
            && std::mem::take(&mut self.library_reveal)
            && let Some(position) = self
                .active
                .and_then(|id| self.index.get(&id))
                .and_then(|index| self.filtered.iter().position(|i| i == index))
        {
            let top = lines.top(position);
            let offset = egui::scroll_area::State::load(
                ui.ctx(),
                ui.make_persistent_id(egui::IdSalt::new(id)),
            )
            .map_or(0.0, |state| state.offset.y);
            if top < offset {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top + lines.row > offset + ui.available_height() {
                scroll = scroll
                    .vertical_scroll_offset((top + lines.row - ui.available_height()).max(0.0));
            }
        }
        scroll
    }
    /// After a Cull grid or strip is drawn: the navigator follows the
    /// photos, as Organize's sidebar follows its board.
    pub(super) fn follow_lines(&mut self, lines: &Lines, viewport: egui::Rect) {
        if self.step != Step::Cull || !self.navigator.enabled {
            return;
        }
        let active = self
            .active
            .and_then(|id| self.index.get(&id))
            .and_then(|index| self.filtered.iter().position(|i| i == index))
            .is_some_and(|position| {
                let top = lines.top(position);
                top + lines.row > viewport.top() && top < viewport.bottom()
            });
        let top = lines.position_at(viewport.top() + lines.row * 0.25);
        self.follow_photos(active, top, viewport.top());
    }
    pub(super) fn grid(&mut self, ui: &mut egui::Ui) {
        if self.step == Step::Cull {
            self.photos_header(ui, false);
        }
        let gap = ui.spacing().item_spacing.x;
        let columns = ((ui.available_width() + gap) / 200.0).floor().max(1.0) as usize;
        let cell_width =
            (ui.available_width() - gap * (columns.saturating_sub(1) as f32)) / columns as f32;
        // Hiding file names gives a denser, image-only grid.
        let captioned = self.preferences.show_filenames;
        let row_height = cell_width * 0.75 + if captioned { CAPTION } else { 0.0 };
        let lines = self.photo_lines(columns, row_height, ui.spacing().item_spacing.y, SET_HEADER);
        let scroll = self.photos_scroll(ui, &lines, egui::Id::new("library_grid"), true);
        let output = scroll.show_viewport(ui, |ui, viewport| {
            let origin = ui.max_rect().min;
            ui.set_min_size(egui::vec2(ui.available_width(), lines.height()));
            for line in lines.visible(viewport.top(), viewport.bottom()) {
                let left = origin + egui::vec2(0.0, line.top);
                match line.kind {
                    LineKind::Header { group, start, end } => {
                        let rect = egui::Rect::from_min_size(
                            left,
                            egui::vec2(ui.available_width(), SET_HEADER),
                        );
                        self.set_header(ui, rect, group, end - start);
                    }
                    LineKind::Cells { start, end } => {
                        for (column, position) in (start..end).enumerate() {
                            let rect = egui::Rect::from_min_size(
                                left + egui::vec2(column as f32 * (cell_width + gap), 0.0),
                                egui::vec2(cell_width, row_height),
                            );
                            let index = self.filtered[position];
                            ui.scope_builder(
                                egui::UiBuilder::new().id_salt(position).max_rect(rect),
                                |ui| self.card(ui, index, cell_width, row_height, captioned),
                            );
                        }
                    }
                }
            }
            viewport
        });
        self.library_offset = output.state.offset.y;
        self.follow_lines(&lines, output.inner);
    }
    /// The header of one section of a grouped grid or strip: a similar set
    /// with its colour, size and best photo, or the photos between sets.
    /// Photos dropped on a set's header move into it.
    pub(super) fn set_header(
        &mut self,
        ui: &mut egui::Ui,
        rect: egui::Rect,
        group: Option<usize>,
        shown: usize,
    ) {
        use egui_phosphor::regular as icon;
        let response = ui.interact(
            rect,
            ui.id().with(("set-header", group, rect.top() as i32)),
            Sense::click(),
        );
        let painter = ui.painter_at(rect);
        let y = rect.center().y + 2.0;
        let Some(group) = group else {
            painter.text(
                egui::pos2(rect.left() + 2.0, y),
                Align2::LEFT_CENTER,
                format!("Not in a similar set · {shown}"),
                medium(10.5),
                FAINT,
            );
            return;
        };
        let (best, size) = self.group_best[&group];
        let color = super::grouping::group_color(group);
        painter.circle_filled(egui::pos2(rect.left() + 6.0, y), 3.5, color);
        let best_name = self
            .index
            .get(&best)
            .map_or_else(String::new, |i| self.media[*i].name());
        let mut job = egui::text::LayoutJob::default();
        job.append(
            &format!("Similar set · {size} photos"),
            0.0,
            egui::TextFormat::simple(medium(11.0), INK),
        );
        job.append(
            &format!("   {} Best: {best_name}", icon::CROWN_SIMPLE),
            0.0,
            egui::TextFormat::simple(FontId::proportional(10.5), MUTED),
        );
        job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 18.0);
        let galley = painter.layout_job(job);
        painter.galley(
            egui::pos2(rect.left() + 16.0, y - galley.size().y / 2.0),
            galley,
            INK,
        );
        let name = format!("Similar set of {size} · best {best_name}");
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
        if response
            .dnd_hover_payload::<super::grouping::PhotoDrag>()
            .is_some()
        {
            ui.painter().rect_stroke(
                rect,
                R_CONTROL,
                Stroke::new(2.0, ACCENT),
                StrokeKind::Inside,
            );
        }
        if let Some(drag) = response.dnd_release_payload::<super::grouping::PhotoDrag>() {
            self.drop_on_group(&drag, group);
        }
        let count = self.group_members(group).len();
        response
            .on_hover_text(
                "Drop photos here to move them into this set. Right-click to compare it.",
            )
            .context_menu(|ui| {
                if ui
                    .add_enabled(
                        (2..=9).contains(&count),
                        egui::Button::new(format!("Compare these {count}")),
                    )
                    .on_disabled_hover_text("Comparison shows 2 to 9 photos")
                    .clicked()
                {
                    self.compare_group(group);
                    ui.close();
                }
            });
    }
    /// A photo tile, shared by grids and photo strips. With `captioned` it
    /// carries a two-line caption (name and detail) under the image;
    /// otherwise it is image-only. Its tooltip always gives the file's path.
    pub(super) fn card(
        &mut self,
        ui: &mut egui::Ui,
        index: usize,
        width: f32,
        height: f32,
        captioned: bool,
    ) {
        use egui_phosphor::regular as icon;
        let media = self.media[index].clone();
        let id = media.id;
        let texture = self.texture(id, false);
        let d = self.decision(id);
        let active = self.active == Some(id);
        let comparison = self.comparison.contains(&id);
        let selected = self.library_selection_mode() && self.selection.contains(id);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), self.tile_sense());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                true,
                active || selected,
                media.name(),
            )
        });
        let painter = ui.painter_at(rect);
        let hovered = response.hovered();
        let hover = ui
            .ctx()
            .animate_bool_responsive(response.id.with("hover"), hovered);
        let fill = lerp_color(CARD, CARD_HOVER, hover);
        painter.rect_filled(rect, R_TILE, fill);
        // Thumbnails sit on the tile itself, inset so their rounded corners
        // follow the tile's.
        let inset = if captioned { 6.0 } else { 4.0 };
        let image_rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(inset, inset),
            egui::pos2(
                rect.right() - inset,
                rect.bottom() - if captioned { CAPTION } else { inset },
            ),
        );
        let image_radius = if captioned { 8.0 } else { 6.0 };
        let shown = reveal(ui, id, texture.is_some());
        let mut drawn = image_rect;
        // The placeholder stays under a thumbnail while it fades in.
        if texture.is_none() || shown < 1.0 {
            painter.rect_filled(image_rect, image_radius, WELL);
        }
        if let Some(texture) = texture {
            drawn = paint_image_tinted(
                ui,
                &texture,
                image_rect,
                1.0,
                Vec2::ZERO,
                self.preview_turns(id),
                Color32::from_white_alpha((shown * 255.0) as u8),
            );
            round_corners(&painter, drawn, image_radius, fill);
        } else if self.requested.contains(&(id, false)) {
            shimmer(ui, image_rect);
        } else {
            painter.text(
                image_rect.center(),
                Align2::CENTER_CENTER,
                if self.failures.contains_key(&id) {
                    icon::IMAGE_BROKEN
                } else if media.kind == MediaKind::Video {
                    icon::FILM_STRIP
                } else {
                    icon::IMAGE
                },
                FontId::proportional(22.0),
                FAINT,
            );
        }
        if media.kind == MediaKind::Video && texture_is_drawn(&self.textures, id) {
            badge(
                &painter,
                image_rect.left_bottom() + egui::vec2(S1, -S1),
                Align2::LEFT_BOTTOM,
                icon::PLAY,
                INK,
            );
        }
        if d.rejected {
            // Edged in red, never darkened: the photo must still be judged.
            painter.rect_stroke(
                drawn,
                image_radius,
                Stroke::new(TRASH_EDGE, RED),
                StrokeKind::Inside,
            );
            badge(
                &painter,
                image_rect.left_top() + egui::vec2(S1, S1),
                Align2::LEFT_TOP,
                &format!("{} Trash", icon::TRASH),
                RED,
            );
        }
        if let Some((_, best, size)) = self.similar_group(id)
            && best == id
        {
            badge(
                &painter,
                image_rect.left_bottom() + egui::vec2(S1, -S1),
                Align2::LEFT_BOTTOM,
                &format!("{} Best of {size}", icon::CROWN_SIMPLE),
                ACCENT,
            );
        }
        let rating = self.selected_rating(id);
        if rating > 0 && !captioned {
            badge(
                &painter,
                image_rect.right_top() + egui::vec2(-S1, S1),
                Align2::RIGHT_TOP,
                &"★".repeat(rating as usize),
                ACCENT,
            );
        }
        ring(
            ui,
            response.id,
            rect,
            R_TILE,
            active,
            comparison || selected,
        );
        let check = image_rect.right_bottom() - egui::vec2(S2 + 1.0, S2 + 1.0);
        if self.library_selection_mode()
            && shows_check(selected, hovered, active, self.selection.len())
        {
            check_mark(&painter, check, selected);
        }
        if captioned {
            let name = media.name();
            let left = rect.left() + S2;
            let text_rect = egui::Rect::from_min_max(
                egui::pos2(left, image_rect.bottom()),
                egui::pos2(rect.right() - S2, rect.bottom()),
            );
            painter.with_clip_rect(text_rect).text(
                egui::pos2(left, image_rect.bottom() + 11.0),
                Align2::LEFT_CENTER,
                name,
                FontId::proportional(11.5),
                INK,
            );
            // Organize names each tile's folder in the board's section header;
            // the guided steps have review cards instead.
            let (detail, color) = if rating > 0 {
                ("★".repeat(rating as usize), ACCENT)
            } else if let Some(a) = self.analysis.get(&id).filter(|a| !a.issues.is_empty()) {
                (a.issues.join(" · "), RED)
            } else {
                (
                    format!(
                        "{} · {:.1} MB",
                        media
                            .path
                            .extension()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .to_uppercase(),
                        media.fingerprint.bytes as f64 / 1048576.0
                    ),
                    MUTED,
                )
            };
            let detail = self
                .browse_group_label(id)
                .map_or(detail.clone(), |g| format!("{g} · {detail}"));
            painter.with_clip_rect(text_rect).text(
                egui::pos2(left, image_rect.bottom() + 25.0),
                Align2::LEFT_CENTER,
                detail,
                FontId::proportional(10.5),
                color,
            );
        }
        self.library_response(
            ui,
            response.on_hover_text(media.path.display().to_string()),
            id,
            Some(check),
        );
    }
    /// One row of Review's compact strip: a small thumbnail with the name
    /// and one line of facts beside it. Clicks, drags and the context menu
    /// behave as on a tile.
    pub(super) fn strip_row(&mut self, ui: &mut egui::Ui, index: usize, rect: egui::Rect) {
        use egui_phosphor::regular as icon;
        let media = self.media[index].clone();
        let id = media.id;
        let texture = self.texture(id, false);
        let d = self.decision(id);
        let active = self.active == Some(id);
        let comparison = self.comparison.contains(&id);
        let response = ui.interact(rect, ui.id().with(("strip-row", id)), self.tile_sense());
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, media.name())
        });
        let painter = ui.painter_at(rect);
        let hover = ui
            .ctx()
            .animate_bool_responsive(response.id.with("hover"), response.hovered());
        if active || hover > 0.0 {
            let fill = if active {
                SELECTED
            } else {
                CARD_HOVER.gamma_multiply(hover)
            };
            painter.rect_filled(rect, R_CONTROL, fill);
        }
        let thumb = egui::Rect::from_min_size(
            rect.min + egui::vec2(S1, S1),
            egui::vec2(rect.height() * 1.25, rect.height()) - egui::vec2(0.0, 2.0 * S1),
        );
        let shown = reveal(ui, id, texture.is_some());
        let mut drawn = thumb;
        if texture.is_none() || shown < 1.0 {
            painter.rect_filled(thumb, 5.0, WELL);
        }
        if let Some(texture) = texture {
            drawn = paint_image_tinted(
                ui,
                &texture,
                thumb,
                1.0,
                Vec2::ZERO,
                self.preview_turns(id),
                Color32::from_white_alpha((shown * 255.0) as u8),
            );
        } else if self.requested.contains(&(id, false)) {
            shimmer(ui, thumb);
        }
        if d.rejected {
            painter.rect_stroke(drawn, 3.0, Stroke::new(2.0, RED), StrokeKind::Inside);
        }
        let left = thumb.right() + S2;
        let text = egui::Rect::from_min_max(
            egui::pos2(left, rect.top()),
            egui::pos2(rect.right() - S1, rect.bottom()),
        );
        let clip = painter.with_clip_rect(text);
        clip.text(
            egui::pos2(left, rect.center().y - 7.0),
            Align2::LEFT_CENTER,
            media.name(),
            FontId::proportional(11.5),
            if active {
                INK
            } else {
                lerp_color(MUTED, INK, 0.5)
            },
        );
        // Most telling first: Trash, then a group's best, then the rating.
        let rating = self.selected_rating(id);
        let mut job = egui::text::LayoutJob::default();
        let add = |job: &mut egui::text::LayoutJob, text: &str, color: Color32| {
            if !job.text.is_empty() {
                job.append(" · ", 0.0, egui::TextFormat::simple(medium(10.5), FAINT));
            }
            job.append(text, 0.0, egui::TextFormat::simple(medium(10.5), color));
        };
        if d.rejected {
            add(&mut job, &format!("{} Trash", icon::TRASH), RED);
        }
        if let Some((_, best, size)) = self.similar_group(id)
            && best == id
        {
            add(
                &mut job,
                &format!("{} Best of {size}", icon::CROWN_SIMPLE),
                ACCENT,
            );
        }
        if rating > 0 {
            add(&mut job, &"★".repeat(rating as usize), ACCENT);
        }
        if job.text.is_empty() {
            let kind = media
                .path
                .extension()
                .unwrap_or_default()
                .to_string_lossy()
                .to_uppercase();
            add(
                &mut job,
                &format!(
                    "{kind} · {:.1} MB",
                    media.fingerprint.bytes as f64 / 1048576.0
                ),
                FAINT,
            );
        }
        job.wrap = egui::text::TextWrapping::truncate_at_width(text.width());
        let galley = ui.painter().layout_job(job);
        clip.galley(egui::pos2(left, rect.center().y + 1.0), galley, MUTED);
        ring(
            ui,
            response.id,
            rect,
            R_CONTROL,
            false,
            comparison && !active,
        );
        self.library_response(
            ui,
            response.on_hover_text(media.path.display().to_string()),
            id,
            None,
        );
    }
    /// All layouts use the same selection, comparison and staged mutation paths.
    /// Tiles can be dragged onto a similarity group once Cull has groups.
    fn tile_sense(&self) -> Sense {
        if self.photo_drag_enabled() {
            Sense::click_and_drag()
        } else {
            Sense::click()
        }
    }
    /// `check` is where the tile draws its selection circle: a click there
    /// adds or removes the photo, as Ctrl/Cmd-click does.
    fn library_response(
        &mut self,
        ui: &egui::Ui,
        response: egui::Response,
        id: MediaId,
        check: Option<egui::Pos2>,
    ) {
        if self.photo_drag_enabled() && response.drag_started() {
            // Drag the whole selection when the tile belongs to it.
            let ids = if self.selection.contains(id) {
                self.selection.ordered(&self.media, &self.filtered)
            } else {
                vec![id]
            };
            response.dnd_set_drag_payload(super::grouping::PhotoDrag(ids));
        }
        if response.clicked() {
            self.pending_bookmark = None;
            if self.library_selection_mode() {
                let modifiers = ui.input(|i| i.modifiers);
                let on_check = check
                    .zip(response.interact_pointer_pos())
                    .is_some_and(|(check, pointer)| check.distance(pointer) <= CHECK_RADIUS + 3.0);
                self.select_library(
                    id,
                    modifiers.command || modifiers.ctrl || on_check,
                    modifiers.shift,
                );
            } else if ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                self.compare_with(id);
                self.review = true;
            } else {
                self.comparison.clear();
                self.set_active(id);
            }
        }
        if response.double_clicked() {
            if matches!(self.step, Step::Cull | Step::Organize) {
                self.selection.single(id);
            }
            self.set_active(id);
            self.review = self.step != Step::Organize;
        }
        if response.secondary_clicked()
            && self.library_selection_mode()
            && !self.selection.contains(id)
        {
            self.select_library(id, false, false);
        }
        response.context_menu(|ui| {
            self.library_path_menu(ui, id);
            ui.separator();
            if self.library_selection_mode() {
                if self.step == Step::Organize {
                    self.board_move_menu(ui, id);
                    if self.scene_editing() {
                        self.group_controls(ui);
                    }
                    ui.separator();
                    let count = self.selection.len();
                    if ui
                        .add_enabled(
                            (2..=9).contains(&count),
                            egui::Button::new("Compare selected"),
                        )
                        .clicked()
                    {
                        self.compare_selection();
                        ui.close();
                    }
                } else {
                    self.selection_actions(ui);
                }
                return;
            }
            if ui.button("Review photo").clicked() {
                self.set_active(id);
                self.review = true;
                ui.close();
            }
            if ui.button("Compare with selected").clicked() {
                self.compare_with(id);
                self.review = true;
                ui.close();
            }
            ui.menu_button("Rotate", |ui| {
                for (label, turns) in [
                    ("Clockwise · R", 1),
                    ("Counterclockwise · Shift+R", 3),
                    ("180° · Alt+R", 2),
                ] {
                    if ui.button(label).clicked() {
                        self.rotate(id, turns);
                        ui.close();
                    }
                }
            });
            if ui
                .button(if self.decision(id).rejected {
                    "Unmark"
                } else {
                    "Mark for Trash"
                })
                .clicked()
            {
                self.reject(id);
                ui.close();
            }
        });
    }
}

/// Caption height under grid tiles.
pub(super) const CAPTION: f32 = 34.0;

fn texture_is_drawn(textures: &HashMap<(MediaId, bool), Texture>, id: MediaId) -> bool {
    textures.contains_key(&(id, false))
}

/// A small pill on top of an image: dark translucent fill, coloured text.
fn badge(painter: &egui::Painter, anchor: egui::Pos2, align: Align2, text: &str, color: Color32) {
    let galley = painter.layout_no_wrap(text.to_owned(), medium(10.5), color);
    let size = galley.size() + egui::vec2(12.0, 4.0);
    let rect = align.anchor_size(anchor, size);
    painter.rect_filled(rect, size.y / 2.0, Color32::from_black_alpha(165));
    painter.galley(rect.min + egui::vec2(6.0, 2.0), galley, color);
}

/// A loading thumbnail's placeholder: a shade lighter than its tile, so a
/// photo arriving brightens the tile instead of flashing dark first.
pub(super) const WELL: Color32 = Color32::from_rgb(0x33, 0x32, 0x2F);

/// How far a thumbnail has faded in. The first frame with pixels starts the
/// fade, so a scrolling grid fills softly instead of popping; thumbnails that
/// were already loaded appear at once.
pub(super) fn reveal(ui: &egui::Ui, id: MediaId, loaded: bool) -> f32 {
    let time = ui.ctx().global_style().animation_time * 1.75;
    ui.ctx().animate_bool_with_time_and_easing(
        egui::Id::new(("thumbnail-reveal", id)),
        loaded,
        time,
        egui::emath::easing::quadratic_out,
    )
}

/// The placeholder for a thumbnail on its way: a recessed well with a soft
/// band of light sweeping across it. The band stays in phase across tiles,
/// so a loading grid reads as one wave rather than many blinking cells.
/// Reduce motion keeps the well and drops the sweep.
pub(super) fn shimmer(ui: &egui::Ui, rect: egui::Rect) {
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    painter.rect_filled(rect, 6.0, WELL);
    if ui.ctx().global_style().animation_time == 0.0 {
        return;
    }
    const PERIOD: f64 = 1.4;
    let band = rect.width().max(rect.height()) * 0.7;
    let skew = rect.height() * 0.35;
    // Offset by position so the wave travels diagonally across the grid.
    let phase = (ui.input(|i| i.time) / PERIOD - f64::from(rect.left() + rect.top() * 0.6) / 1800.0)
        .rem_euclid(1.0) as f32;
    let x = rect.left() - band - skew + phase * (rect.width() + band + skew);
    let mut mesh = egui::Mesh::default();
    for (offset, alpha) in [(0.0, 0), (band / 2.0, 11), (band, 0)] {
        let color = Color32::from_white_alpha(alpha);
        mesh.colored_vertex(egui::pos2(x + offset + skew, rect.top()), color);
        mesh.colored_vertex(egui::pos2(x + offset, rect.bottom()), color);
    }
    for i in [0, 2] {
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i + 1, i + 2, i + 3);
    }
    painter.add(mesh);
    ui.ctx().request_repaint();
}

/// The focus ring: accent for the active photo, a quieter line for compared
/// or selected ones. It eases in and out so moving through photos glides.
pub(super) fn ring(
    ui: &egui::Ui,
    id: egui::Id,
    rect: egui::Rect,
    radius: u8,
    active: bool,
    marked: bool,
) {
    let ctx = ui.ctx();
    let strength = ctx.animate_bool_responsive(id.with("ring"), active);
    let marked = ctx.animate_bool_responsive(id.with("marked"), marked && !active);
    if marked > 0.0 {
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(1.5, MUTED.gamma_multiply(marked)),
            StrokeKind::Inside,
        );
    }
    if strength > 0.0 {
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(2.0, ACCENT.gamma_multiply(strength)),
            StrokeKind::Inside,
        );
        // A soft halo outside the tile lifts the active photo off the grid.
        ui.painter().rect_stroke(
            rect,
            radius,
            Stroke::new(3.0, ACCENT_SOFT.gamma_multiply(strength)),
            StrokeKind::Outside,
        );
    }
}

/// Whether a tile in selection mode carries the check. The active tile's
/// outline already marks a single selection; among several selected photos
/// it is checked like the rest.
fn shows_check(selected: bool, hovered: bool, active: bool, selected_count: usize) -> bool {
    if active {
        selected && selected_count > 1
    } else {
        selected || hovered
    }
}

/// Selection mode's check: a filled accent disc when selected, an empty ring
/// on hover to show the tile can be picked.
const CHECK_RADIUS: f32 = 9.0;
fn check_mark(painter: &egui::Painter, center: egui::Pos2, selected: bool) {
    const RADIUS: f32 = CHECK_RADIUS;
    if selected {
        painter.circle_filled(center, RADIUS, ACCENT);
        painter.text(
            center,
            Align2::CENTER_CENTER,
            egui_phosphor::regular::CHECK,
            FontId::proportional(12.0),
            ON_ACCENT,
        );
    } else {
        painter.circle(
            center,
            RADIUS,
            Color32::from_black_alpha(90),
            Stroke::new(1.5, Color32::from_white_alpha(200)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_active_tile_is_checked_only_among_several_selected() {
        // (selected, hovered, active, selected count) -> check
        for (selected, hovered, active, count, check) in [
            (true, false, false, 1, true),
            (false, true, false, 0, true),
            (false, false, false, 2, false),
            (true, false, true, 1, false),
            (true, true, true, 1, false),
            (false, true, true, 2, false),
            (true, false, true, 2, true),
            (true, false, true, 9, true),
        ] {
            assert_eq!(
                shows_check(selected, hovered, active, count),
                check,
                "selected {selected}, hovered {hovered}, active {active}, {count} selected"
            );
        }
    }
    #[test]
    fn staged_rotation_updates_every_thumbnail_layout_without_new_image_work() {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        let id = MediaId(1);
        app.media = vec![Media {
            id,
            path: temp.path().join("photo.jpg"),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        }];
        app.index.insert(id, 0);
        app.filtered = vec![0];
        app.active = Some(id);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let texture = ctx.load_texture(
            "canonical",
            egui::ColorImage::filled([40, 20], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        let texture_id = texture.id();
        // Easy Delete's grid shows suggestions, so give the photo one.
        app.delete_review
            .replace(vec![crate::core::delete_review::Entry {
                subject: id,
                companion: None,
                category: crate::core::delete_review::Category::Dark,
                tag: "Tag".into(),
                keep_why: String::new(),
                eyes: Vec::new(),
                extra: Vec::new(),
                reason: "dark".into(),
            }]);
        app.textures.insert(
            (id, false),
            Texture {
                handle: texture,
                bytes: 3200,
                used: 0,
            },
        );
        // Pick Best's grid needs a group of two; its cards draw photos the
        // same way as Easy Delete's.
        for step in [Step::Cull, Step::Organize, Step::EasyDelete, Step::Rotation] {
            app.step = step;
            // Each workflow is checked on its own requests.
            app.requested.clear();
            for turns in [0, 1, 2, 3] {
                app.decisions.entry(id).or_default().turns = turns;
                app.rotation_review.replace_suggestions(vec![(id, turns)]);
                {
                    let output = ctx.run_test(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(640.0, 600.0),
                            )),
                            ..Default::default()
                        },
                        |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                app.library(ui);
                            });
                        },
                    );
                    // Every image drawn from the texture (a tile, and in
                    // Organize also its preview panel) keeps the turned shape.
                    let mut images = Vec::new();
                    for clipped in &output.shapes {
                        if let egui::Shape::Mesh(mesh) = &clipped.shape
                            && mesh.texture_id == texture_id
                        {
                            let mut bounds = egui::Rect::NOTHING;
                            for vertex in &mesh.vertices {
                                bounds.extend_with(vertex.pos);
                            }
                            images.push(bounds);
                        }
                    }
                    assert!(!images.is_empty());
                    let turned = if turns % 2 == 0 { 2.0 } else { 0.5 };
                    let count = images.len();
                    for (i, bounds) in images.into_iter().enumerate() {
                        // Fix Rotation's card shows the untouched original
                        // before the correction.
                        let expected = if step == Step::Rotation && count > 1 && i == 0 {
                            2.0
                        } else {
                            turned
                        };
                        assert!(bounds.is_positive());
                        assert!((bounds.width() / bounds.height() - expected).abs() < 0.01);
                    }
                    // Organize's preview panel asks for the larger preview
                    // of the clicked photo, rotated or not; thumbnails are
                    // never decoded again for a rotation.
                    assert!(
                        app.requested
                            .iter()
                            .all(|request| step == Step::Organize && *request == (id, true)),
                        "rotation must reuse canonical pixels"
                    );
                    assert_eq!(app.textures.len(), 1);
                }
            }
        }
    }
    #[test]
    fn photos_marked_for_trash_stay_bright_in_every_grid() {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        let id = MediaId(1);
        app.media = vec![Media {
            id,
            path: temp.path().join("photo.jpg"),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        }];
        app.index.insert(id, 0);
        app.filtered = vec![0];
        app.active = Some(id);
        app.decisions.entry(id).or_default().rejected = true;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        // Fade-ins finish at once, so a dim would show in a single frame.
        ctx.global_style_mut(|style| style.animation_time = 0.0);
        let texture = ctx.load_texture(
            "photo",
            egui::ColorImage::filled([40, 30], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        let texture_id = texture.id();
        app.delete_review
            .replace(vec![crate::core::delete_review::Entry {
                subject: id,
                companion: None,
                category: crate::core::delete_review::Category::Dark,
                tag: "Tag".into(),
                keep_why: String::new(),
                eyes: Vec::new(),
                extra: Vec::new(),
                reason: "dark".into(),
            }]);
        for preview in [true, false] {
            app.textures.insert(
                (id, preview),
                Texture {
                    handle: texture.clone(),
                    bytes: 4800,
                    used: 0,
                },
            );
        }
        for step in [Step::Cull, Step::EasyDelete] {
            app.step = step;
            let output = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| app.library(ui));
                },
            );
            super::super::review_card::assert_trash_drawn_bright(&output, texture_id);
        }
    }
    #[test]
    fn grid_virtualizes_and_keeps_requests_focus_and_comparison() {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        app.step = Step::Cull;
        app.media = (0..100_000)
            .map(|n| Media {
                id: MediaId(n),
                path: temp.path().join(format!("photo-{n}.jpg")),
                kind: MediaKind::Photo,
                fingerprint: Fingerprint::default(),
            })
            .collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.filtered = (0..app.media.len()).collect();
        app.active = Some(MediaId(1));
        app.comparison = vec![MediaId(1), MediaId(2)];
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let render = |app: &mut PhotoSortApp, width| {
            let mut offset = None;
            let _ = ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    let ctx = &ui.ctx().clone();
                    {
                        egui::CentralPanel::default().show(ui, |ui| {
                            app.library(ui);
                            assert!(ui.min_rect().right() <= width);
                            let salt = egui::Id::new("library_grid");
                            offset = egui::scroll_area::State::load(
                                ctx,
                                ui.make_persistent_id(egui::IdSalt::new(salt)),
                            )
                            .map(|s| s.offset.y);
                        });
                    }
                },
            );
            offset.expect("rendered scroll state")
        };
        render(&mut app, 360.0);
        let first = app.requested.clone();
        assert!(
            !first.is_empty() && first.len() < 20,
            "virtualization requested {} of 100k images",
            first.len()
        );
        render(&mut app, 360.0);
        assert_eq!(
            first, app.requested,
            "redraw must not request the same thumbnails twice"
        );
        assert_eq!(app.active, Some(MediaId(1)));
        assert_eq!(app.comparison, vec![MediaId(1), MediaId(2)]);
        assert!(app.decisions.is_empty());
        app.set_active(MediaId(0));
        assert_eq!(
            render(&mut app, 576.0),
            0.0,
            "selecting a visible row must not jump the viewport"
        );
        // The thumbnails asked for so far have arrived; at most a few are
        // queued at once, so the revealed item can be requested next.
        app.requested.clear();
        app.set_active(MediaId(99_999));
        render(&mut app, 576.0);
        assert!(
            app.requested.contains(&(MediaId(99_999), false)),
            "keyboard focus must reveal an offscreen item"
        );
        assert!(
            app.requested.len() < 40,
            "revealing focus must stay virtualized"
        );
    }
}

/// A section header's height in a grouped grid or strip.
pub(super) const SET_HEADER: f32 = 26.0;

/// The lines of a Cull grid or strip, see [`PhotoSortApp::photo_lines`].
/// Without sections every line is a row of photos of one height, computed
/// as needed, so a library of any size costs nothing to lay out.
pub(super) struct Lines {
    pub(super) columns: usize,
    /// A row of photos' height.
    pub(super) row: f32,
    gap: f32,
    count: usize,
    /// Headers and rows of photos, with the height of them all.
    sections: Option<(Vec<Line>, f32)>,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Line {
    pub(super) top: f32,
    pub(super) kind: LineKind,
}
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum LineKind {
    /// A similar set (or `None`, photos in no set) shown from `start` to `end`.
    Header {
        group: Option<usize>,
        start: usize,
        end: usize,
    },
    /// Photos `start..end`, side by side.
    Cells { start: usize, end: usize },
}
impl Lines {
    fn stride(&self) -> f32 {
        self.row + self.gap
    }
    fn rows(&self) -> usize {
        self.count.div_ceil(self.columns)
    }
    pub(super) fn height(&self) -> f32 {
        match &self.sections {
            Some((_, height)) => *height,
            None => (self.rows() as f32 * self.stride() - self.gap).max(0.0),
        }
    }
    fn row_line(&self, row: usize) -> Line {
        let start = row * self.columns;
        Line {
            top: row as f32 * self.stride(),
            kind: LineKind::Cells {
                start,
                end: (start + self.columns).min(self.count),
            },
        }
    }
    /// The lines between `top` and `bottom`.
    pub(super) fn visible(&self, top: f32, bottom: f32) -> Vec<Line> {
        match &self.sections {
            Some((lines, _)) => {
                let first = lines
                    .partition_point(|line| line.top <= top)
                    .saturating_sub(1);
                lines[first..]
                    .iter()
                    .take_while(|line| line.top < bottom)
                    .copied()
                    .collect()
            }
            None => {
                let first = (top / self.stride()).floor().max(0.0) as usize;
                let last = ((bottom / self.stride()).ceil() as usize + 1).min(self.rows());
                (first..last).map(|row| self.row_line(row)).collect()
            }
        }
    }
    /// The top of the row holding `position`.
    pub(super) fn top(&self, position: usize) -> f32 {
        match &self.sections {
            Some((lines, _)) => {
                let first = |line: &Line| match line.kind {
                    LineKind::Header { start, .. } | LineKind::Cells { start, .. } => start,
                };
                let at = lines
                    .partition_point(|line| first(line) <= position)
                    .saturating_sub(1);
                lines[at..]
                    .iter()
                    .find(|line| {
                        matches!(line.kind, LineKind::Cells { start, end } if (start..end).contains(&position))
                    })
                    .map_or(0.0, |line| line.top)
            }
            None => (position / self.columns) as f32 * self.stride(),
        }
    }
    /// The top of the row holding `position`, or of its section's header
    /// when it is the first photo there.
    pub(super) fn section_top(&self, position: usize) -> f32 {
        match &self.sections {
            Some((lines, _)) => lines
                .iter()
                .find(
                    |line| matches!(line.kind, LineKind::Header { start, .. } if start == position),
                )
                .map_or_else(|| self.top(position), |line| line.top),
            None => self.top(position),
        }
    }
    /// The first photo on the line at `y`, or on the next line with photos.
    pub(super) fn position_at(&self, y: f32) -> Option<usize> {
        let lines = self.visible(y, y + self.stride() * 2.0);
        lines.into_iter().find_map(|line| match line.kind {
            LineKind::Cells { start, end } if start < end => Some(start),
            _ => None,
        })
    }
}
