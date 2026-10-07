use super::*;

pub(super) use crate::core::views::Layout;
impl Layout {
    pub const ALL: [Self; 3] = [Self::List, Self::Icons, Self::Grid];
    pub fn title(self) -> &'static str {
        match self {
            Self::List => "List",
            Self::Icons => "Icons",
            Self::Grid => "Grid",
        }
    }
}

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
        if self.step != Step::Cull || self.library_layout == Layout::Grid {
            self.grid(ui);
        } else {
            self.library_rows(ui);
        }
    }
    fn library_scroll(
        &mut self,
        ui: &egui::Ui,
        row_height: f32,
        columns: usize,
        id: egui::Id,
    ) -> egui::ScrollArea {
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt(id)
            .auto_shrink([false, false]);
        if let Some(offset) = self.library_restore_offset.take() {
            return scroll.vertical_scroll_offset(offset);
        }
        if self.step == Step::Cull
            && std::mem::take(&mut self.library_reveal)
            && let Some(position) = self
                .active
                .and_then(|id| self.index.get(&id))
                .and_then(|index| self.filtered.iter().position(|i| i == index))
        {
            let top = (position / columns) as f32 * (row_height + ui.spacing().item_spacing.y);
            let offset =
                egui::scroll_area::State::load(ui.ctx(), ui.make_persistent_id(egui::Id::new(id)))
                    .map_or(0.0, |state| state.offset.y);
            if top < offset {
                scroll = scroll.vertical_scroll_offset(top);
            } else if top + row_height > offset + ui.available_height() {
                scroll = scroll
                    .vertical_scroll_offset((top + row_height - ui.available_height()).max(0.0));
            }
        }
        scroll
    }
    fn library_rows(&mut self, ui: &mut egui::Ui) {
        let icons = self.library_layout == Layout::Icons;
        let height = if icons { 88.0 } else { 40.0 };
        let width = ui.available_width();
        let output = self
            .library_scroll(ui, height, 1, egui::Id::new(("library_rows", icons)))
            .show_rows(ui, height, self.filtered.len(), |ui, range| {
                for row in range {
                    let index = self.filtered[row];
                    let media = self.media[index].clone();
                    let id = media.id;
                    let active = self.active == Some(id);
                    let comparison = self.comparison.contains(&id);
                    let selected = self.library_selection_mode() && self.selection.contains(id);
                    let rejected = self.decision(id).rejected;
                    let texture = self.texture(id, false);
                    let (rect, response) =
                        ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
                    response.widget_info(|| {
                        egui::WidgetInfo::selected(
                            egui::WidgetType::Button,
                            true,
                            active || selected,
                            media.name(),
                        )
                    });
                    ui.painter().rect_filled(rect, 5.0, CARD);
                    if active || comparison || selected {
                        ui.painter().rect_stroke(
                            rect,
                            5.0,
                            Stroke::new(2.0, if active { ACCENT } else { MUTED }),
                            StrokeKind::Inside,
                        );
                    }
                    let size = rect.height() - 8.0;
                    let image = egui::Rect::from_min_size(
                        rect.min + egui::vec2(4.0, 4.0),
                        egui::vec2(size, size),
                    );
                    if let Some(texture) = texture {
                        paint_image(ui, &texture, image, 1.0, Vec2::ZERO, self.preview_turns(id));
                    } else {
                        ui.painter().text(
                            image.center(),
                            Align2::CENTER_CENTER,
                            if media.kind == MediaKind::Video {
                                "V"
                            } else {
                                "PS"
                            },
                            FontId::proportional(10.0),
                            MUTED,
                        );
                    }
                    let left = image.right() + 10.0;
                    let rating = self.selected_rating(id);
                    let trailing = if rejected {
                        "Marked for Trash".into()
                    } else {
                        "★".repeat(rating as usize)
                    };
                    let group = self.browse_group_label(id);
                    let label_width = if group.is_some() {
                        175.0
                    } else if rejected {
                        105.0
                    } else {
                        75.0
                    };
                    let trailing =
                        group.map_or_else(|| trailing.clone(), |g| format!("{g} · {trailing}"));
                    let text_rect = egui::Rect::from_min_max(
                        egui::pos2(left, rect.top()),
                        egui::pos2(rect.right() - label_width - 8.0, rect.bottom()),
                    );
                    let painter = ui.painter().with_clip_rect(text_rect);
                    painter.text(
                        egui::pos2(
                            left,
                            if icons {
                                rect.top() + 23.0
                            } else {
                                rect.center().y
                            },
                        ),
                        Align2::LEFT_CENTER,
                        media.name(),
                        FontId::proportional(12.0),
                        if rejected { MUTED } else { INK },
                    );
                    if icons {
                        let parent = media.path.parent().and_then(|p| {
                            self.root
                                .as_ref()
                                .and_then(|root| p.strip_prefix(root).ok())
                        });
                        let folder = parent
                            .filter(|p| !p.as_os_str().is_empty())
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "Library root".into());
                        painter.text(
                            egui::pos2(left, rect.top() + 47.0),
                            Align2::LEFT_CENTER,
                            folder,
                            FontId::proportional(10.0),
                            MUTED,
                        );
                        painter.text(
                            egui::pos2(left, rect.top() + 66.0),
                            Align2::LEFT_CENTER,
                            format!("{:.1} MB", media.fingerprint.bytes as f64 / 1048576.0),
                            FontId::proportional(10.0),
                            MUTED,
                        );
                    }
                    ui.painter().text(
                        egui::pos2(rect.right() - 9.0, rect.center().y),
                        Align2::RIGHT_CENTER,
                        trailing,
                        FontId::proportional(11.0),
                        if rejected { RED } else { ACCENT },
                    );
                    self.library_response(
                        ui,
                        response.on_hover_text(media.path.display().to_string()),
                        id,
                    );
                }
            });
        self.library_offset = output.state.offset.y;
    }

    pub(super) fn grid(&mut self, ui: &mut egui::Ui) {
        let columns = (ui.available_width() / 190.0).floor().max(1.0) as usize;
        let cell_width =
            (ui.available_width() - 10.0 * (columns.saturating_sub(1) as f32)) / columns as f32;
        let row_height = cell_width * 0.67 + 52.0;
        let rows = self.filtered.len().div_ceil(columns);
        let scroll = self.library_scroll(ui, row_height, columns, egui::Id::new("library_grid"));
        let output = scroll.show_rows(ui, row_height, rows, |ui, range| {
            for row in range {
                ui.horizontal(|ui| {
                    for column in 0..columns {
                        let pos = row * columns + column;
                        if let Some(&index) = self.filtered.get(pos) {
                            self.card(ui, index, cell_width, row_height);
                        }
                    }
                });
            }
        });
        self.library_offset = output.state.offset.y;
    }

    pub(super) fn card(&mut self, ui: &mut egui::Ui, index: usize, width: f32, height: f32) {
        let media = self.media[index].clone();
        let id = media.id;
        let texture = self.texture(id, false);
        let d = self.decision(id);
        let active = self.active == Some(id);
        let comparison = self.comparison.contains(&id);
        let selected = self.library_selection_mode() && self.selection.contains(id);
        let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());
        response.widget_info(|| {
            egui::WidgetInfo::selected(
                egui::WidgetType::Button,
                true,
                active || selected,
                media.name(),
            )
        });
        ui.painter().rect_filled(rect, 8.0, CARD);
        let image_rect = egui::Rect::from_min_max(
            rect.min + egui::vec2(5.0, 5.0),
            egui::pos2(rect.right() - 5.0, rect.bottom() - 49.0),
        );
        if let Some(texture) = texture {
            paint_image(
                ui,
                &texture,
                image_rect,
                1.0,
                Vec2::ZERO,
                self.preview_turns(id),
            );
        } else {
            ui.painter().text(
                image_rect.center(),
                Align2::CENTER_CENTER,
                if self.failures.contains_key(&id) {
                    "Preview unavailable"
                } else if media.kind == MediaKind::Video {
                    "VIDEO"
                } else {
                    "PS"
                },
                FontId::proportional(13.0),
                MUTED,
            );
        }
        if d.rejected {
            ui.painter()
                .rect_filled(image_rect, 4.0, Color32::from_black_alpha(150));
            ui.painter().text(
                image_rect.center(),
                Align2::CENTER_CENTER,
                "MARKED",
                FontId::proportional(13.0),
                RED,
            );
        }
        if active || comparison || selected {
            ui.painter().rect_stroke(
                rect,
                8.0,
                Stroke::new(
                    2.0,
                    if active {
                        ACCENT
                    } else {
                        Color32::from_rgb(216, 187, 128)
                    },
                ),
                StrokeKind::Inside,
            );
        }
        let name = media.name();
        let short = if name.chars().count() > 26 {
            format!("{}…", name.chars().take(24).collect::<String>())
        } else {
            name
        };
        ui.painter().text(
            egui::pos2(rect.left() + 11.0, rect.bottom() - 34.0),
            Align2::LEFT_CENTER,
            short,
            FontId::proportional(12.0),
            INK,
        );
        let rating = self.selected_rating(id);
        let detail = if self.step == Step::Rotation {
            self.rotation_review
                .choice(id)
                .map(|c| {
                    format!(
                        "{} · {}°",
                        if c.confirmed {
                            "Confirmed"
                        } else {
                            "Unconfirmed"
                        },
                        c.degrees()
                    )
                })
                .unwrap_or_else(|| "Not reviewed".into())
        } else if rating > 0 {
            "★".repeat(rating as usize).to_string()
        } else if let Some(a) = self.analysis.get(&id) {
            if !a.issues.is_empty() {
                a.issues.join(" · ")
            } else {
                format!("Quality {:.0}", a.score)
            }
        } else {
            format!(
                "{}  ·  {:.1} MB",
                media
                    .path
                    .extension()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_uppercase(),
                media.fingerprint.bytes as f64 / 1048576.0
            )
        };
        let detail = self
            .browse_group_label(id)
            .map_or_else(|| detail.clone(), |g| format!("{g} · {detail}"));
        ui.painter().with_clip_rect(rect.shrink(4.0)).text(
            egui::pos2(rect.left() + 11.0, rect.bottom() - 15.0),
            Align2::LEFT_CENTER,
            detail,
            FontId::proportional(10.0),
            if rating > 0 { ACCENT } else { MUTED },
        );
        self.library_response(
            ui,
            response.on_hover_text(media.path.display().to_string()),
            id,
        );
    }
    /// All layouts use the same selection, comparison and staged mutation paths.
    fn library_response(&mut self, ui: &egui::Ui, response: egui::Response, id: MediaId) {
        if response.clicked() {
            self.pending_bookmark = None;
            if self.library_selection_mode() {
                let modifiers = ui.input(|i| i.modifiers);
                self.select_library(id, modifiers.command || modifiers.ctrl, modifiers.shift);
            } else if ui.input(|i| i.modifiers.command || i.modifiers.shift) {
                self.compare_with(id);
                self.review = true;
            } else {
                self.comparison.clear();
                self.set_active(id);
            }
        }
        if response.double_clicked() {
            if self.step == Step::Cull || self.scene_editing() {
                self.selection.single(id);
            }
            self.set_active(id);
            self.review = true;
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
                if self.scene_editing() {
                    self.group_controls(ui);
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

#[cfg(test)]
mod tests {
    use super::*;
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
        let texture = ctx.load_texture(
            "canonical",
            egui::ColorImage::filled([40, 20], Color32::WHITE),
            egui::TextureOptions::LINEAR,
        );
        let texture_id = texture.id();
        app.textures.insert(
            (id, false),
            Texture {
                handle: texture,
                bytes: 3200,
                used: 0,
            },
        );
        for step in [
            Step::Cull,
            Step::Organize,
            Step::EasyDelete,
            Step::PickBest,
            Step::Rotation,
        ] {
            app.step = step;
            for turns in [0, 1, 2, 3] {
                app.decisions.entry(id).or_default().turns = turns;
                app.rotation_review.replace_suggestions(vec![(id, turns)]);
                for layout in [Layout::List, Layout::Icons, Layout::Grid] {
                    app.library_layout = layout;
                    let output = ctx.run(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(640.0, 600.0),
                            )),
                            ..Default::default()
                        },
                        |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
                                app.library(ui);
                            });
                        },
                    );
                    let mut bounds = egui::Rect::NOTHING;
                    for primitive in ctx.tessellate(output.shapes, output.pixels_per_point) {
                        if let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive
                            && mesh.texture_id == texture_id
                        {
                            for vertex in mesh.vertices {
                                bounds.extend_with(vertex.pos);
                            }
                        }
                    }
                    assert!(bounds.is_positive());
                    let expected = if turns % 2 == 0 { 2.0 } else { 0.5 };
                    assert!((bounds.width() / bounds.height() - expected).abs() < 0.01);
                    assert!(
                        app.requested.is_empty(),
                        "rotation must reuse canonical pixels"
                    );
                    assert_eq!(app.textures.len(), 1);
                }
            }
        }
    }
    #[test]
    fn list_and_icons_virtualize_and_share_requests_focus_and_comparison() {
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
        app.library_layout = Layout::List;
        let ctx = egui::Context::default();
        theme_install(&ctx);
        let render = |app: &mut PhotoSortApp, width| {
            let mut offset = None;
            let _ = ctx.run(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.0),
                    )),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        app.library(ui);
                        assert!(ui.min_rect().right() <= width);
                        let salt =
                            egui::Id::new(("library_rows", app.library_layout == Layout::Icons));
                        offset = egui::scroll_area::State::load(
                            ctx,
                            ui.make_persistent_id(egui::Id::new(salt)),
                        )
                        .map(|s| s.offset.y);
                    });
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
        app.library_layout = Layout::Icons;
        for width in [360.0, 576.0, 960.0] {
            render(&mut app, width);
        }
        assert_eq!(
            first, app.requested,
            "icon layout must reuse list thumbnail requests"
        );
        assert_eq!(app.active, Some(MediaId(1)));
        assert_eq!(app.comparison, vec![MediaId(1), MediaId(2)]);
        assert!(app.decisions.is_empty());
        app.set_active(MediaId(4));
        assert_eq!(
            render(&mut app, 576.0),
            0.0,
            "selecting a visible row must not jump the viewport"
        );
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
