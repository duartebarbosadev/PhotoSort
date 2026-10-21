//! General visual similarity, distinct from Cull's pooled subject evidence.
//! With min_samples=2, reference DBSCAN is connected components plus isolated noise.
pub mod grouping;
#[cfg(test)]
mod tests;
use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

pub const DEFAULT_EPS: f32 = 0.055;
pub fn normalize_distance(value: f32) -> f32 {
    if value.is_finite() {
        value.clamp(0.02, 0.20)
    } else {
        DEFAULT_EPS
    }
}
const BLOCK: usize = 64;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Embedding {
    pub vectors: Vec<Vec<f32>>,
}
impl Embedding {
    pub fn valid_model_output(&self) -> bool {
        matches!(self.vectors.len(), 1 | 6)
            && self
                .vectors
                .iter()
                .all(|v| v.len() == 384 && v.iter().all(|v| v.is_finite()))
    }
}
/// The whole image is supplied separately. These five crop rectangles round half
/// to even and clamp to the image, including tiny images.
pub fn crop_boxes(width: u32, height: u32) -> Vec<[u32; 4]> {
    if width <= 1 || height <= 1 {
        return Vec::new();
    }
    [
        [0.10, 0.10, 0.90, 0.90],
        [0., 0., 0.62, 1.],
        [0.38, 0., 1., 1.],
        [0., 0., 1., 0.62],
        [0., 0.38, 1., 1.],
    ]
    .into_iter()
    .filter_map(|[l, t, r, b]| {
        let [l, t, r, b] = [
            (l * f64::from(width))
                .round_ties_even()
                .clamp(0., f64::from(width - 1)) as u32,
            (t * f64::from(height))
                .round_ties_even()
                .clamp(0., f64::from(height - 1)) as u32,
            (r * f64::from(width))
                .round_ties_even()
                .clamp(1., f64::from(width)) as u32,
            (b * f64::from(height))
                .round_ties_even()
                .clamp(1., f64::from(height)) as u32,
        ];
        (r > l && b > t).then_some([l, t, r, b])
    })
    .collect()
}
/// Landscape and near-square frames share one partition.
pub fn portrait(width: u32, height: u32) -> bool {
    width > 0 && height > 0 && f64::from(width) / f64::from(height) < 0.9
}
#[derive(Clone)]
pub struct Features {
    rows: Vec<Vec<f32>>,
    dot_upper: f32,
}
impl Features {
    pub fn new(rows: Vec<Vec<f32>>) -> Result<Self> {
        ensure!(
            !rows.is_empty() && !rows[0].is_empty(),
            "Empty regional evidence"
        );
        let width = rows[0].len();
        ensure!(
            rows.iter()
                .all(|v| v.len() == width && v.iter().all(|x| x.is_finite())),
            "Invalid regional evidence"
        );
        let rows = rows
            .into_iter()
            .map(|mut row| {
                let norm = row
                    .iter()
                    .map(|x| f64::from(*x).powi(2))
                    .sum::<f64>()
                    .sqrt();
                if norm > 0. {
                    for x in &mut row {
                        *x = (f64::from(*x) / norm) as f32;
                    }
                }
                row
            })
            .collect();
        // Each nonzero row is normalized through f64 then rounded to f32.
        // Bound both that rounding and the f64 square/sum/sqrt/dot operations.
        // The generous forward-error allowance also covers zero/subnormal input;
        // such rows may be less similar, never more similar than this bound.
        // At impractical dimensions where the small-error bound stops applying,
        // infinity simply disables pruning instead of risking a false rejection.
        let accumulation_error = width as f64 * f64::EPSILON;
        let dot_upper = if accumulation_error < 0.01 {
            (1. + 8. * f64::from(f32::EPSILON) + 8. * accumulation_error) as f32
        } else {
            f32::INFINITY
        }
        .next_up();
        Ok(Self { rows, dot_upper })
    }
    fn digest(&self) -> [u8; 32] {
        let mut hash = blake3::Hasher::new();
        hash.update(&(self.rows.len() as u64).to_le_bytes());
        for row in &self.rows {
            hash.update(&(row.len() as u64).to_le_bytes());
            for x in row {
                hash.update(&(if *x == 0. { 0f32 } else { *x }).to_le_bytes());
            }
        }
        *hash.finalize().as_bytes()
    }
    /// Reject only when even maximally similar remaining regions cannot reach
    /// the threshold. The optimistic sum uses the same f32 operations/order as
    /// `distance`; replacing each unknown dot with an upper bound is monotone,
    /// so threshold ties and partially zero evidence keep their exact behavior.
    /// Returns evaluated regional dots for shared performance diagnostics.
    fn within_distance(&self, other: &Self, eps: f32) -> Result<(bool, usize)> {
        ensure!(
            self.rows[0].len() == other.rows[0].len(),
            "Regional dimensions differ"
        );
        let count = self.rows.len();
        if count != other.rows.len() || count == 1 {
            return Ok((self.distance(other)? <= eps, count * other.rows.len()));
        }
        let upper = self.dot_upper.max(other.dot_upper);
        let mut sum = 0.;
        for (index, (a, b)) in self.rows.iter().zip(&other.rows).enumerate() {
            sum += dot(a, b);
            let mut optimistic = sum;
            for _ in index + 1..count {
                optimistic += upper;
            }
            if (1. - optimistic / count as f32).clamp(0., 2.) > eps {
                return Ok((false, index + 1));
            }
        }
        Ok((true, count))
    }
    /// Ordered regions must correspond. Only incomplete/unequal records use the
    /// reference best-pair fallback; a matching background is not enough otherwise.
    pub fn distance(&self, other: &Self) -> Result<f32> {
        ensure!(
            self.rows[0].len() == other.rows[0].len(),
            "Regional dimensions differ"
        );
        let similarity = if self.rows.len() == other.rows.len() && self.rows.len() > 1 {
            self.rows
                .iter()
                .zip(&other.rows)
                .map(|(a, b)| dot(a, b))
                .sum::<f32>()
                / self.rows.len() as f32
        } else {
            self.rows
                .iter()
                .flat_map(|a| other.rows.iter().map(move |b| dot(a, b)))
                .fold(f32::NEG_INFINITY, f32::max)
        };
        Ok((1. - similarity).clamp(0., 2.))
    }
}
fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter()
        .zip(b)
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum::<f64>() as f32
}
#[derive(Clone)]
pub struct Input {
    pub key: String,
    pub portrait: bool,
}
#[derive(Default, Clone, Copy, Debug)]
pub struct Stats {
    pub evidence_loads: usize,
    pub comparisons: usize,
    /// Regional dots evaluated between distinct descriptors (excludes deduplication).
    pub region_comparisons: usize,
    pub unique_records: usize,
    pub peak_resident_records: usize,
}
pub struct Groups {
    pub labels: Vec<usize>,
    pub stats: Stats,
}
struct Sets {
    parents: Vec<usize>,
    sizes: Vec<usize>,
}
impl Sets {
    fn root(&mut self, mut n: usize) -> usize {
        while self.parents[n] != n {
            self.parents[n] = self.parents[self.parents[n]];
            n = self.parents[n];
        }
        n
    }
    fn join(&mut self, a: usize, b: usize) {
        let (mut a, mut b) = (self.root(a), self.root(b));
        if a == b {
            return;
        }
        if self.sizes[a] < self.sizes[b] {
            std::mem::swap(&mut a, &mut b);
        }
        self.parents[b] = a;
        self.sizes[a] += self.sizes[b];
    }
}
/// Exact DBSCAN membership for min_samples=2, retaining O(n) identity/union state
/// and at most two blocks of descriptors. No matrix or adjacency graph is built.
/// Output places portraits first and actual clusters before singleton noise.
/// Evidence-content ordering keeps numbers independent of paths and scan order. Identical admissible descriptors are
/// collapsed before comparison; zero vectors are deliberately not presumed equal.
pub fn cluster(
    inputs: &[Input],
    eps: f32,
    load: impl FnMut(&str) -> Result<Features>,
    cancelled: impl Fn() -> bool,
) -> Result<Option<Groups>> {
    cluster_with(inputs, eps, load, cancelled, Features::within_distance)
}

fn cluster_with(
    inputs: &[Input],
    eps: f32,
    mut load: impl FnMut(&str) -> Result<Features>,
    cancelled: impl Fn() -> bool,
    mut neighbors: impl FnMut(&Features, &Features, f32) -> Result<(bool, usize)>,
) -> Result<Option<Groups>> {
    ensure!(
        eps.is_finite() && eps > 0. && eps <= 2.,
        "Invalid clustering distance"
    );
    let mut stats = Stats::default();
    let mut identities = HashMap::new();
    let mut nodes = Vec::<usize>::new();
    let mut node_for = Vec::with_capacity(inputs.len());
    let mut weights = Vec::<usize>::new();
    let mut digests = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        let features = load(&input.key)?;
        stats.evidence_loads += 1;
        stats.peak_resident_records = 1;
        let identity = (input.portrait, features.digest());
        let mergeable = features.distance(&features)? <= eps;
        if mergeable && let Some(&node) = identities.get(&identity) {
            node_for.push(node);
            weights[node] += 1;
            continue;
        }
        let node = nodes.len();
        nodes.push(index);
        digests.push(identity.1);
        weights.push(1);
        node_for.push(node);
        if mergeable {
            identities.insert(identity, node);
        }
    }
    drop(identities);
    stats.unique_records = nodes.len();
    let mut sets = Sets {
        parents: (0..nodes.len()).collect(),
        sizes: weights,
    };
    for orientation in [true, false] {
        let partition: Vec<_> = (0..nodes.len())
            .filter(|i| inputs[nodes[*i]].portrait == orientation)
            .collect();
        for (block, outer) in partition.chunks(BLOCK).enumerate() {
            if cancelled() {
                return Ok(None);
            }
            let mut first = Vec::with_capacity(outer.len());
            for &node in outer {
                if cancelled() {
                    return Ok(None);
                }
                first.push(load(&inputs[nodes[node]].key)?);
                stats.evidence_loads += 1;
            }
            stats.peak_resident_records = stats.peak_resident_records.max(first.len());
            for (later, inner) in partition.chunks(BLOCK).enumerate().skip(block) {
                if cancelled() {
                    return Ok(None);
                }
                let second = if later == block {
                    None
                } else {
                    let mut values = Vec::with_capacity(inner.len());
                    for &node in inner {
                        if cancelled() {
                            return Ok(None);
                        }
                        values.push(load(&inputs[nodes[node]].key)?);
                        stats.evidence_loads += 1;
                    }
                    Some(values)
                };
                let second = second.as_ref().unwrap_or(&first);
                stats.peak_resident_records = stats
                    .peak_resident_records
                    .max(first.len() + if later == block { 0 } else { second.len() });
                for (i, &a) in outer.iter().enumerate() {
                    for (j, &b) in inner.iter().enumerate() {
                        if later == block && j <= i {
                            continue;
                        }
                        if cancelled() {
                            return Ok(None);
                        }
                        if sets.root(a) == sets.root(b) {
                            continue;
                        }
                        stats.comparisons += 1;
                        let (close, regions) = neighbors(&first[i], &second[j], eps)?;
                        stats.region_comparisons += regions;
                        if close {
                            sets.join(a, b);
                        }
                    }
                }
            }
        }
    }
    // One digest per unique node was already computed during deduplication.
    // The minimum digest identifies a component independently of union roots,
    // source/cache keys and the order in which edges were visited. No extra
    // descriptor loads or model calls are needed for stable folder numbering.
    let mut component_digests = vec![[u8::MAX; 32]; nodes.len()];
    for (node, digest) in digests.into_iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        let root = sets.root(node);
        component_digests[root] = component_digests[root].min(digest);
    }
    let mut ordered = BTreeMap::new();
    for (node, digest) in component_digests.into_iter().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        if sets.root(node) == node {
            let input = &inputs[nodes[node]];
            // Identical non-mergeable evidence (such as zero vectors) has no
            // content identity. Only that tie uses a cache key, then node index;
            // distinct valid components never depend on a path-derived key.
            ordered.insert(
                (
                    !input.portrait,
                    sets.sizes[node] == 1,
                    digest,
                    &input.key,
                    node,
                ),
                node,
            );
        }
    }
    let mut root_labels = vec![0; nodes.len()];
    for (label, node) in ordered.into_values().enumerate() {
        if cancelled() {
            return Ok(None);
        }
        root_labels[node] = label + 1;
    }
    let mut labels = Vec::with_capacity(inputs.len());
    for node in node_for {
        if cancelled() {
            return Ok(None);
        }
        labels.push(root_labels[sets.root(node)]);
    }
    Ok((!cancelled()).then_some(Groups { labels, stats }))
}
