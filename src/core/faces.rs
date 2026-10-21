//! Native BlazeFace and Face Mesh inference through ONNX Runtime.
use super::{models::model_path, pipeline::Asset};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Faces {
    pub count: usize,
    pub closed_eyes: usize,
    /// Largest verified face rectangle, in normalized source coordinates.
    pub primary: Option<[f32; 4]>,
    #[serde(default)]
    pub geometry: Vec<Face>,
}
/// Lightweight review evidence. Detailed landmarks stay in the shared artifact
/// cache instead of being retained by every UI analysis row.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct FaceSummary {
    pub count: usize,
    pub closed_eyes: usize,
    pub primary: Option<[f32; 4]>,
}
impl Faces {
    pub fn complete(&self) -> bool {
        self.count == self.geometry.len()
            && self.count <= 10
            && self.closed_eyes <= self.count
            && self.geometry.iter().all(|f| {
                !f.landmarks.is_empty()
                    && f.blink
                        .is_some_and(|b| b.iter().all(|v| (0.0..=1.0).contains(v)))
                    && f.bbox
                        .iter()
                        .chain(f.eyes.iter().flatten())
                        .chain(f.landmarks.iter().flatten())
                        .all(|v| v.is_finite())
            })
    }
    pub fn summary(&self) -> FaceSummary {
        FaceSummary {
            count: self.count,
            closed_eyes: self.closed_eyes,
            primary: self.primary,
        }
    }
}
/// Geometry from the same verified landmarks used for eye scoring.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Face {
    pub bbox: [f32; 4],
    pub eyes: [[f32; 2]; 2],
    /// All normalized points from the existing verified Face Mesh pass.
    /// Empty only for legacy cache records; motion comparison treats this as unknown.
    #[serde(default)]
    pub landmarks: Vec<[f32; 2]>,
    /// MediaPipe blendshape eye-blink scores (subject's left, right), 0–1.
    /// `None` only in legacy records, which `Faces::complete` rejects.
    #[serde(default)]
    pub blink: Option<[f32; 2]>,
}
#[derive(Clone)]
struct Detection {
    score: f32,
    x: f32,
    y: f32,
    size: f32,
    angle: f32,
}
/// Part of the face-cache identity; change it whenever a rule below changes.
pub const RECIPE: &str = "faces-v4-cew";
/// Eyes count as closed when the Face Mesh eye aspect ratio is clearly shut
/// (`SHUT_EYE_RATIO`), or when both MediaPipe blink scores reach `CLOSED_BLINK`
/// and the ratio agrees (`CLOSED_EYE_RATIO`).
///
/// Tuned on half of the CEW dataset (1,186 closed / 1,231 open faces, false
/// alarms held at ≤ 1%) and checked on the other half, which it never saw:
/// 86.2% of closed eyes caught, 1.0% of open eyes flagged (the blink-and-ratio
/// rule alone: 55.5% / 0.2%). On 76 hand-labelled faces from the sample
/// library it still catches all 10 closed and flags none of the 64 others,
/// including downward gazes (ratio ≥ 0.10), squints and sunglasses.
const SHUT_EYE_RATIO: f32 = 0.10;
const CLOSED_BLINK: f32 = 0.55;
const CLOSED_EYE_RATIO: f32 = 0.15;
/// Minimum detector score on a crop centred on the candidate. In the same set it
/// rejected 3 of 5 false faces (roof, blur, dark patch); real faces scored ≥ 0.66.
const MIN_REDETECT: f32 = 0.6;
/// MediaPipe's landmark subset for the blendshape model (face_blendshapes_graph),
/// verified to reproduce MediaPipe's own scores within 2e-6.
const BLENDSHAPE_POINTS: [usize; 146] = [
    0, 1, 4, 5, 6, 7, 8, 10, 13, 14, 17, 21, 33, 37, 39, 40, 46, 52, 53, 54, 55, 58, 61, 63, 65,
    66, 67, 70, 78, 80, 81, 82, 84, 87, 88, 91, 93, 95, 103, 105, 107, 109, 127, 132, 133, 136,
    144, 145, 146, 148, 149, 150, 152, 153, 154, 155, 157, 158, 159, 160, 161, 162, 163, 168, 172,
    173, 176, 178, 181, 185, 191, 195, 197, 234, 246, 249, 251, 263, 267, 269, 270, 276, 282, 283,
    284, 285, 288, 291, 293, 295, 296, 297, 300, 308, 310, 311, 312, 314, 317, 318, 321, 323, 324,
    332, 334, 336, 338, 356, 361, 362, 365, 373, 374, 375, 377, 378, 379, 380, 381, 382, 384, 385,
    386, 387, 388, 389, 390, 397, 398, 400, 402, 405, 409, 415, 454, 466, 468, 469, 470, 471, 472,
    473, 474, 475, 476, 477,
];
const EYE_BLINK_LEFT: usize = 9;
const EYE_BLINK_RIGHT: usize = 10;
pub fn analyze(models: &mut super::models::Models, asset: &Asset) -> Result<Faces> {
    analyze_cancellable(models, asset, || false)
}
pub fn analyze_cancellable(
    models: &mut super::models::Models,
    asset: &Asset,
    cancelled: impl Fn() -> bool,
) -> Result<Faces> {
    detect(models, asset, cancelled, None)
}
/// Every detector candidate with the evidence used to accept or reject it, for
/// offline threshold evaluation. Not used by the application itself.
#[derive(Clone, Debug, Serialize)]
pub struct Candidate {
    pub detector_score: f32,
    pub presence: f32,
    /// Mean eye aspect ratio; `NaN` for rejected candidates.
    pub eye_ratio: f32,
    /// Per-eye aspect ratios (subject's left, right).
    pub eye_ratios: [f32; 2],
    /// Detector square in normalized source coordinates.
    pub bbox: [f32; 4],
    pub accepted: bool,
    /// Best detector score on a crop centred on this candidate.
    pub redetect: f32,
    /// Blendshape eye-blink scores; `NaN` for rejected candidates.
    pub blink: [f32; 2],
}
pub fn analyze_with_candidates(
    models: &mut super::models::Models,
    asset: &Asset,
) -> Result<(Faces, Vec<Candidate>)> {
    let mut candidates = Vec::new();
    let faces = detect(models, asset, || false, Some(&mut candidates))?;
    Ok((faces, candidates))
}
fn detect(
    models: &mut super::models::Models,
    asset: &Asset,
    cancelled: impl Fn() -> bool,
    mut report: Option<&mut Vec<Candidate>>,
) -> Result<Faces> {
    let detector = model_path("face_detector.onnx").context("Face detector is not installed")?;
    let landmarks_model =
        model_path("face_landmarks_detector.onnx").context("Face Mesh is not installed")?;
    let blendshapes_model =
        model_path("face_blendshapes.onnx").context("Face blendshapes are not installed")?;
    let image = &*asset.pixels;
    let edge = image.width().max(image.height()) as f32;
    let cx = image.width() as f32 / 2.0;
    let cy = image.height() as f32 / 2.0;
    let mut candidates = Vec::new();
    let mut regions = vec![(cx, cy, edge)];
    if image.width().min(image.height()) > 512 {
        for fy in [0.25, 0.75] {
            for fx in [0.25, 0.75] {
                regions.push((
                    image.width() as f32 * fx,
                    image.height() as f32 * fy,
                    edge * 0.6,
                ));
            }
        }
    }
    for (cx, cy, edge) in regions {
        anyhow::ensure!(!cancelled(), "Face analysis cancelled");
        let input = sample(image, cx, cy, edge, 0.0, 128, true)?;
        let output = models.infer_all("face_detector", &detector, 128, input)?;
        anyhow::ensure!(output.len() >= 2, "Face detector returned no boxes");
        let boxes = &output[0];
        let scores = &output[1];
        anyhow::ensure!(
            boxes.len() == 896 * 16 && scores.len() == 896,
            "Unexpected BlazeFace output"
        );
        let mut index = 0;
        for (grid, repetitions) in [(16, 2), (8, 6)] {
            for y in 0..grid {
                for x in 0..grid {
                    for _ in 0..repetitions {
                        let score = sigmoid(scores[index]);
                        let b = &boxes[index * 16..index * 16 + 16];
                        if score >= 0.5 {
                            let center_x = b[0] / 128.0 + (x as f32 + 0.5) / grid as f32;
                            let center_y = b[1] / 128.0 + (y as f32 + 0.5) / grid as f32;
                            let size = b[2].max(b[3]) / 128.0 * edge;
                            if size.is_finite() && size > 4.0 {
                                candidates.push(Detection {
                                    score,
                                    x: (center_x - 0.5) * edge + cx,
                                    y: (center_y - 0.5) * edge + cy,
                                    size,
                                    angle: (b[7] - b[5]).atan2(b[6] - b[4]),
                                });
                            }
                        }
                        index += 1;
                    }
                }
            }
        }
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut selected: Vec<Detection> = Vec::new();
    for detection in candidates {
        if selected.iter().all(|old| overlap(old, &detection) < 0.3) {
            selected.push(detection);
        }
        if selected.len() == 10 {
            break;
        }
    }
    let mut faces = Faces::default();
    let mut largest = 0.0;
    for face in selected {
        anyhow::ensure!(!cancelled(), "Face analysis cancelled");
        let input = sample(
            image,
            face.x,
            face.y,
            face.size * 1.5,
            face.angle,
            256,
            false,
        )?;
        let output = models.infer_all("face_landmarks", &landmarks_model, 256, input)?;
        anyhow::ensure!(output.len() >= 2, "Face Mesh returned no landmarks");
        let landmarks = &output[0];
        let presence = &output[1];
        anyhow::ensure!(
            landmarks.len() >= 468 * 3 && !presence.is_empty(),
            "Unexpected Face Mesh output"
        );
        let square = [
            (face.x - face.size / 2.0) / image.width() as f32,
            (face.y - face.size / 2.0) / image.height() as f32,
            (face.x + face.size / 2.0) / image.width() as f32,
            (face.y + face.size / 2.0) / image.height() as f32,
        ];
        // Texture that barely passes in the whole photo rarely looks like a face
        // once centred and upright; real faces re-detect strongly.
        let redetect = {
            let input = sample(
                image,
                face.x,
                face.y,
                face.size * 2.0,
                face.angle,
                128,
                true,
            )?;
            let output = models.infer_all("face_detector", &detector, 128, input)?;
            output[1].iter().copied().map(sigmoid).fold(0.0, f32::max)
        };
        let mut record = |presence: f32, eye_ratios: [f32; 2], blink: [f32; 2], accepted: bool| {
            if let Some(report) = report.as_deref_mut() {
                report.push(Candidate {
                    detector_score: face.score,
                    presence,
                    eye_ratio: (eye_ratios[0] + eye_ratios[1]) / 2.0,
                    eye_ratios,
                    bbox: square,
                    accepted,
                    redetect,
                    blink,
                });
            }
        };
        if sigmoid(presence[0]) < 0.5 || redetect < MIN_REDETECT {
            record(sigmoid(presence[0]), [f32::NAN; 2], [f32::NAN; 2], false);
            continue;
        }
        let point = |index: usize| {
            let u = (landmarks[index * 3] / 256.0 - 0.5) * face.size * 1.5;
            let v = (landmarks[index * 3 + 1] / 256.0 - 0.5) * face.size * 1.5;
            let (sin, cos) = face.angle.sin_cos();
            [
                (face.x + u * cos - v * sin) / image.width() as f32,
                (face.y + u * sin + v * cos) / image.height() as f32,
            ]
        };
        // The blendshape model expects photo-frame pixel coordinates, as MediaPipe
        // feeds it; it is invariant to translation and scale.
        anyhow::ensure!(
            landmarks.len() >= 478 * 3,
            "Face Mesh returned no iris landmarks"
        );
        let (width, height) = (image.width() as f32, image.height() as f32);
        let points = BLENDSHAPE_POINTS
            .iter()
            .flat_map(|&i| {
                let [x, y] = point(i);
                [x * width, y * height]
            })
            .collect();
        let shapes = models.infer_points("face_blendshapes", &blendshapes_model, points)?;
        anyhow::ensure!(
            shapes.first().is_some_and(|s| s.len() == 52),
            "Unexpected blendshape output"
        );
        let blink =
            [shapes[0][EYE_BLINK_LEFT], shapes[0][EYE_BLINK_RIGHT]].map(|v| v.clamp(0.0, 1.0));
        let mut bbox = [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ];
        for i in 0..landmarks.len() / 3 {
            let [x, y] = point(i);
            bbox[0] = bbox[0].min(x);
            bbox[1] = bbox[1].min(y);
            bbox[2] = bbox[2].max(x);
            bbox[3] = bbox[3].max(y);
        }
        faces.geometry.push(Face {
            bbox,
            eyes: [point(33), point(263)],
            landmarks: (0..landmarks.len() / 3).map(point).collect(),
            blink: Some(blink),
        });
        faces.count += 1;
        let left = eye_aspect_ratio(landmarks, [33, 160, 158, 133, 153, 144]);
        let right = eye_aspect_ratio(landmarks, [362, 385, 387, 263, 373, 380]);
        record(sigmoid(presence[0]), [left, right], blink, true);
        let ratio = (left + right) / 2.0;
        if ratio < SHUT_EYE_RATIO
            || (blink.iter().all(|&b| b >= CLOSED_BLINK) && ratio < CLOSED_EYE_RATIO)
        {
            faces.closed_eyes += 1;
        }
        if face.size > largest {
            largest = face.size;
            faces.primary = Some([
                ((face.x - face.size * 0.75) / image.width() as f32).clamp(0.0, 1.0),
                ((face.y - face.size * 0.75) / image.height() as f32).clamp(0.0, 1.0),
                ((face.x + face.size * 0.75) / image.width() as f32).clamp(0.0, 1.0),
                ((face.y + face.size * 0.75) / image.height() as f32).clamp(0.0, 1.0),
            ]);
        }
    }
    Ok(faces)
}
/// One shared face artifact used by Organize, quality, subjects and duplicates.
/// Existing complete subject records can seed it without another model pass.
pub fn cached(
    models: &mut super::models::Models,
    asset: &Asset,
    store: &super::subjects::store::Store,
    source: &str,
    legacy: Option<&Faces>,
    cancelled: impl Fn() -> bool,
) -> Result<Faces> {
    cached_lazy(
        models,
        store,
        source,
        legacy,
        || Ok(asset.clone()),
        cancelled,
    )
}
/// Resolve existing face evidence before requesting pixels from the shared pipeline.
pub fn cached_lazy(
    models: &mut super::models::Models,
    store: &super::subjects::store::Store,
    source: &str,
    legacy: Option<&Faces>,
    mut asset: impl FnMut() -> Result<Asset>,
    cancelled: impl Fn() -> bool,
) -> Result<Faces> {
    anyhow::ensure!(!cancelled(), "Face analysis cancelled");
    let key = super::subjects::store::Store::face_key(source, &super::models::face_signature());
    if let Some(faces) = store.faces(&key) {
        return Ok(faces);
    }
    let faces = if let Some(faces) = legacy.filter(|f| f.complete()) {
        faces.clone()
    } else {
        analyze_cancellable(models, &asset()?, &cancelled)?
    };
    anyhow::ensure!(!cancelled(), "Face analysis cancelled");
    store.put_faces(&key, &faces)?;
    Ok(faces)
}
fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value.clamp(-80.0, 80.0)).exp())
}
fn overlap(a: &Detection, b: &Detection) -> f32 {
    let width =
        (a.x + a.size / 2.0).min(b.x + b.size / 2.0) - (a.x - a.size / 2.0).max(b.x - b.size / 2.0);
    let height =
        (a.y + a.size / 2.0).min(b.y + b.size / 2.0) - (a.y - a.size / 2.0).max(b.y - b.size / 2.0);
    let area = width.max(0.0) * height.max(0.0);
    area / (a.size * a.size + b.size * b.size - area).max(f32::EPSILON)
}
pub fn eye_aspect_ratio(points: &[f32], indices: [usize; 6]) -> f32 {
    if indices.iter().any(|i| i * 3 + 1 >= points.len()) {
        return 0.0;
    }
    let distance = |a: usize, b: usize| {
        ((points[indices[a] * 3] - points[indices[b] * 3]).powi(2)
            + (points[indices[a] * 3 + 1] - points[indices[b] * 3 + 1]).powi(2))
        .sqrt()
    };
    (distance(1, 5) + distance(2, 4)) / (2.0 * distance(0, 3)).max(f32::EPSILON)
}
fn sample(
    image: &image::RgbaImage,
    cx: f32,
    cy: f32,
    edge: f32,
    angle: f32,
    size: usize,
    signed: bool,
) -> Result<Vec<f32>> {
    let mut data = Vec::with_capacity(size * size * 3);
    let (sin, cos) = angle.sin_cos();
    for y in 0..size {
        for x in 0..size {
            let u = ((x as f32 + 0.5) / size as f32 - 0.5) * edge;
            let v = ((y as f32 + 0.5) / size as f32 - 0.5) * edge;
            let sx = cx + u * cos - v * sin - 0.5;
            let sy = cy + u * sin + v * cos - 0.5;
            for c in 0..3 {
                let value = if sx < 0.0
                    || sy < 0.0
                    || sx >= image.width() as f32 - 1.0
                    || sy >= image.height() as f32 - 1.0
                {
                    0.0
                } else {
                    let (ix, iy) = (sx as u32, sy as u32);
                    let (fx, fy) = (sx - ix as f32, sy - iy as f32);
                    let pixel = |x, y| f32::from(image.get_pixel(x, y)[c]);
                    (pixel(ix, iy) * (1.0 - fx) + pixel(ix + 1, iy) * fx) * (1.0 - fy)
                        + (pixel(ix, iy + 1) * (1.0 - fx) + pixel(ix + 1, iy + 1) * fx) * fy
                } / 255.0;
                data.push(if signed { value * 2.0 - 1.0 } else { value });
            }
        }
    }
    let mut nchw = vec![0.0; data.len()];
    for i in 0..size * size {
        for c in 0..3 {
            nchw[c * size * size + i] = data[i * 3 + c];
        }
    }
    Ok(nchw)
}

/// Eye-levelled, padded crop used by every face-identity consumer. Only the crop
/// is sampled, avoiding a rotated full-resolution allocation for each face.
pub fn aligned_crop(asset: &Asset, face: &Face) -> Result<Asset> {
    let image = &*asset.pixels;
    let (w, h) = (image.width() as f32, image.height() as f32);
    anyhow::ensure!(
        face.bbox
            .iter()
            .chain(face.eyes.iter().flatten())
            .all(|v| v.is_finite()),
        "Invalid face geometry"
    );
    let [left, top, right, bottom] = face.bbox;
    let pad_x = ((right - left) * w).max(1.0) * 0.25;
    let pad_y = ((bottom - top) * h).max(1.0) * 0.25;
    // Expand the padded crop outward to whole pixels, clipped to the source.
    let x0 = ((left * w - pad_x).trunc().max(0.0)).min(w) as u32;
    let y0 = ((top * h - pad_y).trunc().max(0.0)).min(h) as u32;
    let x1 = ((right * w + pad_x).ceil().max(0.0)).min(w) as u32;
    let y1 = ((bottom * h + pad_y).ceil().max(0.0)).min(h) as u32;
    anyhow::ensure!(x1 > x0 && y1 > y0, "Face crop is outside the image");
    let [[ax, ay], [bx, by]] = face.eyes;
    let angle = ((by - ay) * h).atan2((bx - ax) * w);
    let (sin, cos) = angle.sin_cos();
    let (cx, cy) = ((ax + bx) * w * 0.5, (ay + by) * h * 0.5);
    let pixels = std::sync::Arc::new(image::RgbaImage::from_fn(x1 - x0, y1 - y0, |x, y| {
        let u = (x + x0) as f32 + 0.5 - cx;
        let v = (y + y0) as f32 + 0.5 - cy;
        let sx = cx + u * cos - v * sin - 0.5;
        let sy = cy + u * sin + v * cos - 0.5;
        image::Rgba(std::array::from_fn(|c| {
            if c == 3 {
                255
            } else {
                cubic_pixel(image, sx, sy, c)
            }
        }))
    }));
    Ok(Asset {
        decoder_warning: asset.decoder_warning.clone(),
        pixels: pixels.clone(),
        thumbnail: pixels,
        metadata: asset.metadata.clone(),
    })
}
fn cubic_pixel(image: &image::RgbaImage, x: f32, y: f32, channel: usize) -> u8 {
    if x < -0.5 || y < -0.5 || x >= image.width() as f32 - 0.5 || y >= image.height() as f32 - 0.5 {
        return 0;
    }
    // Pillow geometric-transform cubic interpolation (a=-1), with clamped edges.
    let kernel = |v: f32| {
        let v = v.abs();
        if v <= 1.0 {
            v * v * v - 2.0 * v * v + 1.0
        } else if v < 2.0 {
            -v * v * v + 5.0 * v * v - 8.0 * v + 4.0
        } else {
            0.0
        }
    };
    let (ix, iy) = (x.floor() as i64, y.floor() as i64);
    let mut value = 0.0;
    for dy in -1..=2 {
        for dx in -1..=2 {
            let px = (ix + dx).clamp(0, i64::from(image.width()) - 1) as u32;
            let py = (iy + dy).clamp(0, i64::from(image.height()) - 1) as u32;
            value += f32::from(image.get_pixel(px, py)[channel])
                * kernel(x - (ix + dx) as f32)
                * kernel(y - (iy + dy) as f32);
        }
    }
    value.clamp(0.0, 255.0) as u8
}
