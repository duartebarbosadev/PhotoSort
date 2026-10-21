//! People grouping from verified landmarks and native SFace recognition.
//! Detection and decoding belong to the shared image owner; grouping consumes
//! immutable cached evidence only and never allocates a pairwise distance matrix.
use super::{
    faces::{Face, Faces},
    models::{self, Models},
    pipeline::Asset,
    regional,
    subjects::store::Store,
};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const MODEL: &str = "face_recognition_sface.onnx";
/// Deliberately stricter than the upstream pair-verification threshold. This is
/// a conservative initial grouping policy, not a calibrated probability.
pub const MIN_SIMILARITY: f32 = 0.5;
const SIZE: usize = 112;
const TEMPLATE: [[f64; 2]; 5] = [
    [38.2946, 51.6963],
    [73.5318, 51.5014],
    [56.0252, 71.7366],
    [41.5493, 92.3655],
    [70.7299, 92.2041],
];
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Recognition {
    /// One normalized descriptor per sufficiently large main face. Empty means
    /// no reliable complete set; never silently omit an unclear main person.
    pub vectors: Vec<Vec<f32>>,
}
impl Recognition {
    pub fn valid(&self) -> bool {
        self.vectors.len() <= 10
            && self.vectors.iter().all(|v| {
                v.len() == 128
                    && v.iter().all(|x| x.is_finite())
                    && (v.iter().map(|x| x * x).sum::<f32>() - 1.).abs() < 0.001
            })
    }
}
/// Retain the established main-subject area threshold, but recognize all main
/// people instead of discarding the whole group portrait. Small background faces
/// do not change the destination of a foreground portrait.
fn prominent(faces: &Faces) -> Vec<&Face> {
    let mut candidates: Vec<_> = faces
        .geometry
        .iter()
        .filter_map(|f| {
            let [l, t, r, b] = f.bbox;
            (f.bbox.iter().all(|x| x.is_finite()) && r > l && b > t)
                .then_some((f, (r - l) * (b - t)))
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.1.total_cmp(&a.1)
            .then_with(|| a.0.bbox[0].total_cmp(&b.0.bbox[0]))
    });
    let minimum = candidates
        .first()
        .map_or(f32::INFINITY, |(_, area)| area * 0.6);
    candidates
        .into_iter()
        .take_while(|(_, area)| *area >= minimum)
        .map(|(face, _)| face)
        .collect()
}
/// Least-squares orientation-preserving similarity transform, from five source
/// landmarks to the SFace template. Returns inverse coefficients for sampling.
fn transform(points: [[f64; 2]; 5]) -> Option<[f64; 4]> {
    if !points.iter().flatten().all(|x| x.is_finite()) {
        return None;
    }
    let mean = |p: &[[f64; 2]; 5]| {
        [
            p.iter().map(|v| v[0]).sum::<f64>() / 5.,
            p.iter().map(|v| v[1]).sum::<f64>() / 5.,
        ]
    };
    let s = mean(&points);
    let d = mean(&TEMPLATE);
    let mut den = 0.;
    let mut a = 0.;
    let mut b = 0.;
    for (p, q) in points.iter().zip(TEMPLATE) {
        let x = p[0] - s[0];
        let y = p[1] - s[1];
        let u = q[0] - d[0];
        let v = q[1] - d[1];
        den += x * x + y * y;
        a += x * u + y * v;
        b += x * v - y * u;
    }
    if den < 1e-8 {
        return None;
    }
    a /= den;
    b /= den;
    let tx = d[0] - a * s[0] + b * s[1];
    let ty = d[1] - b * s[0] - a * s[1];
    let error = points
        .iter()
        .zip(TEMPLATE)
        .map(|(p, q)| {
            (a * p[0] - b * p[1] + tx - q[0]).powi(2) + (b * p[0] + a * p[1] + ty - q[1]).powi(2)
        })
        .sum::<f64>()
        / 5.;
    let scale = a * a + b * b;
    if scale < 1e-10 || !scale.is_finite() || error > 64. {
        return None;
    }
    Some([a / scale, b / scale, tx, ty])
}
/// RGB float 0..255, NCHW. The model contains its own input normalization.
/// Bilinear sampling uses a black border, with no full-frame intermediate copy.
fn aligned_input(
    asset: &Asset,
    face: &Face,
    cancelled: &impl Fn() -> bool,
) -> Result<Option<Vec<f32>>> {
    let lm = &face.landmarks;
    if lm.len() < 363 {
        return Ok(None);
    }
    let midpoint = |a: usize, b: usize| [(lm[a][0] + lm[b][0]) / 2., (lm[a][1] + lm[b][1]) / 2.];
    let points = [
        midpoint(33, 133),
        midpoint(362, 263),
        lm[1],
        lm[61],
        lm[291],
    ];
    if !points
        .iter()
        .flatten()
        .all(|x| x.is_finite() && (0.0..=1.0).contains(x))
    {
        return Ok(None);
    }
    let image = &*asset.pixels;
    let points = points.map(|p| {
        [
            f64::from(p[0]) * f64::from(image.width()),
            f64::from(p[1]) * f64::from(image.height()),
        ]
    });
    // Insufficient eye separation means too few source pixels for recognition.
    if (points[0][0] - points[1][0]).hypot(points[0][1] - points[1][1]) < 12. {
        return Ok(None);
    }
    let Some([a, b, tx, ty]) = transform(points) else {
        return Ok(None);
    };
    let mut input = vec![0.; 3 * SIZE * SIZE];
    for y in 0..SIZE {
        anyhow::ensure!(!cancelled(), "Face recognition cancelled");
        for x in 0..SIZE {
            let u = x as f64 - tx;
            let v = y as f64 - ty;
            let sx = a * u + b * v;
            let sy = -b * u + a * v;
            let left = sx.floor() as i64;
            let top = sy.floor() as i64;
            let fx = (sx - sx.floor()) as f32;
            let fy = (sy - sy.floor()) as f32;
            for (px, py, w) in [
                (left, top, (1. - fx) * (1. - fy)),
                (left + 1, top, fx * (1. - fy)),
                (left, top + 1, (1. - fx) * fy),
                (left + 1, top + 1, fx * fy),
            ] {
                if px >= 0
                    && py >= 0
                    && px < i64::from(image.width())
                    && py < i64::from(image.height())
                {
                    let p = image.get_pixel(px as u32, py as u32);
                    for c in 0..3 {
                        input[c * SIZE * SIZE + y * SIZE + x] += f32::from(p[c]) * w;
                    }
                }
            }
        }
    }
    Ok(Some(input))
}
pub fn describe(
    models: &mut Models,
    asset: &Asset,
    faces: &Faces,
    cancelled: impl Fn() -> bool,
) -> Result<Recognition> {
    anyhow::ensure!(!cancelled(), "Face recognition cancelled");
    let people = prominent(faces);
    if people.is_empty() || people.len() > 10 {
        return Ok(Recognition::default());
    }
    // Align the complete set before inference: one unusable main face makes the
    // photo unassigned and avoids spending model work on a partial destination.
    let mut inputs = Vec::with_capacity(people.len());
    for face in people {
        let Some(input) = aligned_input(asset, face, &cancelled)? else {
            return Ok(Recognition::default());
        };
        inputs.push(input);
    }
    let path = models::model_path(MODEL).context("SFace recognition model is not installed")?;
    let mut vectors = Vec::with_capacity(inputs.len());
    for input in inputs {
        anyhow::ensure!(!cancelled(), "Face recognition cancelled");
        let mut outputs = models.infer_all("sface", &path, SIZE, input)?;
        anyhow::ensure!(!cancelled(), "Face recognition cancelled");
        anyhow::ensure!(
            outputs.len() == 1 && outputs[0].len() == 128,
            "SFace must return one 128-dimensional descriptor"
        );
        let mut vector = outputs.remove(0);
        let norm = vector
            .iter()
            .map(|x| f64::from(*x).powi(2))
            .sum::<f64>()
            .sqrt();
        anyhow::ensure!(
            norm.is_finite() && norm > 1e-8,
            "SFace returned an invalid descriptor"
        );
        for x in &mut vector {
            *x = (f64::from(*x) / norm) as f32;
        }
        vectors.push(vector);
    }
    Ok(Recognition { vectors })
}

fn similarity(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}
/// Deterministic, conservative all-member admission. Unlike subject grouping,
/// this has no geometric pair graph: a compact 128-float descriptor per unique
/// face is cheaper than storing all candidate edges. Exact descriptors collapse
/// immediately. Memory is O(n); diverse-library worst-case CPU remains O(n²).
pub fn group(
    records: &[regional::grouping::Record],
    store: &Store,
    cancelled: impl Fn() -> bool,
) -> Result<Option<regional::grouping::Output>> {
    // Gather exact descriptors once, retaining only compact indices per photo.
    // Photo IDs include paths, so neither identity admission nor numbering may
    // depend on those IDs: a verified rename must not reshuffle people folders.
    let mut vectors = Vec::<Vec<f32>>::new();
    let mut exact = HashMap::new();
    let mut faces_by_photo = Vec::with_capacity(records.len());
    for record in records {
        if cancelled() {
            return Ok(None);
        }
        let Some(key) = &record.key else {
            faces_by_photo.push((record.id, Vec::new()));
            continue;
        };
        let recognition = store
            .recognition(key)
            .context("Face recognition evidence is missing; retry analysis")?;
        let mut indices = Vec::with_capacity(recognition.vectors.len());
        for vector in recognition.vectors {
            if cancelled() {
                return Ok(None);
            }
            let mut hash = blake3::Hasher::new();
            for x in &vector {
                hash.update(&x.to_le_bytes());
            }
            let index = *exact.entry(hash.finalize()).or_insert_with(|| {
                let index = vectors.len();
                vectors.push(vector);
                index
            });
            indices.push(index);
        }
        faces_by_photo.push((record.id, indices));
    }
    drop(exact);
    let mut order: Vec<_> = (0..vectors.len()).collect();
    order.sort_unstable_by(|&a, &b| {
        vectors[b]
            .iter()
            .zip(&vectors[a])
            .map(|(a, b)| a.total_cmp(b))
            .find(|c| !c.is_eq())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut groups = Vec::<Vec<usize>>::new();
    let mut identity = vec![0; vectors.len()];
    for index in order {
        if cancelled() {
            return Ok(None);
        }
        let vector = &vectors[index];
        let mut best = None;
        let mut best_score = f32::NEG_INFINITY;
        for (g, members) in groups.iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            let mut minimum = 1.;
            for &i in members {
                if cancelled() {
                    return Ok(None);
                }
                minimum = similarity(vector, &vectors[i]).min(minimum);
                if minimum < MIN_SIMILARITY || minimum <= best_score {
                    break;
                }
            }
            if minimum >= MIN_SIMILARITY && minimum > best_score {
                best = Some(g);
                best_score = minimum;
            }
        }
        let group = if let Some(g) = best {
            groups[g].push(index);
            g
        } else {
            groups.push(vec![index]);
            groups.len() - 1
        };
        identity[index] = group + 1;
    }
    let individual_groups = groups.len();
    drop(groups);
    drop(vectors);
    let mut people_by_photo = Vec::with_capacity(records.len());
    let mut combinations = BTreeMap::new();
    for (id, mut people) in faces_by_photo {
        if cancelled() {
            return Ok(None);
        }
        for index in &mut people {
            *index = identity[*index];
        }
        people.sort_unstable();
        // A repeated identity within one frame is ambiguous (lookalikes, collage,
        // or recognition error). Never silently reduce it to a single person.
        if people.windows(2).any(|pair| pair[0] == pair[1]) {
            people.clear();
        }
        if people.len() > 1 && !combinations.contains_key(&people) {
            combinations.insert(people.clone(), 0);
        }
        people_by_photo.push((id, people));
    }
    for (i, group) in combinations.values_mut().enumerate() {
        *group = individual_groups + i + 1;
    }

    let mut output = regional::grouping::Output {
        groups: HashMap::with_capacity(records.len()),
        folders: HashMap::with_capacity(records.len()),
    };
    for (id, people) in people_by_photo {
        if cancelled() {
            return Ok(None);
        }
        let (group, label) = match people.as_slice() {
            [] => (0, "Unassigned".into()),
            [person] => (*person, format!("Person {person:03}")),
            _ => {
                let label = format!(
                    "People {}",
                    people
                        .iter()
                        .map(|person| format!("{person:03}"))
                        .collect::<Vec<_>>()
                        .join(" + ")
                );
                let group = combinations[&people];
                (group, label)
            }
        };
        output.groups.insert(id, group);
        output.folders.insert(id, label);
    }
    Ok((!cancelled()).then_some(output))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn alignment_recovers_rotation_scale_translation_and_rejects_bad_geometry() {
        let points = TEMPLATE.map(|[x, y]| [2. * y + 80., -2. * x + 300.]);
        let [a, b, tx, ty] = transform(points).unwrap();
        for (source, [u, v]) in points.into_iter().zip(TEMPLATE) {
            assert!((a * (u - tx) + b * (v - ty) - source[0]).abs() < 1e-6);
            assert!((-b * (u - tx) + a * (v - ty) - source[1]).abs() < 1e-6);
        }
        assert!(transform([[0.; 2]; 5]).is_none());
        let mut bad = TEMPLATE;
        bad[2][0] = f64::NAN;
        assert!(transform(bad).is_none());
        let mut bad = TEMPLATE;
        bad[2][0] += 60.;
        assert!(transform(bad).is_none());
    }
    #[test]
    fn prominent_faces_include_main_people_but_exclude_small_background_faces() {
        let face = |bbox| Face {
            bbox,
            eyes: [[0.; 2]; 2],
            landmarks: Vec::new(),
            blink: None,
        };
        let faces = Faces {
            count: 4,
            geometry: vec![
                face([0., 0., 1., 1.]),
                face([1., 0., 1.8, 1.]),
                face([0., 0., 0.1, 0.1]),
                face([f32::NAN, 0., 1., 1.]),
            ],
            ..Default::default()
        };
        assert_eq!(prominent(&faces).len(), 2);
    }
    #[test]
    fn sampling_preserves_rgb_channels_and_model_scale() {
        let mut landmarks = vec![[0.; 2]; 468];
        for (indexes, p) in [
            (&[33, 133][..], TEMPLATE[0]),
            (&[362, 263][..], TEMPLATE[1]),
            (&[1][..], TEMPLATE[2]),
            (&[61][..], TEMPLATE[3]),
            (&[291][..], TEMPLATE[4]),
        ] {
            for &i in indexes {
                landmarks[i] = p.map(|x| x as f32 / 112.);
            }
        }
        let face = Face {
            bbox: [0., 0., 1., 1.],
            eyes: [[0.; 2]; 2],
            landmarks,
            blink: None,
        };
        let pixels = std::sync::Arc::new(image::RgbaImage::from_pixel(
            112,
            112,
            image::Rgba([23, 117, 231, 255]),
        ));
        let asset = Asset {
            decoder_warning: None,
            pixels: pixels.clone(),
            thumbnail: pixels,
            metadata: Default::default(),
        };
        let input = aligned_input(&asset, &face, &|| false).unwrap().unwrap();
        for (c, expected) in [23., 117., 231.].into_iter().enumerate() {
            assert!((input[c * 112 * 112 + 56 * 112 + 56] - expected).abs() < 0.001);
        }
        assert!(aligned_input(&asset, &face, &|| true).is_err());
        let mut models = Models::default();
        let empty = describe(&mut models, &asset, &Faces::default(), || false).unwrap();
        assert!(empty.vectors.is_empty());
        let group = Faces {
            count: 2,
            geometry: vec![face.clone(), face],
            ..Default::default()
        };
        assert_eq!(prominent(&group).len(), 2);
        let mut unclear = group.clone();
        unclear.geometry[1].landmarks.clear();
        assert!(
            describe(&mut models, &asset, &unclear, || false)
                .unwrap()
                .vectors
                .is_empty()
        );
        assert_eq!(models.inferences, 0);
    }
}
