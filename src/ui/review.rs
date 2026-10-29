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
            self.board_selection_comparison()
                .unwrap_or_else(|| self.active.into_iter().collect())
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
    pub(super) fn view_for(&self, slot: u64) -> InspectionView {
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
    /// Zooms around the centre of the view (keyboard and slider).
    pub(super) fn zoom_view(&mut self, factor: f32) {
        self.zoom_view_at(factor, egui::Vec2::ZERO);
    }
    /// Zooms keeping the image point at `anchor` (relative to the slot's
    /// centre) under the pointer. Zooming out stops at Fit, recentred.
    pub(super) fn zoom_view_at(&mut self, factor: f32, anchor: egui::Vec2) {
        let fit = self.fit_zooms.get(&self.focused_slot).copied();
        let mut view = self.view_for(self.focused_slot);
        let old = view.zoom;
        view.zoom = (old * factor).clamp(view.min_zoom(fit), InspectionView::MAX_ZOOM);
        let ratio = view.zoom / old.max(f32::EPSILON);
        view.pan = [
            anchor.x - (anchor.x - view.pan[0]) * ratio,
            anchor.y - (anchor.y - view.pan[1]) * ratio,
        ];
        self.set_view(self.focused_slot, view.clamped(fit));
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
            self.rotation_canvas_ids(active)
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
        // The strip can sit in a column on the left instead.
        // Easy Delete and Fix Rotation show this beside their grid, which
        // is their strip.
        let beside_grid = matches!(self.step, Step::EasyDelete | Step::Rotation);
        let side_strip = !beside_grid
            && self.preferences.strip_position == crate::core::preferences::StripPosition::Left
            && ui.available_width() >= 640.0;
        if side_strip {
            egui::Panel::left("review_side_strip")
                .resizable(true)
                .default_size(150.0)
                .size_range(100.0..=300.0)
                .frame(egui::Frame::new().inner_margin(egui::Margin {
                    left: 0,
                    right: 8,
                    top: 0,
                    bottom: 0,
                }))
                .show(ui, |ui| self.side_strip(ui));
        }
        let show_filmstrip = !side_strip && !beside_grid && ui.available_height() >= 360.0;
        let filmstrip_height = FILMSTRIP + if self.strip_names() { STRIP_NAME } else { 0.0 };
        let short = ui.ctx().content_rect().height() < 760.0;
        let controls_height = match (compact, short) {
            (true, true) => 80.0,
            (true, false) => 100.0,
            (false, _) => 60.0,
        };
        let height = (ui.available_height()
            - controls_height
            - if show_filmstrip {
                filmstrip_height
            } else {
                0.0
            })
        .max(40.0);
        let ideal_columns = if ids.len() == 4 { 2 } else { 3 };
        let columns = if beside_grid && ids.len() == 2 {
            // A narrow panel: side by side or stacked, whichever shows the
            // photos larger for their shape.
            let ratio = if self.step == Step::Rotation {
                self.rotation_card_ratio(active)
            } else {
                self.delete_current
                    .map_or(0.75, |id| self.delete_card_ratio(id))
            };
            let width = ui.available_width();
            let side_by_side = (width / 2.0).min(height / ratio);
            let stacked = width.min(height / 2.0 / ratio);
            if side_by_side >= stacked { 2 } else { 1 }
        } else {
            ((ui.available_width() / 250.0).floor() as usize)
                .clamp(1, ideal_columns)
                .min(ids.len())
        };
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
                drag: egui::scroll_area::DragScroll::Never,
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
        ui.add_space(S1);
        floating_bar(
            ui,
            egui::Id::new(("review_bar", compact, self.step)),
            |ui| {
                use egui_phosphor::regular as icon;
                const NAVIGATION: &str = "Cull follows Up/Down navigation preferences. Hold Ctrl or Cmd to include marked photos and bypass movement filters.";
                if icon_button(
                    ui,
                    icon::CARET_LEFT,
                    &format!("Previous photo · Up\n{NAVIGATION}"),
                )
                .clicked()
                {
                    self.navigate_selection(
                        -1,
                        false,
                        ui.input(|i| i.modifiers.command || i.modifiers.ctrl),
                    );
                }
                if icon_button(
                    ui,
                    icon::CARET_RIGHT,
                    &format!("Next photo · Down\n{NAVIGATION}"),
                )
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
                        let keep = !self.decision(*id).rejected;
                        if ui
                            .add(decision_button(side, keep))
                            .on_hover_text(format!("Toggle photo {} · {}", side + 1, side + 1))
                            .clicked()
                        {
                            self.toggle_delete_candidate(side);
                        }
                    }
                    divider(ui);
                    self.delete_undo_button(ui);
                } else if self.step == Step::PickBest {
                    let paths = self
                        .pick_review
                        .comparison()
                        .map(|c| c.paths.clone())
                        .unwrap_or_default();
                    for (side, id) in paths.iter().enumerate() {
                        let keep = self.pick_review.comparison().is_none_or(|c| c.keeps(*id));
                        if ui
                            .add(decision_button(side, keep))
                            .on_hover_text(format!("Toggle photo {} · {}", side + 1, side + 1))
                            .clicked()
                        {
                            self.toggle_pick(*id);
                        }
                    }
                    divider(ui);
                    self.pick_confirm_buttons(ui);
                } else if self.step == Step::Rotation {
                    self.rotation_bar_buttons(ui);
                } else {
                    let rejected = self.decision(active).rejected;
                    // Both labels get the wider label's width so toggling never
                    // resizes the bar.
                    let font = egui::TextStyle::Button.resolve(ui.style());
                    let wide = ui
                        .painter()
                        .layout_no_wrap(format!("{}  Marked for Trash", icon::TRASH), font, INK)
                        .size()
                        .x
                        + 2.0 * ui.spacing().button_padding.x;
                    if icon_text_button_min(
                        ui,
                        true,
                        rejected,
                        icon::TRASH,
                        if rejected {
                            "Marked for Trash"
                        } else {
                            "Trash"
                        },
                        wide,
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
                }
                if !self.comparison.is_empty() && ui.small_button("End comparison").clicked() {
                    self.comparison.clear();
                    self.focused_slot = 0;
                }
                if compact {
                    ui.end_row();
                }
                // Fix Rotation stages turns only: no stars.
                if self.step == Step::Rotation {
                    self.review_view_controls(ui);
                } else {
                    self.review_ratings(ui, active);
                }
            },
        );
        if show_filmstrip {
            ui.add_space(S1);
            self.filmstrip(ui);
        }
    }
    pub(super) fn review_ratings(&mut self, ui: &mut egui::Ui, active: MediaId) {
        if self
            .index
            .get(&active)
            .is_some_and(|i| self.media[*i].kind == MediaKind::Video)
        {
            self.review_view_controls(ui);
            return;
        }
        // A comparison rates each photo from its own stars on the canvas.
        if matches!(self.step, Step::Cull | Step::Organize) && self.comparison_ids().len() > 1 {
            self.review_view_controls(ui);
            return;
        }
        ui.add_space(S2);
        let rating = self.selected_rating(active);
        for star in 1..=5 {
            let name = if star == 1 {
                "Rate 1 star".to_owned()
            } else {
                format!("Rate {star} stars")
            };
            if named(
                ui.add(
                    egui::Button::new(RichText::new("★").size(16.0).color(if rating >= star {
                        ACCENT
                    } else {
                        FAINT
                    }))
                    .frame(false),
                ),
                egui::WidgetType::Button,
                &name,
            )
            .on_hover_text(if rating == star {
                "Clear rating · Ctrl/Cmd+0".to_owned()
            } else {
                format!("Rate {star} · Ctrl/Cmd+{star}")
            })
            .clicked()
            {
                // Clicking the current rating again clears it.
                self.rating(active, if rating == star { 0 } else { star });
            }
        }
        // Offered only when there is a rating to clear.
        if rating > 0
            && ui
                .small_button("Clear rating")
                .on_hover_text("Ctrl/Cmd+0")
                .clicked()
        {
            self.rating(active, 0);
        }
        self.review_view_controls(ui);
    }
    fn review_view_controls(&mut self, ui: &mut egui::Ui) {
        if self
            .active
            .and_then(|id| self.index.get(&id))
            .is_some_and(|i| self.media[*i].kind == MediaKind::Video)
        {
            ui.label(
                RichText::new("Space: play/pause · Alt+←/→: seek 10 s")
                    .small()
                    .color(MUTED),
            );
            return;
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
        // A single photo has nothing to sync with. Fix Rotation always shows
        // the original beside its correction.
        let several = self.step == Step::Rotation || self.comparison_ids().len() > 1;
        if several
            && ui
                .selectable_label(self.sync_views, "Sync")
                .on_hover_text("Synchronize pan and zoom across the comparison (F3)")
                .clicked()
        {
            self.toggle_sync();
        }
        let mut view = self.view_for(self.focused_slot);
        let fit = self.fit_zooms.get(&self.focused_slot).copied();
        let lowest = view.min_zoom(fit);
        ui.spacing_mut().slider_width = 60.0;
        // A small handle suits the floating bar.
        ui.spacing_mut().interact_size.y = 14.0;
        // The slider's lowest position is Fit.
        if ui
            .add(
                egui::Slider::new(&mut view.zoom, lowest..=InspectionView::MAX_ZOOM)
                    .logarithmic(true)
                    .text("Zoom")
                    .show_value(false),
            )
            .changed()
        {
            self.set_view(self.focused_slot, view.clamped(fit));
        }
        ui.label(
            RichText::new(format!("{:.0}%", view.zoom * 100.0))
                .small()
                .color(MUTED),
        )
        .on_hover_text("Zoom relative to fitting the whole photo");
    }
    /// Whether photo-strip tiles show file names, in every step.
    pub(super) fn strip_names(&self) -> bool {
        self.preferences.show_filenames
    }
    /// One strip tile: the shared card, with its name below when enabled.
    fn strip_tile(&mut self, ui: &mut egui::Ui, position: usize, rect: egui::Rect, image: Vec2) {
        let index = self.filtered[position];
        ui.scope_builder(
            egui::UiBuilder::new().id_salt(position).max_rect(rect),
            |ui| {
                ui.set_clip_rect(ui.clip_rect().intersect(rect));
                self.card(ui, index, image.x, image.y, false);
            },
        );
        if self.strip_names() {
            let active = Some(self.media[index].id) == self.active;
            let color = if active { INK } else { MUTED };
            let mut job = egui::text::LayoutJob::simple_singleline(
                self.media[index].name(),
                FontId::proportional(10.0),
                color,
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width(rect.width() - 4.0);
            let galley = ui.painter().layout_job(job);
            let at = egui::pos2(
                rect.center().x - galley.size().x / 2.0,
                rect.top() + image.y + (STRIP_NAME - galley.size().y) / 2.0 + 1.0,
            );
            ui.painter().with_clip_rect(rect).galley(at, galley, color);
        }
    }
    fn filmstrip(&mut self, ui: &mut egui::Ui) {
        // The strip is for orientation, the canvas for judging.
        const WIDTH: f32 = 84.0;
        const HEIGHT: f32 = 56.0;
        let name = if self.strip_names() { STRIP_NAME } else { 0.0 };
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
            .max_height(HEIGHT + name + 8.0)
            .show_viewport(ui, |ui, viewport| {
                let origin = ui.max_rect().min;
                ui.set_min_size(egui::vec2(
                    (self.filtered.len() as f32 * stride - ui.spacing().item_spacing.x).max(0.0),
                    HEIGHT + name,
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
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(position as f32 * stride, 0.0),
                        egui::vec2(WIDTH, HEIGHT + name),
                    );
                    self.strip_tile(ui, position, rect, egui::vec2(WIDTH, HEIGHT));
                }
            });
    }
    /// The photo strip as a vertical column beside the canvas.
    fn side_strip(&mut self, ui: &mut egui::Ui) {
        let name = if self.strip_names() { STRIP_NAME } else { 0.0 };
        let width = (ui.available_width() - 10.0).max(60.0);
        let height = (width * 0.66).round();
        let stride = height + name + ui.spacing().item_spacing.y;
        let selection_key = egui::Id::new("side_strip_selection");
        let previous = ui.data(|data| data.get_temp::<MediaId>(selection_key));
        let follow = previous != self.active;
        if let Some(active) = self.active {
            ui.data_mut(|data| data.insert_temp(selection_key, active));
        }
        egui::ScrollArea::vertical()
            .id_salt("review_side_strip")
            .auto_shrink([false, false])
            .show_viewport(ui, |ui, viewport| {
                let origin = ui.max_rect().min;
                ui.set_min_size(egui::vec2(
                    width,
                    (self.filtered.len() as f32 * stride - ui.spacing().item_spacing.y).max(0.0),
                ));
                if follow
                    && let Some(position) = self
                        .filtered
                        .iter()
                        .position(|index| Some(self.media[*index].id) == self.active)
                {
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(0.0, position as f32 * stride),
                        egui::vec2(width, height),
                    );
                    ui.scroll_to_rect(rect, Some(Align::Center));
                }
                let start = (viewport.min.y / stride).floor().max(0.0) as usize;
                let end = ((viewport.max.y / stride).ceil() as usize + 1).min(self.filtered.len());
                for position in start..end {
                    let rect = egui::Rect::from_min_size(
                        origin + egui::vec2(0.0, position as f32 * stride),
                        egui::vec2(width, height + name),
                    );
                    self.strip_tile(ui, position, rect, egui::vec2(width, height));
                }
            });
    }

    pub(super) fn photo_canvas(
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
            self.rotation_canvas_ids(id).len()
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
            // At Fit the whole photo is on screen: a drag still counts as a
            // pan, so it never clicks, but the photo stays put.
            if !view.shows_whole_photo(self.fit_zooms.get(&slot).copied()) {
                view.pan[0] += delta.x;
                view.pan[1] += delta.y;
                self.set_view(slot, view);
            }
            ui.ctx().request_repaint();
        }
        let texture = self
            .texture(id, count <= 4)
            .or_else(|| self.texture(id, false));
        if count == 1 && self.textures.contains_key(&(id, true)) {
            self.prefetch_neighbours(id);
        }
        let d = self.decision(id);
        // Compared photos sit on soft cards; the focused one is outlined.
        if count > 1 {
            ui.painter().rect_filled(rect, R_SURFACE, CHROME);
        }
        // Easy Delete outlines nothing: an outline could be read as Keep or
        // Trash. Fix Rotation shows one photo twice: nothing to pick.
        if count > 1
            && self.focused_slot == slot
            && !matches!(self.step, Step::EasyDelete | Step::Rotation)
        {
            ui.painter().rect_stroke(
                rect,
                R_SURFACE,
                Stroke::new(1.5, ACCENT.gamma_multiply(0.85)),
                StrokeKind::Inside,
            );
        }
        let content = egui::Rect::from_min_max(
            rect.min + egui::vec2(8.0, 28.0),
            rect.max - egui::vec2(8.0, 8.0),
        );
        let mut view = self.view_for(slot);
        let turns = if !corrected {
            0
        } else {
            self.preview_turns(id)
        };
        let known_size = self
            .metadata
            .get(&id)
            .is_some_and(|m| m.width > 0 && m.height > 0);
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
            let fit = InspectionView::fit_zoom(source, viewport, turns, ppp);
            self.fit_zooms.insert(slot, fit);
            // After a resize a view at Fit still fits, and one now below the
            // new Fit snaps to it. Synced views follow the focused photo, so
            // photos of different shapes cannot pull the shared view apart.
            let limited = view.clamped(Some(fit));
            if limited != view && (!self.sync_views || slot == self.focused_slot) {
                view = limited;
                if self.sync_views {
                    self.inspection_view = view;
                } else {
                    self.independent_views.insert(slot, view);
                }
                ui.ctx().request_repaint();
            }
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
            let preview = texture
                .as_ref()
                .map(|t| [t.size()[0] as u32, t.size()[1] as u32]);
            // Only a size read from metadata can show the preview is complete;
            // the texture fallback above would always look complete. 1:1 is an
            // explicit request for source pixels and always loads them.
            let complete = !view.actual_size
                && known_size
                && preview
                    .is_some_and(|preview| InspectionView::preview_is_complete(source, preview));
            let needs_original =
                !complete && view.needs_original(source, preview, viewport, turns, ppp);
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
        if let Some(texture) = &texture {
            self.held.insert(
                slot,
                Held {
                    id,
                    handle: texture.clone(),
                    turns,
                    shown: Instant::now(),
                },
            );
        } else if !detail {
            // Until this photo's pixels arrive, Cull's single view keeps the
            // previous photo on screen rather than flashing an empty canvas.
            // Guided comparisons never do: their pills, badges and keys
            // already belong to the new photo, so they show it loading.
            let hold = self.step == Step::Cull && count == 1;
            let held = self
                .held
                .get(&slot)
                .filter(|held| hold && held.id != id && held.shown.elapsed() < HOLD)
                .map(|held| (held.handle.clone(), held.turns));
            let failed = self.failures.contains_key(&id);
            if let Some((handle, turns)) = held.filter(|_| !failed) {
                paint_image(ui, &handle, content, 1.0, Vec2::ZERO, turns);
                ui.ctx().request_repaint_after(HOLD);
            } else if failed || self.view_started.elapsed() >= LOADING_TEXT_DELAY {
                ui.painter().text(
                    content.center(),
                    Align2::CENTER_CENTER,
                    if failed {
                        "Preview unavailable · see Details"
                    } else {
                        "Loading preview…"
                    },
                    FontId::proportional(13.0),
                    MUTED,
                );
            } else {
                ui.ctx().request_repaint_after(LOADING_TEXT_DELAY);
            }
        }
        // Decision pills, badges and stars appear with the photo's own pixels.
        let pixels = texture.is_some() || detail;
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
                    "Original · 1:1"
                } else {
                    "Original detail"
                },
                FontId::proportional(10.0),
                ACCENT,
            );
        }
        if let Some(&index) = self.index.get(&id) {
            // The file name follows the app-wide file name preference.
            let named = |head: &str| {
                if self.preferences.show_filenames {
                    format!("{head} / {}", self.media[index].name())
                } else {
                    head.to_owned()
                }
            };
            // Fix Rotation's photos need no label: as it is, then turned,
            // under the panel's header with the file name.
            let label = if self.step == Step::Rotation {
                String::new()
            } else {
                let mut label = named(&(slot + 1).to_string());
                // Easy Delete shows what tells the two photos apart, as on
                // its cards, in place of the scores.
                let facts = (self.step == Step::EasyDelete)
                    .then(|| self.delete_photo_facts(id))
                    .flatten();
                if let Some(facts) = &facts {
                    label.push_str(" · ");
                    label.push_str(facts);
                }
                // Pick Best ranks by its cluster score; elsewhere show the
                // photo's quality score once analysis has run.
                let score = (self.step == Step::PickBest)
                    .then(|| self.pick_review.current())
                    .flatten()
                    .and_then(|t| t.cluster.scores.get(&id).copied())
                    .or_else(|| self.analysis.get(&id).map(|a| a.score));
                if let Some(score) = score.filter(|_| facts.is_none()) {
                    label.push_str(&format!(" · score {score:.1}"));
                }
                if let Some(analysis) = self.analysis.get(&id).filter(|_| facts.is_none()) {
                    for issue in &analysis.issues {
                        label.push_str(" · ");
                        label.push_str(issue);
                    }
                }
                label
            };
            // Leave room for the Keep/Trash pill in the top-right corner.
            let pill = matches!(self.step, Step::EasyDelete | Step::PickBest) || d.rejected;
            let mut label_clip = rect;
            if pill {
                // Pick Best and Easy Delete add a Suggested mark beside it.
                label_clip.max.x -= if self.step == Step::PickBest
                    || (self.step == Step::EasyDelete && self.delete_suggested_keeper(id))
                {
                    210.0
                } else {
                    104.0
                };
            }
            ui.painter().with_clip_rect(label_clip).text(
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
        if pixels && matches!(self.step, Step::EasyDelete | Step::PickBest) {
            let keep = if self.step == Step::PickBest {
                self.pick_review.comparison().is_none_or(|c| c.keeps(id))
            } else {
                !self.decision(id).rejected
            };
            let pill = decision_pill(ui.painter(), rect, keep);
            let suggested = if self.step == Step::PickBest {
                self.pick_review
                    .comparison()
                    .is_some_and(|c| c.ai_pick == id)
            } else {
                self.delete_suggested_keeper(id)
            };
            if suggested {
                let badge = suggested_badge(ui.painter(), pill);
                if let Some(why) = (self.step == Step::EasyDelete)
                    .then(|| self.delete_keep_why(id))
                    .flatten()
                {
                    ui.interact(badge, ui.id().with(("suggested", slot)), Sense::hover())
                        .on_hover_text(why);
                }
            }
        } else if pixels && d.rejected {
            decision_pill(ui.painter(), rect, false);
        }
        // In a comparison each photo carries its own stars, so several can be
        // rated without changing focus. Single view keeps the toolbar stars.
        if pixels
            && matches!(self.step, Step::Cull | Step::Organize)
            && self.comparison_ids().len() > 1
        {
            self.slot_stars(ui, rect, id);
        }
        if pan.is_none() && response.clicked() && self.step == Step::EasyDelete {
            self.toggle_delete_candidate(slot as usize);
        }
        if pan.is_none() && response.clicked() && self.step == Step::PickBest {
            self.toggle_pick(id);
        }
        if response.clicked() {
            self.pending_bookmark = None;
            self.active = Some(id);
            self.focused_slot = slot;
        }
        if response.hovered() {
            // Scrolling zooms, and so does a trackpad pinch (or Ctrl+scroll,
            // which egui reports as a zoom instead of a scroll).
            let (delta, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (delta * 0.003).exp() * pinch;
            if factor != 1.0 {
                self.focused_slot = slot;
                let anchor = response
                    .hover_pos()
                    .map_or(egui::Vec2::ZERO, |pointer| pointer - rect.center());
                self.zoom_view_at(factor, anchor);
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

/// The image a canvas slot last showed. See `PhotoSortApp::held`.
pub(super) struct Held {
    id: MediaId,
    handle: TextureHandle,
    turns: u8,
    shown: Instant,
}
/// How long a slot keeps showing the previous photo while the next loads.
const HOLD: Duration = Duration::from_millis(700);
/// How long a slot stays quiet before saying the preview is loading.
const LOADING_TEXT_DELAY: Duration = Duration::from_millis(250);

impl PhotoSortApp {
    /// Requests the previews of the photos before and after `id` in view
    /// order, so stepping to them is instant. Once per active photo, and
    /// only after its own preview has arrived, so it never delays it.
    fn prefetch_neighbours(&mut self, id: MediaId) {
        if self.prefetched_for == Some(id) {
            return;
        }
        self.prefetched_for = Some(id);
        let Some(position) = self
            .index
            .get(&id)
            .and_then(|index| self.filtered.iter().position(|i| i == index))
        else {
            return;
        };
        for next in [position + 1, position.wrapping_sub(1)] {
            if let Some(&index) = self.filtered.get(next)
                && self.media[index].kind != MediaKind::Video
            {
                let _ = self.texture(self.media[index].id, true);
            }
        }
    }
}

/// Height reserved for the filmstrip below the review canvas.
const FILMSTRIP: f32 = 72.0;
/// Height of a file name under a strip tile.
const STRIP_NAME: f32 = 15.0;

/// Keep/Trash toggle for one side of a guided comparison.
fn decision_button(side: usize, keep: bool) -> egui::Button<'static> {
    use egui_phosphor::regular as icon;
    egui::Button::new(
        RichText::new(format!(
            "{}  {}  {}",
            side + 1,
            if keep { icon::CHECK } else { icon::TRASH },
            if keep { "Keep" } else { "Trash" }
        ))
        .color(if keep { GREEN } else { RED }),
    )
}

#[cfg(test)]
#[path = "review/tests.rs"]
mod tests;

/// The Keep/Trash state of a compared photo, pinned to the slot's top-right
/// corner where it stays readable on any image.
fn decision_pill(painter: &egui::Painter, slot: egui::Rect, keep: bool) -> egui::Rect {
    use egui_phosphor::regular as icon;
    let (text, fill) = if keep {
        (format!("{}  Keep", icon::CHECK), GREEN)
    } else {
        (format!("{}  Trash", icon::TRASH), RED)
    };
    let galley = painter.layout_no_wrap(text, medium(12.0), Color32::from_rgb(16, 16, 16));
    let size = galley.size() + egui::vec2(20.0, 8.0);
    let rect = Align2::RIGHT_TOP.anchor_size(slot.right_top() + egui::vec2(-S2, S2), size);
    painter.rect_filled(rect, R_CONTROL, fill);
    painter.galley(rect.min + egui::vec2(10.0, 4.0), galley, Color32::BLACK);
    rect
}
/// Marks Pick Best's suggested keeper, beside its Keep/Trash pill.
fn suggested_badge(painter: &egui::Painter, pill: egui::Rect) -> egui::Rect {
    let text = format!("{}  Suggested", egui_phosphor::regular::STAR);
    let galley = painter.layout_no_wrap(text, medium(12.0), ACCENT);
    let size = galley.size() + egui::vec2(16.0, 8.0);
    let rect = Align2::RIGHT_TOP.anchor_size(pill.left_top() - egui::vec2(S1 + 2.0, 0.0), size);
    painter.rect_filled(rect, R_CONTROL, Color32::from_black_alpha(170));
    painter.rect_stroke(
        rect,
        R_CONTROL,
        Stroke::new(1.0, ACCENT.gamma_multiply(0.6)),
        StrokeKind::Inside,
    );
    painter.galley(rect.min + egui::vec2(8.0, 4.0), galley, ACCENT);
    rect
}

impl PhotoSortApp {
    /// Five frameless stars over a comparison slot's bottom-left corner.
    fn slot_stars(&mut self, ui: &mut egui::Ui, slot: egui::Rect, id: MediaId) {
        let rating = self.selected_rating(id);
        let size = egui::vec2(22.0, 22.0);
        let origin = slot.left_bottom() + egui::vec2(S2, -S2 - size.y);
        let backdrop = egui::Rect::from_min_size(
            origin - egui::vec2(4.0, 2.0),
            egui::vec2(size.x * 5.0 + 8.0, size.y + 4.0),
        );
        ui.painter()
            .rect_filled(backdrop, R_CONTROL, Color32::from_black_alpha(150));
        let responses: Vec<_> = (1..=5u8)
            .map(|star| {
                let rect = egui::Rect::from_min_size(
                    origin + egui::vec2(size.x * f32::from(star - 1), 0.0),
                    size,
                );
                let id = ui.id().with(("slot-star", id, star));
                (star, ui.interact(rect, id, Sense::click()))
            })
            .collect();
        // Hovering previews the rating a click would set.
        let shown = responses
            .iter()
            .find(|(_, r)| r.hovered())
            .map_or(rating, |(star, _)| *star);
        for (star, response) in responses {
            ui.painter().text(
                response.rect.center(),
                Align2::CENTER_CENTER,
                "★",
                FontId::proportional(16.0),
                if star <= shown {
                    ACCENT
                } else {
                    Color32::from_gray(150)
                },
            );
            let label = if rating == star {
                "Clear rating".to_owned()
            } else {
                format!("Rate {star}")
            };
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &label));
            if response.on_hover_text(&label).clicked() {
                self.rating(id, if rating == star { 0 } else { star });
            }
        }
    }
}
