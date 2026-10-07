//! Shared same-subject evidence and decisions. Model ownership stays in Models;
//! persistence is isolated in store and pure decisions in policy/geometry.
mod extraction;
mod geometry;
pub mod grouping;
mod policy;
pub mod store;
pub use extraction::DenseEmbedding;
pub use geometry::verify_geometry;
pub use policy::{Pair, Strictness, verify_pair};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Evidence {
    pub descriptor: Vec<f32>,
    pub bbox: [f32; 4],
    pub area: f32,
    pub kind: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub global: Vec<f32>,
    pub subjects: Vec<Evidence>,
    pub faces: Vec<Evidence>,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct Geometry {
    pub inlier_count: usize,
    pub inlier_ratio: f64,
    pub coverage_a: f64,
    pub coverage_b: f64,
    pub evaluated: bool,
}
impl Geometry {
    pub fn strong(self) -> bool {
        self.inlier_count >= 8
            && self.inlier_ratio >= 0.50
            && self.coverage_a.min(self.coverage_b) >= 0.12
    }
}
pub(super) fn cosine(a: &[f32], b: &[f32]) -> Option<f64> {
    if a.len() != b.len() || a.is_empty() || a.iter().chain(b).any(|v| !v.is_finite()) {
        return None;
    }
    let denominator = length(a) * length(b);
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let similarity = dot(a, b) / denominator;
    similarity
        .is_finite()
        .then(|| f64::from(similarity.clamp(-1.0, 1.0)))
}
fn length(values: &[f32]) -> f32 {
    values.iter().map(|v| v * v).sum::<f32>().sqrt()
}
pub(crate) fn normalized(values: &[f32]) -> Vec<f32> {
    let norm = length(values).max(f32::EPSILON);
    values.iter().map(|v| v / norm).collect()
}
pub(super) fn dot(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(a, b)| a * b).sum()
}

/// Exact reference candidates with O(n*k) retained storage, instead of a full
/// n-by-n similarity matrix. Rows are supplied in stable path order. Callers can
/// cancel even within a long nearest-neighbor scan.
pub fn candidates(
    artifacts: &[Artifact],
    times: &[Option<i64>],
    cancelled: impl Fn() -> bool,
) -> Option<Vec<(usize, usize)>> {
    use std::{cmp::Reverse, collections::BinaryHeap};
    #[derive(Clone, Copy, Debug, PartialEq)]
    struct Neighbor(f64, usize);
    impl Eq for Neighbor {}
    impl PartialOrd for Neighbor {
        fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
            Some(self.cmp(other))
        }
    }
    impl Ord for Neighbor {
        fn cmp(&self, other: &Self) -> std::cmp::Ordering {
            self.0
                .total_cmp(&other.0)
                .then_with(|| other.1.cmp(&self.1))
        }
    }
    let mut vectors = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        if cancelled() {
            return None;
        }
        vectors.push(normalized(&artifact.global));
    }
    let mut pairs = BTreeSet::new();
    for (i, a) in vectors.iter().enumerate() {
        if cancelled() {
            return None;
        }
        let mut nearest = BinaryHeap::new();
        for (j, b) in vectors.iter().enumerate() {
            if j % 128 == 0 && cancelled() {
                return None;
            }
            if i == j {
                continue;
            }
            nearest.push(Reverse(Neighbor(f64::from(dot(a, b)), j)));
            if nearest.len() > 32 {
                nearest.pop();
            }
        }
        for Reverse(Neighbor(_, j)) in nearest {
            pairs.insert((i.min(j), i.max(j)));
        }
    }
    let mut dated: Vec<_> = times
        .iter()
        .take(artifacts.len())
        .enumerate()
        .filter_map(|(i, t)| t.map(|t| (t, i)))
        .collect();
    dated.sort_unstable();
    for (time, id) in &dated {
        if cancelled() {
            return None;
        }
        let first = dated.partition_point(|(t, _)| *t < time.saturating_sub(600_000));
        let last = dated.partition_point(|(t, _)| *t <= time.saturating_add(600_000));
        let mut nearby = BinaryHeap::new();
        for (n, &(other_time, other)) in dated[first..last].iter().enumerate() {
            if n % 128 == 0 && cancelled() {
                return None;
            }
            if other == *id {
                continue;
            }
            nearby.push((time.abs_diff(other_time), other));
            if nearby.len() > 8 {
                nearby.pop();
            }
        }

        for (_, other) in nearby.into_iter().take(8) {
            pairs.insert(((*id).min(other), (*id).max(other)));
        }
    }
    (!cancelled()).then(|| pairs.into_iter().collect())
}

/// Every cross-pair must be accepted; missing evidence never permits chaining.
/// Indices and edge ties follow the caller's stable path order.
pub fn clusters(
    count: usize,
    pairs: &BTreeMap<(usize, usize), Pair>,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<usize>> {
    if cancelled() {
        return None;
    }
    let mut owners: Vec<_> = (0..count).collect();
    let mut members: Vec<_> = (0..count).map(|i| vec![i]).collect();
    let mut edges: Vec<_> = pairs
        .iter()
        .filter(|((a, b), p)| p.accepted && *a < count && *b < count && p.confidence.is_finite())
        .map(|(&key, p)| (key, p.confidence))
        .collect();
    edges.sort_by(|(a, ac), (b, bc)| bc.total_cmp(ac).then_with(|| a.cmp(b)));
    for ((a, b), _) in edges {
        if cancelled() {
            return None;
        }
        let (a, b) = (owners[a], owners[b]);
        if a == b {
            continue;
        }
        let mut accepted = true;
        for &left in &members[a] {
            if cancelled() {
                return None;
            }
            if members[b].iter().any(|&right| {
                !pairs
                    .get(&(left.min(right), left.max(right)))
                    .is_some_and(|p| p.accepted)
            }) {
                accepted = false;
                break;
            }
        }
        if accepted {
            let moved = std::mem::take(&mut members[b]);
            for &id in &moved {
                owners[id] = a;
            }
            members[a].extend(moved);
        }
    }
    let mut nonempty: Vec<_> = members.into_iter().filter(|m| !m.is_empty()).collect();
    nonempty.sort_by_key(|m| *m.iter().min().unwrap());
    let mut result = vec![0; count];
    for (n, group) in nonempty.into_iter().enumerate() {
        for id in group {
            result[id] = n + 1;
        }
    }
    (!cancelled()).then_some(result)
}
