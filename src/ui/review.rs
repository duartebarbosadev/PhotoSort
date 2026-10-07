use super::*;

impl PhotoSortApp {
    pub(super) fn comparison_ids(&self) -> Vec<MediaId> {
        if self.step == Step::PickBest {
            return self
                .pick_review
                .comparison()
                .map(|c| {
                    if self.pick_focus {
                        c.paths
                            .iter()
                            .copied()
                            .filter(|id| Some(*id) == self.active)
                            .collect()
                    } else {
                        c.paths.clone()
                    }
                })
                .unwrap_or_default();
        }
        if self.step == Step::EasyDelete {
            return self
                .delete_current
                .and_then(|id| self.delete_review.entry(id))
                .map(|e| e.candidates())
                .unwrap_or_default();
        }
        if self.comparison.is_empty() {
            self.active.into_iter().collect()
        } else {
            self.comparison.clone()
        }
    }
    pub(super) fn compare_with(&mut self, id: MediaId) {
        if self.comparison.is_empty()
            && let Some(active) = self.active
        {
            self.comparison.push(active);
        }
        if !self.comparison.contains(&id) && self.comparison.len() < 9 {
            self.comparison.push(id);
        }
        if self.step == Step::Cull {
            self.selection.replace(self.comparison.iter().copied());
        }
        self.review = true;
    }
    fn manual_review_ready(&self, id: MediaId) -> bool {
        self.review
            && !self.scanning
            && !self.modal_active()
            && self.error.is_none()
            && matches!(self.step, Step::Organize | Step::Cull)
            && self
                .index
                .get(&id)
                .and_then(|i| self.media.get(*i))
                .is_some_and(|m| m.id == id)
            && self.comparison_ids().contains(&id)
    }
    fn comparison_others(&self, id: MediaId) -> Vec<MediaId> {
        if !self.manual_review_ready(id) {
            return Vec::new();
        }
        let mut seen = HashSet::new();
        self.comparison_ids()
            .into_iter()
            .filter(|other| {
                *other != id
                    && seen.insert(*other)
                    && self
                        .index
                        .get(other)
                        .and_then(|i| self.media.get(*i))
                        .is_some_and(|m| m.id == *other)
            })
            .collect()
    }
    fn mark_comparison_others(&mut self, id: MediaId, rejected: bool) {
        let others = self
            .comparison_others(id)
            .into_iter()
            .filter(|other| self.decision(*other).rejected != rejected)
            .collect::<Vec<_>>();
        if !others.is_empty() {
            self.stage_manual(others, crate::core::decisions::Change::Trash(rejected));
        }
    }
    pub(super) fn review_context_menu(&mut self, ui: &mut egui::Ui, id: MediaId) {
        self.library_path_menu(ui, id);
        // Guided reviews retain their own provisional choices and confirmation.
        if !matches!(self.step, Step::Organize | Step::Cull) {
            return;
        }
        ui.separator();
        let ready = self.manual_review_ready(id);
        if ui
            .add_enabled(
                ready,
                egui::Button::new(if self.decision(id).rejected {
                    "Unmark this item"
                } else {
                    "Mark this item for Trash"
                }),
            )
            .clicked()
        {
            self.reject(id);
            ui.close();
        }
        let others = self.comparison_others(id);
        if others.is_empty() {
            return;
        }
        let count = others.len();
        ui.label(format!("Affects {count} other displayed items"));
        for (rejected, label) in [
            (true, format!("Mark other {count} for Trash")),
            (false, format!("Unmark other {count}")),
        ] {
            let changes = others
                .iter()
                .any(|other| self.decision(*other).rejected != rejected);
            if ui.add_enabled(changes, egui::Button::new(label)).on_hover_text(
                "Changes only the other items in this comparison. Review changes before moving anything to Trash."
            ).clicked() {
                self.mark_comparison_others(id, rejected);
                ui.close();
            }
        }
    }
    fn view_for(&self, slot: u64) -> InspectionView {
        if self.sync_views {
            self.inspection_view
        } else {
            self.independent_views
                .get(&slot)
                .copied()
                .unwrap_or_default()
        }
    }
    fn set_view(&mut self, slot: u64, view: InspectionView) {
        self.pending_bookmark = None;
        if self.sync_views {
            self.inspection_view = view;
        } else {
            self.independent_views.insert(slot, view);
        }
    }
    pub(super) fn fit_view(&mut self) {
        self.set_view(self.focused_slot, InspectionView::default());
    }
    pub(super) fn actual_size(&mut self) {
        self.set_view(
            self.focused_slot,
            InspectionView {
                actual_size: true,
                ..InspectionView::default()
            },
        );
    }
    pub(super) fn zoom_view(&mut self, factor: f32) {
        let mut view = self.view_for(self.focused_slot);
        view.zoom = (view.zoom * factor).clamp(InspectionView::MIN_ZOOM, InspectionView::MAX_ZOOM);
        self.set_view(self.focused_slot, view);
    }
    pub(super) fn toggle_sync(&mut self) {
        if self.sync_views {
            for slot in 0..9 {
                self.independent_views.insert(slot, self.inspection_view);
            }
        } else {
            self.inspection_view = self.view_for(self.focused_slot);
        }
        self.sync_views ^= true;
    }
    pub(super) fn review_canvas(&mut self, ui: &mut egui::Ui) {
        let Some(active) = self.active else {
            ui.centered_and_justified(|ui| {
                ui.label("Select a photo to begin.");
            });
            return;
        };
        let ids = if self.step == Step::Rotation {
            vec![active, active]
        } else {
            self.comparison_ids()
        };
        if ids.is_empty() {
            ui.label("No review candidates yet. Analyze the library to begin.");
            return;
        }
        let compact = ui.available_width() < 1100.0;
        // On short windows, give the photos room and use Previous/Next instead
        // of squeezing both the canvas and a clipped thumbnail strip below it.
        let show_filmstrip = ui.available_height() >= 360.0;
        let short = ui.ctx().content_rect().height() < 760.0;
        let controls_height = match (compact, short) {
            (true, true) => 80.0,
            (true, false) => 100.0,
            (false, _) => 60.0,
        };
        let height =
            (ui.available_height() - controls_height - if show_filmstrip { 125.0 } else { 0.0 })
                .max(40.0);
        let ideal_columns = if ids.len() == 4 { 2 } else { 3 };
        let columns = ((ui.available_width() / 250.0).floor() as usize)
            .clamp(1, ideal_columns)
            .min(ids.len());
        let rows = ids.len().div_ceil(columns);
        // A single comparison must fit even in a short window. Minimum row
        // height is useful only when multiple rows need a scrolling viewport.
        let row_height = if rows == 1 {
            height
        } else {
            ((height - 10.0 * (rows - 1) as f32) / rows as f32).max(100.0)
        };
        let focus_key = egui::Id::new("inspection_focus_layout");
        let focus_layout = (
            self.focused_slot,
            columns,
            height.round() as u32,
            self.step.index(),
        );
        let follow_focus = ui.data(|data| data.get_temp::<(u64, usize, u32, usize)>(focus_key))
            != Some(focus_layout);
        ui.data_mut(|data| data.insert_temp(focus_key, focus_layout));
        // Floating bars fade out even with AlwaysVisible. Review reserves the
        // wheel for zoom, so its scroll affordance must stay discoverable.
        let scroll_style = ui.spacing().scroll;
        ui.spacing_mut().scroll = egui::style::ScrollStyle::solid();
        egui::ScrollArea::vertical()
            .id_salt("inspection_canvas")
            .scroll_bar_visibility(if rows > 1 {
                egui::scroll_area::ScrollBarVisibility::AlwaysVisible
            } else {
                egui::scroll_area::ScrollBarVisibility::VisibleWhenNeeded
            })
            .scroll_source(egui::scroll_area::ScrollSource {
                scroll_bar: true,
                drag: false,
                mouse_wheel: false,
            })
            .max_height(height)
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for row in 0..rows {
                    if follow_focus && row == self.focused_slot as usize / columns {
                        ui.scroll_to_rect(
                            egui::Rect::from_min_size(
                                ui.next_widget_position(),
                                egui::vec2(ui.available_width(), row_height),
                            ),
                            Some(Align::Center),
                        );
                    }
                    ui.horizontal(|ui| {
                        let width = (ui.available_width()
                            - ui.spacing().item_spacing.x * columns.saturating_sub(1) as f32)
                            / columns as f32;
                        for column in 0..columns {
                            let slot = row * columns + column;
                            if let Some(id) = ids.get(slot) {
                                self.photo_canvas(
                                    ui,
                                    *id,
                                    slot as u64,
                                    egui::vec2(width, row_height),
                                    self.step != Step::Rotation || slot == 1,
                                );
                            }
                        }
                    });
                }
            });
        ui.spacing_mut().scroll = scroll_style;
        ui.horizontal_wrapped(|ui| {
            if ui
                .button("< Previous")
                .on_hover_text("Cull follows Up/Down navigation preferences. Hold Ctrl or Cmd to include marked photos and bypass movement filters.")
                .clicked()
            {
                self.navigate_selection(
                    -1,
                    false,
                    ui.input(|i| i.modifiers.command || i.modifiers.ctrl),
                );
            }
            if ui
                .button("Next >")
                .on_hover_text("Cull follows Up/Down navigation preferences. Hold Ctrl or Cmd to include marked photos and bypass movement filters.")
                .clicked()
            {
                self.navigate_selection(
                    1,
                    false,
                    ui.input(|i| i.modifiers.command || i.modifiers.ctrl),
                );
            }
            if self.step == Step::EasyDelete {
                for (side, id) in ids.iter().enumerate() {
                    let keep = self
                        .delete_current
                        .is_none_or(|review| self.delete_review.keeps(review, *id));
                    if ui
                        .button(format!(
                            "{}: {}",
                            side + 1,
                            if keep { "Keep" } else { "Trash" }
                        ))
                        .clicked()
                    {
                        self.toggle_delete_candidate(side);
                    }
                }
            } else if self.step == Step::PickBest {
                let paths = self
                    .pick_review
                    .comparison()
                    .map(|c| c.paths.clone())
                    .unwrap_or_default();
                for (side, id) in paths.iter().enumerate() {
                    let keep = self.pick_review.comparison().is_none_or(|c| c.keeps(*id));
                    if ui
                        .button(format!(
                            "{}: {}",
                            side + 1,
                            if keep { "Keep" } else { "Trash" }
                        ))
                        .clicked()
                    {
                        self.toggle_pick(*id);
                    }
                }
            } else if ui
                .button(if self.decision(active).rejected {
                    "Unmark"
                } else {
                    "Mark for Trash"
                })
                .clicked()
            {
                self.reject(active);
            }
            if !self.comparison.is_empty() && ui.small_button("End comparison").clicked() {
                self.comparison.clear();
                self.focused_slot = 0;
            }
            if !compact {
                self.review_ratings(ui, active);
            }
        });
        if compact {
            ui.horizontal_wrapped(|ui| self.review_ratings(ui, active));
        }
        if show_filmstrip {
            self.filmstrip(ui);
        }
    }
    fn review_ratings(&mut self, ui: &mut egui::Ui, active: MediaId) {
        if self
            .index
            .get(&active)
            .is_some_and(|i| self.media[*i].kind == MediaKind::Video)
        {
            ui.label(
                RichText::new("Space: play/pause · Alt+←/→: seek 10 s")
                    .small()
                    .color(MUTED),
            );
            return;
        }
        ui.spacing_mut().button_padding = egui::vec2(8.0, 6.0);
        for star in 1..=5 {
            if ui
                .selectable_label(self.selected_rating(active) >= star, "★")
                .clicked()
            {
                self.rating(active, star);
            }
        }
        if ui.small_button("Clear rating").clicked() {
            self.rating(active, 0);
        }
        if ui.small_button("Fit").clicked() {
            self.fit_view();
        }
        if ui
            .small_button("1:1")
            .on_hover_text("One source pixel per display pixel; loads original detail")
            .clicked()
        {
            self.actual_size();
        }
        if ui
            .selectable_label(self.sync_views, "Sync")
            .on_hover_text("Synchronize pan and zoom across the comparison (F3)")
            .clicked()
        {
            self.toggle_sync();
        }
        let mut view = self.view_for(self.focused_slot);
        ui.spacing_mut().slider_width = 65.0;
        if ui
            .add(
                egui::Slider::new(
                    &mut view.zoom,
                    InspectionView::MIN_ZOOM..=InspectionView::MAX_ZOOM,
                )
                .logarithmic(true)
                .text("Zoom")
                .show_value(false),
            )
            .changed()
        {
            self.set_view(self.focused_slot, view);
        }
    }
    fn filmstrip(&mut self, ui: &mut egui::Ui) {
        const WIDTH: f32 = 156.0;
        const HEIGHT: f32 = 100.0;
        let stride = WIDTH + ui.spacing().item_spacing.x;
        let selection_key = egui::Id::new("filmstrip_selection");
        let previous = ui.data(|data| data.get_temp::<MediaId>(selection_key));
        let follow = previous != self.active;
        if let Some(active) = self.active {
            ui.data_mut(|data| data.insert_temp(selection_key, active));
        }
        egui::ScrollArea::horizontal()
            .id_salt("review_filmstrip")
            .auto_shrink([false, false])
            .max_height(HEIGHT + 8.0)
            .show_viewport(ui, |ui, viewport| {
                let origin = ui.max_rect().min;
                ui.set_min_size(egui::vec2(
                    (self.filtered.len() as f32 * stride - ui.spacing().item_spacing.x).max(0.0),
                    HEIGHT,
                ));
                if follow
                    && let Some(position) = self
                        .filtered
                        .iter()
                        .position(|index| Some(self.media[*index].id) == self.active)
                {
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(position as f32 * stride, 0.0),
                        egui::vec2(WIDTH, HEIGHT),
                    );
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
                let start = (viewport.min.x / stride).floor().max(0.0) as usize;
                let end = ((viewport.max.x / stride).ceil() as usize + 1).min(self.filtered.len());
                for position in start..end {
                    let index = self.filtered[position];
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(position as f32 * stride, 0.0),
                        egui::vec2(WIDTH, HEIGHT),
                    );
                    ui.scope_builder(
                        egui::UiBuilder::new().id_salt(position).max_rect(rect),
                        |ui| {
                            ui.set_clip_rect(ui.clip_rect().intersect(rect));
                            self.card(ui, index, WIDTH, HEIGHT);
                        },
                    );
                }
            });
    }

    fn photo_canvas(
        &mut self,
        ui: &mut egui::Ui,
        id: MediaId,
        slot: u64,
        size: Vec2,
        corrected: bool,
    ) {
        if let Some(index) = self.index.get(&id).copied()
            && self.media[index].kind == MediaKind::Video
        {
            self.video_canvas(ui, id, slot, size);
            return;
        }
        let count = if self.step == Step::Rotation {
            2
        } else {
            self.comparison_ids().len().max(1)
        };
        self.canvas_drag.begin_frame(
            ui.ctx(),
            egui::Id::new(("review-pan", self.step, self.comparison_ids())),
        );
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        if !ui.is_rect_visible(rect) {
            return;
        }
        response.context_menu(|ui| self.review_context_menu(ui, id));
        let pan = self.canvas_drag.take_pan_delta(ui, &response);
        if let Some(delta) = pan {
            self.active = Some(id);
            self.focused_slot = slot;
            let mut view = self.view_for(slot);
            view.pan[0] += delta.x;
            view.pan[1] += delta.y;
            self.set_view(slot, view);
            ui.ctx().request_repaint();
        }
        let texture = self
            .texture(id, count <= 4)
            .or_else(|| self.texture(id, false));
        let d = self.decision(id);
        ui.painter().rect_filled(rect, 10.0, BG);
        if count > 1 && self.focused_slot == slot {
            ui.painter()
                .rect_stroke(rect, 10.0, Stroke::new(1.0, ACCENT), StrokeKind::Inside);
        }
        let content = egui::Rect::from_min_max(
            rect.min + egui::vec2(8.0, 28.0),
            rect.max - egui::vec2(8.0, 8.0),
        );
        let view = self.view_for(slot);
        let turns = if !corrected {
            0
        } else {
            self.preview_turns(id)
        };
        let source = self
            .metadata
            .get(&id)
            .filter(|m| m.width > 0 && m.height > 0)
            .map(|m| {
                if (5..=8).contains(&m.orientation) {
                    [m.height, m.width]
                } else {
                    [m.width, m.height]
                }
            })
            .or_else(|| {
                texture
                    .as_ref()
                    .map(|t| [t.size()[0] as u32, t.size()[1] as u32])
            });
        let mut detail = false;
        if let Some(source) = source {
            let ppp = ui.ctx().pixels_per_point();
            let viewport = [content.width(), content.height()];
            let scale = view.scale(source, viewport, turns, ppp);
            if let Some(texture) = texture.as_ref() {
                paint_region(
                    ui,
                    texture,
                    content,
                    source,
                    inspection::Region::full(source[0], source[1]),
                    view,
                    turns,
                );
            }
            let needs_original = view.needs_original(
                source,
                texture
                    .as_ref()
                    .map(|t| [t.size()[0] as u32, t.size()[1] as u32]),
                viewport,
                turns,
                ppp,
            );
            if needs_original
                && (view.actual_size
                    || view.zoom > 1.0
                    || self.view_started.elapsed() >= Duration::from_millis(300))
                && let Some(region) = view.visible_region(source, viewport, turns, ppp)
                && let Some(index) = self.index.get(&id).copied()
                && self.media[index].kind != MediaKind::Video
            {
                let request = InspectionRequest {
                    slot,
                    media: self.media[index].clone(),
                    region,
                    output_edge: (region.width.max(region.height) as f32 * scale * ppp)
                        .ceil()
                        .clamp(1.0, 4096.0) as u32,
                };
                if let Some(frame) = self
                    .inspection_frames
                    .get(&slot)
                    .filter(|frame| frame.request == request)
                {
                    if let Some(texture) = &frame.texture {
                        paint_region(ui, texture, content, source, region, view, turns);
                        detail = true;
                    }
                    if let Some(error) = &frame.error {
                        ui.painter().text(
                            rect.left_bottom() + egui::vec2(10.0, -8.0),
                            Align2::LEFT_BOTTOM,
                            "Detail unavailable · preview retained",
                            FontId::proportional(11.0),
                            RED,
                        );
                        response.clone().on_hover_text(error);
                    }
                }
                self.inspection_wanted.push(request);
            } else if needs_original {
                ui.ctx().request_repaint_after(Duration::from_millis(300));
            }
        }
        if texture.is_none() && !detail {
            ui.painter().text(
                content.center(),
                Align2::CENTER_CENTER,
                self.failures
                    .get(&id)
                    .map(|_| "Preview unavailable · see Details")
                    .unwrap_or("Loading preview…"),
                FontId::proportional(13.0),
                MUTED,
            );
        }
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                true,
                self.focused_slot == slot,
                format!(
                    "Photo {}: {} · {}",
                    slot + 1,
                    self.index
                        .get(&id)
                        .map(|index| self.media[*index].name())
                        .unwrap_or_default(),
                    if detail { "Original detail" } else { "Preview" }
                ),
            )
        });
        if detail {
            ui.painter().text(
                rect.left_bottom() + egui::vec2(10.0, -8.0),
                Align2::LEFT_BOTTOM,
                if view.actual_size && view.zoom == 1.0 {
                    "ORIGINAL · 1:1"
                } else {
                    "ORIGINAL DETAIL"
                },
                FontId::proportional(10.0),
                ACCENT,
            );
        }
        if let Some(&index) = self.index.get(&id) {
            let label = if self.step == Step::Rotation {
                format!(
                    "{} / {}",
                    if self
                        .rotation_review
                        .choice(id)
                        .is_some_and(|c| c.selected == corrected)
                    {
                        if corrected {
                            "CORRECTION · SELECTED"
                        } else {
                            "ORIGINAL · SELECTED"
                        }
                    } else if corrected {
                        "CORRECTION"
                    } else {
                        "ORIGINAL"
                    },
                    self.media[index].name()
                )
            } else {
                format!("{} / {}", slot + 1, self.media[index].name())
            };
            ui.painter().with_clip_rect(rect).text(
                rect.left_top() + egui::vec2(10.0, 14.0),
                Align2::LEFT_CENTER,
                label,
                FontId::proportional(11.0),
                MUTED,
            );
        }
        if let Some(warning) = self.image_warnings.get(&id) {
            let area = egui::Rect::from_min_size(
                rect.left_top() + egui::vec2(10.0, 28.0),
                egui::vec2(130.0, 20.0),
            );
            ui.painter().with_clip_rect(rect).text(
                area.left_center(),
                Align2::LEFT_CENTER,
                "Preview warning",
                FontId::proportional(11.0),
                egui::Color32::from_rgb(233, 191, 116),
            );
            ui.interact(
                area,
                ui.id().with(("image-warning", id)),
                egui::Sense::hover(),
            )
            .on_hover_text(warning);
        }
        if matches!(self.step, Step::EasyDelete | Step::PickBest) {
            let keep = if self.step == Step::PickBest {
                self.pick_review.comparison().is_none_or(|c| c.keeps(id))
            } else {
                self.delete_current
                    .is_none_or(|review| self.delete_review.keeps(review, id))
            };
            ui.painter().text(
                rect.right_bottom() + egui::vec2(-10.0, -8.0),
                Align2::RIGHT_BOTTOM,
                if keep { "KEEP" } else { "TRASH" },
                FontId::proportional(12.0),
                if keep { ACCENT } else { RED },
            );
        } else if d.rejected {
            ui.painter().text(
                rect.right_bottom() + egui::vec2(-10.0, -8.0),
                Align2::RIGHT_BOTTOM,
                "MARKED FOR TRASH",
                FontId::proportional(10.0),
                RED,
            );
        }
        if pan.is_none() && response.clicked() && self.step == Step::EasyDelete {
            self.toggle_delete_candidate(slot as usize);
        }
        if pan.is_none() && response.clicked() && self.step == Step::PickBest {
            self.toggle_pick(id);
        }
        if pan.is_none() && response.clicked() && self.step == Step::Rotation {
            self.select_rotation(corrected);
        }
        if response.clicked() {
            self.pending_bookmark = None;
            self.active = Some(id);
            self.focused_slot = slot;
        }
        if response.hovered() {
            let delta = ui.input(|i| i.smooth_scroll_delta.y);
            if delta.abs() > 0.0 {
                self.focused_slot = slot;
                self.zoom_view((delta * 0.003).exp());
            }
        }
        if pan.is_none() && response.double_clicked() {
            self.focused_slot = slot;
            if self.view_for(slot).actual_size {
                self.fit_view();
            } else {
                self.actual_size();
            }
        }
    }
}

fn paint_region(
    ui: &egui::Ui,
    texture: &TextureHandle,
    rect: egui::Rect,
    source: [u32; 2],
    region: inspection::Region,
    view: InspectionView,
    turns: u8,
) {
    let scale = view.scale(
        source,
        [rect.width(), rect.height()],
        turns,
        ui.ctx().pixels_per_point(),
    );
    let center = rect.center() + egui::vec2(view.pan[0], view.pan[1]);
    let mut mesh = egui::Mesh::with_texture(texture.id());
    for ([x, y], uv) in [
        ([region.x, region.y], egui::pos2(0.0, 0.0)),
        ([region.x + region.width, region.y], egui::pos2(1.0, 0.0)),
        (
            [region.x + region.width, region.y + region.height],
            egui::pos2(1.0, 1.0),
        ),
        ([region.x, region.y + region.height], egui::pos2(0.0, 1.0)),
    ] {
        let [x, y] = inspection::rotate(
            [
                (x as f32 - source[0] as f32 / 2.0) * scale,
                (y as f32 - source[1] as f32 / 2.0) * scale,
            ],
            turns,
        );
        mesh.vertices.push(egui::epaint::Vertex {
            pos: center + egui::vec2(x, y),
            uv,
            color: Color32::WHITE,
        });
    }
    mesh.indices = vec![0, 1, 2, 0, 2, 3];
    ui.painter()
        .with_clip_rect(rect)
        .add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
mod tests {
    use super::*;
    fn comparison_fixture(step: Step) -> (tempfile::TempDir, PhotoSortApp) {
        let temp = tempfile::tempdir().unwrap();
        let mut app =
            PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
        app.media = (0..12)
            .map(|n| Media {
                id: MediaId(n),
                path: temp.path().join(format!("{n}.jpg")),
                kind: if n == 2 {
                    MediaKind::Video
                } else {
                    MediaKind::Photo
                },
                fingerprint: Default::default(),
            })
            .collect();
        app.index = app
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect();
        app.filtered = (0..12).collect();
        app.selection.replace(app.media.iter().map(|m| m.id));
        app.comparison = (0..3).map(MediaId).collect();
        app.active = Some(MediaId(0));
        app.review = true;
        app.step = step;
        (temp, app)
    }
    #[test]
    fn comparison_others_uses_clicked_pane_not_active_selection_or_library() {
        for step in [Step::Organize, Step::Cull] {
            let (_temp, mut app) = comparison_fixture(step);
            app.decisions.entry(MediaId(1)).or_default().rating = Some(5);
            app.decisions.entry(MediaId(9)).or_default().rejected = true;
            app.comparison.extend([MediaId(2), MediaId(99)]);
            assert_eq!(
                app.comparison_others(MediaId(1)),
                vec![MediaId(0), MediaId(2)]
            );
            app.mark_comparison_others(MediaId(1), true);
            assert!(app.decision(MediaId(0)).rejected && app.decision(MediaId(2)).rejected);
            assert!(!app.decision(MediaId(1)).rejected);
            assert_eq!(app.decision(MediaId(1)).rating, Some(5));
            assert!(app.decision(MediaId(9)).rejected);
            assert_eq!(app.decisions.len(), 4);
            assert_eq!(app.active, Some(MediaId(0)));
            assert_eq!(app.selection.len(), 12);
            app.mark_comparison_others(MediaId(1), false);
            assert!(!app.decision(MediaId(0)).rejected && !app.decision(MediaId(2)).rejected);
            assert!(app.decision(MediaId(9)).rejected);
            assert!(app.requested.is_empty() && app.inspection_wanted.is_empty());
            assert!(app.analysis_run.is_none() && app.rotation_run.is_none() && !app.busy);
            assert_eq!(app.stats.decodes, 0);
            assert!(app.plan.is_none() && app.applying_plan.is_none());
            app.runtime.shutdown();
        }
    }
    #[test]
    fn comparison_others_rejects_stale_busy_single_and_guided_contexts() {
        for step in Step::ALL {
            let (_temp, mut app) = comparison_fixture(step);
            if matches!(step, Step::Organize | Step::Cull) {
                for state in 0..7 {
                    app.busy = state == 0;
                    app.scanning = state == 1;
                    app.review = state != 2;
                    app.plan_open = state == 3;
                    app.error = (state == 4).then(|| "error".into());
                    app.comparison = if state == 5 {
                        vec![MediaId(0)]
                    } else if state == 6 {
                        vec![MediaId(1), MediaId(2)]
                    } else {
                        vec![MediaId(0), MediaId(1), MediaId(2)]
                    };
                    app.mark_comparison_others(MediaId(0), true);
                    assert!(app.decisions.is_empty());
                }
            } else {
                app.mark_comparison_others(MediaId(0), true);
                assert!(app.decisions.is_empty());
            }
            assert!(app.requested.is_empty());
            app.runtime.shutdown();
        }
    }
    #[test]
    fn comparison_menu_pointer_action_is_bounded_and_compact() {
        for step in [Step::Organize, Step::Cull] {
            let (_temp, mut app) = comparison_fixture(step);
            let ctx = egui::Context::default();
            ctx.enable_accesskit();
            let input = || egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(340.0, 430.0),
                )),
                ..Default::default()
            };
            let mut draw = |ctx: &egui::Context| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| app.review_context_menu(ui, MediaId(1)));
            };
            let output = ctx.run(input(), &mut draw);
            let tree = output.platform_output.accesskit_update.unwrap();
            let (_, node) = tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some("Mark other 2 for Trash"))
                .unwrap();
            let b = node.bounds().unwrap();
            assert!(b.x0 >= 0.0 && b.x1 <= 340.0 && b.y0 >= 0.0 && b.y1 <= 430.0);
            let pos = egui::pos2(((b.x0 + b.x1) / 2.0) as f32, ((b.y0 + b.y1) / 2.0) as f32);
            let mut raw = input();
            raw.events = vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                },
            ];
            let _ = ctx.run(raw, &mut draw);
            assert!(app.decision(MediaId(0)).rejected && app.decision(MediaId(2)).rejected);
            assert!(!app.decision(MediaId(1)).rejected && !app.decision(MediaId(9)).rejected);
            assert!(app.requested.is_empty() && app.inspection_wanted.is_empty());
            assert_eq!(app.stats.decodes, 0);
            app.runtime.shutdown();
        }
    }
    #[test]
    fn zoom_controls_report_the_same_limits_and_values_as_keyboard_and_wheel() {
        for step in Step::ALL {
            let cache = tempfile::tempdir().unwrap();
            let mut app =
                PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, cache.path().into()), None);
            app.step = step;
            for (factor, expected) in [
                (0.075, 0.075),
                (20.0, 20.0),
                (1000.0, 32.0),
                (0.00001, 0.05),
            ] {
                app.fit_view();
                app.zoom_view(factor);
                let ctx = egui::Context::default();
                ctx.enable_accesskit();
                let output = ctx.run(egui::RawInput::default(), |ctx| {
                    egui::CentralPanel::default()
                        .show(ctx, |ui| app.review_ratings(ui, MediaId(1)));
                });
                let tree = output.platform_output.accesskit_update.unwrap();
                let slider = tree
                    .nodes
                    .iter()
                    .map(|(_, node)| node)
                    .find(|node| node.role() == egui::accesskit::Role::Slider)
                    .unwrap();
                assert_eq!(slider.numeric_value(), Some(f64::from(expected)));
                assert_eq!(
                    slider.min_numeric_value(),
                    Some(f64::from(InspectionView::MIN_ZOOM))
                );
                assert_eq!(
                    slider.max_numeric_value(),
                    Some(f64::from(InspectionView::MAX_ZOOM))
                );
                assert_eq!(app.inspection_view.zoom, expected);
            }
        }
    }
    #[test]
    fn adequate_previews_do_not_request_idle_originals_in_any_workflow() {
        for step in Step::ALL {
            let cache = tempfile::tempdir().unwrap();
            let mut app =
                PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, cache.path().into()), None);
            let id = MediaId(1);
            app.media.push(Media {
                id,
                path: "/missing.ARW".into(),
                kind: MediaKind::Raw,
                fingerprint: Fingerprint::default(),
            });
            app.index.insert(id, 0);
            app.metadata.insert(
                id,
                Metadata {
                    width: 6000,
                    height: 4000,
                    orientation: 1,
                    ..Default::default()
                },
            );
            app.active = Some(id);
            app.step = step;
            app.view_started = Instant::now() - Duration::from_secs(2);
            let ctx = egui::Context::default();
            let texture = ctx.load_texture(
                "preview",
                egui::ColorImage::filled([1200, 800], Color32::WHITE),
                egui::TextureOptions::LINEAR,
            );
            app.textures.insert(
                (id, true),
                Texture {
                    handle: texture,
                    bytes: 1200 * 800 * 4,
                    used: 0,
                },
            );
            let draw = |app: &mut PhotoSortApp| {
                app.inspection_wanted.clear();
                let _ = ctx.run(
                    egui::RawInput {
                        screen_rect: Some(egui::Rect::from_min_size(
                            egui::Pos2::ZERO,
                            egui::vec2(800., 600.),
                        )),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            app.photo_canvas(ui, id, 0, egui::vec2(640., 400.), false);
                        });
                    },
                );
            };
            draw(&mut app);
            assert!(
                app.inspection_wanted.is_empty(),
                "{step:?} scheduled unnecessary full RAW rendering"
            );
            app.actual_size();
            draw(&mut app);
            assert_eq!(
                app.inspection_wanted.len(),
                1,
                "{step:?} did not request explicit original detail"
            );
            assert!(app.decisions.is_empty());
        }
    }
    #[test]
    fn shared_canvas_pans_each_pointer_gesture_once_across_frame_boundaries() {
        for step in Step::ALL {
            for (timing, synced) in
                (0..4).flat_map(|timing| [true, false].map(|synced| (timing, synced)))
            {
                let cache = tempfile::tempdir().unwrap();
                let mut app = PhotoSortApp::with_runtime(
                    Runtime::with_cache(|| {}, cache.path().into()),
                    None,
                );
                let id = MediaId(1);
                app.media.push(Media {
                    id,
                    path: "/missing-photo.jpg".into(),
                    kind: MediaKind::Photo,
                    fingerprint: Fingerprint::default(),
                });
                app.index.insert(id, 0);
                app.metadata.insert(
                    id,
                    Metadata {
                        width: 6000,
                        height: 4000,
                        orientation: 1,
                        ..Default::default()
                    },
                );
                app.active = Some(id);
                app.step = step;
                app.sync_views = synced;
                app.actual_size();
                let ctx = egui::Context::default();
                let frame = |app: &mut PhotoSortApp, events| {
                    app.inspection_wanted.clear();
                    let _ = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(800.0, 600.0),
                            )),
                            events,
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                app.photo_canvas(ui, id, 0, egui::vec2(640.0, 400.0), false);
                            });
                        },
                    );
                };
                for _ in 0..3 {
                    frame(&mut app, vec![]);
                }
                let original = app.inspection_wanted[0].region;
                let start = egui::pos2(100.0, 100.0);
                let middle = egui::pos2(130.0, 115.0);
                let end = egui::pos2(220.0, 160.0);
                let button = |pos, pressed| egui::Event::PointerButton {
                    pos,
                    pressed,
                    button: egui::PointerButton::Primary,
                    modifiers: Default::default(),
                };
                use egui::Event::PointerMoved;
                match timing {
                    0 => {
                        frame(&mut app, vec![PointerMoved(start), button(start, true)]);
                        frame(&mut app, vec![PointerMoved(middle)]);
                        frame(&mut app, vec![PointerMoved(end)]);
                        frame(&mut app, vec![button(end, false)]);
                    }
                    1 => frame(
                        &mut app,
                        vec![
                            PointerMoved(start),
                            button(start, true),
                            PointerMoved(end),
                            button(end, false),
                        ],
                    ),
                    2 => {
                        frame(&mut app, vec![PointerMoved(start), button(start, true)]);
                        frame(&mut app, vec![PointerMoved(end), button(end, false)]);
                    }
                    _ => {
                        frame(
                            &mut app,
                            vec![PointerMoved(start), button(start, true), PointerMoved(end)],
                        );
                        frame(&mut app, vec![button(end, false)]);
                    }
                }
                let moved_now = app.inspection_wanted[0].region;
                assert_eq!(
                    (original.x - moved_now.x, original.y - moved_now.y),
                    (120, 60),
                    "The completed gesture must request the visible region in the same frame"
                );
                for _ in 0..2 {
                    frame(&mut app, vec![]);
                }
                assert_eq!(
                    app.view_for(0).pan,
                    [120.0, 60.0],
                    "{step:?}, timing {timing}, synced {synced}"
                );
                assert_eq!(
                    app.view_for(1).pan,
                    if synced { [120.0, 60.0] } else { [0.0, 0.0] }
                );
                let moved = app.inspection_wanted[0].region;
                assert_eq!((original.x - moved.x, original.y - moved.y), (120, 60));
                assert!(
                    app.decisions.is_empty(),
                    "Panning changed a review decision"
                );
            }
        }
    }
    #[test]
    fn entering_easy_delete_before_analysis_renders_an_empty_review_safely() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.active = Some(MediaId(1));
        app.set_step(Step::EasyDelete);
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| app.review_canvas(ui));
        });
    }
    #[test]
    fn a_single_comparison_row_fits_the_short_canvas_without_clipping() {
        let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
        app.media.push(Media {
            id: MediaId(1),
            path: "/missing-photo.jpg".into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        });
        app.index.insert(MediaId(1), 0);
        app.active = Some(MediaId(1));
        app.set_step(Step::Rotation);
        let ctx = egui::Context::default();
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(640.0, 180.0),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| app.review_canvas(ui));
            },
        );
        let panels: Vec<_> = output
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Rect(rect) = &shape.shape
                    && rect.fill == BG
                    && rect.rect.width() > 200.0
                    && rect.rect.width() < 400.0
                {
                    Some((shape.clip_rect, rect.rect))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            panels.len(),
            2,
            "Exercise both panels in the shared comparison canvas"
        );
        for (clip, panel) in panels {
            assert!(
                clip.contains_rect(panel.shrink(1.0)),
                "Photo panel {panel:?} exceeds viewport {clip:?}"
            );
        }
    }
}
