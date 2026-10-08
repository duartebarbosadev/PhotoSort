use eframe::egui::{self, Color32, FontId, TextStyle};
pub const BG: Color32 = Color32::from_rgb(17, 20, 25);
pub const PANEL: Color32 = Color32::from_rgb(23, 27, 33);
pub const CARD: Color32 = Color32::from_rgb(31, 36, 43);
pub const LINE: Color32 = Color32::from_rgb(48, 55, 63);
pub const INK: Color32 = Color32::from_rgb(233, 236, 236);
pub const MUTED: Color32 = Color32::from_rgb(145, 157, 165);
pub const ACCENT: Color32 = Color32::from_rgb(165, 222, 195);
pub const RED: Color32 = Color32::from_rgb(238, 139, 136);
/// Height of a one-line framed button, shared with virtualized list geometry.
/// show_rows adds item spacing itself; its row height must match the widget.
pub(super) fn button_row_height(ui: &egui::Ui) -> f32 {
    (ui.text_style_height(&TextStyle::Button) + 2.0 * ui.spacing().button_padding.y)
        .max(ui.spacing().interact_size.y)
}
pub fn install(ctx: &egui::Context) {
    // Egui's proportional defaults omit the bundled monospace symbol glyphs.
    // Append those existing fonts only as fallbacks, keeping the normal typeface
    // and avoiding platform font scans or additional bundled font assets.
    let mut fonts = egui::FontDefinitions::default();
    let fallbacks = fonts.families[&egui::FontFamily::Monospace].clone();
    let proportional = fonts
        .families
        .entry(egui::FontFamily::Proportional)
        .or_default();
    for name in fallbacks {
        if !proportional.contains(&name) {
            proportional.push(name);
        }
    }
    ctx.set_fonts(fonts);
    let mut style = (*ctx.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = PANEL;
    style.visuals.window_fill = PANEL;
    style.visuals.extreme_bg_color = BG;
    style.visuals.faint_bg_color = CARD;
    style.visuals.override_text_color = Some(INK);
    style.visuals.selection.bg_fill = Color32::from_rgb(51, 84, 73);
    style.visuals.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    style.visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, LINE);
    style.visuals.widgets.inactive.weak_bg_fill = CARD;
    style.visuals.widgets.inactive.bg_fill = CARD;
    style.visuals.widgets.hovered.weak_bg_fill = Color32::from_rgb(47, 59, 59);
    style.visuals.widgets.active.weak_bg_fill = Color32::from_rgb(51, 84, 73);
    style.spacing.item_spacing = egui::vec2(10.0, 10.0);
    style.spacing.button_padding = egui::vec2(14.0, 9.0);
    style
        .text_styles
        .insert(TextStyle::Heading, FontId::proportional(27.0));
    style
        .text_styles
        .insert(TextStyle::Body, FontId::proportional(14.0));
    style
        .text_styles
        .insert(TextStyle::Button, FontId::proportional(13.0));
    style
        .text_styles
        .insert(TextStyle::Small, FontId::proportional(11.0));
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
