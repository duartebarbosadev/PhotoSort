//! Whole people in a photo, seen from any side: the person class of
//! YOLOX-nano, a COCO object detector. The face detector finds only faces,
//! so it misses someone seen from behind or in profile; this does not.
use super::pipeline::Asset;
use serde::{Deserialize, Serialize};

/// The installed file (`model_assets::Id::Persons`).
pub const MODEL: &str = "person_detector.onnx";
/// Part of the cache key with the model's own signature: change it when what
/// is kept for a photo changes.
pub const RECIPE: &str = "persons-v1";
/// The model's square input.
const SIZE: usize = 416;
/// YOLOX pads the letterboxed photo with this grey.
const PAD: f32 = 114.0;
/// The detections kept for a photo, a little below what counts as a person:
/// what counts can then change without detecting again.
const KEPT: f32 = 0.25;
const MOST: usize = 32;
/// A person counts when the model is this sure, and the person is at least
/// this tall a share of the photo: someone in it, not a speck in the
/// distance. See `Person::clear`.
const CLEAR_SCORE: f32 = 0.5;
const CLEAR_HEIGHT: f32 = 0.12;

/// One person found: how sure the model is, and the box in the photo's
/// own shape (left, top, right, bottom, from 0 to 1).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Person {
    pub score: f32,
    pub bbox: [f32; 4],
}
impl Person {
    pub fn valid(&self) -> bool {
        let [left, top, right, bottom] = self.bbox;
        (0.0..=1.0).contains(&self.score)
            && self.bbox.iter().all(|v| (0.0..=1.0).contains(v))
            && left <= right
            && top <= bottom
    }
    /// Clearly someone in the photo, the size a person nearby or in the
    /// scene has: the two walkers behind someone on a bridge count, a hiker
    /// a tiny figure in a wide landscape does not.
    pub fn clear(&self) -> bool {
        self.score >= CLEAR_SCORE && self.bbox[3] - self.bbox[1] >= CLEAR_HEIGHT
    }
}

/// How many people are clearly in the photo.
pub fn count(people: &[Person]) -> usize {
    people.iter().filter(|p| p.clear()).count()
}

/// The model's input: the photo scaled to fit 416 pixels, top-left, the rest
/// grey, as BGR values from 0 to 255 in planes. With the scale used.
pub fn input(asset: &Asset) -> (Vec<f32>, f32) {
    let (width, height) = asset.pixels.dimensions();
    let scale = (SIZE as f32 / width.max(1) as f32).min(SIZE as f32 / height.max(1) as f32);
    let (w, h) = (
        ((width as f32 * scale).round() as u32).clamp(1, SIZE as u32),
        ((height as f32 * scale).round() as u32).clamp(1, SIZE as u32),
    );
    let resized =
        image::imageops::resize(&*asset.pixels, w, h, image::imageops::FilterType::Triangle);
    let plane = SIZE * SIZE;
    let mut tensor = vec![PAD; 3 * plane];
    for (x, y, pixel) in resized.enumerate_pixels() {
        let at = y as usize * SIZE + x as usize;
        let [r, g, b, _] = pixel.0;
        tensor[at] = f32::from(b);
        tensor[plane + at] = f32::from(g);
        tensor[2 * plane + at] = f32::from(r);
    }
    (tensor, scale)
}

/// The people in the model's output: one row of 85 values per cell of its
/// 52, 26 and 13 square grids (box centre and size relative to the cell,
/// objectness, then the 80 COCO classes, person first). Boxes are mapped
/// back to the photo, overlapping ones merged, the surest first.
pub fn decode(output: &[f32], scale: f32, width: u32, height: u32) -> Vec<Person> {
    const ROW: usize = 85;
    let (width, height) = (width.max(1) as f32, height.max(1) as f32);
    let mut found = Vec::new();
    let mut row = 0;
    for stride in [8usize, 16, 32] {
        let side = SIZE / stride;
        for y in 0..side {
            for x in 0..side {
                let Some(values) = output.get(row * ROW..(row + 1) * ROW) else {
                    return Vec::new();
                };
                row += 1;
                let score = values[4] * values[5];
                if !score.is_finite() || score < KEPT {
                    continue;
                }
                let stride = stride as f32;
                let cx = (values[0] + x as f32) * stride;
                let cy = (values[1] + y as f32) * stride;
                let (w, h) = (values[2].exp() * stride, values[3].exp() * stride);
                let edge = |v: f32, size: f32| (v / scale / size).clamp(0.0, 1.0);
                found.push(Person {
                    score: score.min(1.0),
                    bbox: [
                        edge(cx - w / 2.0, width),
                        edge(cy - h / 2.0, height),
                        edge(cx + w / 2.0, width),
                        edge(cy + h / 2.0, height),
                    ],
                });
            }
        }
    }
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut kept: Vec<Person> = Vec::new();
    for person in found {
        if kept.len() < MOST && kept.iter().all(|k| overlap(k, &person) < 0.45) {
            kept.push(person);
        }
    }
    kept.retain(Person::valid);
    kept
}

/// How much two boxes overlap: their intersection over their union.
fn overlap(a: &Person, b: &Person) -> f32 {
    let [al, at, ar, ab] = a.bbox;
    let [bl, bt, br, bb] = b.bbox;
    let inter = (ar.min(br) - al.max(bl)).max(0.0) * (ab.min(bb) - at.max(bt)).max(0.0);
    let union = (ar - al) * (ab - at) + (br - bl) * (bb - bt) - inter;
    if union > 0.0 { inter / union } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_person_counts_when_sure_and_tall_enough_and_a_speck_does_not() {
        // A 832x416 photo, scaled by half into the model's square.
        let mut output = vec![0.0; 3549 * 85];
        let mut cell = |stride: usize, x: usize, y: usize, size: [f32; 2], sure: f32| {
            let before: usize = [8, 16, 32]
                .iter()
                .take_while(|s| **s != stride)
                .map(|s| (SIZE / s).pow(2))
                .sum();
            let row = &mut output[(before + y * (SIZE / stride) + x) * 85..][..85];
            row[0] = 0.5;
            row[1] = 0.5;
            row[2] = (size[0] / stride as f32).ln();
            row[3] = (size[1] / stride as f32).ln();
            row[4] = sure;
            row[5] = sure;
        };
        // Someone standing in the middle, half the photo's height: the
        // model sees them 104 pixels tall.
        cell(32, 6, 3, [40.0, 104.0], 0.9);
        // A hiker far off, 5% of the height.
        cell(8, 10, 10, [4.0, 10.0], 0.9);
        // Something the model is unsure of.
        cell(16, 3, 3, [30.0, 100.0], 0.4);
        let people = decode(&output, 0.5, 832, 416);
        assert_eq!(people.len(), 2, "the unsure one is not kept");
        let height = |p: &Person| p.bbox[3] - p.bbox[1];
        let standing = *people
            .iter()
            .max_by(|a, b| height(a).total_cmp(&height(b)))
            .unwrap();
        assert!((height(&standing) - 0.5).abs() < 1e-3);
        assert!(
            (standing.bbox[0] + standing.bbox[2] - 1.0).abs() < 1e-3,
            "centred"
        );
        assert_eq!(count(&people), 1);
    }

    #[test]
    #[ignore = "requires PHOTOSORT_SAMPLE_DIR and the person detector; prints what it finds"]
    fn people_in_sample_photos() {
        let root = std::path::PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap());
        let mut models = super::super::models::Models::default();
        let mut paths: Vec<_> = std::fs::read_dir(&root)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        paths.sort();
        for path in paths {
            let pixels = image::open(&path).unwrap().to_rgba8();
            let pixels = std::sync::Arc::new(pixels);
            let asset = Asset {
                decoder_warning: None,
                thumbnail: pixels.clone(),
                pixels,
                metadata: Default::default(),
            };
            let people = models.persons(&asset).unwrap();
            eprintln!(
                "{}: {} clear, found {:?}",
                path.file_name().unwrap().to_string_lossy(),
                count(&people),
                people
                    .iter()
                    .map(|p| (p.score, p.bbox[3] - p.bbox[1]))
                    .collect::<Vec<_>>()
            );
        }
    }
}
