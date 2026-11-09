//! Windows: the window has no native frame. PhotoSort's top bar is its title
//! bar, and this draws the rest of the frame: Minimize, Maximize and Close at
//! the bar's right, and the edges that resize the window. Both sit above
//! everything else, dialogs included, as the native frame would.
use super::*;
use egui::emath::GuiRounding as _;
use egui::{CursorIcon, Order, Rect, ResizeDirection, ViewportCommand, pos2, vec2};

/// Each window button's width, as Windows draws them; they are as tall as
/// the bar.
const BUTTON_WIDTH: f32 = 46.0;
/// The room the window's buttons take at the right of the bar.
pub(super) const BUTTONS_WIDTH: f32 = 3.0 * BUTTON_WIDTH;
/// How far in from the window's border a press resizes the window, and how
/// far along the border from each corner it resizes both ways.
const EDGE: f32 = 5.0;
const CORNER: f32 = 16.0;
/// Close turns this red under the pointer, as on Windows.
const CLOSE_RED: Color32 = Color32::from_rgb(0xC4, 0x2B, 0x1C);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Button {
    Minimize,
    Maximize,
    Close,
}

impl PhotoSortApp {
    /// The window's buttons and resize edges, when the window has no native
    /// frame. A maximized window has no edges to resize from; a full-screen
    /// one shows neither.
    pub(super) fn draw_window_frame(&self, ctx: &egui::Context) {
        if !self.window_frame {
            return;
        }
        let (maximized, fullscreen, focused) = ctx.input(|i| {
            let viewport = i.viewport();
            (
                viewport.maximized.unwrap_or(false),
                viewport.fullscreen.unwrap_or(false),
                viewport.focused.unwrap_or(true),
            )
        });
        if fullscreen {
            return;
        }
        let screen = ctx.content_rect();
        window_buttons(ctx, screen, maximized, focused);
        if !maximized {
            resize_edges(ctx, screen);
        }
    }
}

/// Minimize, Maximize (Restore down while maximized) and Close, flush with
/// the window's top-right corner. Close goes through the same close request
/// as the system's own, so unsaved work is still asked about.
fn window_buttons(ctx: &egui::Context, screen: Rect, maximized: bool, focused: bool) {
    let origin = pos2(screen.right() - BUTTONS_WIDTH, screen.top());
    egui::Area::new(egui::Id::new("window-buttons"))
        .order(Order::Tooltip)
        .fixed_pos(origin)
        .constrain(false)
        .fade_in(false)
        .sense(Sense::hover())
        .show(ctx, |ui| {
            let buttons = [Button::Minimize, Button::Maximize, Button::Close];
            for (i, button) in buttons.into_iter().enumerate() {
                let rect = Rect::from_min_size(
                    origin + vec2(i as f32 * BUTTON_WIDTH, 0.0),
                    vec2(BUTTON_WIDTH, top_bar::HEADER_HEIGHT),
                );
                let name = match button {
                    Button::Minimize => "Minimize",
                    Button::Maximize if maximized => "Restore down",
                    Button::Maximize => "Maximize",
                    Button::Close => "Close",
                };
                let response = ui.interact(rect, ui.id().with(i), Sense::click());
                paint_button(ui, &response, button, maximized, focused);
                let response = named(response, egui::WidgetType::Button, name).on_hover_text(name);
                if response.clicked() {
                    ctx.send_viewport_cmd(match button {
                        Button::Minimize => ViewportCommand::Minimized(true),
                        Button::Maximize => ViewportCommand::Maximized(!maximized),
                        Button::Close => ViewportCommand::Close,
                    });
                }
            }
            ui.advance_cursor_after_rect(Rect::from_min_size(
                origin,
                vec2(BUTTONS_WIDTH, top_bar::HEADER_HEIGHT),
            ));
        });
}

/// One window button: a hover fill, red for Close, and a 10 point glyph
/// drawn in thin lines like the system's own.
fn paint_button(
    ui: &egui::Ui,
    response: &egui::Response,
    button: Button,
    maximized: bool,
    focused: bool,
) {
    let pressed = response.is_pointer_button_down_on();
    let hovered = response.hovered() || pressed;
    let glyph = if focused { INK } else { MUTED };
    let (fill, glyph) = match button {
        Button::Close if pressed => (CLOSE_RED.gamma_multiply(0.85), Color32::WHITE),
        Button::Close if hovered => (CLOSE_RED, Color32::WHITE),
        _ if pressed => (INK.gamma_multiply(0.06), glyph),
        _ if hovered => (INK.gamma_multiply(0.1), glyph),
        _ => (Color32::TRANSPARENT, glyph),
    };
    let painter = ui.painter();
    painter.rect_filled(response.rect, 0.0, fill);
    let ppp = ui.ctx().pixels_per_point();
    let center = response.rect.center().round_to_pixel_center(ppp);
    let stroke = Stroke::new(1.0, glyph);
    let half = 5.0;
    match button {
        Button::Minimize => {
            painter.hline(center.x - half..=center.x + half, center.y, stroke);
        }
        Button::Maximize if maximized => {
            // Restore down: a square with another behind it, up and right.
            let front = Rect::from_center_size(center + vec2(-1.0, 1.0), vec2(8.0, 8.0));
            painter.rect_stroke(front, 1.0, stroke, StrokeKind::Middle);
            painter.add(egui::Shape::line(
                vec![
                    front.left_top() + vec2(2.0, -2.0),
                    front.right_top() + vec2(2.0, -2.0),
                    front.right_bottom() + vec2(2.0, -2.0),
                ],
                stroke,
            ));
        }
        Button::Maximize => {
            let square = Rect::from_center_size(center, vec2(2.0 * half, 2.0 * half));
            painter.rect_stroke(square, 1.0, stroke, StrokeKind::Middle);
        }
        Button::Close => {
            painter.line_segment(
                [center + vec2(-half, -half), center + vec2(half, half)],
                stroke,
            );
            painter.line_segment(
                [center + vec2(-half, half), center + vec2(half, -half)],
                stroke,
            );
        }
    }
}

/// Thin zones along the window's border that resize it, and longer ones at
/// each corner that resize it both ways, with the matching cursors.
fn resize_edges(ctx: &egui::Context, screen: Rect) {
    use ResizeDirection::*;
    let (l, r, t, b) = (screen.left(), screen.right(), screen.top(), screen.bottom());
    let zone = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    let zones = [
        (North, zone(l + CORNER, t, r - CORNER, t + EDGE)),
        (South, zone(l + CORNER, b - EDGE, r - CORNER, b)),
        (West, zone(l, t + CORNER, l + EDGE, b - CORNER)),
        (East, zone(r - EDGE, t + CORNER, r, b - CORNER)),
        (NorthWest, zone(l, t, l + CORNER, t + EDGE)),
        (NorthWest, zone(l, t, l + EDGE, t + CORNER)),
        (NorthEast, zone(r - CORNER, t, r, t + EDGE)),
        (NorthEast, zone(r - EDGE, t, r, t + CORNER)),
        (SouthWest, zone(l, b - EDGE, l + CORNER, b)),
        (SouthWest, zone(l, b - CORNER, l + EDGE, b)),
        (SouthEast, zone(r - CORNER, b - EDGE, r, b)),
        (SouthEast, zone(r - EDGE, b - CORNER, r, b)),
    ];
    for (i, (direction, rect)) in zones.into_iter().enumerate() {
        egui::Area::new(egui::Id::new(("window-edge", i)))
            .order(Order::Tooltip)
            .fixed_pos(rect.min)
            .constrain(false)
            .fade_in(false)
            .sense(Sense::hover())
            .show(ctx, |ui| {
                // Clicks and drags both, so the edge wins over any widget
                // beneath it.
                let response = ui.interact(rect, ui.id().with("edge"), Sense::click_and_drag());
                ui.advance_cursor_after_rect(rect);
                let down = response.is_pointer_button_down_on();
                if response.hovered() || down {
                    ctx.set_cursor_icon(match direction {
                        North | South => CursorIcon::ResizeVertical,
                        East | West => CursorIcon::ResizeHorizontal,
                        NorthWest | SouthEast => CursorIcon::ResizeNwSe,
                        NorthEast | SouthWest => CursorIcon::ResizeNeSw,
                    });
                }
                // The system resizes from where the press began.
                if down && ctx.input(|i| i.pointer.primary_pressed()) {
                    ctx.send_viewport_cmd(ViewportCommand::BeginResize(direction));
                }
            });
    }
}
