use super::{Artifact, Evidence, Geometry, cosine};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Strictness {
    Conservative,
    #[default]
    Standard,
    Broad,
}
#[derive(Clone, Copy)]
struct Policy {
    subject: f64,
    scene: f64,
    exceptional: f64,
    confidence: f64,
    geometry_subject: f64,
    geometry_scene: f64,
}
impl Strictness {
    fn policy(self) -> Policy {
        let (subject, scene, exceptional, confidence, geometry_subject, geometry_scene) = match self
        {
            Self::Conservative => (0.88, 0.82, 0.95, 0.88, 0.68, 0.86),
            Self::Standard => (0.84, 0.78, 0.92, 0.84, 0.64, 0.82),
            Self::Broad => (0.80, 0.74, 0.89, 0.80, 0.60, 0.78),
        };
        Policy {
            subject,
            scene,
            exceptional,
            confidence,
            geometry_subject,
            geometry_scene,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Pair {
    pub subject_similarity: f64,
    pub face_similarity: Option<f64>,
    pub scene_similarity: f64,
    pub complete: bool,
    pub geometry: Geometry,
    pub time_delta_seconds: Option<f64>,
    pub confidence: f64,
    pub accepted: bool,
    pub reason: String,
}
impl Pair {
    /// Reclassify immutable raw evidence without any decoder, model or geometry work.
    pub fn with_strictness(&self, strictness: Strictness) -> Self {
        let mut result = self.clone();
        let p = strictness.policy();
        let face_safe = self.face_similarity.is_none_or(|v| v >= p.subject);
        let exceptional = self.subject_similarity >= p.exceptional
            || self.face_similarity.is_some_and(|v| v >= p.exceptional);
        let semantic = self.subject_similarity >= p.subject
            && (self.geometry.strong() || exceptional)
            && self.confidence >= p.confidence;
        let geometry = self.geometry.strong()
            && self.subject_similarity >= p.geometry_subject
            && self.scene_similarity >= p.geometry_scene;
        result.accepted = self.complete
            && face_safe
            && self.scene_similarity >= p.scene
            && (semantic || geometry);
        result.reason = if result.accepted {
            "same_subject"
        } else if !self.complete {
            "subject_set_changed"
        } else if !face_safe {
            "face_identity_uncertain"
        } else if self.scene_similarity < p.scene {
            "scene_changed"
        } else if self.subject_similarity < p.geometry_subject {
            "subject_identity_uncertain"
        } else if !self.geometry.evaluated || !self.geometry.strong() {
            "geometry_uncertain"
        } else if self.subject_similarity < p.subject {
            "subject_identity_uncertain"
        } else {
            "confidence_below_policy"
        }
        .into();
        result
    }
    pub fn needs_geometry(&self, strictness: Strictness) -> bool {
        if self.accepted || self.geometry.evaluated || !self.complete {
            return false;
        }
        let p = strictness.policy();
        self.face_similarity.is_none_or(|v| v >= p.subject)
            && ((self.subject_similarity >= p.subject && self.scene_similarity >= p.scene)
                || (self.subject_similarity >= p.geometry_subject
                    && self.scene_similarity >= p.geometry_scene))
    }
}

pub fn verify_pair(
    a: &Artifact,
    b: &Artifact,
    geometry: Geometry,
    time_a: Option<i64>,
    time_b: Option<i64>,
    strictness: Strictness,
) -> Pair {
    let (complete_subjects, subject_similarity) = match_sets(&a.subjects, &b.subjects);
    let (complete_faces, face_similarity) = match_sets(&a.faces, &b.faces);
    let scene_similarity = cosine(&a.global, &b.global).unwrap_or(0.0);
    let time_delta_seconds = time_a
        .zip(time_b)
        .map(|(a, b)| a.abs_diff(b) as f64 / 1000.0);
    let time_boost = time_delta_seconds
        .filter(|t| *t <= 600.0)
        .map_or(0.0, |t| 0.03 * (1.0 - t / 600.0));
    let geometry_score =
        (geometry.inlier_ratio * 1.5 + geometry.coverage_a.min(geometry.coverage_b)).min(1.0);
    Pair {
        subject_similarity,
        face_similarity: (!a.faces.is_empty() || !b.faces.is_empty()).then_some(face_similarity),
        scene_similarity,
        complete: complete_subjects && complete_faces,
        geometry,
        time_delta_seconds,
        confidence: (0.68 * subject_similarity
            + 0.22 * scene_similarity
            + 0.10 * geometry_score
            + time_boost)
            .min(1.0),
        accepted: false,
        reason: String::new(),
    }
    .with_strictness(strictness)
}

fn match_sets(a: &[Evidence], b: &[Evidence]) -> (bool, f64) {
    if a.len() != b.len() {
        return (false, 0.0);
    }
    if a.is_empty() {
        return (true, 1.0);
    }
    if a.iter().chain(b).any(|e| {
        e.descriptor.is_empty()
            || !e.area.is_finite()
            || e.area < 0.0
            || e.bbox.iter().chain(&e.descriptor).any(|v| !v.is_finite())
    }) {
        return (false, 0.0);
    }
    let n = a.len();
    let mut scores = vec![vec![0.0; n]; n];
    for (i, a) in a.iter().enumerate() {
        for (j, b) in b.iter().enumerate() {
            if a.kind != b.kind {
                continue;
            }
            let Some(similarity) = cosine(&a.descriptor, &b.descriptor) else {
                continue;
            };
            let aspect = |e: &Evidence| {
                (f64::from(e.bbox[2]) - f64::from(e.bbox[0])).max(1e-6)
                    / (f64::from(e.bbox[3]) - f64::from(e.bbox[1])).max(1e-6)
            };
            let (aa, ba) = (aspect(a), aspect(b));
            let area = (a.area.min(b.area) as f64) / (a.area.max(b.area) as f64).max(1e-6);
            scores[i][j] = (0.90 * similarity.max(0.0)
                + 0.05 * area
                + 0.05 * aa.min(ba) / aa.max(ba)) as f32 as f64;
        }
    }
    // Hungarian assignment maximizes total similarity, then returns the weakest
    // chosen match, exactly as the reference's linear_sum_assignment path does.
    let (mut u, mut v, mut p, mut way) = (
        vec![0.0; n + 1],
        vec![0.0; n + 1],
        vec![0; n + 1],
        vec![0; n + 1],
    );
    for i in 1..=n {
        p[0] = i;
        let mut j0 = 0;
        let mut min = vec![f64::INFINITY; n + 1];
        let mut used = vec![false; n + 1];
        loop {
            used[j0] = true;
            let i0 = p[j0];
            let (mut delta, mut j1) = (f64::INFINITY, 0);
            for j in 1..=n {
                if !used[j] {
                    let cur = -scores[i0 - 1][j - 1] - u[i0] - v[j];
                    if cur < min[j] {
                        min[j] = cur;
                        way[j] = j0;
                    }
                    if min[j] < delta {
                        delta = min[j];
                        j1 = j;
                    }
                }
            }
            for j in 0..=n {
                if used[j] {
                    u[p[j]] += delta;
                    v[j] -= delta;
                } else {
                    min[j] -= delta;
                }
            }
            j0 = j1;
            if p[j0] == 0 {
                break;
            }
        }
        loop {
            let j1 = way[j0];
            p[j0] = p[j1];
            j0 = j1;
            if j0 == 0 {
                break;
            }
        }
    }
    (
        true,
        (1..=n)
            .map(|j| scores[p[j] - 1][j - 1])
            .fold(f64::INFINITY, f64::min),
    )
}
