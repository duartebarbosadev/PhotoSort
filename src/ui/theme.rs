//! Visual system ("Studio"). A cool, low-saturation slate surround keeps colour
//! judgement honest; one azure accent marks focus and the primary action, and
//! selection is a lifted neutral. Type is Inter (compact); icons are Phosphor.
use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
/// Surround behind photographs and the main canvas.
pub const BG: Color32 = rgb(0x15171B);
/// Bars and panels.
pub const PANEL: Color32 = rgb(0x1B1E23);
/// Raised controls and tiles.
pub const CARD: Color32 = rgb(0x23272E);
pub const CARD_HOVER: Color32 = rgb(0x2C313A);
pub const LINE: Color32 = rgb(0x2B3038);
pub const INK: Color32 = rgb(0xE9ECF1);
pub const MUTED: Color32 = rgb(0x8B93A0);
pub const FAINT: Color32 = rgb(0x59616E);
/// The one accent: focus, the primary action per screen.
pub const ACCENT: Color32 = rgb(0x4F8CFF);
/// Text and icons drawn on an accent fill.
pub const ON_ACCENT: Color32 = rgb(0xFFFFFF);
/// Selected toggles and rows: a lifted neutral, never a muddy tint.
pub const SELECTED: Color32 = rgb(0x283B5C);
/// Translucent accent for text selection.
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(15, 28, 51, 51);
pub const RED: Color32 = rgb(0xF0605A);
pub const GREEN: Color32 = rgb(0x4CC38A);

/// Spacing scale (points).
pub const S1: f32 = 4.0;
pub const S2: f32 = 8.0;
pub const S3: f32 = 12.0;
pub const S4: f32 = 16.0;
pub const S6: f32 = 24.0;
/// Corner radii by hierarchy: controls, tiles, floating surfaces.
pub const R_CONTROL: u8 = 6;
pub const R_TILE: u8 = 8;
pub const R_SURFACE: u8 = 10;

pub const MEDIUM: &str = "ui-medium";
pub const SEMIBOLD: &str = "ui-semibold";
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Height of a one-line framed button, shared with virtualized list geometry.
/// show_rows adds item spacing itself; its row height must match the widget.
pub(super) fn button_row_height(ui: &egui::Ui) -> f32 {
    // Buttons lay out their text in the Body style (egui 0.36) and size to the
    // galley, which egui rounds to whole pixels; measure exactly that.
    let font = TextStyle::Body.resolve(ui.style());
    let text = ui
        .painter()
        .layout_no_wrap("Ag".into(), font, Color32::WHITE)
        .size()
        .y;
    (text + 2.0 * ui.spacing().button_padding.y).max(ui.spacing().interact_size.y)
}

/// The screen's main action: accent fill, dark text.
pub(super) fn primary(text: impl Into<String>) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.into())
            .font(medium(13.0))
            .color(ON_ACCENT),
    )
    .fill(ACCENT)
    .stroke(Stroke::NONE)
    .corner_radius(R_CONTROL)
}
/// A frameless icon button. Its tooltip is also its accessible name, so
/// assistive technology never reads the icon font's private-use glyph.
pub(super) fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str) -> egui::Response {
    let response = ui
        .add(
            egui::Button::new(egui::RichText::new(icon).size(17.0))
                .frame(false)
                .min_size(egui::vec2(30.0, 30.0)),
        )
        .on_hover_text(tooltip);
    let name = tooltip.lines().next().unwrap_or(tooltip).to_owned();
    let enabled = response.enabled();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, enabled, &name));
    response
}
/// A button with a leading icon; `text` alone is the accessible name.
pub(super) fn icon_text_button(
    ui: &mut egui::Ui,
    enabled: bool,
    selected: bool,
    icon: &str,
    text: &str,
) -> egui::Response {
    let response = ui.add_enabled(
        enabled,
        egui::Button::new(format!("{icon}  {text}")).selected(selected),
    );
    response.widget_info(|| {
        egui::WidgetInfo::selected(egui::WidgetType::Button, enabled, selected, text)
    });
    response
}

/// A pill-shaped toggle for filters and scopes, such as Easy Delete categories.
pub(super) fn chip(ui: &mut egui::Ui, selected: bool, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(egui::RichText::new(text).color(if selected { INK } else { MUTED }))
            .selected(selected)
            .corner_radius(14)
            .stroke(Stroke::new(1.0, if selected { ACCENT } else { LINE })),
    )
}
/// An on/off switch with a sliding knob, followed by its label. Assistive
/// technology sees a checkbox with the same label and state.
pub(super) struct Switch<'a> {
    on: &'a mut bool,
    label: String,
}
impl<'a> Switch<'a> {
    pub fn new(on: &'a mut bool, label: impl Into<String>) -> Self {
        Self {
            on,
            label: label.into(),
        }
    }
}
impl egui::Widget for Switch<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let track = egui::vec2(26.0, 15.0);
        let font = TextStyle::Body.resolve(ui.style());
        let galley = ui.painter().layout_no_wrap(self.label.clone(), font, INK);
        let gap = S2;
        let height = track
            .y
            .max(galley.size().y)
            .max(ui.spacing().interact_size.y);
        let size = egui::vec2(track.x + gap + galley.size().x, height);
        let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
        if response.clicked() {
            *self.on = !*self.on;
            response.mark_changed();
        }
        let on = *self.on;
        let enabled = ui.is_enabled();
        let label = self.label.clone();
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Checkbox, enabled, on, &label)
        });
        if ui.is_rect_visible(rect) {
            let t = ui.ctx().animate_bool_responsive(response.id, on);
            let track_rect = egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.center().y - track.y / 2.0),
                track,
            );
            let off_fill = if response.hovered() { CARD_HOVER } else { LINE };
            let fill = lerp_color(off_fill, ACCENT, t);
            let fill = if enabled {
                fill
            } else {
                fill.gamma_multiply(0.45)
            };
            let painter = ui.painter();
            painter.rect_filled(track_rect, CornerRadius::same((track.y / 2.0) as u8), fill);
            let radius = track.y / 2.0 - 2.0;
            let x = egui::lerp(
                (track_rect.left() + radius + 2.0)..=(track_rect.right() - radius - 2.0),
                t,
            );
            painter.circle_filled(
                egui::pos2(x, track_rect.center().y),
                radius,
                if enabled { Color32::WHITE } else { MUTED },
            );
            let text = if enabled { INK } else { FAINT };
            painter.galley(
                egui::pos2(
                    track_rect.right() + gap,
                    rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                text,
            );
        }
        response
    }
}
pub(super) fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_premultiplied(
        mix(a.r(), b.r()),
        mix(a.g(), b.g()),
        mix(a.b(), b.b()),
        mix(a.a(), b.a()),
    )
}
/// One option of a segmented control: what is shown, and its accessible name.
pub(super) struct Segment {
    pub shown: String,
    pub name: String,
    pub hover: Option<String>,
}
/// A row of options on a recessed track. The selected one sits on a lifted
/// pill that slides to the new option when the selection changes. Returns the
/// clicked index.
pub(super) fn segmented(
    ui: &mut egui::Ui,
    id: egui::Id,
    segments: &[Segment],
    selected: Option<usize>,
    height: f32,
) -> Option<usize> {
    let font = egui::FontId::proportional(ui.style().text_styles[&TextStyle::Button].size);
    let pad = 2.0;
    let widths: Vec<f32> = segments
        .iter()
        .map(|s| {
            ui.painter()
                .layout_no_wrap(s.shown.clone(), medium(font.size), INK)
                .size()
                .x
                + 2.0 * S3
        })
        .collect();
    let total = widths.iter().sum::<f32>() + 2.0 * pad;
    let (track, _) = ui.allocate_exact_size(egui::vec2(total, height), egui::Sense::hover());
    let painter = ui.painter_at(track.expand(4.0));
    painter.rect_filled(track, CornerRadius::same(R_CONTROL + 2), BG);
    painter.rect_stroke(
        track,
        CornerRadius::same(R_CONTROL + 2),
        Stroke::new(1.0, LINE),
        egui::StrokeKind::Inside,
    );
    let mut x = track.left() + pad;
    let mut rects = Vec::with_capacity(segments.len());
    for width in &widths {
        rects.push(egui::Rect::from_min_size(
            egui::pos2(x, track.top() + pad),
            egui::vec2(*width, height - 2.0 * pad),
        ));
        x += width;
    }
    if let Some(index) = selected.filter(|i| *i < rects.len()) {
        let target = rects[index];
        let time = ui.style().animation_time;
        let left = ui
            .ctx()
            .animate_value_with_time(id.with("left"), target.left(), time);
        let right = ui
            .ctx()
            .animate_value_with_time(id.with("right"), target.right(), time);
        let pill = egui::Rect::from_x_y_ranges(left..=right, target.y_range());
        painter.rect_filled(pill, CornerRadius::same(R_CONTROL), SELECTED);
        painter.hline(
            pill.x_range().shrink(4.0),
            pill.top() + 0.5,
            Stroke::new(1.0, Color32::from_white_alpha(14)),
        );
    }
    let mut clicked = None;
    for (index, (segment, rect)) in segments.iter().zip(&rects).enumerate() {
        let response = ui.interact(*rect, id.with(index), egui::Sense::click());
        let is_selected = selected == Some(index);
        let name = segment.name.clone();
        response.widget_info(|| {
            egui::WidgetInfo::selected(egui::WidgetType::Button, true, is_selected, &name)
        });
        let response = match &segment.hover {
            Some(hover) => response.on_hover_text(hover),
            None => response,
        };
        let color = if is_selected || response.hovered() {
            INK
        } else {
            MUTED
        };
        let galley = painter.layout_no_wrap(
            segment.shown.clone(),
            if is_selected {
                medium(font.size)
            } else {
                font.clone()
            },
            color,
        );
        painter.galley(rect.center() - galley.size() / 2.0, galley, color);
        if response.clicked() {
            clicked = Some(index);
        }
    }
    clicked
}
/// In a wrapping row, start a new line unless `width` still fits. Combo boxes
/// and text fields do not take part in the row's own wrapping decision.
pub(super) fn wrap_before(ui: &mut egui::Ui, width: f32) {
    // In a wrapping layout available_width() is the whole row; measure what is
    // left before the row's right edge instead.
    let left = ui.max_rect().right() - ui.cursor().left();
    if left < width && ui.cursor().left() > ui.max_rect().left() + 1.0 {
        ui.end_row();
    }
}
/// A thin vertical rule between groups of controls in one toolbar row.
pub(super) fn divider(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 20.0), egui::Sense::hover());
    ui.painter()
        .vline(rect.center().x, rect.y_range(), Stroke::new(1.0, LINE));
}
/// The quiet guidance line under a step's controls.
pub(super) fn step_hint(ui: &mut egui::Ui, text: &str) {
    ui.add(egui::Label::new(egui::RichText::new(text).small().color(MUTED)).wrap());
}

/// Seconds for interface animations; zero when motion is reduced.
pub const MOTION: f32 = 0.16;
pub(super) fn set_reduce_motion(ctx: &egui::Context, reduce: bool) {
    ctx.global_style_mut(|style| style.animation_time = if reduce { 0.0 } else { MOTION });
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let faces: [&'static [u8]; 3] = [
        include_bytes!("../../resources/fonts/Inter-Regular.ttf"),
        include_bytes!("../../resources/fonts/Inter-Medium.ttf"),
        include_bytes!("../../resources/fonts/Inter-SemiBold.ttf"),
    ];
    for (name, bytes) in ["ui", MEDIUM, SEMIBOLD].into_iter().zip(faces) {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // Inter first; egui's bundled fonts and the icon font remain as fallbacks for
    // symbols Inter lacks (★, ↶, ↷), so no platform font scan is needed.
    let mut fallbacks = fonts.families[&FontFamily::Proportional].clone();
    for name in fonts.families[&FontFamily::Monospace].clone() {
        if !fallbacks.contains(&name) {
            fallbacks.push(name);
        }
    }
    for (family, primary) in [
        (FontFamily::Proportional, "ui"),
        (FontFamily::Name(MEDIUM.into()), MEDIUM),
        (FontFamily::Name(SEMIBOLD.into()), SEMIBOLD),
    ] {
        // The bundled Inter has its private-use glyphs removed, so icon
        // codepoints fall through to the icon font.
        let mut chain = vec![primary.to_string()];
        chain.extend(fallbacks.iter().filter(|n| n.as_str() != primary).cloned());
        fonts.families.insert(family, chain);
    }
    ctx.set_fonts(fonts);

    let mut style = (*ctx.global_style()).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0, LINE);
    visuals.window_corner_radius = CornerRadius::same(R_SURFACE + 2);
    visuals.menu_corner_radius = CornerRadius::same(R_SURFACE);
    let shadow = egui::Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(150),
    };
    visuals.window_shadow = shadow;
    visuals.popup_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 18,
        spread: 0,
        color: Color32::from_black_alpha(130),
    };
    // Text fields sit inset: darker than the controls around them.
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = CARD;
    visuals.code_bg_color = CARD;
    visuals.override_text_color = Some(INK);
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.warn_fg_color = rgb(0xF2B544);
    visuals.error_fg_color = RED;
    visuals.handle_shape = egui::style::HandleShape::Circle;
    visuals.slider_trailing_fill = true;
    visuals.striped = false;
    let radius = CornerRadius::same(R_CONTROL);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK);
    w.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut w.inactive, CARD),
        (&mut w.hovered, CARD_HOVER),
        (&mut w.active, SELECTED),
        (&mut w.open, CARD_HOVER),
    ] {
        state.bg_fill = fill;
        state.weak_bg_fill = fill;
        state.bg_stroke = Stroke::NONE;
        state.corner_radius = radius;
        state.expansion = 0.0;
    }
    w.inactive.fg_stroke = Stroke::new(1.0, INK);
    w.hovered.fg_stroke = Stroke::new(1.0, INK);
    w.active.fg_stroke = Stroke::new(1.0, INK);
    w.open.fg_stroke = Stroke::new(1.0, INK);
    w.hovered.bg_stroke = Stroke::new(1.0, LINE);
    // Selected toggles reuse the selection colours: a lifted neutral.
    visuals.selection.bg_fill = SELECTED;
    visuals.selection.stroke = Stroke::new(1.0, INK);
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(6.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 4.0);
    style.spacing.interact_size.y = 24.0;
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.combo_width = 120.0;
    style.spacing.slider_rail_height = 4.0;
    style.spacing.scroll = egui::style::ScrollStyle::floating();
    style.spacing.scroll.bar_width = 6.0;
    style.spacing.icon_width = 14.0;
    style.spacing.icon_width_inner = 8.0;
    style.animation_time = MOTION;
    let (body, button, small) = (12.5, 12.5, 11.0);
    for (text, font) in [
        (TextStyle::Heading, semibold(18.0)),
        (TextStyle::Body, FontId::proportional(body)),
        (TextStyle::Button, FontId::proportional(button)),
        (TextStyle::Small, FontId::proportional(small)),
        (TextStyle::Monospace, FontId::monospace(12.0)),
    ] {
        style.text_styles.insert(text, font);
    }
    ctx.set_global_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::test_frame::TestFrame;

    #[test]
    fn virtual_button_rows_match_themed_widgets_and_reach_last_item() {
        for padding in [2.0, 9.0] {
            for font_size in [13.0, 18.0] {
                let ctx = egui::Context::default();
                install(&ctx);
                install(&ctx);
                ctx.global_style_mut(|s| {
                    s.spacing.button_padding.y = padding;
                    s.text_styles
                        .insert(TextStyle::Body, FontId::proportional(font_size));
                });
                for position in [0, 50_000, 99_999] {
                    for _ in 0..3 {
                        let _ = ctx.run_test(Default::default(), |ui| {
                            egui::CentralPanel::default().show(ui, |ui| {
                                let height = button_row_height(ui);
                                let pitch = height + ui.spacing().item_spacing.y;
                                let mut painted = Vec::new();
                                let output = egui::ScrollArea::vertical()
                                    .id_salt("button_geometry")
                                    .max_height(180.0)
                                    .vertical_scroll_offset(position as f32 * pitch)
                                    .show_rows(ui, height, 100_000, |ui, rows| {
                                        for i in rows {
                                            let response = ui.add(
                                                egui::Button::selectable(
                                                    false,
                                                    format!(
                                                        "Folder {i:05} {}",
                                                        "a long name ".repeat(20)
                                                    ),
                                                )
                                                .truncate(),
                                            );
                                            painted.push((i, response.rect));
                                        }
                                    });
                                assert!(painted.len() <= 12, "Only viewport rows should render");
                                for pair in painted.windows(2) {
                                    assert!(
                                        (pair[1].1.top() - pair[0].1.top() - pitch).abs() < 0.6
                                    );
                                }
                                assert!(
                                    painted
                                        .iter()
                                        .all(|(_, r)| (r.height() - height).abs() < 0.6)
                                );
                                assert!(
                                    (output.content_size.y
                                        - (100_000.0 * pitch - ui.spacing().item_spacing.y))
                                        .abs()
                                        <= pitch,
                                    "content {:?}, pitch {pitch}, position {position}, padding {padding}, font {font_size}", output.content_size
                                );
                                if position == 99_999 {
                                    assert!(
                                        painted.iter().any(|(i, r)| *i == 99_999
                                            && r.intersects(output.inner_rect))
                                    );
                                }
                            });
                        });
                    }
                }
            }
        }
    }
    #[test]
    fn shared_ui_fonts_cover_workflow_controls_without_replacement_glyphs() {
        let ctx = egui::Context::default();
        install(&ctx);
        install(&ctx);
        let styles = ctx.global_style().text_styles.clone();
        let _ = ctx.run_test(Default::default(), |ui| {
            let ctx = &ui.ctx().clone();
            {
                ctx.fonts_mut(|fonts| {
                    // Navigation, confirmation, rotation, comparison, disclosure,
                    // ratings and measurement labels used throughout the app.
                    let controls =
                        "Confirm →  ↶ Left (Q)  Right (E) ↷  ←  ↔  ⌄  ★  −  ×  °  ·  –  ’  …";
                    for style in [
                        TextStyle::Body,
                        TextStyle::Button,
                        TextStyle::Small,
                        TextStyle::Heading,
                    ] {
                        let font = styles[&style].clone();
                        for ch in controls.chars() {
                            assert!(
                                fonts.has_glyph(&font, ch),
                                "Missing proportional glyph {ch:?} in {style:?}"
                            );
                        }
                    }
                });
            }
        });
    }
}
