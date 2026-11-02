//! Swipe compare: the photos being compared on top of each other in one
//! canvas, aligned and at one zoom, cut into slices side by side, one per
//! photo, with a divider to drag between each two. Number keys show one
//! photo whole, to flick between them.
use super::*;

/// The narrowest a slice can be dragged, as a share of the canvas.
const NARROWEST: f32 = 0.03;

/// The open swipe compare. See [`PhotoSortApp::swipe_key`].
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Swipe {
    /// The photos compared, left to right, in the order of their number keys.
    pub(super) ids: Vec<MediaId>,
    /// Where each divider sits, from the left edge (0) to the right (1):
    /// one fewer than the photos, in order.
    pub(super) splits: Vec<f32>,
    /// One photo shown whole, from its number key.
    pub(super) solo: Option<usize>,
}
impl Swipe {
    fn new(ids: Vec<MediaId>) -> Self {
        let mut swipe = Self {
            ids,
            splits: Vec::new(),
            solo: None,
        };
        swipe.even();
        swipe
    }
    /// Slices of equal width: halves for two photos, thirds for three.
    fn even(&mut self) {
        let n = self.ids.len();
        self.splits = (1..n).map(|i| i as f32 / n as f32).collect();
    }
    /// Number key `n` (from 1): shows that photo whole; the same number
    /// again goes back to the slices.
    fn number(&mut self, n: usize) {
        let photo = n - 1;
        if photo < self.ids.len() {
            self.solo = (self.solo != Some(photo)).then_some(photo);
        }
    }
    /// Moves divider `i` to `at`, keeping every slice at least [`NARROWEST`].
    fn drag(&mut self, i: usize, at: f32) {
        let low = if i == 0 { 0.0 } else { self.splits[i - 1] } + NARROWEST;
        let high = self.splits.get(i + 1).copied().unwrap_or(1.0) - NARROWEST;
        self.splits[i] = at.clamp(low, high.max(low));
    }
    /// Each photo's slice, from the left edge (0) to the right (1).
    fn slices(&self) -> Vec<(f32, f32)> {
        let edges: Vec<f32> = std::iter::once(0.0)
            .chain(self.splits.iter().copied())
            .chain(std::iter::once(1.0))
            .collect();
        edges.windows(2).map(|w| (w[0], w[1])).collect()
    }
}

impl PhotoSortApp {
    /// C opens or closes swipe compare over the photos being compared, or
    /// the 2 to 9 selected, and does nothing for fewer; while it is open,
    /// 1 to 9 show one photo whole, Ctrl/Cmd and 0 to 5 rate it, and Esc
    /// closes it. Returns whether it took the key.
    pub(super) fn swipe_key(&mut self, pressed: egui::Key, modifiers: egui::Modifiers) -> bool {
        if !matches!(self.step, Step::Cull | Step::Organize) || modifiers.alt {
            return false;
        }
        if modifiers.command || modifiers.ctrl {
            // Ctrl/Cmd and 0 to 5 rate the photo you are looking at, not
            // every photo selected for the comparison.
            let digit = super::shortcuts::digit(pressed).filter(|n| *n <= 5);
            if self.swipe.is_some()
                && let (Some(stars), Some(id)) = (digit, self.active)
            {
                self.rating(id, stars as u8);
                return true;
            }
            return false;
        }
        match pressed {
            egui::Key::C if self.swipe.is_some() => self.swipe = None,
            egui::Key::C => self.open_swipe(),
            egui::Key::Escape if self.swipe.is_some() => self.swipe = None,
            key => {
                let Some(swipe) = self.swipe.as_mut() else {
                    return false;
                };
                let Some(n) = super::shortcuts::digit(key).filter(|n| *n > 0) else {
                    return false;
                };
                swipe.number(n);
                if let Some(&id) = swipe.ids.get(n - 1) {
                    self.active = Some(id);
                }
            }
        }
        true
    }
    /// Swipe compare over the photos being compared, or the 2 to 9
    /// selected. Videos cannot be overlaid, so they are left out; with
    /// fewer than two photos nothing happens.
    pub(super) fn open_swipe(&mut self) {
        if self.comparison.len() < 2 {
            if !(2..=9).contains(&self.selection.len()) && self.step == Step::Cull {
                return;
            }
            self.compare_selection();
        }
        let ids: Vec<MediaId> = self
            .comparison_ids()
            .into_iter()
            .filter(|id| {
                self.index
                    .get(id)
                    .is_some_and(|i| self.media[*i].kind == MediaKind::Photo)
            })
            .take(9)
            .collect();
        if ids.len() < 2 {
            return;
        }
        if self.step == Step::Cull {
            self.review = true;
        }
        self.focused_slot = 0;
        self.swipe = Some(Swipe::new(ids));
    }
    /// The open swipe compare, while it still covers the photos compared.
    pub(super) fn swipe_shown(&mut self) -> bool {
        let Some(swipe) = &self.swipe else {
            return false;
        };
        let ids = self.comparison_ids();
        if swipe.ids.iter().any(|id| !ids.contains(id)) {
            self.swipe = None;
            return false;
        }
        true
    }
    /// Draws swipe compare in a canvas of `size`: the photos sliced side by
    /// side over one another, or one whole, under a row of numbered labels
    /// that pick the photo shown.
    pub(super) fn swipe_canvas(&mut self, ui: &mut egui::Ui, size: Vec2) {
        let Some(swipe) = self.swipe.clone() else {
            return;
        };
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        ui.painter().rect_filled(rect, R_SURFACE, CHROME);
        let content = egui::Rect::from_min_max(
            rect.min + egui::vec2(8.0, 30.0),
            rect.max - egui::vec2(8.0, 8.0),
        );
        // Slices share out the photo, not the canvas around it.
        let first = swipe.ids[swipe.solo.unwrap_or(0)];
        let area = self.swipe_source(first).map_or(content, |source| {
            let turns = self.preview_turns(first);
            let ppp = ui.ctx().pixels_per_point();
            let viewport = [content.width(), content.height()];
            let fit = InspectionView::fit_zoom(source, viewport, turns, ppp);
            let view = self.view_for(0).clamped(Some(fit));
            let scale = view.scale(source, viewport, turns, ppp);
            let [w, h] = if turns % 2 == 1 {
                [source[1], source[0]]
            } else {
                source
            };
            let photo = egui::Rect::from_center_size(
                content.center() + egui::vec2(view.pan[0], view.pan[1]),
                egui::vec2(w as f32, h as f32) * scale,
            );
            let area = photo.intersect(content);
            if area.width() > 40.0 { area } else { content }
        });
        let x = |at: f32| area.left() + area.width() * at;
        // Dividers, registered after the canvas so dragging one never pans.
        // A double click evens the slices out again.
        let mut dragging = false;
        if swipe.solo.is_none() {
            for (i, split) in swipe.splits.iter().enumerate() {
                let handle = ui
                    .interact(
                        egui::Rect::from_center_size(
                            egui::pos2(x(*split), area.center().y),
                            egui::vec2(18.0, area.height()),
                        ),
                        ui.id().with(("swipe-divider", i)),
                        Sense::click_and_drag(),
                    )
                    .on_hover_cursor(egui::CursorIcon::ResizeHorizontal)
                    .on_hover_text("Drag to compare · double-click to even out the slices");
                dragging |= handle.dragged();
                if let Some(swipe) = self.swipe.as_mut() {
                    if handle.double_clicked() {
                        swipe.even();
                    } else if handle.dragged()
                        && let Some(pointer) = handle.interact_pointer_pos()
                    {
                        swipe.drag(i, (pointer.x - area.left()) / area.width());
                    }
                }
            }
        }
        let swipe = self.swipe.clone().unwrap_or(swipe);
        let slices = swipe.slices();
        // The photo under the pointer, for a click to make it the active one.
        let under = |pointer: egui::Pos2| {
            slices
                .iter()
                .position(|(from, to)| pointer.x >= x(*from) && pointer.x < x(*to))
        };
        // Dragging the photos pans them all; scrolling zooms them all.
        let mut view = self.view_for(0);
        let ppp = ui.ctx().pixels_per_point();
        let viewport = [content.width(), content.height()];
        let first = swipe.ids[swipe.solo.unwrap_or(0)];
        if let Some(source) = self.swipe_source(first) {
            let fit = InspectionView::fit_zoom(source, viewport, self.preview_turns(first), ppp);
            self.fit_zooms.insert(0, fit);
            view = view.clamped(Some(fit));
        }
        if response.dragged() && !dragging {
            let delta = response.drag_delta();
            if !view.shows_whole_photo(self.fit_zooms.get(&0).copied()) {
                view.pan[0] += delta.x;
                view.pan[1] += delta.y;
                self.set_view(0, view);
            }
        }
        if response.clicked()
            && swipe.solo.is_none()
            && let Some(photo) = response.interact_pointer_pos().and_then(under)
        {
            self.active = Some(swipe.ids[photo]);
        }
        if response.hovered() {
            let (delta, pinch) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = (delta * 0.003).exp() * pinch;
            if factor != 1.0 {
                self.focused_slot = 0;
                let anchor = response
                    .hover_pos()
                    .map_or(egui::Vec2::ZERO, |pointer| pointer - content.center());
                self.zoom_view_at(factor, anchor);
            }
        }
        let view = self.view_for(0);
        let paint = |app: &mut Self, ui: &egui::Ui, id: MediaId, clip: egui::Rect| {
            let texture = app.texture(id, true).or_else(|| app.texture(id, false));
            if let (Some(texture), Some(source)) = (texture, app.swipe_source(id)) {
                super::review::paint_region_clipped(
                    ui,
                    &texture,
                    content,
                    clip,
                    source,
                    inspection::Region::full(source[0], source[1]),
                    view,
                    app.preview_turns(id),
                );
            }
        };
        let name = |app: &Self, photo: usize| {
            let id = swipe.ids[photo];
            app.index
                .get(&id)
                .map_or_else(String::new, |i| app.media[*i].name())
        };
        let painter = ui.painter().with_clip_rect(rect);
        if let Some(photo) = swipe.solo {
            paint(self, ui, swipe.ids[photo], content);
        } else {
            for (photo, (from, to)) in slices.iter().enumerate() {
                let mut clip = content;
                clip.min.x = x(*from);
                clip.max.x = x(*to);
                paint(self, ui, swipe.ids[photo], clip);
            }
            let hover = response.hover_pos().and_then(under);
            for split in &swipe.splits {
                let at = x(*split);
                let line = painter.with_clip_rect(content);
                line.line_segment(
                    [egui::pos2(at, area.top()), egui::pos2(at, area.bottom())],
                    Stroke::new(2.0, Color32::from_white_alpha(220)),
                );
                let knob = egui::Rect::from_center_size(
                    egui::pos2(at, area.center().y),
                    egui::vec2(22.0, 34.0),
                );
                line.rect_filled(knob, 11.0, Color32::from_black_alpha(190));
                line.rect_stroke(
                    knob,
                    11.0,
                    Stroke::new(1.0, Color32::from_white_alpha(90)),
                    StrokeKind::Inside,
                );
                line.text(
                    knob.center(),
                    Align2::CENTER_CENTER,
                    egui_phosphor::regular::ARROWS_HORIZONTAL,
                    FontId::proportional(13.0),
                    Color32::WHITE,
                );
            }
            // The slice under the pointer is lit, so you know which photo
            // a click picks.
            if let Some(photo) = hover {
                let (from, to) = slices[photo];
                let mut lit = area;
                lit.min.x = x(from);
                lit.max.x = x(to);
                painter.rect_stroke(
                    lit.shrink(1.0),
                    0.0,
                    Stroke::new(1.5, ACCENT.gamma_multiply(0.6)),
                    StrokeKind::Inside,
                );
            }
        }
        // One numbered label per photo above its slice; whole, a row of
        // numbers to pick another and "All" to slice them again. A click on
        // a label shows that photo whole, or goes back to the slices.
        let mut picked = None;
        let label = |ui: &mut egui::Ui,
                     at: egui::Rect,
                     photo: Option<usize>,
                     text: String,
                     on: bool,
                     picked: &mut Option<Option<usize>>| {
            let mut job = egui::text::LayoutJob::simple_singleline(
                text,
                medium(10.5),
                if on { ON_ACCENT } else { INK },
            );
            job.wrap = egui::text::TextWrapping::truncate_at_width((at.width() - 14.0).max(8.0));
            let galley = ui.painter().layout_job(job);
            let pill = Align2::CENTER_CENTER
                .anchor_size(at.center(), galley.size() + egui::vec2(14.0, 6.0));
            let response = ui.interact(
                pill,
                ui.id().with(("swipe-label", photo, at.left() as i32)),
                Sense::click(),
            );
            let fill = if on {
                ACCENT
            } else if response.hovered() {
                CARD_HOVER
            } else {
                Color32::from_black_alpha(150)
            };
            ui.painter().rect_filled(pill, pill.height() / 2.0, fill);
            ui.painter().galley(
                pill.min + egui::vec2(7.0, 3.0),
                galley,
                if on { ON_ACCENT } else { INK },
            );
            let name = match photo {
                Some(photo) => format!("Show photo {} whole", photo + 1),
                None => "Show every photo in slices".to_owned(),
            };
            response
                .widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Button, true, &name));
            if response.on_hover_text(&name).clicked() {
                *picked = Some(photo);
            }
        };
        let band = egui::Rect::from_min_max(
            egui::pos2(area.left(), rect.top() + 4.0),
            egui::pos2(area.right(), content.top() - 4.0),
        );
        match swipe.solo {
            None => {
                for (photo, (from, to)) in slices.iter().enumerate() {
                    let at = egui::Rect::from_x_y_ranges(x(*from)..=x(*to), band.y_range());
                    let text = if at.width() > 90.0 {
                        format!("{}  {}", photo + 1, name(self, photo))
                    } else {
                        (photo + 1).to_string()
                    };
                    let on = self.active == Some(swipe.ids[photo]);
                    label(ui, at, Some(photo), text, on, &mut picked);
                }
            }
            Some(solo) => {
                let count = swipe.ids.len();
                let cell = 30.0;
                let row = egui::Rect::from_center_size(
                    band.center(),
                    egui::vec2(cell * (count as f32 + 1.4), band.height()),
                );
                for photo in 0..count {
                    let at = egui::Rect::from_min_size(
                        row.min + egui::vec2(photo as f32 * cell, 0.0),
                        egui::vec2(cell, band.height()),
                    );
                    label(
                        ui,
                        at,
                        Some(photo),
                        (photo + 1).to_string(),
                        photo == solo,
                        &mut picked,
                    );
                }
                let all = egui::Rect::from_min_max(
                    egui::pos2(row.left() + count as f32 * cell, row.top()),
                    row.max,
                );
                label(ui, all, None, "All".into(), false, &mut picked);
                painter.text(
                    egui::pos2(band.left() + 2.0, band.center().y),
                    Align2::LEFT_CENTER,
                    name(self, solo),
                    FontId::proportional(11.0),
                    MUTED,
                );
            }
        }
        if let Some(choice) = picked
            && let Some(swipe) = self.swipe.as_mut()
        {
            match choice {
                Some(photo) if swipe.solo != Some(photo) => {
                    swipe.solo = Some(photo);
                    self.active = Some(swipe.ids[photo]);
                }
                Some(_) | None => swipe.solo = None,
            }
        }
        let text = if swipe.solo.is_some() {
            "Swipe compare"
        } else {
            "Swipe compare: drag the dividers"
        };
        response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, text));
    }
    /// A photo's size as shown, turned upright, for swipe compare.
    fn swipe_source(&mut self, id: MediaId) -> Option<[u32; 2]> {
        self.metadata
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
                self.texture(id, true)
                    .or_else(|| self.texture(id, false))
                    .map(|t| [t.size()[0] as u32, t.size()[1] as u32])
            })
    }
}

#[cfg(test)]
mod tests {
    use super::super::compact_cull_tests::{fixture, rendered};
    use super::*;

    #[test]
    fn c_slices_the_compared_photos_numbers_flick_and_one_photo_does_nothing() {
        let (_temp, mut app) = fixture();
        let size = egui::vec2(1200.0, 800.0);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let ids: Vec<MediaId> = app.filtered[..3].iter().map(|i| app.media[*i].id).collect();
        app.selection.replace(ids.iter().copied());
        app.set_active(ids[0]);
        let press = |app: &mut PhotoSortApp, key, modifiers| {
            let _ = ctx.run_test(
                egui::RawInput {
                    events: vec![egui::Event::Key {
                        key,
                        physical_key: None,
                        pressed: true,
                        repeat: false,
                        modifiers,
                    }],
                    ..Default::default()
                },
                |ui| app.keyboard(ui.ctx()),
            );
        };
        let none = egui::Modifiers::NONE;
        press(&mut app, egui::Key::C, none);
        let swipe = app.swipe.clone().expect("C opens swipe compare");
        assert!(app.review);
        assert_eq!(swipe.ids, ids);
        // Three photos, three slices.
        assert_eq!(swipe.slices().len(), 3);
        let nodes = rendered(&mut app, &ctx, size);
        assert!(
            nodes
                .iter()
                .any(|(l, _)| l == "Swipe compare: drag the dividers"),
            "the canvas shows the dividers"
        );
        // 3 shows the third photo whole.
        press(&mut app, egui::Key::Num3, none);
        assert_eq!(app.swipe.as_ref().unwrap().solo, Some(2));
        assert_eq!(app.active, Some(ids[2]));
        // Ctrl/Cmd and a number rate the photo shown, and only that one,
        // even while the photos shown are being refreshed.
        app.browse_run = Some(u64::MAX);
        press(&mut app, egui::Key::Num2, egui::Modifiers::COMMAND);
        app.browse_run = None;
        assert_eq!(app.decisions[&ids[2]].rating, Some(2));
        assert!(
            ids[..2]
                .iter()
                .all(|id| app.decisions.get(id).is_none_or(|d| d.rating.is_none()))
        );
        // The same number again goes back to the slices.
        press(&mut app, egui::Key::Num3, none);
        assert_eq!(app.swipe.as_ref().unwrap().solo, None);
        // Esc leaves swipe compare only; the comparison and selection stay.
        press(&mut app, egui::Key::Escape, none);
        assert!(app.swipe.is_none());
        assert_eq!(app.comparison, ids);
        assert_eq!(app.selection.len(), 3);
        // With one photo selected, C does nothing.
        app.comparison.clear();
        app.review = false;
        app.selection.single(ids[0]);
        press(&mut app, egui::Key::C, none);
        assert!(app.swipe.is_none() && !app.review);
        app.runtime.shutdown();
    }
}
