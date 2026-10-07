//! Face motion after camera alignment, before the separate face-crop SSIM gate.
use super::Alignment;
use crate::core::faces::Face;
use nalgebra::{Matrix2, Vector2};

#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub struct FaceMotion {
    pub changed: bool,
    pub min_iou: Option<f64>,
    pub max_median: Option<f64>,
    pub max_p90: Option<f64>,
}
struct Geometry {
    bbox: [f64; 4],
    points: Vec<Vector2<f64>>,
}
impl Geometry {
    fn from_face(face: &Face) -> Option<Self> {
        if face.landmarks.is_empty()
            || face.landmarks.iter().flatten().any(|v| !v.is_finite())
            || face.bbox.iter().any(|v| !v.is_finite())
        {
            return None;
        }
        Some(Self {
            bbox: face.bbox.map(f64::from),
            points: face
                .landmarks
                .iter()
                .map(|p| Vector2::new(f64::from(p[0]), f64::from(p[1])))
                .collect(),
        })
    }
    fn transformed(
        mut self,
        matrix: Matrix2<f64>,
        translation: Vector2<f64>,
        width: usize,
        height: usize,
    ) -> Self {
        self.bbox = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for point in &mut self.points {
            *point = matrix * Vector2::new(point.x * width as f64, point.y * height as f64)
                + translation;
            point.x /= width as f64;
            point.y /= height as f64;
            self.bbox[0] = self.bbox[0].min(point.x);
            self.bbox[1] = self.bbox[1].min(point.y);
            self.bbox[2] = self.bbox[2].max(point.x);
            self.bbox[3] = self.bbox[3].max(point.y);
        }
        self
    }
}
fn iou(a: [f64; 4], b: [f64; 4]) -> f64 {
    let intersection =
        (a[2].min(b[2]) - a[0].max(b[0])).max(0.0) * (a[3].min(b[3]) - a[1].max(b[1])).max(0.0);
    let area = |v: [f64; 4]| (v[2] - v[0]).max(0.0) * (v[3] - v[1]).max(0.0);
    intersection / (area(a) + area(b) - intersection).max(1e-12)
}
fn displacement(first: &Geometry, second: &Geometry) -> Option<(f64, f64)> {
    if first.points.len() != second.points.len() || first.points.is_empty() {
        return None;
    }
    let normalized = |g: &Geometry| -> Option<Vec<Vector2<f64>>> {
        let scale = (g.bbox[2] - g.bbox[0]).hypot(g.bbox[3] - g.bbox[1]);
        if scale <= 1e-8 {
            return None;
        }
        let origin = Vector2::new(g.bbox[0], g.bbox[1]);
        let mut pts: Vec<_> = g.points.iter().map(|v| (*v - origin) / scale).collect();
        let center = pts.iter().copied().sum::<Vector2<f64>>() / pts.len() as f64;
        for v in &mut pts {
            *v -= center;
        }
        Some(pts)
    };
    let a = normalized(first)?;
    let b = normalized(second)?;
    let covariance = a
        .iter()
        .zip(&b)
        .fold(Matrix2::zeros(), |sum, (a, b)| sum + b * a.transpose());
    let svd = covariance.svd(true, true);
    let rotation = svd.u? * svd.v_t?;
    let mut distances: Vec<_> = a
        .iter()
        .zip(&b)
        .map(|(a, b)| (a - rotation.transpose() * b).norm())
        .collect();
    if distances.iter().any(|v| !v.is_finite()) {
        return None;
    }
    distances.sort_unstable_by(f64::total_cmp);
    let percentile = |q: f64| {
        let p = (distances.len() - 1) as f64 * q;
        let lo = p.floor() as usize;
        let hi = p.ceil() as usize;
        distances[lo] + (distances[hi] - distances[lo]) * (p - lo as f64)
    };
    Some((percentile(0.5), percentile(0.9)))
}

/// Match faces by reference greedy overlap (leftmost first; higher source index
/// breaks equal overlap), then remove uniform translation/scale/roll from landmarks.
/// Missing or incompatible evidence is unknown, never a stable-face conclusion.
/// A stable motion result still requires aligned crop SSIM before acceptance.
pub fn face_motion(
    first: &[Face],
    second: &[Face],
    alignment: &Alignment,
    width: usize,
    height: usize,
    cancelled: impl Fn() -> bool,
) -> Option<FaceMotion> {
    if cancelled() || width == 0 || height == 0 {
        return None;
    }
    Alignment::validate(alignment.transform, alignment.correlation, width, height)?;
    let mut result = FaceMotion {
        changed: false,
        min_iou: None,
        max_median: None,
        max_p90: None,
    };
    if first.len() != second.len() {
        result.changed = true;
        return Some(result);
    }
    if first.is_empty() {
        return Some(result);
    }
    let [[a, b, x], [c, d, y]] = alignment.transform;
    let inverse = Matrix2::new(a, b, c, d).try_inverse()?;
    let translation = -(inverse * Vector2::new(x, y));
    let mut first: Vec<_> = first
        .iter()
        .map(Geometry::from_face)
        .collect::<Option<_>>()?;
    let second: Vec<_> = second
        .iter()
        .map(|f| Some(Geometry::from_face(f)?.transformed(inverse, translation, width, height)))
        .collect::<Option<_>>()?;
    first.sort_by(|a, b| a.bbox[0].total_cmp(&b.bbox[0]));
    let mut remaining = vec![true; second.len()];
    let mut matches = Vec::new();
    let mut minimum = 1.0_f64;
    for face in &first {
        if cancelled() {
            return None;
        }
        let (index, overlap) = second
            .iter()
            .enumerate()
            .filter(|(i, _)| remaining[*i])
            .map(|(i, other)| (i, iou(face.bbox, other.bbox)))
            .max_by(|(i, a), (j, b)| a.total_cmp(b).then(i.cmp(j)))?;
        remaining[index] = false;
        minimum = minimum.min(overlap);
        matches.push((face, &second[index]));
    }
    result.min_iou = Some(minimum);
    if minimum < 0.60 {
        result.changed = true;
        return Some(result);
    }
    let (mut median, mut p90) = (0.0_f64, 0.0_f64);
    for (a, b) in matches {
        if cancelled() {
            return None;
        }
        let (m, p) = displacement(a, b)?;
        median = median.max(m);
        p90 = p90.max(p);
    }
    result.max_median = Some(median);
    result.max_p90 = Some(p90);
    result.changed = median > 0.018 || p90 > 0.040;
    Some(result)
}
