//! Exact semantic and temporal retrieval shared by Cull and Pick Best.
use super::{Artifact, dot, normalized};
use crate::core::parallel;
use std::{
    cmp::Reverse,
    collections::{BTreeSet, BinaryHeap},
    ops::Range,
};

#[derive(Clone, Copy, Debug)]
struct Neighbor(f64, usize);
impl PartialEq for Neighbor {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}
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

fn offer(heap: &mut BinaryHeap<Reverse<Neighbor>>, value: Neighbor) {
    if heap.len() < 32 {
        heap.push(Reverse(value));
    } else if let Some(mut worst) = heap.peek_mut()
        && value > worst.0
    {
        *worst = Reverse(value);
    }
}

/// Exact candidates with O(n*k) retained storage and no n-by-n matrix. Stable
/// path order breaks semantic/temporal ties.
pub fn candidates(
    artifacts: &[Artifact],
    times: &[Option<i64>],
    cancelled: impl Fn() -> bool,
) -> Option<Vec<(usize, usize)>> {
    candidates_with(artifacts, times, 1, cancelled, |_, _| {})
}

/// `candidates` on up to `threads` threads. `progress` hears how many photos
/// have their nearest neighbours, of how many.
pub fn candidates_with(
    artifacts: &[Artifact],
    times: &[Option<i64>],
    threads: usize,
    cancelled: impl Fn() -> bool,
    progress: impl FnMut(usize, usize),
) -> Option<Vec<(usize, usize)>> {
    let mut vectors = Vec::with_capacity(artifacts.len());
    let mut finite = Vec::with_capacity(artifacts.len());
    for artifact in artifacts {
        if cancelled() {
            return None;
        }
        let vector = normalized(&artifact.global);
        finite.push(vector.iter().all(|v| v.is_finite()));
        vectors.push(vector);
    }
    let width = vectors.first().map_or(0, Vec::len);
    let nearest = if width > 0
        && finite.iter().all(|finite| *finite)
        && vectors.iter().all(|v| v.len() == width)
    {
        nearest_screened(vectors, width, threads, &cancelled, progress)?
    } else {
        nearest_exhaustive(&vectors, &finite, &cancelled)?
    };
    let mut pairs = BTreeSet::new();
    for (i, row) in nearest.into_iter().enumerate() {
        if cancelled() {
            return None;
        }
        for j in row {
            pairs.insert((i.min(j), i.max(j)));
        }
    }
    if !add_temporal(times, artifacts.len(), &mut pairs, &cancelled) {
        return None;
    }
    (!cancelled()).then(|| pairs.into_iter().collect())
}

/// Each photo's 32 nearest, every pair compared with the exact dot product.
/// Finite dot products are symmetric; evaluate each pair once, feeding both
/// bounded top-32 heaps. Invalid vectors retain directional IEEE behavior
/// rather than assuming NaN payload symmetry. Also the reference the screened
/// search is tested against.
fn nearest_exhaustive(
    vectors: &[Vec<f32>],
    finite: &[bool],
    cancelled: &impl Fn() -> bool,
) -> Option<Vec<Vec<usize>>> {
    let mut nearest: Vec<_> = (0..vectors.len())
        .map(|_| BinaryHeap::with_capacity(32.min(vectors.len().saturating_sub(1))))
        .collect();
    for (i, a) in vectors.iter().enumerate() {
        if cancelled() {
            return None;
        }
        for (j, b) in vectors.iter().enumerate().skip(i + 1) {
            if j % 128 == 0 && cancelled() {
                return None;
            }
            let forward = f64::from(dot(a, b));
            let backward = if finite[i] && finite[j] {
                forward
            } else {
                f64::from(dot(b, a))
            };
            offer(&mut nearest[i], Neighbor(forward, j));
            offer(&mut nearest[j], Neighbor(backward, i));
        }
    }
    Some(
        nearest
            .into_iter()
            .map(|heap| heap.into_iter().map(|Reverse(Neighbor(_, j))| j).collect())
            .collect(),
    )
}

/// Photos searched per block, and photos compared against at a time so the
/// block's vectors stay in the processor cache.
const ROWS: usize = 16;
const COLUMNS: usize = 128;

/// The same neighbours as `nearest_exhaustive` for finite vectors of one
/// width, several times faster and on several threads. A vectorized dot
/// product ranks every other photo first; it can differ from the exact one by
/// less than `dot_error`, so every photo that close to the 32nd place is
/// compared exactly, and the exact values alone choose and order the 32.
fn nearest_screened(
    mut vectors: Vec<Vec<f32>>,
    width: usize,
    threads: usize,
    cancelled: &impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
) -> Option<Vec<Vec<usize>>> {
    let count = vectors.len();
    let mut flat = Vec::with_capacity(count * width);
    let mut lengths = Vec::with_capacity(count);
    for vector in vectors.drain(..) {
        if cancelled() {
            return None;
        }
        lengths.push(
            vector
                .iter()
                .map(|v| f64::from(*v) * f64::from(*v))
                .sum::<f64>()
                .sqrt(),
        );
        flat.extend_from_slice(&vector);
    }
    let longest = lengths.iter().copied().fold(0.0, f64::max);
    let row = |i: usize| &flat[i * width..(i + 1) * width];
    let blocks = parallel::blocks(
        threads,
        count.div_ceil(ROWS),
        cancelled,
        |blocks| progress((blocks * ROWS).min(count), count),
        |block, stop| {
            let rows = block * ROWS..((block + 1) * ROWS).min(count);
            let mut scores = vec![0.0f32; rows.len() * count];
            for columns in (0..count).step_by(COLUMNS) {
                if stop() {
                    return None;
                }
                for (r, i) in rows.clone().enumerate() {
                    let a = row(i);
                    for j in columns..(columns + COLUMNS).min(count) {
                        scores[r * count + j] = parallel::fast_dot(a, row(j));
                    }
                }
            }
            let mut nearest = Vec::with_capacity(rows.len());
            for (r, i) in rows.enumerate() {
                let scores = &scores[r * count..(r + 1) * count];
                let mut screened = BinaryHeap::with_capacity(32);
                for (j, score) in scores.iter().enumerate() {
                    if j != i {
                        offer(&mut screened, Neighbor(f64::from(*score), j));
                    }
                }
                // Below this no photo can be among the exact 32: the 32 best
                // screened scores are each at least `place - error` exactly.
                let cut = if screened.len() < 32 {
                    f64::NEG_INFINITY
                } else {
                    let place = screened.peek().map_or(f64::NEG_INFINITY, |n| n.0.0);
                    place - 2.0 * parallel::dot_error(width, lengths[i], longest)
                };
                let a = row(i);
                let mut exact = BinaryHeap::with_capacity(32);
                for (j, score) in scores.iter().enumerate() {
                    if j != i && f64::from(*score) >= cut {
                        offer(&mut exact, Neighbor(f64::from(dot(a, row(j))), j));
                    }
                }
                nearest.push(
                    exact
                        .into_iter()
                        .map(|Reverse(Neighbor(_, j))| j)
                        .collect::<Vec<_>>(),
                );
            }
            Some(nearest)
        },
    )?;
    Some(blocks.into_iter().flatten().collect())
}

fn add_temporal(
    times: &[Option<i64>],
    count: usize,
    pairs: &mut BTreeSet<(usize, usize)>,
    cancelled: &impl Fn() -> bool,
) -> bool {
    let mut dated = Vec::with_capacity(count.min(times.len()));
    for (id, time) in times.iter().take(count).enumerate() {
        if cancelled() {
            return false;
        }
        if let Some(time) = time {
            dated.push((*time, id));
        }
    }
    dated.sort_unstable();
    // Equal-time groups let the left cursor walk backwards in time while still
    // visiting tied IDs in ascending order. A simple reversed slice would pick
    // the wrong neighbors in bursts with identical capture timestamps.
    let mut groups = Vec::<Range<usize>>::new();
    for i in 0..dated.len() {
        if cancelled() {
            return false;
        }
        if i > 0 && dated[i - 1].0 == dated[i].0 {
            groups.last_mut().unwrap().end = i + 1;
        } else {
            groups.push(i..i + 1);
        }
    }
    for (group_index, group) in groups.iter().enumerate() {
        for &(time, id) in &dated[group.clone()] {
            if cancelled() {
                return false;
            }
            let mut left_group = group_index.checked_sub(1);
            let mut left = left_group.map(|g| groups[g].start);
            let mut right = group.start;
            for _ in 0..8 {
                if dated.get(right).is_some_and(|(_, other)| *other == id) {
                    right += 1;
                }
                let neighbor = |index: usize| {
                    dated.get(index).and_then(|&(other_time, other)| {
                        let distance = time.abs_diff(other_time);
                        (distance <= 600_000).then_some((distance, other))
                    })
                };
                let a = left.and_then(neighbor);
                let b = neighbor(right);
                let from_left = match (a, b) {
                    (Some(a), Some(b)) => a < b,
                    (Some(_), None) => true,
                    (None, Some(_)) => false,
                    (None, None) => break,
                };
                let (_, other) = if from_left {
                    let chosen = a.unwrap();
                    let next = left.unwrap() + 1;
                    if next < groups[left_group.unwrap()].end {
                        left = Some(next);
                    } else {
                        left_group = left_group.unwrap().checked_sub(1);
                        left = left_group.map(|g| groups[g].start);
                    }
                    chosen
                } else {
                    right += 1;
                    b.unwrap()
                };
                pairs.insert((id.min(other), id.max(other)));
            }
        }
    }
    !cancelled()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    #[ignore = "requires PHOTOSORT_SAMPLE_DIR and native model/runtime resources; reads originals only"]
    fn candidate_search_matches_real_sample_worker_embeddings() {
        use crate::core::{
            catalog::Catalog,
            jobs::{AnalysisProfile, Event, Runtime},
            model::MediaKind,
        };
        use std::{
            sync::Arc,
            time::{Duration, Instant},
        };
        let root = std::path::PathBuf::from(
            std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("sample directory"),
        );
        let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
        assert!(catalog.warnings.is_empty());
        let mut media: Vec<_> = catalog
            .media
            .into_iter()
            .filter(|m| m.kind != MediaKind::Video)
            .collect();
        media.sort_by(|a, b| a.path.cmp(&b.path));
        assert!(!media.is_empty());
        let cache = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::with_cache(|| {}, cache.path().to_path_buf());
        let run = runtime
            .analyze(Arc::new(media.clone()), AnalysisProfile::Subjects)
            .unwrap();
        let mut received = std::collections::HashMap::new();
        let deadline = Instant::now() + Duration::from_secs(600);
        loop {
            assert!(Instant::now() < deadline, "sample analysis timed out");
            let event = match runtime.events.recv_timeout(Duration::from_secs(1)) {
                Ok(envelope) => envelope.event,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(error) => panic!("sample worker stopped: {error}"),
            };
            match event {
                Event::Analyzed {
                    run: completed,
                    id,
                    embedding,
                    metadata,
                    ..
                } if completed == run => {
                    received.insert(
                        id,
                        (
                            embedding.expect("subject global descriptor"),
                            metadata.captured_ms,
                        ),
                    );
                }
                Event::AnalysisDone { run: completed, .. } if completed == run => break,
                Event::AnalysisError { error, .. }
                | Event::RunError { error, .. }
                | Event::Error(error) => panic!("{error}"),
                Event::ModelsNeeded { .. } => panic!("sample model resources missing"),
                _ => {}
            }
        }
        runtime.shutdown();
        assert_eq!(received.len(), media.len());
        let mut artifacts = Vec::new();
        let mut times = Vec::new();
        for photo in media {
            let (global, time) = received.remove(&photo.id).unwrap();
            artifacts.push(Artifact {
                // The grouping worker uses the same f16 evidence precision on
                // fresh and reopened caches; reproduce that input boundary.
                global: global
                    .iter()
                    .map(|v| half::f16::from_f32(*v).to_f32())
                    .collect(),
                subjects: vec![],
                faces: vec![],
            });
            times.push(time);
        }
        let actual = candidates(&artifacts, &times, || false).unwrap();
        assert_eq!(actual, oracle(&artifacts, &times));
        eprintln!(
            "Exact candidate equality: {} photos, {} pairs",
            artifacts.len(),
            actual.len()
        );
    }

    fn temporal_oracle(times: &[Option<i64>], count: usize) -> BTreeSet<(usize, usize)> {
        let mut pairs = BTreeSet::new();
        for (i, time) in times.iter().take(count).enumerate() {
            let Some(time) = time else { continue };
            let mut row: Vec<_> = times
                .iter()
                .take(count)
                .enumerate()
                .filter_map(|(j, other)| {
                    let distance = time.abs_diff((*other)?);
                    (i != j && distance <= 600_000).then_some((distance, j))
                })
                .collect();
            row.sort_unstable();
            for (_, j) in row.into_iter().take(8) {
                pairs.insert((i.min(j), i.max(j)));
            }
        }
        pairs
    }

    fn oracle(artifacts: &[Artifact], times: &[Option<i64>]) -> Vec<(usize, usize)> {
        let vectors: Vec<_> = artifacts.iter().map(|a| normalized(&a.global)).collect();
        let mut pairs = temporal_oracle(times, artifacts.len());
        for (i, a) in vectors.iter().enumerate() {
            let mut row: Vec<_> = vectors
                .iter()
                .enumerate()
                .filter(|(j, _)| *j != i)
                .map(|(j, b)| (f64::from(dot(a, b)), j))
                .collect();
            row.sort_unstable_by(|(a, ai), (b, bi)| b.total_cmp(a).then_with(|| ai.cmp(bi)));
            for (_, j) in row.into_iter().take(32) {
                pairs.insert((i.min(j), i.max(j)));
            }
        }
        pairs.into_iter().collect()
    }

    #[test]
    fn candidate_search_matches_full_sort_with_ties_invalid_vectors_and_partial_dates() {
        let mut seed = 29_u64;
        for count in [0, 1, 2, 8, 31, 32, 33, 65, 129] {
            for width in [0, 1, 7, 384] {
                let mut artifacts: Vec<_> = (0..count)
                    .map(|i| Artifact {
                        global: (0..width)
                            .map(|_| {
                                seed ^= seed << 13;
                                seed ^= seed >> 7;
                                seed ^= seed << 17;
                                if i % 11 == 0 {
                                    0.0
                                } else {
                                    (seed % 200) as f32 / 100.0 - 1.0
                                }
                            })
                            .collect(),
                        subjects: vec![],
                        faces: vec![],
                    })
                    .collect();
                let mut times: Vec<_> = (0..count)
                    .map(|i| match i % 7 {
                        0 => None,
                        1 => Some(i64::MIN),
                        2 => Some(i64::MAX),
                        _ => Some((i % 13) as i64 * 300_000 - 600_000),
                    })
                    .collect();
                if count > 33 && width > 0 {
                    artifacts[1].global[0] = f32::from_bits(0x7fc0_1234);
                    artifacts[2].global[0] = f32::from_bits(0xffc0_5678);
                    artifacts[3].global[0] = f32::INFINITY;
                    artifacts[4].global.pop();
                    artifacts[5].global = artifacts[6].global.clone();
                }
                for length in [count + 3, count / 2, 0] {
                    times.resize(length, Some(0));
                    assert_eq!(
                        candidates(&artifacts, &times, || false).unwrap(),
                        oracle(&artifacts, &times),
                        "{count}/{width}/{length}"
                    );
                }
            }
        }
        // The threaded search ranks with a faster dot product first. Photos of
        // one scene differ in the last bits of their similarity, far closer
        // than that product is exact, and exact copies tie: the 32 must still
        // be chosen and ordered by the exact values alone.
        let scenes: Vec<Vec<f32>> = (0..5)
            .map(|scene| {
                (0..384)
                    .map(|i| ((i * 31 + scene * 17) % 97) as f32 - 48.0)
                    .collect()
            })
            .collect();
        let artifacts: Vec<_> = (0..230)
            .map(|i| Artifact {
                global: scenes[i % scenes.len()]
                    .iter()
                    .map(|v| {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        let noise = (seed % 2001) as f32 / 1000.0 - 1.0;
                        v + noise * [0.0, 1e-4, 1e-2, 3.0][i / 5 % 4]
                    })
                    .collect(),
                subjects: vec![],
                faces: vec![],
            })
            .collect();
        let expected = oracle(&artifacts, &[]);
        for threads in [1, 3] {
            let mut reported = Vec::new();
            let found = candidates_with(
                &artifacts,
                &[],
                threads,
                || false,
                |done, total| reported.push((done, total)),
            );
            assert_eq!(found.unwrap(), expected, "{threads} threads");
            assert_eq!(reported.last(), Some(&(230, 230)));
        }
    }

    #[test]
    fn temporal_search_matches_full_sort_at_window_boundaries_and_equal_time_groups() {
        let times: Vec<_> = (0..137)
            .map(|i| {
                Some(match i % 11 {
                    0 => i64::MIN,
                    1 => i64::MIN + 600_000,
                    2 => i64::MAX,
                    3 => i64::MAX - 600_000,
                    4 => -600_001,
                    5 => -600_000,
                    6 => 0,
                    7 => 600_000,
                    8 => 600_001,
                    _ => (i % 3) as i64,
                })
            })
            .collect();
        for count in [0, 1, 8, 30, times.len(), times.len() + 20] {
            let mut actual = BTreeSet::new();
            assert!(add_temporal(&times, count, &mut actual, &|| false));
            assert_eq!(actual, temporal_oracle(&times, count));
        }
    }

    #[test]
    fn temporal_search_bounds_dense_bursts_and_cancels_without_publishing_partial_pairs() {
        let count = 20_000;
        let times = vec![Some(7); count];
        let calls = Cell::new(0);
        let mut pairs = BTreeSet::new();
        assert!(add_temporal(&times, count, &mut pairs, &|| {
            calls.set(calls.get() + 1);
            false
        }));
        let expected: BTreeSet<_> = (0..count)
            .flat_map(|i| {
                (0..9)
                    .filter(move |&j| j != i)
                    .take(8)
                    .map(move |j| (i.min(j), i.max(j)))
            })
            .collect();
        assert_eq!(pairs, expected);
        assert_eq!(calls.get(), 3 * count + 1);
        let artifacts: Vec<_> = (0..100)
            .map(|_| Artifact {
                global: vec![1.0],
                subjects: vec![],
                faces: vec![],
            })
            .collect();
        for stop in [0, 110, 230, 350] {
            let calls = Cell::new(0);
            assert!(
                candidates(&artifacts, &times[..100], || {
                    calls.set(calls.get() + 1);
                    calls.get() > stop
                })
                .is_none()
            );
        }
    }
}
