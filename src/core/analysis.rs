use super::pipeline::Asset;
use image::{DynamicImage, imageops::FilterType};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Analysis {
    pub sharpness: f64,
    #[serde(default)]
    pub quality_sharpness: Option<f64>,
    pub aesthetic: Option<f32>,
    pub faces: Option<super::faces::FaceSummary>,
    pub brightness: f64,
    #[serde(default)]
    pub black_fraction: f64,
    pub score: f64,
    pub hash: u64,
    pub color: [f32; 3],
    pub issues: Vec<String>,
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Thresholds {
    pub blur: f64,
    pub dark: f64,
    pub bright: f64,
}
impl Default for Thresholds {
    fn default() -> Self {
        Self {
            blur: 100.0,
            dark: 15.0,
            bright: 248.0,
        }
    }
}
impl Thresholds {
    pub fn normalize(&mut self) {
        let default = Self::default();
        for (value, fallback, min, max) in [
            (&mut self.blur, default.blur, 10.0, 1000.0),
            (&mut self.dark, default.dark, 0.0, 128.0),
            (&mut self.bright, default.bright, 128.0, 255.0),
        ] {
            *value = if value.is_finite() {
                value.clamp(min, max)
            } else {
                fallback
            };
        }
    }
}
impl Analysis {
    /// Classify cached measurements. No pixels, metadata, models or IO are needed.
    pub fn reclassify(&mut self, thresholds: Thresholds) {
        self.issues.clear();
        // Check darkness before blur so night photos are not reported as blurry.
        if self.brightness < thresholds.dark {
            if self.black_fraction >= 0.98 {
                self.issues.push("Very dark".into());
            }
        } else if self.sharpness < thresholds.blur {
            self.issues.push("Possibly soft".into());
        } else if self.brightness > thresholds.bright {
            self.issues.push("Overexposed".into());
        }
    }
}
/// One measurement pass supplies Easy Delete and future Pick Best scoring. Results
/// are cached by the image actor; changing a threshold never repeats this work.
pub fn analyze(asset: &Asset, thresholds: Thresholds) -> Analysis {
    analyze_cancellable(asset, thresholds, || false).expect("Uncancelled image measurements")
}
fn measurement_frame(asset: &Asset, cancelled: &impl Fn() -> bool) -> Option<image::RgbImage> {
    let rgb = super::pixels::rgb(&asset.pixels, cancelled)?;
    if rgb.width().max(rgb.height()) > 1024 {
        let resized = DynamicImage::ImageRgb8(rgb)
            .resize(1024, 1024, FilterType::Lanczos3)
            .to_rgb8();
        (!cancelled()).then_some(resized)
    } else {
        Some(rgb)
    }
}
/// Never enlarge small photos. Detection dimensions round half to even.
pub fn detection_size(width: u32, height: u32) -> (u32, u32) {
    let scale = (640. / f64::from(width))
        .min(480. / f64::from(height))
        .min(1.);
    (
        (f64::from(width) * scale).round_ties_even().max(1.) as u32,
        (f64::from(height) * scale).round_ties_even().max(1.) as u32,
    )
}
pub fn analyze_cancellable(
    asset: &Asset,
    thresholds: Thresholds,
    cancelled: impl Fn() -> bool,
) -> Option<Analysis> {
    use super::pixels;
    let rgb = measurement_frame(asset, &cancelled)?;
    let whole_gray = pixels::gray(&rgb, &cancelled)?;
    let quality_sharpness =
        pixels::laplacian_variance(&whole_gray, [0, 0, rgb.width(), rgb.height()], &cancelled)?;
    let (w, h) = detection_size(rgb.width(), rgb.height());
    let resized;
    let detection = if (w, h) == rgb.dimensions() {
        &rgb
    } else {
        resized = pixels::resize_area(&rgb, w, h, &cancelled)?;
        &resized
    };
    let gray_resized;
    let gray = if (w, h) == rgb.dimensions() {
        &whole_gray
    } else {
        gray_resized = pixels::gray(detection, &cancelled)?;
        &gray_resized
    };
    let count = gray.len().max(1) as f64;
    let brightness = gray.as_raw().iter().map(|v| f64::from(*v)).sum::<f64>() / count;
    let black_fraction = gray.as_raw().iter().filter(|v| **v <= 10).count() as f64 / count;
    let mut peak = 0f64;
    if w < 4 || h < 4 {
        peak = pixels::laplacian_variance(gray, [0, 0, w, h], &cancelled)?;
    } else {
        let split = |n: u32, i: u32| i * (n / 4) + i.min(n % 4);
        for y in 0..4 {
            for x in 0..4 {
                let (left, top) = (split(w, x), split(h, y));
                peak = peak.max(pixels::laplacian_variance(
                    gray,
                    [left, top, split(w, x + 1) - left, split(h, y + 1) - top],
                    &cancelled,
                )?);
            }
        }
    }
    let small = image::imageops::resize(gray, 9, 8, FilterType::Triangle);
    let mut hash = 0;
    for y in 0..8 {
        for x in 0..8 {
            if small.get_pixel(x, y)[0] > small.get_pixel(x + 1, y)[0] {
                hash |= 1 << (y * 8 + x);
            }
        }
    }
    let mut color = [0f32; 3];
    for row in detection.rows() {
        if cancelled() {
            return None;
        }
        for p in row {
            for c in 0..3 {
                color[c] += f32::from(p[c]) / 255.;
            }
        }
    }
    for c in &mut color {
        *c /= count as f32;
    }
    let exposure = (1. - (brightness - 127.5).abs() / 127.5).max(0.);
    let mut result = Analysis {
        sharpness: peak,
        quality_sharpness: Some(quality_sharpness),
        brightness,
        black_fraction,
        score: (peak.ln_1p() / 10.).min(1.) * 80. + exposure * 20.,
        hash,
        color,
        ..Default::default()
    };
    result.reclassify(thresholds);
    (!cancelled()).then_some(result)
}
pub fn similar(a: &Analysis, b: &Analysis) -> bool {
    (a.hash ^ b.hash).count_ones() <= 6
        && a.color
            .iter()
            .zip(b.color)
            .map(|(a, b)| (a - b).abs())
            .sum::<f32>()
            < 0.18
}
/// Bounded candidate index; avoids a quadratic all-pairs comparison.
#[derive(Default)]
pub struct VisualGroups {
    buckets: std::collections::HashMap<(u8, u16), Vec<(usize, Analysis)>>,
    next: usize,
}
impl VisualGroups {
    pub fn assign(&mut self, a: &Analysis) -> usize {
        let mut candidate = None;
        for band in 0..4u8 {
            let key = (band, (a.hash >> (band * 16)) as u16);
            if let Some(group) = self.buckets.get(&key) {
                for (id, b) in group.iter().rev().take(64) {
                    if similar(a, b) {
                        candidate = Some(candidate.map_or(*id, |old: usize| old.min(*id)));
                    }
                }
            }
        }
        let id = candidate.unwrap_or_else(|| {
            self.next += 1;
            self.next
        });
        for band in 0..4u8 {
            let group = self
                .buckets
                .entry((band, (a.hash >> (band * 16)) as u16))
                .or_default();
            if !group.iter().any(|(old, _)| *old == id) {
                group.push((id, a.clone()));
            }
        }
        id
    }
}

pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

/// Select a candidate per group after the relative-sharpness gate.
/// This only suggests keepers; it never creates deletion marks.
pub fn best_per_group(
    entries: impl IntoIterator<Item = (super::model::MediaId, usize, Analysis)>,
) -> Vec<super::model::MediaId> {
    let mut groups: std::collections::BTreeMap<usize, Vec<_>> = std::collections::BTreeMap::new();
    for (id, group, analysis) in entries {
        groups.entry(group).or_default().push((id, analysis));
    }
    groups
        .into_values()
        .filter_map(|items| {
            let sharpest = items.iter().map(|(_, a)| a.sharpness).fold(0.0, f64::max);
            items
                .iter()
                .filter(|(_, a)| a.sharpness >= sharpest * 0.6)
                .max_by(|(aid, a), (bid, b)| {
                    a.score.total_cmp(&b.score).then_with(|| bid.0.cmp(&aid.0))
                })
                .map(|(id, _)| *id)
        })
        .collect()
}

/// Whole-image score for callers that do not need an Easy Delete result. Runtime
/// normally obtains this from its cached combined measurement pass instead.
pub fn quality_sharpness(asset: &Asset) -> f64 {
    let rgb = measurement_frame(asset, &|| false).unwrap();
    let gray = super::pixels::gray(&rgb, &|| false).unwrap();
    super::pixels::laplacian_variance(&gray, [0, 0, rgb.width(), rgb.height()], &|| false).unwrap()
}

/// Threshold-independent evidence. Review choices and AI-dependent scores are
/// deliberately absent from this persistent record.
#[derive(Clone, Serialize, Deserialize)]
pub struct Measurements {
    pub analysis: Analysis,
    pub width: u32,
    pub height: u32,
    pub content_hash: String,
}
impl Measurements {
    pub fn valid(&self) -> bool {
        let a = &self.analysis;
        (1..=super::pipeline::ANALYSIS_EDGE).contains(&self.width)
            && (1..=super::pipeline::ANALYSIS_EDGE).contains(&self.height)
            && a.sharpness.is_finite()
            && a.sharpness >= 0.0
            && a.quality_sharpness
                .is_some_and(|v| v.is_finite() && v >= 0.0)
            && (0.0..=255.0).contains(&a.brightness)
            && (0.0..=1.0).contains(&a.black_fraction)
            && a.score.is_finite()
            && a.color
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.001).contains(v))
            && a.aesthetic.is_none()
            && a.faces.is_none()
            && a.issues.is_empty()
            && self.content_hash.len() == 64
            && self.content_hash.bytes().all(|b| b.is_ascii_hexdigit())
    }
}
