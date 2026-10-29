use super::*;
use crate::core::details::Row;

impl PhotoSortApp {
    pub(super) fn inspector(&mut self, ui: &mut egui::Ui) {
        ui.add_space(S2);
        let Some(id) = self.active else {
            ui.label(RichText::new("Select a photo to see its details.").color(MUTED));
            return;
        };
        let Some(&index) = self.index.get(&id) else {
            return;
        };
        let media = self.media[index].clone();
        ui.add(egui::Label::new(RichText::new(media.name()).font(semibold(15.0))).truncate())
            .on_hover_text(media.path.display().to_string());
        ui.label(
            RichText::new(format!(
                "{:.1} MB · {:?}",
                media.fingerprint.bytes as f64 / 1048576.0,
                media.kind
            ))
            .small()
            .color(MUTED),
        );
        self.reveal_button(ui, id);
        ui.add_space(10.0);
        ui.separator();
        self.details_tables(ui, id);
        if let Some(analysis) = self.analysis.get(&id) {
            ui.add_space(15.0);
            ui.separator();
            section_title(ui, "Photo quality");
            ui.label(
                RichText::new(format!("{:.0} / 100", analysis.score))
                    .size(24.0)
                    .color(ACCENT),
            );
            ui.label(format!("Sharpness  {:.1}", analysis.sharpness));
            ui.label(format!("Brightness  {:.0} / 255", analysis.brightness));
            if let Some(faces) = &analysis.faces {
                ui.label(format!(
                    "{} faces · {} possibly closed eyes",
                    faces.count, faces.closed_eyes
                ));
            }
            for issue in &analysis.issues {
                ui.colored_label(RED, issue);
            }
            ui.label(
                RichText::new(if analysis.aesthetic.is_some() {
                    "Aesthetic quality, sharpness and open-eye suggestions."
                } else {
                    "Sharpness and exposure. Run Pick Best for aesthetic scoring."
                })
                .small()
                .color(MUTED),
            );
        }
        if let Some(original) = self.duplicates.get(&id)
            && let Some(&index) = self.index.get(original)
        {
            ui.colored_label(RED, "Exact duplicate");
            ui.label(self.media[index].name());
        }
        if let Some(other) = self.near_duplicates.get(&id).copied() {
            ui.label(RichText::new("Similar frame").color(ACCENT));
            if ui.button("Compare before marking").clicked() {
                self.compare_with(other);
                self.review = true;
            }
        }
        if let Some(warning) = self.image_warnings.get(&id) {
            ui.add_space(10.0);
            ui.colored_label(egui::Color32::from_rgb(233, 191, 116), "Preview warning");
            ui.label(warning);
        }
        if let Some(error) = self.analysis_failures.get(&id) {
            ui.add_space(10.0);
            ui.colored_label(RED, "This photo could not be analyzed");
            ui.add(egui::Label::new(RichText::new(error).small().color(MUTED)).wrap());
            ui.label(
                RichText::new("It gets no suggestions and is never marked for Trash automatically. Analyze again to retry.")
                    .small()
                    .color(MUTED),
            );
        }
        if let Some(error) = self.failures.get(&id) {
            ui.add_space(10.0);
            ui.colored_label(RED, error);
            if ui.button("Retry preview").clicked() {
                self.failures.remove(&id);
            }
        }
        if media.kind == MediaKind::Video
            && crate::core::playback::supported()
            && ui.button("Watch here").clicked()
        {
            self.review = true;
            self.runtime.playback.select(Some(&media));
            self.video_context = Some((self.step, media.id));
            self.runtime.playback.toggle();
        }
        if media.kind == MediaKind::Video
            && ui
                .add_enabled(
                    self.can_open_desktop(id),
                    egui::Button::new("Open in external player"),
                )
                .clicked()
        {
            self.open_desktop(id, crate::core::desktop::Action::OpenVideo);
        }
        ui.add_space(15.0);
        ui.separator();
        ui.label(
            RichText::new("Changes stay staged until you review and apply them.")
                .small()
                .color(MUTED),
        );
    }
}

impl PhotoSortApp {
    /// Details in windows too narrow for a side panel: a full-height drawer
    /// over the right edge, so the photos keep their width when it closes.
    pub(super) fn inspector_drawer(&mut self, ctx: &egui::Context) {
        let screen = ctx.content_rect();
        let width = (screen.width() * 0.85).min(320.0);
        let top = screen.top() + navigation::HEADER_HEIGHT;
        let height = screen.bottom() - navigation::STATUS_HEIGHT - top;
        egui::Area::new(egui::Id::new("inspector-drawer"))
            .order(egui::Order::Middle)
            .fixed_pos(egui::pos2(screen.right() - width, top))
            .show(ctx, |ui| {
                egui::Frame::new()
                    .fill(PANEL)
                    .stroke(Stroke::new(1.0, EDGE))
                    .shadow(egui::Shadow {
                        offset: [-6, 0],
                        blur: 18,
                        spread: 0,
                        color: Color32::from_black_alpha(140),
                    })
                    .inner_margin(S4 as i8)
                    .show(ui, |ui| {
                        ui.set_width(width - 2.0 * S4);
                        ui.set_height(height - 2.0 * S4);
                        ui.horizontal(|ui| {
                            ui.label(RichText::new("Details").font(semibold(14.0)));
                            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                if icon_button(ui, egui_phosphor::regular::X, "Close details · I")
                                    .clicked()
                                {
                                    self.inspector = false;
                                }
                            });
                        });
                        egui::ScrollArea::vertical()
                            .id_salt("inspector-drawer-scroll")
                            .show(ui, |ui| self.inspector(ui));
                    });
            });
    }

    /// Metadata for the active photo as soft cards of icon, label and value,
    /// or every compared photo: side by side, one column per photo, when the
    /// panel is wide enough, otherwise stacked with shared values said once.
    fn details_tables(&self, ui: &mut egui::Ui, active: MediaId) {
        let mut ids = self.comparison_ids();
        if ids.len() < 2 || !ids.contains(&active) {
            ids = vec![active];
        }
        let empty = Metadata::default();
        let items: Vec<(&Media, &Metadata)> = ids
            .iter()
            .filter_map(|id| {
                let media = self.media.get(*self.index.get(id)?)?;
                Some((media, self.metadata.get(id).unwrap_or(&empty)))
            })
            .collect();
        let sections = crate::core::details::sections(&items);
        if items.len() < 2 {
            for section in &sections {
                ui.add_space(S2);
                details_card(ui, |ui| {
                    section_title(ui, section.title);
                    let tiles: Vec<&Row> = section
                        .rows
                        .iter()
                        .filter(|row| EXPOSURE.contains(&row.label))
                        .collect();
                    let mut tiled = false;
                    for row in &section.rows {
                        if !EXPOSURE.contains(&row.label) {
                            ui.horizontal_top(|ui| {
                                ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                                field_label(ui, row.label);
                                value_cell(ui, ui.available_width(), &row.values[0], INK);
                            });
                        } else if !tiled {
                            tiled = true;
                            exposure_tiles(ui, &tiles);
                        }
                    }
                });
            }
            return;
        }
        ui.add_space(S3);
        section_title(ui, "Compare details");
        for (n, (media, _)) in items.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = S2;
                number_badge(ui, n);
                ui.add(egui::Label::new(media.name()).truncate())
                    .on_hover_text(media.path.display().to_string());
            });
        }
        let names: Vec<String> = items.iter().map(|(media, _)| media.name()).collect();
        // Decided once from the panel width, before any cell is laid out, so
        // long values wrap inside their column and never widen the panel.
        let column = table_column_width(ui.available_width() - 2.0 * CARD_MARGIN_X, names.len());
        for section in &sections {
            ui.add_space(S2);
            // A section every photo agrees on reads as one plain list.
            let column = column.filter(|_| section.rows.iter().any(Row::differs));
            details_card(ui, |ui| match column {
                Some(column) => {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                        ui.allocate_ui_with_layout(
                            egui::vec2(LABEL_WIDTH, 0.0),
                            egui::Layout::top_down(Align::Min),
                            |ui| {
                                ui.set_width(LABEL_WIDTH);
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(section.title).font(medium(12.5)).color(INK),
                                    )
                                    .truncate(),
                                );
                            },
                        );
                        for (n, name) in names.iter().enumerate() {
                            ui.allocate_ui_with_layout(
                                egui::vec2(column, 0.0),
                                egui::Layout::left_to_right(Align::Center),
                                |ui| {
                                    ui.set_width(column);
                                    number_badge(ui, n).on_hover_text(name);
                                },
                            );
                        }
                    });
                    for row in &section.rows {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                            field_label(ui, row.label);
                            if !row.differs() {
                                value_cell(ui, ui.available_width(), &row.values[0], INK);
                                return;
                            }
                            for (value, name) in row.values.iter().zip(&names) {
                                value_cell(ui, column, value, ACCENT).on_hover_text(name);
                            }
                        });
                    }
                }
                None => {
                    section_title(ui, section.title);
                    for row in &section.rows {
                        ui.horizontal_top(|ui| {
                            ui.spacing_mut().item_spacing.x = COLUMN_GAP;
                            field_label(ui, row.label);
                            if !row.differs() {
                                value_cell(ui, ui.available_width(), &row.values[0], INK);
                                return;
                            }
                            ui.allocate_ui_with_layout(
                                egui::vec2(ui.available_width(), 0.0),
                                egui::Layout::top_down(Align::Min),
                                |ui| {
                                    for (value, photos) in row.groups() {
                                        ui.horizontal_top(|ui| {
                                            ui.spacing_mut().item_spacing.x = 3.0;
                                            for &n in &photos {
                                                number_badge(ui, n).on_hover_text(&names[n]);
                                            }
                                            ui.add_space(S1);
                                            value_cell(ui, ui.available_width(), value, ACCENT);
                                        });
                                    }
                                },
                            );
                        });
                    }
                }
            });
        }
    }
}

/// Label column of the Details grids, icon included.
const LABEL_WIDTH: f32 = 96.0;
/// Narrowest photo column before a comparison stacks instead.
const MIN_COLUMN: f32 = 56.0;
const COLUMN_GAP: f32 = S2;
const CARD_MARGIN_X: f32 = 10.0;
const ICON_SIZE: f32 = 14.0;
/// Camera settings shown as tiles for a single photo.
const EXPOSURE: [&str; 4] = ["Focal length", "Aperture", "Shutter", "ISO"];

/// Width of each photo column when `photos` compared photos fit side by side
/// in `width` after the label column, or `None` when they should stack.
fn table_column_width(width: f32, photos: usize) -> Option<f32> {
    let photos = photos.max(1) as f32;
    let column = (width - LABEL_WIDTH - photos * COLUMN_GAP) / photos;
    (column >= MIN_COLUMN).then_some(column)
}

/// The glyph drawn before a Details label, from the app's icon font.
fn field_icon(label: &str) -> &'static str {
    use egui_phosphor::regular as icon;
    match label {
        "Format" => icon::FILE_IMAGE,
        "Size" => icon::HARD_DRIVE,
        "Created" => icon::CALENDAR_BLANK,
        "Location" => icon::MAP_PIN,
        "Source" => icon::CAMERA,
        "Lens" => icon::TARGET,
        "Focal length" => icon::ARROWS_IN_LINE_HORIZONTAL,
        "Aperture" => icon::APERTURE,
        "Shutter" => icon::TIMER,
        "ISO" => icon::GAUGE,
        "Flash" => icon::LIGHTNING,
        "Dimensions" => icon::FRAME_CORNERS,
        "Megapixels" => icon::GRID_FOUR,
        "Orientation" => icon::DEVICE_ROTATE,
        "Bit depth" => icon::STACK,
        "Duration" => icon::CLOCK,
        "Frame rate" => icon::FILM_STRIP,
        "Codec" => icon::FILE_VIDEO,
        "Bitrate" => icon::WAVEFORM,
        "Audio" => icon::SPEAKER_HIGH,
        "Color space" => icon::PALETTE,
        "White balance" => icon::THERMOMETER_SIMPLE,
        "Metering" => icon::SCAN,
        "Exposure mode" => icon::SLIDERS_HORIZONTAL,
        "Exposure comp." => icon::PLUS_MINUS,
        "Scene" => icon::SUN_HORIZON,
        "Software" => icon::MAGIC_WAND,
        _ => icon::INFO,
    }
}

/// A glyph painted rather than added as a label, so screen readers hear the
/// field name next to it and not the icon's code point.
fn paint_icon(ui: &mut egui::Ui, icon: &str, size: egui::Vec2, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        icon,
        egui::FontId::proportional(ICON_SIZE),
        color,
    );
}

/// Icon and field name in the fixed label column; long names wrap.
fn field_label(ui: &mut egui::Ui, label: &str) {
    ui.allocate_ui_with_layout(
        egui::vec2(LABEL_WIDTH, 0.0),
        egui::Layout::left_to_right(Align::Min),
        |ui| {
            ui.set_width(LABEL_WIDTH);
            ui.spacing_mut().item_spacing.x = 6.0;
            let height = ui.text_style_height(&egui::TextStyle::Body);
            paint_icon(ui, field_icon(label), egui::vec2(16.0, height), MUTED);
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), 0.0),
                egui::Layout::top_down(Align::Min),
                |ui| ui.add(egui::Label::new(RichText::new(label).color(MUTED)).wrap()),
            );
        },
    );
}

/// One value in a column of `width`; a missing value reads as a faint dash.
fn value_cell(ui: &mut egui::Ui, width: f32, value: &str, color: Color32) -> egui::Response {
    let text = if value.is_empty() {
        RichText::new("-").color(FAINT)
    } else {
        RichText::new(value).color(color)
    };
    ui.allocate_ui_with_layout(
        egui::vec2(width, 0.0),
        egui::Layout::top_down(Align::Min),
        |ui| {
            ui.set_width(width);
            ui.add(egui::Label::new(text).wrap())
        },
    )
    .inner
}

/// The small numbered chip that names compared photo `n` (0-based) as "1", "2"...
fn number_badge(ui: &mut egui::Ui, n: usize) -> egui::Response {
    let text = (n + 1).to_string();
    let galley = ui
        .painter()
        .layout_no_wrap(text.clone(), semibold(10.5), INK);
    let height = ui.text_style_height(&egui::TextStyle::Body);
    let size = egui::vec2((galley.size().x + 8.0).max(16.0), height.min(16.0));
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size.x, height), egui::Sense::hover());
    let pill = egui::Rect::from_center_size(rect.center(), size);
    ui.painter().rect_filled(pill, 5.0, SELECTED);
    ui.painter()
        .galley(pill.center() - galley.size() / 2.0, galley, INK);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Label, true, &text));
    response
}

/// A soft rounded card that groups one Details section.
fn details_card(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(ELEVATED)
        .corner_radius(R_TILE)
        .inner_margin(egui::Margin::symmetric(CARD_MARGIN_X as i8, S2 as i8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 6.0;
            add(ui);
        });
}

/// Focal length, aperture, shutter and ISO as small tiles, the way a camera
/// shows them, wrapping onto a second line in a narrow panel.
fn exposure_tiles(ui: &mut egui::Ui, rows: &[&Row]) {
    const GAP: f32 = 6.0;
    const MIN_TILE: f32 = 58.0;
    let width = ui.available_width();
    let fit = (((width + GAP) / (MIN_TILE + GAP)).floor() as usize).clamp(1, rows.len());
    // Even lines, two and two rather than three and one.
    let per_line = rows.len().div_ceil(rows.len().div_ceil(fit));
    let tile = (width - GAP * (per_line - 1) as f32) / per_line as f32;
    for line in rows.chunks(per_line) {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            for row in line {
                let value = row.values[0].as_str();
                let value = if row.label == "ISO" {
                    value.trim_start_matches("ISO ")
                } else {
                    value
                };
                egui::Frame::new()
                    .fill(CARD_HOVER)
                    .corner_radius(R_CONTROL)
                    .inner_margin(egui::Margin::symmetric(4, 6))
                    .show(ui, |ui| {
                        ui.set_width(tile - 8.0);
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.vertical_centered(|ui| {
                            paint_icon(ui, field_icon(row.label), egui::vec2(16.0, 16.0), MUTED);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(value).font(semibold(12.5)).color(INK),
                                )
                                .truncate(),
                            )
                            .on_hover_text(row.label);
                            ui.add(
                                egui::Label::new(RichText::new(row.label).size(10.5).color(FAINT))
                                    .truncate(),
                            );
                        });
                    });
            }
        });
    }
}

impl PhotoSortApp {
    /// Where an item goes in the current folder proposal, edits included.
    pub(super) fn placement(&self, media: &Media) -> Placement {
        let label = match self.organize_drafts.get(&self.group_mode) {
            Some(draft) => draft.labels().get(&media.id).cloned(),
            None => self.organize_label(media),
        };
        let parent = media.path.parent().unwrap_or(std::path::Path::new(""));
        // Compared as folders on disk: with another destination, an item
        // whose folder name stays the same still moves.
        let target = label.as_deref().map(|label| {
            crate::core::operations::folder_destination(std::path::Path::new(&self.output), label)
        });
        match target.filter(|target| target != parent) {
            Some(target) => Placement {
                label,
                moves: true,
                folder: self.readable_folder(&target),
            },
            None => Placement {
                label,
                moves: false,
                folder: self.readable_folder(parent),
            },
        }
    }
    /// A folder as "Photos / 2025 / Trip": from the opened folder's name when
    /// it is inside it, otherwise the path with the home folder as "~".
    pub(super) fn readable_folder(&self, path: &std::path::Path) -> String {
        if let Some(root) = self.root.as_deref()
            && let Ok(relative) = path.strip_prefix(root)
        {
            let name = root
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.display().to_string());
            return std::iter::once(name)
                .chain(
                    relative
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned()),
                )
                .collect::<Vec<_>>()
                .join(" / ");
        }
        readable_path(path)
    }
}
/// Where an item goes: its proposed folder label (`None` keeps it where it
/// is), whether that moves it, and the folder it ends up in, readably.
pub(super) struct Placement {
    pub(super) label: Option<String>,
    pub(super) moves: bool,
    pub(super) folder: String,
}
/// A path with the home folder shown as "~".
pub(super) fn readable_path(path: &std::path::Path) -> String {
    if let Some(home) = dirs::home_dir()
        && let Ok(rest) = path.strip_prefix(&home)
    {
        return if rest.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", rest.display())
        };
    }
    path.display().to_string()
}
/// Sentence-case section heading for the Details panel.
fn section_title(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title).font(medium(12.5)).color(INK));
    ui.add_space(2.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compared_photos_sit_side_by_side_only_when_every_column_stays_readable() {
        // Card widths inside the Details panel at its default 300 px, at
        // 440 px and at its widest 560 px.
        let (default, wider, widest) = (232.0, 372.0, 492.0);
        assert!(table_column_width(default, 2).is_some());
        assert_eq!(table_column_width(default, 3), None);
        assert!(table_column_width(wider, 4).is_some());
        assert_eq!(table_column_width(wider, 5), None);
        let column = table_column_width(widest, 4).unwrap();
        assert!(column >= MIN_COLUMN);
        assert!((LABEL_WIDTH + 4.0 * (column + COLUMN_GAP) - widest).abs() < 0.01);
        assert_eq!(table_column_width(0.0, 0), None);
    }

    #[test]
    fn every_details_field_has_its_own_icon() {
        use crate::core::model::{Details, Media, MediaId, MediaKind, Metadata};
        use crate::core::video_info::VideoInfo;
        let media = Media {
            id: MediaId(1),
            path: "photo.jpg".into(),
            kind: MediaKind::Photo,
            fingerprint: Default::default(),
        };
        let meta = Metadata {
            width: 4,
            height: 3,
            orientation: 6,
            camera: "Camera".into(),
            lens: "Lens".into(),
            captured: "2025".into(),
            exposure: "1/250".into(),
            aperture: "f/2".into(),
            iso: "100".into(),
            gps: Some((1.0, 2.0)),
            details: Details {
                focal_length_mm: Some(50.0),
                flash: Some(0),
                bit_depth: Some(8),
                color_space: Some(1),
                white_balance: Some(0),
                metering: Some(5),
                exposure_mode: Some(0),
                exposure_bias_ev: Some(0.0),
                scene: Some(0),
                software: Some("App".into()),
                video: Some(VideoInfo {
                    duration_s: 1.0,
                    frame_rate: Some(25.0),
                    codec: Some("H.264".into()),
                    bitrate: Some(1),
                    audio: true,
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        let labels: Vec<&str> = crate::core::details::sections(&[(&media, &meta)])
            .iter()
            .flat_map(|section| section.rows.iter().map(|row| row.label))
            .collect();
        assert!(labels.len() >= 27, "{labels:?}");
        for label in labels {
            assert_ne!(field_icon(label), egui_phosphor::regular::INFO, "{label}");
        }
    }
}
