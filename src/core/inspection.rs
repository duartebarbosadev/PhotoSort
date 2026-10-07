//! Shared image-inspection geometry, independent of the GUI and decoder.
use super::model::Media;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}
impl Region {
    pub fn full(width: u32, height: u32) -> Self {
        Self {
            x: 0,
            y: 0,
            width,
            height,
        }
    }
}
#[derive(Clone, Debug, PartialEq)]
pub struct Request {
    pub slot: u64,
    pub media: Media,
    pub region: Region,
    pub output_edge: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct View {
    /// A multiplier of fit scale, or native scale when actual_size is selected.
    pub zoom: f32,
    pub pan: [f32; 2],
    pub actual_size: bool,
}
impl Default for View {
    fn default() -> Self {
        Self {
            zoom: 1.0,
            pan: [0.0; 2],
            actual_size: false,
        }
    }
}
impl View {
    pub const MIN_ZOOM: f32 = 0.05;
    pub const MAX_ZOOM: f32 = 32.0;
    pub fn scale(
        self,
        source: [u32; 2],
        viewport: [f32; 2],
        turns: u8,
        pixels_per_point: f32,
    ) -> f32 {
        let [w, h] = if turns.is_multiple_of(2) {
            source
        } else {
            [source[1], source[0]]
        };
        let base = if self.actual_size {
            1.0 / pixels_per_point.max(0.1)
        } else {
            (viewport[0] / w.max(1) as f32).min(viewport[1] / h.max(1) as f32)
        };
        (base * self.zoom).max(0.0001)
    }
    /// Full decoding is useful only when the displayed preview is undersampled,
    /// or when the user explicitly requests source-pixel inspection.
    pub fn needs_original(
        self,
        source: [u32; 2],
        preview: Option<[u32; 2]>,
        viewport: [f32; 2],
        turns: u8,
        pixels_per_point: f32,
    ) -> bool {
        if self.actual_size {
            return true;
        }
        let Some(preview) = preview else { return false };
        let density =
            self.scale(source, viewport, turns, pixels_per_point) * pixels_per_point.max(0.1);
        (0..2).any(|axis| source[axis] as f32 * density > preview[axis] as f32 + 1.0)
    }
    pub fn visible_region(
        self,
        source: [u32; 2],
        viewport: [f32; 2],
        turns: u8,
        pixels_per_point: f32,
    ) -> Option<Region> {
        let scale = self.scale(source, viewport, turns, pixels_per_point);
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for x in [-viewport[0] / 2.0, viewport[0] / 2.0] {
            for y in [-viewport[1] / 2.0, viewport[1] / 2.0] {
                let [x, y] = rotate(
                    [(x - self.pan[0]) / scale, (y - self.pan[1]) / scale],
                    (4 - turns % 4) % 4,
                );
                for (axis, value) in [x + source[0] as f32 / 2.0, y + source[1] as f32 / 2.0]
                    .into_iter()
                    .enumerate()
                {
                    min[axis] = min[axis].min(value);
                    max[axis] = max[axis].max(value);
                }
            }
        }
        let x = min[0].floor().clamp(0.0, source[0] as f32) as u32;
        let y = min[1].floor().clamp(0.0, source[1] as f32) as u32;
        let right = max[0].ceil().clamp(0.0, source[0] as f32) as u32;
        let bottom = max[1].ceil().clamp(0.0, source[1] as f32) as u32;
        (right > x && bottom > y).then_some(Region {
            x,
            y,
            width: right.saturating_sub(x),
            height: bottom.saturating_sub(y),
        })
    }
}
pub fn rotate([x, y]: [f32; 2], turns: u8) -> [f32; 2] {
    match turns % 4 {
        1 => [-y, x],
        2 => [-x, -y],
        3 => [y, -x],
        _ => [x, y],
    }
}

/// A bounded latest-view mailbox. Completed unchanged requests stay completed;
/// navigation replaces queued work instead of building an obsolete decode backlog.
#[derive(Default)]
pub struct Queue {
    epoch: u64,
    desired: Vec<Request>,
    pending: std::collections::VecDeque<Request>,
}
impl Queue {
    pub fn update(&mut self, epoch: u64, requests: Vec<Request>) {
        let requests: Vec<_> = requests.into_iter().take(9).collect();
        if self.epoch != epoch {
            self.desired.clear();
            self.pending.clear();
            self.epoch = epoch;
        }
        self.pending.retain(|request| requests.contains(request));
        for request in &requests {
            if !self.desired.contains(request) {
                self.pending.push_back(request.clone());
            }
        }
        self.desired = requests;
    }
    pub fn take(&mut self) -> Option<(u64, Request)> {
        self.pending
            .pop_front()
            .map(|request| (self.epoch, request))
    }
    pub fn accepts(&self, epoch: u64, request: &Request) -> bool {
        self.epoch == epoch && self.desired.contains(request)
    }
}
