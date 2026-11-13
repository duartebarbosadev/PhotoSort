//! Shared same-subject evidence and decisions. Model ownership stays in Models;
//! persistence is isolated in store and pure decisions in policy/geometry.
mod candidates;
pub use candidates::{candidates, candidates_with};
mod extraction;
mod geometry;
pub mod grouping;
mod policy;
pub mod store;
pub use extraction::DenseEmbedding;
pub use geometry::verify_geometry;
pub use policy::{Pair, Strictness, verify_pair};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

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
    cosine_of(a, b, length(a), length(b))
}
/// `cosine` of finite vectors of one width whose lengths are already known:
/// a pair of photos compares each region with sixteen others.
pub(super) fn cosine_of(a: &[f32], b: &[f32], length_a: f32, length_b: f32) -> Option<f64> {
    let denominator = length_a * length_b;
    if denominator == 0.0 || !denominator.is_finite() {
        return None;
    }
    let similarity = dot(a, b) / denominator;
    similarity
        .is_finite()
        .then(|| f64::from(similarity.clamp(-1.0, 1.0)))
}
pub(super) fn length(values: &[f32]) -> f32 {
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

/// Every cross-pair must be accepted; missing evidence never permits chaining.
/// Indices and edge ties follow the caller's stable path order.
pub fn clusters(
    count: usize,
    pairs: &BTreeMap<(usize, usize), Pair>,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<usize>> {
    let mut links = Links::new(count);
    for (&pair, verdict) in pairs {
        links.add(pair, verdict.accepted, verdict.confidence);
    }
    // No group is small enough to have its missing pairs judged.
    links
        .clusters(0, cancelled, |_, _| {}, |_| Ok(Judged::Accepted))
        .ok()
        .flatten()
}

/// What judging the missing pairs of one merge found.
pub(super) enum Judged {
    Accepted,
    /// This pair is rejected; the others may not have been judged.
    Rejected(usize, usize),
    Cancelled,
}

/// The verdicts as merging needs them: the accepted pairs, strongest first,
/// and for each photo who it is accepted and who it is rejected with.
pub(super) struct Links {
    edges: Vec<((usize, usize), f64)>,
    accepted: Vec<Vec<u32>>,
    rejected: Vec<Vec<u32>>,
}
impl Links {
    pub(super) fn new(count: usize) -> Self {
        Self {
            edges: Vec::new(),
            accepted: vec![Vec::new(); count],
            rejected: vec![Vec::new(); count],
        }
    }
    /// The verdict of one pair, looked up as (lower, higher) when merging.
    pub(super) fn add(&mut self, (a, b): (usize, usize), accepted: bool, confidence: f64) {
        let count = self.accepted.len();
        if a >= count || b >= count {
            return;
        }
        if accepted && confidence.is_finite() {
            self.edges.push(((a, b), confidence));
        }
        if a < b {
            let known = if accepted {
                &mut self.accepted
            } else {
                &mut self.rejected
            };
            known[a].push(b as u32);
            known[b].push(a as u32);
        }
    }
    /// Complete-link groups, numbered from 1 in the order of their first
    /// photo. Two groups merge when every pair across them is accepted. Where
    /// some of those pairs were never compared and the merged group would
    /// have at most `limit` photos, `judge` is given them; larger groups stay
    /// apart, as they do for a rejected pair. `progress` hears how many of
    /// the accepted pairs have been tried.
    pub(super) fn clusters(
        mut self,
        limit: usize,
        cancelled: impl Fn() -> bool,
        mut progress: impl FnMut(usize, usize),
        mut judge: impl FnMut(Vec<(usize, usize)>) -> anyhow::Result<Judged>,
    ) -> anyhow::Result<Option<Vec<usize>>> {
        if cancelled() {
            return Ok(None);
        }
        let count = self.accepted.len();
        let mut owners: Vec<_> = (0..count).collect();
        let mut members: Vec<_> = (0..count).map(|i| vec![i]).collect();
        let mut edges = std::mem::take(&mut self.edges);
        edges.sort_by(|(a, ac), (b, bc)| bc.total_cmp(ac).then_with(|| a.cmp(b)));
        // marks[other] == left: `other` is accepted with the photo `left`.
        let mut marks = vec![usize::MAX; count];
        let total = edges.len();
        for (tried, ((a, b), _)) in edges.into_iter().enumerate() {
            if cancelled() {
                return Ok(None);
            }
            progress(tried, total);
            let (a, b) = (owners[a], owners[b]);
            if a == b {
                continue;
            }
            // Groups only grow, so one rejected pair keeps them apart for good.
            let (small, large) = if members[a].len() <= members[b].len() {
                (a, b)
            } else {
                (b, a)
            };
            if members[small].iter().any(|&photo| {
                self.rejected[photo]
                    .iter()
                    .any(|&other| owners[other as usize] == large)
            }) {
                continue;
            }
            let judged = members[a].len() + members[b].len() <= limit;
            let mut missing = Vec::new();
            let mut complete = true;
            'cross: for &left in &members[a] {
                if cancelled() {
                    return Ok(None);
                }
                for &other in &self.accepted[left] {
                    marks[other as usize] = left;
                }
                for &right in &members[b] {
                    if marks[right] != left {
                        if !judged {
                            complete = false;
                            break 'cross;
                        }
                        missing.push((left.min(right), left.max(right)));
                    }
                }
            }
            if !complete {
                continue;
            }
            if !missing.is_empty() {
                match judge(missing)? {
                    Judged::Accepted => {}
                    Judged::Rejected(x, y) => {
                        self.rejected[x].push(y as u32);
                        self.rejected[y].push(x as u32);
                        continue;
                    }
                    Judged::Cancelled => return Ok(None),
                }
            }
            let moved = std::mem::take(&mut members[b]);
            for &id in &moved {
                owners[id] = a;
            }
            members[a].extend(moved);
        }
        progress(total, total);
        let mut nonempty: Vec<_> = members.into_iter().filter(|m| !m.is_empty()).collect();
        nonempty.sort_by_key(|m| *m.iter().min().unwrap());
        let mut result = vec![0; count];
        for (n, group) in nonempty.into_iter().enumerate() {
            for id in group {
                result[id] = n + 1;
            }
        }
        Ok((!cancelled()).then_some(result))
    }
}
