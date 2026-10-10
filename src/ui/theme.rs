//! Visual system. A neutral graphite surround keeps colour judgement honest
//! (nothing tints the photographs); one safelight-amber accent marks focus,
//! selection and the primary action. Type is IBM Plex Sans; icons are Phosphor.
use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}
/// Surround behind photographs and the main canvas.
pub const BG: Color32 = rgb(0x1B1B1B);
/// Bars and panels.
pub const PANEL: Color32 = rgb(0x222222);
/// Raised controls and tiles.
pub const CARD: Color32 = rgb(0x2B2B2B);
pub const CARD_HOVER: Color32 = rgb(0x343434);
pub const LINE: Color32 = rgb(0x363636);
pub const INK: Color32 = rgb(0xECECEC);
pub const MUTED: Color32 = rgb(0x9C9C9C);
pub const FAINT: Color32 = rgb(0x6A6A6A);
/// Safelight amber: focus, selection, the one primary action per screen.
pub const ACCENT: Color32 = rgb(0xF0A43A);
/// Text and icons drawn on an accent fill.
pub const ON_ACCENT: Color32 = rgb(0x1E1407);
/// Translucent accent for selected fills.
pub const ACCENT_SOFT: Color32 = Color32::from_rgba_premultiplied(60, 41, 15, 64);
pub const RED: Color32 = rgb(0xE5534B);
pub const GREEN: Color32 = rgb(0x5DBB7E);

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

pub const MEDIUM: &str = "plex-medium";
pub const SEMIBOLD: &str = "plex-semibold";
pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(MEDIUM.into()))
}
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(SEMIBOLD.into()))
}

/// Height of a one-line framed button, shared with virtualized list geometry.
/// show_rows adds item spacing itself; its row height must match the widget.
pub(super) fn button_row_height(ui: &egui::Ui) -> f32 {
    (ui.text_style_height(&TextStyle::Button) + 2.0 * ui.spacing().button_padding.y)
        .max(ui.spacing().interact_size.y)
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

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    for (name, bytes) in [
        (
            "plex",
            &include_bytes!("../../resources/fonts/IBMPlexSans-Regular.ttf")[..],
        ),
        (
            MEDIUM,
            &include_bytes!("../../resources/fonts/IBMPlexSans-Medium.ttf")[..],
        ),
        (
            SEMIBOLD,
            &include_bytes!("../../resources/fonts/IBMPlexSans-SemiBold.ttf")[..],
        ),
    ] {
        fonts
            .font_data
            .insert(name.into(), egui::FontData::from_static(bytes).into());
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    // Plex first; egui's bundled fonts and the icon font remain as fallbacks for
    // symbols Plex lacks (★, ↶, ↷), so no platform font scan is needed.
    let mut fallbacks = fonts.families[&FontFamily::Proportional].clone();
    for name in fonts.families[&FontFamily::Monospace].clone() {
        if !fallbacks.contains(&name) {
            fallbacks.push(name);
        }
    }
    for (family, primary) in [
        (FontFamily::Proportional, "plex"),
        (FontFamily::Name(MEDIUM.into()), MEDIUM),
        (FontFamily::Name(SEMIBOLD.into()), SEMIBOLD),
    ] {
        let mut chain = vec![primary.to_string()];
        chain.extend(fallbacks.iter().filter(|n| n.as_str() != primary).cloned());
        fonts.families.insert(family, chain);
    }
    ctx.set_fonts(fonts);

    let mut style = (*ctx.style()).clone();
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.window_stroke = Stroke::new(1.0, LINE);
    visuals.window_corner_radius = CornerRadius::same(R_SURFACE);
    visuals.menu_corner_radius = CornerRadius::same(R_SURFACE);
    // Text fields sit inset: darker than panels, with a visible fill on BG.
    visuals.extreme_bg_color = rgb(0x141414);
    visuals.faint_bg_color = CARD;
    visuals.code_bg_color = CARD;
    visuals.override_text_color = Some(INK);
    visuals.hyperlink_color = ACCENT;
    visuals.selection.bg_fill = ACCENT_SOFT;
    visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    visuals.warn_fg_color = ACCENT;
    visuals.error_fg_color = RED;
    let radius = CornerRadius::same(R_CONTROL);
    let w = &mut visuals.widgets;
    w.noninteractive.bg_stroke = Stroke::new(1.0, LINE);
    w.noninteractive.fg_stroke = Stroke::new(1.0, INK);
    w.noninteractive.corner_radius = radius;
    for (state, fill) in [
        (&mut w.inactive, CARD),
        (&mut w.hovered, CARD_HOVER),
        (&mut w.active, rgb(0x3D3D3D)),
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
    w.hovered.bg_stroke = Stroke::new(1.0, rgb(0x464646));
    style.visuals = visuals;
    style.spacing.item_spacing = egui::vec2(S2, S2);
    style.spacing.button_padding = egui::vec2(S3, 6.0);
    style.spacing.interact_size.y = 28.0;
    style.spacing.menu_margin = egui::Margin::same(6);
    style.spacing.window_margin = egui::Margin::same(16);
    style.spacing.combo_width = 120.0;
    for (text, font) in [
        (TextStyle::Heading, semibold(20.0)),
        (TextStyle::Body, FontId::proportional(13.5)),
        (TextStyle::Button, FontId::proportional(13.0)),
        (TextStyle::Small, FontId::proportional(11.5)),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ] {
        style.text_styles.insert(text, font);
    }
    ctx.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn virtual_button_rows_match_themed_widgets_and_reach_last_item() {
        for padding in [2.0, 9.0] {
            for font_size in [13.0, 18.0] {
                let ctx = egui::Context::default();
                install(&ctx);
                install(&ctx);
                ctx.style_mut(|s| {
                    s.spacing.button_padding.y = padding;
                    s.text_styles
                        .insert(TextStyle::Button, FontId::proportional(font_size));
                });
                for position in [0, 50_000, 99_999] {
                    for _ in 0..3 {
                        let _ = ctx.run(Default::default(), |ctx| {
                            egui::CentralPanel::default().show(ctx, |ui| {
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
        let styles = ctx.style().text_styles.clone();
        let _ = ctx.run(Default::default(), |ctx| {
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
        });
    }
}
