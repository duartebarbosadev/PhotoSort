//! The preview beside or over the board: the clicked photo, the
//! compared selection, or a contact sheet of a large selection.
use super::*;

/// The contact sheet's square tile: the largest that fits every photo in
/// the panel, between a small and a comfortable size; past that the sheet
/// scrolls.
pub(in crate::ui::workspace) fn sheet_tile(count: usize, width: f32, height: f32, gap: f32) -> f32 {
    const SMALLEST: f32 = 72.0;
    const LARGEST: f32 = 160.0;
    let fitting = (1..=count.max(1))
        .map(|columns| {
            let side = (width - gap * (columns - 1) as f32) / columns as f32;
            let rows = count.div_ceil(columns) as f32;
            (side, rows * side + (rows - 1.0) * gap <= height)
        })
        .filter(|(_, fits)| *fits)
        .map(|(side, _)| side)
        .fold(0.0_f32, f32::max);
    fitting.clamp(SMALLEST, LARGEST).min(width.max(1.0))
}

impl PhotoSortApp {
    /// The preview panel: the clicked photo (or the compared selection) at a
    /// useful size, with its name, folder and the usual mark, rating and zoom
    /// controls. Scroll to zoom, drag to pan, double-click for 1:1.
    pub(super) fn board_preview(&mut self, ui: &mut egui::Ui) {
        // The preview keeps the width it was given: nothing in it can widen
        // it over the board or past the window.
        let rect = ui.available_rect_before_wrap();
        let mut inner = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(*ui.layout()));
        inner.set_clip_rect(ui.clip_rect().intersect(rect));
        self.board_preview_inside(&mut inner);
        ui.advance_cursor_after_rect(rect);
    }
    /// The preview over the right of the board (`board`), when it does not
    /// fit beside it and was opened from the board's header. It gives way to
    /// the Details drawer, which opens in the same place.
    pub(super) fn board_preview_overlay(&mut self, ctx: &egui::Context, board: egui::Rect) {
        let drawer =
            self.inspector && self.catalog.is_some() && ctx.content_rect().width() <= 1180.0;
        if !self.board.preview_open || drawer {
            return;
        }
        let width = (board.width() * 0.85).clamp(PREVIEW_MIN.min(board.width()), 480.0);
        let rect = egui::Rect::from_min_size(
            egui::pos2(board.right() - width, board.top()),
            egui::vec2(width, board.height()),
        );
        egui::Area::new(egui::Id::new("organize-preview-overlay"))
            .order(egui::Order::Middle)
            .fixed_pos(rect.min)
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, EDGE))
                    .corner_radius(R_SURFACE)
                    .shadow(egui::Shadow {
                        offset: [-6, 0],
                        blur: 18,
                        spread: 0,
                        color: Color32::from_black_alpha(140),
                    })
                    .inner_margin(S2 - 1.0)
                    .show(ui, |ui| {
                        let inner = rect.shrink(S2 - 1.0);
                        ui.set_width(inner.width());
                        ui.set_height(inner.height());
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Preview").font(semibold(13.0)));
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if icon_button(ui, egui_phosphor::regular::X, "Close preview")
                                    .clicked()
                                {
                                    self.board.preview_open = false;
                                }
                            });
                        });
                        self.board_preview(ui);
                    });
            });
    }
    /// The board header's button for the preview when it does not fit beside
    /// the board: it opens the preview over the board, and closes it again.
    pub(super) fn board_preview_toggle(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let open = self.board.preview_open;
        let response = named(
            ui.add(
                egui::Button::new(RichText::new(icon::SIDEBAR_SIMPLE).color(if open {
                    ACCENT
                } else {
                    INK
                }))
                .frame(open)
                .selected(open)
                .min_size(egui::vec2(28.0, 0.0)),
            ),
            egui::WidgetType::Button,
            if open { "Hide preview" } else { "Show preview" },
        )
        .on_hover_text("The clicked photo, over the board");
        if response.clicked() {
            self.board.preview_open = !open;
            // The Details drawer opens in the same place.
            if self.board.preview_open {
                self.inspector = false;
            }
        }
    }
    fn board_preview_inside(&mut self, ui: &mut egui::Ui) {
        // A click on the preview's empty space lets go of a clicked photo,
        // so D marks every selected photo again. Registered first, so every
        // photo and control drawn over it keeps its clicks.
        let background = self.selection_compared().then(|| {
            ui.interact(
                ui.max_rect(),
                ui.id().with("preview-background"),
                Sense::click(),
            )
        });
        self.board_preview_contents(ui);
        if background.is_some_and(|response| response.clicked()) {
            self.comparison_focus = None;
        }
    }
    fn board_preview_contents(&mut self, ui: &mut egui::Ui) {
        use egui_phosphor::regular as icon;
        let ids = self.comparison_ids();
        let Some(active) = self.active.filter(|id| self.index.contains_key(id)) else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Click a photo to preview it here").color(MUTED));
            });
            return;
        };
        if self.board_sheet() {
            self.board_sheet_view(ui, active);
            return;
        }
        let media = self.media[self.index[&active]].clone();
        if self
            .board
            .renaming
            .as_ref()
            .is_some_and(|(id, _)| *id != active)
        {
            self.board.renaming = None;
        }
        let mut finish = false;
        ui.horizontal(|ui| {
            if let Some((_, name)) = &mut self.board.renaming {
                let extension = media
                    .path
                    .extension()
                    .map(|e| format!(".{}", e.to_string_lossy()))
                    .unwrap_or_default();
                let mut field = egui::TextEdit::singleline(name)
                    .id_salt("organize-photo-name")
                    .desired_width((ui.available_width() - 60.0).max(80.0))
                    .show(ui);
                ui.label(RichText::new(extension).color(MUTED));
                if std::mem::take(&mut self.board.rename_focus) {
                    field.response.request_focus();
                    field
                        .state
                        .cursor
                        .set_char_range(Some(egui::text::CCursorRange::two(
                            egui::text::CCursor::new(0),
                            egui::text::CCursor::new(name.chars().count()),
                        )));
                    field.state.store(ui.ctx(), field.response.id);
                }
                finish = field.response.lost_focus();
                self.board.typing |= field.response.has_focus() || finish;
                return;
            }
            let title = self.file_names.get(&active).map_or_else(
                || media.name(),
                |stem| {
                    crate::core::organize::renamed_file_name(
                        &media.path,
                        std::ffi::OsStr::new(stem),
                    )
                    .to_string_lossy()
                    .into_owned()
                },
            );
            let label = ui
                .add(
                    egui::Label::new(RichText::new(title).font(semibold(13.0)))
                        .truncate()
                        .sense(Sense::click()),
                )
                .on_hover_text(format!(
                    "{} · double-click or F2 to rename",
                    media.path.display()
                ));
            if label.double_clicked() {
                self.board_rename_photo(active);
            }
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
                    // The single preview of a large selection: back to all of them.
                    if ui
                        .small_button(format!("Show all {}", self.selection.len()))
                        .on_hover_text("Every selected photo as a contact sheet")
                        .clicked()
                    {
                        self.board.single = None;
                    }
                }
            });
        });
        // Enter or clicking elsewhere keeps the name; Escape keeps the old one.
        if finish {
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                self.board.renaming = None;
            } else {
                self.board_finish_rename();
            }
        }
        let pending = self
            .board_pending()
            .filter(|_| !self.organize_drafts.contains_key(&self.group_mode));
        let mut reason = None;
        let line = match pending {
            None if self.board.original => format!(
                "{}  Now in {}",
                icon::FOLDER_SIMPLE,
                self.readable_folder(media.path.parent().unwrap_or(std::path::Path::new("")))
            ),
            Some(pending) => format!("{}  {}", icon::CIRCLE_DASHED, unplaced_title(Some(pending))),
            None => {
                let placement = self.placement(&media);
                if placement.label.is_none() {
                    reason = Some(format!("Not moving · {}", self.unplaced_reason(&media)));
                }
                if placement.moves {
                    format!("{}  Moves to {}", icon::ARROW_RIGHT, placement.folder)
                } else {
                    format!("{}  Stays in {}", icon::FOLDER_SIMPLE, placement.folder)
                }
            }
        };
        let line = ui.add(egui::Label::new(RichText::new(line).small().color(MUTED)).truncate());
        if let Some(reason) = reason {
            line.on_hover_text(reason);
        }
        ui.add_space(S1);
        // The mark, rating and zoom controls take two rows in a narrow panel,
        // so they never widen it over the board.
        let narrow = ui.available_width() < 560.0;
        let controls = if narrow { 70.0 } else { 34.0 };
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
        if self.swipe_shown() {
            self.swipe_canvas(ui, size);
        } else {
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
        }
        let row = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
            if narrow {
                ui.horizontal_wrapped(|ui| add(ui));
            } else {
                ui.horizontal_centered(|ui| add(ui));
            }
        };
        row(ui, &mut |ui| {
            let (all, rejected) = self.trash_button_state(active);
            let (label, hover) = selection::trash_button_text(all, rejected, ids.len() > 1);
            // Compared selected photos give every label the widest one's
            // width, so picking a photo never moves the controls beside it.
            let wide = if self.selection_compared() {
                selection::trash_button_width(ui, true)
            } else {
                0.0
            };
            if icon_text_button_min(ui, true, rejected, icon::TRASH, label, wide)
                .on_hover_text(hover)
                .clicked()
            {
                self.trash_button_clicked(active);
            }
            self.review_ratings(ui, active);
        });
    }
    /// More than nine selected photos show in the preview as a contact
    /// sheet, unless the single preview was chosen for this selection.
    pub(in crate::ui::workspace) fn board_sheet(&self) -> bool {
        self.step == Step::Organize
            && self.library_selection_mode()
            && self.selection.len() > 9
            && self.board.single != Some(self.selection.len())
    }
    /// The contact sheet: every selected photo in board order, sized to fit
    /// the panel where they can. A click makes a photo active and the only
    /// one D marks; a double-click shows it large, keeping the selection.
    /// Thumbnails are asked for only as their rows scroll into view.
    fn board_sheet_view(&mut self, ui: &mut egui::Ui, active: MediaId) {
        use egui_phosphor::regular as icon;
        let ids = self.selection.ordered(&self.media, &self.filtered);
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{} selected", ids.len())).font(semibold(13.0)));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button("Show one")
                    .on_hover_text("Only the active photo, large · Esc")
                    .clicked()
                {
                    self.board.single = Some(ids.len());
                }
                let marked = ids.iter().all(|id| self.decision(*id).rejected);
                if icon_text_button(
                    ui,
                    !self.busy,
                    marked,
                    icon::TRASH,
                    if marked {
                        "Unmark all"
                    } else {
                        "Mark all for Trash"
                    },
                )
                .on_hover_text(if self.focused_comparison_photo().is_some() {
                    "Every selected photo\nD marks only the outlined photo · Esc lets go of it"
                } else {
                    "Every selected photo · D"
                })
                .clicked()
                {
                    self.toggle_selection_trash();
                }
            });
        });
        ui.label(
            RichText::new("Click a photo so D marks only that one · double-click to see it large")
                .small()
                .color(MUTED),
        );
        ui.add_space(S1);
        let gap = ui.spacing().item_spacing.x;
        let width = ui.available_width();
        let tile = sheet_tile(ids.len(), width, ui.available_height(), gap);
        let columns = (((width + gap) / (tile + gap)).floor() as usize).max(1);
        let rows = ids.len().div_ceil(columns);
        egui::ScrollArea::vertical()
            .id_salt("organize-sheet")
            .auto_shrink([false, false])
            .show_rows(ui, tile, rows, |ui, range| {
                let focus = self.focused_comparison_photo();
                for row in range {
                    ui.horizontal(|ui| {
                        for &id in ids.iter().skip(row * columns).take(columns) {
                            self.sheet_tile(ui, id, tile, id == active, focus);
                        }
                    });
                }
            });
    }
    /// One contact sheet photo. The outline shows what D acts on: every
    /// photo until one is clicked, then that photo alone.
    fn sheet_tile(
        &mut self,
        ui: &mut egui::Ui,
        id: MediaId,
        size: f32,
        active: bool,
        focus: Option<MediaId>,
    ) {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), Sense::click());
        let name = self.media[self.index[&id]].name();
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, active, &name)
        });
        let painter = ui.painter();
        painter.rect_filled(rect, R_CONTROL, CARD);
        if let Some(texture) = self.texture(id, false) {
            // The whole photo, fitted inside the square.
            let image = texture.size_vec2();
            let scale = ((size - 4.0) / image.x.max(1.0)).min((size - 4.0) / image.y.max(1.0));
            let shown = egui::Rect::from_center_size(rect.center(), image * scale);
            painter.image(
                texture.id(),
                shown,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        if self.decision(id).rejected {
            painter.circle_filled(rect.right_top() + egui::vec2(-12.0, 12.0), 9.0, RED);
            painter.text(
                rect.right_top() + egui::vec2(-12.0, 12.0),
                Align2::CENTER_CENTER,
                egui_phosphor::regular::TRASH,
                FontId::proportional(11.0),
                Color32::WHITE,
            );
        }
        let outline = match focus {
            Some(focus) => (focus == id).then_some(Stroke::new(3.0, ACCENT)),
            None => Some(Stroke::new(1.5, ACCENT.gamma_multiply(0.85))),
        };
        if let Some(stroke) = outline {
            painter.rect_stroke(rect, R_CONTROL, stroke, StrokeKind::Inside);
        }
        let response = response.on_hover_text(name);
        if response.clicked() {
            self.set_active(id);
            self.comparison_focus = Some(id);
        }
        if response.double_clicked() {
            self.set_active(id);
            self.board.single = Some(self.selection.len());
        }
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
}
