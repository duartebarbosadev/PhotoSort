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
pub const RECIPE: &str = "faces-v5-tiles";
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
/// Eyes are judged open or closed only when their centres are at least this many
/// pixels apart. On smaller faces glasses, squints and blur decide more than the
/// eyelids: in a 79-photo phone library the same 11-pixel face was "closed" in a
/// HEIC and open in its JPEG twin. The five fixed detector windows used before
/// tiling never found faces this small, so verdicts on larger faces are unchanged.
const MIN_EYE_PIXELS: f32 = 20.0;
/// Per-person verdicts (`eye_state`): closed needs both blink scores at
/// least `STRICT_BLINK`; open needs each eye's aspect ratio at least
/// `OPEN_EYE_RATIO` and neither blink score above `OPEN_BLINK`.
const STRICT_BLINK: f32 = 0.6;
const OPEN_EYE_RATIO: f32 = 0.18;
const OPEN_BLINK: f32 = 0.4;
/// Minimum detector score for a box centred on the candidate, on a crop centred
/// on it, and minimum Face Mesh presence. Tiling gives the detector many more
/// chances to fire on texture than five fixed windows did, so both are stricter
/// than before (0.6 for any box in the crop, 0.5). On a 79-photo phone library
/// and the SFace demo group, 74 real faces had a centred re-detection of at least
/// 0.79 and presence of at least 0.91; the three false faces the tiles added
/// (a wheel, a ceiling, wall texture) had centred re-detections of 0.73, under
/// 0.6 and 0.66, and the wheel a presence of 0.65; the ceiling only passed the
/// old check because a real face was beside it.
const MIN_REDETECT: f32 = 0.7;
const MIN_PRESENCE: f32 = 0.8;
/// Detector tiles shrink by this factor per level, down to `MIN_TILE` pixels.
const TILE_STEP: f32 = 0.6;
const MIN_TILE: f32 = 160.0;
/// Detector candidates verified with Face Mesh, strongest first; at most ten
/// faces are kept.
const MAX_CANDIDATES: usize = 24;
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
/// Whether faces can be analyzed: the native runtime is built in and the
/// three face models are installed.
pub fn installed() -> bool {
    cfg!(feature = "onnx")
        && [
            "face_detector.onnx",
            "face_landmarks_detector.onnx",
            "face_blendshapes.onnx",
        ]
        .iter()
        .all(|name| model_path(name).is_some())
}
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
    let mut candidates = Vec::new();
    for (cx, cy, edge) in regions(image.width(), image.height()) {
        anyhow::ensure!(!cancelled(), "Face analysis cancelled");
        let input = sample(image, cx, cy, edge, 0.0, 128, true)?;
        let output = models.infer_all("face_detector", &detector, 128, input)?;
        for d in decode(&output)? {
            let size = d.size * edge;
            if d.score >= 0.5 && size.is_finite() && size > 4.0 {
                candidates.push(Detection {
                    x: (d.x - 0.5) * edge + cx,
                    y: (d.y - 0.5) * edge + cy,
                    size,
                    ..d
                });
            }
        }
    }
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));
    let mut selected: Vec<Detection> = Vec::new();
    for detection in candidates {
        if selected.iter().all(|old| !same_face(old, &detection)) {
            selected.push(detection);
        }
        if selected.len() == MAX_CANDIDATES {
            break;
        }
    }
    let mut faces = Faces::default();
    let mut largest = 0.0;
    for face in selected {
        anyhow::ensure!(!cancelled(), "Face analysis cancelled");
        if faces.count == 10 {
            break;
        }
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
        // once centred and upright; real faces re-detect strongly. Only a box at
        // the centre and of about the expected size (half the crop) counts, so a
        // real face next to the candidate cannot vouch for it.
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
            decode(&output)?
                .into_iter()
                .filter(|d| {
                    (d.x - 0.5).abs() < 0.25
                        && (d.y - 0.5).abs() < 0.25
                        && (0.25..1.0).contains(&d.size)
                })
                .map(|d| d.score)
                .fold(0.0, f32::max)
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
        if sigmoid(presence[0]) < MIN_PRESENCE || redetect < MIN_REDETECT {
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
        // Face Mesh can settle two nearby candidates on the same face.
        if faces.geometry.iter().any(|kept| {
            let (intersection, _, smaller) = overlap(kept.bbox, bbox);
            intersection >= 0.5 * smaller
        }) {
            record(sigmoid(presence[0]), [f32::NAN; 2], [f32::NAN; 2], false);
            continue;
        }
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
        let kept = Face {
            bbox,
            eyes: [point(33), point(263)],
            landmarks: (0..landmarks.len() / 3).map(point).collect(),
            blink: Some(blink),
        };
        let left = eye_aspect_ratio(landmarks, [33, 160, 158, 133, 153, 144]);
        let right = eye_aspect_ratio(landmarks, [362, 385, 387, 263, 373, 380]);
        record(sigmoid(presence[0]), [left, right], blink, true);
        if eye_measures(&kept, image.width(), image.height()).is_some_and(|m| counted_closed(&m)) {
            faces.closed_eyes += 1;
        }
        faces.geometry.push(kept);
        faces.count += 1;
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
/// Face Mesh landmarks for `face` measured again on `asset`, a larger rendering
/// of the photo the face was found in. Landmarks found on a few analysis pixels
/// are too coarse to align a distant face for recognition. `None` when Face
/// Mesh no longer sees a face there.
pub fn refine(
    models: &mut super::models::Models,
    asset: &Asset,
    face: &Face,
) -> Result<Option<Face>> {
    let path = model_path("face_landmarks_detector.onnx").context("Face Mesh is not installed")?;
    let image = &*asset.pixels;
    let (width, height) = (image.width() as f32, image.height() as f32);
    let [left, top, right, bottom] = face.bbox;
    let [[ax, ay], [bx, by]] = face.eyes;
    let (x, y) = ((left + right) / 2.0 * width, (top + bottom) / 2.0 * height);
    let size = ((right - left) * width).max((bottom - top) * height);
    let angle = ((by - ay) * height).atan2((bx - ax) * width);
    anyhow::ensure!(
        [x, y, size, angle].iter().all(|v| v.is_finite()) && size > 1.0,
        "Invalid face geometry"
    );
    let input = sample(image, x, y, size * 1.5, angle, 256, false)?;
    let output = models.infer_all("face_landmarks", &path, 256, input)?;
    anyhow::ensure!(output.len() >= 2, "Face Mesh returned no landmarks");
    let (landmarks, presence) = (&output[0], &output[1]);
    anyhow::ensure!(
        landmarks.len() >= 468 * 3 && !presence.is_empty(),
        "Unexpected Face Mesh output"
    );
    // The face was already verified on the analysis image; only a clear miss
    // here drops it.
    if sigmoid(presence[0]) < 0.5 {
        return Ok(None);
    }
    let (sin, cos) = angle.sin_cos();
    let points: Vec<[f32; 2]> = (0..landmarks.len() / 3)
        .map(|i| {
            let u = (landmarks[i * 3] / 256.0 - 0.5) * size * 1.5;
            let v = (landmarks[i * 3 + 1] / 256.0 - 0.5) * size * 1.5;
            [
                (x + u * cos - v * sin) / width,
                (y + u * sin + v * cos) / height,
            ]
        })
        .collect();
    let bbox = points.iter().fold(
        [
            f32::INFINITY,
            f32::INFINITY,
            f32::NEG_INFINITY,
            f32::NEG_INFINITY,
        ],
        |b, [x, y]| [b[0].min(*x), b[1].min(*y), b[2].max(*x), b[3].max(*y)],
    );
    Ok(Some(Face {
        bbox,
        eyes: [points[33], points[263]],
        landmarks: points,
        blink: face.blink,
    }))
}
/// `face`, found on a small analysis image, measured again on `asset`, a
/// larger rendering of the same photo: Face Mesh landmarks and blink scores.
/// On the display image a distant face has about twice the pixels, enough to
/// judge eyes that were too small on the analysis image. `None` when Face
/// Mesh no longer sees a face there.
pub fn rejudge(
    models: &mut super::models::Models,
    asset: &Asset,
    face: &Face,
) -> Result<Option<Face>> {
    let Some(mut refined) = refine(models, asset, face)? else {
        return Ok(None);
    };
    anyhow::ensure!(
        refined.landmarks.len() >= 478,
        "Face Mesh returned no iris landmarks"
    );
    let path = model_path("face_blendshapes.onnx").context("Face blendshapes are not installed")?;
    let (width, height) = (asset.pixels.width() as f32, asset.pixels.height() as f32);
    let points = BLENDSHAPE_POINTS
        .iter()
        .flat_map(|&i| {
            let [x, y] = refined.landmarks[i];
            [x * width, y * height]
        })
        .collect();
    let shapes = models.infer_points("face_blendshapes", &path, points)?;
    anyhow::ensure!(
        shapes.first().is_some_and(|s| s.len() == 52),
        "Unexpected blendshape output"
    );
    refined.blink =
        Some([shapes[0][EYE_BLINK_LEFT], shapes[0][EYE_BLINK_RIGHT]].map(|v| v.clamp(0.0, 1.0)));
    Ok(Some(refined))
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
/// Whether one person's eyes are open, closed, or cannot be told.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EyeState {
    Open,
    Closed,
    /// Too small, turned away, behind glasses, squinting or laughing, or one
    /// eye open and one shut: never counted as closed.
    Unknown,
}
/// What the eye verdict is made of, from a face's stored landmarks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EyeMeasures {
    /// Pixels between the eye centres in the image the face was found in.
    pub spacing: f32,
    /// Eye aspect ratios (subject's left, right).
    pub ratios: [f32; 2],
    /// MediaPipe blendshape eye-blink scores (subject's left, right).
    pub blink: [f32; 2],
}
impl EyeMeasures {
    fn ratio(&self) -> f32 {
        (self.ratios[0] + self.ratios[1]) / 2.0
    }
}
const LEFT_EYE: [usize; 6] = [33, 160, 158, 133, 153, 144];
const RIGHT_EYE: [usize; 6] = [362, 385, 387, 263, 373, 380];
/// The eye measures of `face`, found in a `width` x `height` image (the
/// analysis image). Landmarks are normalized, so they are scaled back to
/// pixels first: the eye aspect ratio must not change with the photo's
/// shape. `None` for legacy records without landmarks or blink scores.
pub fn eye_measures(face: &Face, width: u32, height: u32) -> Option<EyeMeasures> {
    let blink = face.blink?;
    let (w, h) = (width as f32, height as f32);
    let point = |i: usize| face.landmarks.get(i).map(|[x, y]| [x * w, y * h]);
    let ratio = |eye: [usize; 6]| -> Option<f32> {
        let p: Vec<[f32; 2]> = eye.iter().map(|&i| point(i)).collect::<Option<_>>()?;
        let distance = |a: usize, b: usize| (p[a][0] - p[b][0]).hypot(p[a][1] - p[b][1]);
        Some((distance(1, 5) + distance(2, 4)) / (2.0 * distance(0, 3)).max(f32::EPSILON))
    };
    let centre = |a: usize, b: usize| -> Option<[f32; 2]> {
        let ([ax, ay], [bx, by]) = (point(a)?, point(b)?);
        Some([(ax + bx) / 2.0, (ay + by) / 2.0])
    };
    let ([ax, ay], [bx, by]) = (centre(33, 133)?, centre(362, 263)?);
    let measures = EyeMeasures {
        spacing: (ax - bx).hypot(ay - by),
        ratios: [ratio(LEFT_EYE)?, ratio(RIGHT_EYE)?],
        blink,
    };
    (measures.spacing.is_finite()
        && measures
            .ratios
            .iter()
            .chain(&measures.blink)
            .all(|v| v.is_finite()))
    .then_some(measures)
}
/// The rule `Faces::closed_eyes` counts with, which the quality score uses
/// and the face cache stores (see `SHUT_EYE_RATIO`).
fn counted_closed(m: &EyeMeasures) -> bool {
    let ratio = m.ratio();
    m.spacing >= MIN_EYE_PIXELS
        && (ratio < SHUT_EYE_RATIO
            || (m.blink.iter().all(|&b| b >= CLOSED_BLINK) && ratio < CLOSED_EYE_RATIO))
}
/// One person's eyes, for decisions about that person: Easy Delete's Trash
/// suggestion and Pick Best. Stricter than `counted_closed` both ways, so
/// a doubtful face is Unknown, never closed:
/// - Closed: both eyes shut by shape (each ratio under `CLOSED_EYE_RATIO`,
///   the mean under `SHUT_EYE_RATIO`) and by both blink scores (at least
///   `STRICT_BLINK`). A squint or a laugh narrows the eyes to a ratio of
///   about 0.12 to 0.2 and raises the blink scores: it stays Unknown. So
///   does a wink or one eye hidden. On a 79-photo phone library, the six
///   closed faces had ratios of 0.016 to 0.037 and blink scores of 0.72 to
///   0.85.
/// - Open: each ratio at least `OPEN_EYE_RATIO` and neither blink score
///   above `OPEN_BLINK`. Open faces there had ratios of 0.2 to 0.43 and
///   blink scores up to 0.3.
/// - Unknown: everything else, and faces under `MIN_EYE_PIXELS`.
///
/// Every face it calls Closed is counted by `counted_closed` too, so the
/// cached count can tell which photos are worth a closer look.
pub fn eye_state(face: &Face, width: u32, height: u32) -> EyeState {
    eye_measures(face, width, height).map_or(EyeState::Unknown, |m| judge(&m))
}
/// Whether `face`'s eyes are too small to judge on this `width` x `height`
/// image but would be on one twice as large, such as the display image.
pub fn rejudgeable(face: &Face, width: u32, height: u32) -> bool {
    eye_measures(face, width, height)
        .is_some_and(|m| (MIN_EYE_PIXELS / 2.0..MIN_EYE_PIXELS).contains(&m.spacing))
}
/// The `eye_state` verdict on measures alone.
pub fn judge(m: &EyeMeasures) -> EyeState {
    if m.spacing < MIN_EYE_PIXELS {
        EyeState::Unknown
    } else if m.ratio() < SHUT_EYE_RATIO
        && m.ratios.iter().all(|&r| r < CLOSED_EYE_RATIO)
        && m.blink.iter().all(|&b| b >= STRICT_BLINK)
    {
        EyeState::Closed
    } else if m.ratios.iter().all(|&r| r >= OPEN_EYE_RATIO)
        && m.blink.iter().all(|&b| b <= OPEN_BLINK)
    {
        EyeState::Open
    } else {
        EyeState::Unknown
    }
}
/// The strip around both eyes of `face`, found in a `width` x `height`
/// image, in normalized `[left, top, right, bottom]`: wide enough to read the
/// eyelids, about three times as wide as it is tall, inside the photo.
pub fn eye_rect(face: &Face, width: u32, height: u32) -> Option<[f32; 4]> {
    let (w, h) = (width as f32, height as f32);
    let points: Vec<[f32; 2]> = LEFT_EYE
        .iter()
        .chain(&RIGHT_EYE)
        .map(|&i| face.landmarks.get(i).map(|[x, y]| [x * w, y * h]))
        .collect::<Option<_>>()?;
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for [x, y] in &points {
        (x0, y0, x1, y1) = (x0.min(*x), y0.min(*y), x1.max(*x), y1.max(*y));
    }
    let span = (x1 - x0).max(y1 - y0);
    if !(span.is_finite() && span > 1.0 && w > 0.0 && h > 0.0) {
        return None;
    }
    let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
    let half_w = span * 0.7;
    let half_h = (half_w / 3.0).max((y1 - y0) * 0.9);
    Some([
        ((cx - half_w) / w).clamp(0.0, 1.0),
        ((cy - half_h) / h).clamp(0.0, 1.0),
        ((cx + half_w) / w).clamp(0.0, 1.0),
        ((cy + half_h) / h).clamp(0.0, 1.0),
    ])
}
/// Square detector windows (centre x, centre y, edge) in source pixels: the whole
/// frame, then overlapping tiles that shrink by `TILE_STEP` down to
/// `MIN_TILE`. The 128-pixel detector only finds faces that fill a fair part of
/// its input, so faces of people standing a few metres away need small tiles.
fn regions(width: u32, height: u32) -> Vec<(f32, f32, f32)> {
    let (width, height) = (width as f32, height as f32);
    let long = width.max(height);
    let mut regions = vec![(width / 2.0, height / 2.0, long)];
    let mut tile = long * TILE_STEP;
    while tile >= MIN_TILE {
        // Half-tile overlap keeps every face whole in at least one tile.
        let axis = |length: f32| {
            let count = ((length - tile).max(0.0) / (tile / 2.0)).ceil() as usize + 1;
            let stride = (length - tile).max(0.0) / (count - 1).max(1) as f32;
            (0..count).map(move |i| {
                if count == 1 {
                    length / 2.0
                } else {
                    tile / 2.0 + stride * i as f32
                }
            })
        };
        for cy in axis(height) {
            for cx in axis(width) {
                regions.push((cx, cy, tile));
            }
        }
        tile *= TILE_STEP;
    }
    regions
}
fn sigmoid(value: f32) -> f32 {
    1.0 / (1.0 + (-value.clamp(-80.0, 80.0)).exp())
}
/// Every BlazeFace anchor, with position and size as fractions of the input.
fn decode(output: &[Vec<f32>]) -> Result<Vec<Detection>> {
    anyhow::ensure!(output.len() >= 2, "Face detector returned no boxes");
    let (boxes, scores) = (&output[0], &output[1]);
    anyhow::ensure!(
        boxes.len() == 896 * 16 && scores.len() == 896,
        "Unexpected BlazeFace output"
    );
    let mut detections = Vec::with_capacity(896);
    for (grid, repetitions) in [(16, 2), (8, 6)] {
        for y in 0..grid {
            for x in 0..grid {
                for _ in 0..repetitions {
                    let index = detections.len();
                    let b = &boxes[index * 16..index * 16 + 16];
                    detections.push(Detection {
                        score: sigmoid(scores[index]),
                        x: b[0] / 128.0 + (x as f32 + 0.5) / grid as f32,
                        y: b[1] / 128.0 + (y as f32 + 0.5) / grid as f32,
                        size: b[2].max(b[3]) / 128.0,
                        angle: (b[7] - b[5]).atan2(b[6] - b[4]),
                    });
                }
            }
        }
    }
    Ok(detections)
}
/// Tiles of different sizes report one face at several scales, so a box mostly
/// inside another counts as the same face, not only boxes of similar size.
fn same_face(a: &Detection, b: &Detection) -> bool {
    let square = |d: &Detection| {
        let half = d.size / 2.0;
        [d.x - half, d.y - half, d.x + half, d.y + half]
    };
    let (intersection, union, smaller) = overlap(square(a), square(b));
    intersection >= 0.3 * union || intersection >= 0.5 * smaller
}
/// Intersection, union and smaller area of two `[left, top, right, bottom]` boxes.
fn overlap(a: [f32; 4], b: [f32; 4]) -> (f32, f32, f32) {
    let area = |r: [f32; 4]| (r[2] - r[0]).max(0.0) * (r[3] - r[1]).max(0.0);
    let width = a[2].min(b[2]) - a[0].max(b[0]);
    let height = a[3].min(b[3]) - a[1].max(b[1]);
    let intersection = width.max(0.0) * height.max(0.0);
    (
        intersection,
        (area(a) + area(b) - intersection).max(f32::EPSILON),
        area(a).min(area(b)).max(f32::EPSILON),
    )
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
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn every_face_from_28_pixels_fills_an_eighth_of_some_detector_window() {
        // Phone photos at analysis size, a small screenshot and the SFace group.
        for (width, height) in [(577, 1024), (1024, 768), (1024, 1024), (512, 267)] {
            let windows = regions(width, height);
            let (w, h) = (width as f32, height as f32);
            for size in [28.0, 40.0, 64.0, 120.0, 200.0, 260.0] {
                if size > w.min(h) {
                    continue;
                }
                let mut y = 0.0;
                while y + size <= h {
                    let mut x = 0.0;
                    while x + size <= w {
                        assert!(
                            windows.iter().any(|&(cx, cy, edge)| {
                                let half = edge / 2.0;
                                cx - half <= x
                                    && cy - half <= y
                                    && x + size <= cx + half
                                    && y + size <= cy + half
                                    && size >= edge / 8.0
                            }),
                            "{size}px face at {x},{y} in {width}x{height}"
                        );
                        x += 7.0;
                    }
                    y += 7.0;
                }
            }
        }
    }
    #[test]
    fn one_face_seen_at_several_scales_is_one_candidate_but_neighbours_are_two() {
        let detection = |x, y, size| Detection {
            score: 0.9,
            x,
            y,
            size,
            angle: 0.0,
        };
        let face = detection(100.0, 100.0, 40.0);
        // The same face from a coarser tile, in a box three times as large.
        assert!(same_face(&detection(110.0, 90.0, 120.0), &face));
        assert!(same_face(&detection(102.0, 101.0, 44.0), &face));
        // Two people cheek to cheek.
        assert!(!same_face(&detection(136.0, 104.0, 40.0), &face));
        assert!(!same_face(&detection(300.0, 100.0, 40.0), &face));
    }
    #[test]
    fn a_new_face_recipe_retires_subject_records_that_embed_faces() {
        assert!(super::super::models::analysis_signature().starts_with(RECIPE));
        assert!(super::super::models::face_signature().starts_with(RECIPE));
    }
}
