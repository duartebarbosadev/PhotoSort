//! How RAW photos are shown. Neutral is the camera-faithful render; Enhanced
//! adds a punchier look (per-channel auto levels, then
//! 1.2× saturation). The look only changes display pixels: analysis, scores
//! and the shared disk proxies always use the neutral render.
use image::RgbaImage;
use serde::{Deserialize, Serialize};
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawLook {
    #[default]
    Neutral,
    Enhanced,
}

/// Share of pixels clipped at each end when stretching levels, so a few hot
/// or dead pixels cannot hold the range open.
const CLIP: f64 = 0.001;
const SATURATION: f32 = 1.2;

/// Per-channel black and white points measured on one image.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Levels {
    low: [u8; 3],
    high: [u8; 3],
}

impl Levels {
    /// Measured on at most ~250k sampled pixels, so a full-resolution source
    /// costs the same as a preview.
    pub fn measure(image: &RgbaImage) -> Self {
        let pixels = image.as_raw();
        let count = pixels.len() / 4;
        let stride = (count / 250_000).max(1);
        let mut histograms = [[0u32; 256]; 3];
        let mut sampled = 0u64;
        for pixel in pixels.as_chunks::<4>().0.iter().step_by(stride) {
            for (channel, histogram) in histograms.iter_mut().enumerate() {
                histogram[pixel[channel] as usize] += 1;
            }
            sampled += 1;
        }
        let clip = (sampled as f64 * CLIP) as u64;
        let mut low = [0u8; 3];
        let mut high = [255u8; 3];
        for (channel, histogram) in histograms.iter().enumerate() {
            let mut seen = 0u64;
            for (value, count) in histogram.iter().enumerate() {
                seen += u64::from(*count);
                if seen > clip {
                    low[channel] = value as u8;
                    break;
                }
            }
            seen = 0;
            for (value, count) in histogram.iter().enumerate().rev() {
                seen += u64::from(*count);
                if seen > clip {
                    high[channel] = value as u8;
                    break;
                }
            }
        }
        Self { low, high }
    }

    /// Stretches each channel to its levels, then raises saturation around
    /// Rec. 601 luma. Alpha is untouched.
    pub fn apply(self, image: &mut RgbaImage) {
        let tables: [[u8; 256]; 3] = std::array::from_fn(|channel| {
            let (low, high) = (self.low[channel], self.high[channel]);
            std::array::from_fn(|value| {
                if high <= low {
                    return value as u8;
                }
                let scaled = (value as f32 - f32::from(low)) * 255.0 / f32::from(high - low);
                scaled.round().clamp(0.0, 255.0) as u8
            })
        });
        for pixel in image.as_mut().as_chunks_mut::<4>().0 {
            let [r, g, b] = std::array::from_fn(|c| f32::from(tables[c][pixel[c] as usize]));
            let luma = 0.299 * r + 0.587 * g + 0.114 * b;
            for (channel, value) in [r, g, b].into_iter().enumerate() {
                pixel[channel] = (luma + (value - luma) * SATURATION)
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
}

/// The current look, shared between Settings and the image worker.
#[derive(Clone, Default)]
pub struct Control(Arc<AtomicU8>);

impl Control {
    pub fn set(&self, look: RawLook) {
        self.0.store(look as u8, Ordering::Release);
    }
    pub fn get(&self) -> RawLook {
        match self.0.load(Ordering::Acquire) {
            1 => RawLook::Enhanced,
            _ => RawLook::Neutral,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enhanced_look_stretches_flat_raws_and_keeps_grey_grey() {
        // A flat, low-contrast render: values between 60 and 180.
        let mut image = RgbaImage::from_fn(64, 64, |x, y| {
            let v = 60 + ((x + y) * 120 / 126) as u8;
            image::Rgba([v, v, v, 200])
        });
        let levels = Levels::measure(&image);
        levels.apply(&mut image);
        let values: Vec<u8> = image.pixels().map(|p| p[0]).collect();
        assert_eq!(*values.iter().min().unwrap(), 0);
        assert_eq!(*values.iter().max().unwrap(), 255);
        // Neutral greys stay neutral, alpha is untouched.
        assert!(
            image
                .pixels()
                .all(|p| p[0] == p[1] && p[1] == p[2] && p[3] == 200)
        );
    }

    #[test]
    fn saturation_moves_colour_away_from_grey_and_flat_images_survive() {
        let mut image = RgbaImage::from_pixel(4, 4, image::Rgba([200, 100, 50, 255]));
        // A single colour has no range to stretch; only saturation applies.
        Levels::measure(&image).apply(&mut image);
        let p = image.get_pixel(0, 0);
        assert!(p[0] > 200 && p[2] < 50, "{p:?}");
        let control = Control::default();
        assert_eq!(control.get(), RawLook::Neutral);
        control.set(RawLook::Enhanced);
        assert_eq!(control.get(), RawLook::Enhanced);
    }
}
