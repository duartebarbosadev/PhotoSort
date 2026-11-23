//! Tiles that ease to their new places when a grid's rows are laid out
//! again around photos whose shapes changed, instead of jumping there.
use eframe::egui;
use std::collections::HashMap;
use std::hash::Hash;

/// How fast a tile closes in on its place: about nine tenths of the way in
/// a fifth of a second, and a new place mid-way bends the path smoothly.
const PACE: f32 = 12.0;

/// Where each tile was drawn, so the next frame can ease it from there.
/// Places are relative to the grid, not the screen, so scrolling moves the
/// tiles at once and never lags behind.
pub(super) struct Reflow<T, K> {
    /// Each tile as drawn in the last frame. Only tiles drawn then are
    /// here, so a tile scrolling into view appears in its place.
    was: HashMap<T, egui::Rect>,
    now: HashMap<T, egui::Rect>,
    /// What the grid was laid out from in the last frame.
    key: Option<K>,
    /// The frame the grid was last drawn in.
    frame: u64,
    /// How far tiles move this frame, `None` when they snap.
    step: Option<f32>,
    moving: bool,
}

impl<T, K> Default for Reflow<T, K> {
    fn default() -> Self {
        Self {
            was: HashMap::new(),
            now: HashMap::new(),
            key: None,
            frame: 0,
            step: None,
            moving: false,
        }
    }
}

impl<T: Copy + Eq + Hash, K: PartialEq + Clone> Reflow<T, K> {
    /// Starts a frame of a grid laid out from `key`. Only a new shape
    /// moves tiles: a different key (another width, photo size or folder)
    /// puts every tile in its place at once, and so does reduced motion
    /// (`still`). `held` is how far the view scrolled this frame to keep
    /// the photo at its top still while the rows above it changed height:
    /// the tiles' last places move with it, so nothing on screen jumps.
    /// A grid not drawn in the frame before `frame` starts afresh too.
    pub(super) fn begin(&mut self, key: &K, frame: u64, held: f32, dt: f32, still: bool) {
        self.now.clear();
        self.moving = false;
        let gap = frame > self.frame + 1;
        self.frame = frame;
        if still || gap || self.key.as_ref() != Some(key) {
            self.was.clear();
            self.key = Some(key.clone());
        }
        if held != 0.0 {
            let shift = egui::vec2(0.0, held);
            for rect in self.was.values_mut() {
                *rect = rect.translate(shift);
            }
        }
        // A frame after a long idle counts as one at 30 a second, so a
        // tile never jumps most of the way in its first frame.
        self.step = (!still).then(|| 1.0 - (-dt.clamp(0.0, 1.0 / 30.0) * PACE).exp());
    }

    /// Where `tile` is drawn this frame on its way to `target`.
    pub(super) fn place(&mut self, tile: T, target: egui::Rect) -> egui::Rect {
        let rect = match (self.was.get(&tile), self.step) {
            (Some(was), Some(step)) => {
                let rect = egui::Rect::from_min_max(
                    was.min.lerp(target.min, step),
                    was.max.lerp(target.max, step),
                );
                let near = |a: egui::Pos2, b: egui::Pos2| (a - b).length() < 0.5;
                if near(rect.min, target.min) && near(rect.max, target.max) {
                    target
                } else {
                    self.moving = true;
                    rect
                }
            }
            _ => target,
        };
        self.now.insert(tile, rect);
        rect
    }

    /// Ends the frame: `true` while a tile is still on its way, so the next
    /// frame should come soon.
    pub(super) fn end(&mut self) -> bool {
        std::mem::swap(&mut self.was, &mut self.now);
        self.now.clear();
        self.moving
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
    }

    #[test]
    fn a_reshaped_tile_glides_to_its_place_and_comes_to_rest() {
        let mut reflow = Reflow::<u32, u8>::default();
        let dt = 1.0 / 60.0;
        let wide = rect(0.0, 0.0, 160.0, 120.0);
        let tall = rect(0.0, 0.0, 90.0, 120.0);
        let next = rect(100.0, 0.0, 160.0, 120.0);
        reflow.begin(&0, 1, 0.0, dt, false);
        assert_eq!(reflow.place(1, wide), wide);
        assert_eq!(reflow.place(2, next), next);
        assert!(!reflow.end());
        // Photo 1 turns out tall and its neighbour moves up beside it: both
        // start from where they were and close in on their new places.
        let moved = rect(98.0, 0.0, 160.0, 120.0);
        reflow.begin(&0, 1, 0.0, dt, false);
        let first = reflow.place(1, tall);
        // A tile seen for the first time appears in its place.
        assert_eq!(reflow.place(3, next), next);
        let second = reflow.place(2, moved);
        assert!(reflow.end());
        assert!(first.width() < wide.width() && first.width() > tall.width());
        assert!(second.left() < next.left() && second.left() > moved.left());
        let mut frames = 1;
        loop {
            reflow.begin(&0, 1, 0.0, dt, false);
            let (a, b) = (reflow.place(1, tall), reflow.place(2, moved));
            if !reflow.end() {
                assert_eq!((a, b), (tall, moved));
                break;
            }
            frames += 1;
            assert!(frames < 40, "never came to rest");
        }
        assert!(frames > 5, "snapped in {frames} frames");
        // Once at rest, nothing moves and nothing asks for frames.
        reflow.begin(&0, 1, 0.0, dt, false);
        assert_eq!(reflow.place(1, tall), tall);
        assert!(!reflow.end());
    }

    #[test]
    fn a_new_layout_reduced_motion_and_a_held_view_never_slide_tiles() {
        let mut reflow = Reflow::<u32, u8>::default();
        let dt = 1.0 / 60.0;
        let start = rect(0.0, 300.0, 160.0, 120.0);
        let lower = rect(0.0, 340.0, 160.0, 120.0);
        reflow.begin(&0, 1, 0.0, dt, false);
        reflow.place(1, start);
        reflow.end();
        // Rows above the view grew by 40 and the view scrolled 40 to hold
        // still: the tile stays where it is on screen.
        reflow.begin(&0, 2, 40.0, dt, false);
        assert_eq!(reflow.place(1, lower), lower);
        assert!(!reflow.end());
        // Another width or photo size snaps.
        reflow.begin(&1, 3, 0.0, dt, false);
        assert_eq!(reflow.place(1, start), start);
        assert!(!reflow.end());
        // Reduced motion snaps too.
        reflow.begin(&1, 4, 0.0, dt, true);
        assert_eq!(reflow.place(1, lower), lower);
        assert!(!reflow.end());
    }
}
