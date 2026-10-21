//! Group order over cached records only. Runs on the browse worker, never the UI.
use super::GroupSort;
use nalgebra::{DMatrix, linalg::SymmetricEigen};
use std::collections::BTreeMap;

#[derive(Default)]
struct Group {
    earliest: Option<i64>,
    sum: Vec<f64>,
    count: usize,
    invalid: bool,
}

pub(super) fn order<'a>(
    rows: impl Iterator<Item = (usize, Option<i64>, Option<&'a [f32]>)>,
    mode: GroupSort,
    cancelled: impl Fn() -> bool,
) -> Option<Vec<usize>> {
    let mut groups: BTreeMap<usize, Group> = BTreeMap::new();
    for (n, (id, time, embedding)) in rows.enumerate() {
        if n % 128 == 0 && cancelled() {
            return None;
        }
        let group = groups.entry(id).or_default();
        if let Some(time) = time {
            group.earliest = Some(group.earliest.map_or(time, |old| old.min(time)));
        }
        if mode != GroupSort::Similarity {
            continue;
        }
        if let Some(vector) = embedding {
            if vector.is_empty() || vector.len() > 2048 || vector.iter().any(|x| !x.is_finite()) {
                group.invalid = true;
                continue;
            }
            if group.count == 0 {
                group.sum = vec![0.0; vector.len()];
            }
            if group.sum.len() != vector.len() {
                group.invalid = true;
                continue;
            }
            for (sum, value) in group.sum.iter_mut().zip(vector) {
                *sum += *value as f64;
            }
            group.count += 1;
        }
    }
    let mut valid = Vec::new();
    for (&id, group) in &mut groups {
        if cancelled() {
            return None;
        }
        if group.count > 0 && !group.invalid {
            let mut centroid = std::mem::take(&mut group.sum);
            for value in &mut centroid {
                *value /= group.count as f64;
            }
            valid.push((id, centroid));
        } else {
            group.sum.clear();
        }
    }
    let scores = if mode == GroupSort::Similarity {
        principal_scores(&mut valid, &cancelled)?
    } else {
        BTreeMap::new()
    };
    if cancelled() {
        return None;
    }
    let mut ids: Vec<_> = groups.keys().copied().collect();
    ids.sort_by(|a, b| {
        let score = match (scores.get(a), scores.get(b)) {
            (Some(a), Some(b)) => a.total_cmp(b),
            (Some(_), None) => std::cmp::Ordering::Less,
            (None, Some(_)) => std::cmp::Ordering::Greater,
            _ => std::cmp::Ordering::Equal,
        };
        score
            .then_with(|| {
                groups[a]
                    .earliest
                    .unwrap_or(i64::MAX)
                    .cmp(&groups[b].earliest.unwrap_or(i64::MAX))
            })
            .then_with(|| a.cmp(b))
    });
    (!cancelled()).then_some(ids)
}

// PCA uses the smaller of the feature covariance and group Gram matrices. Its
// workspace is bounded by descriptor width, not the square of library size.
// Eigenvector sign follows sklearn's largest-absolute-loading-positive rule.
fn principal_scores(
    rows: &mut [(usize, Vec<f64>)],
    cancelled: &impl Fn() -> bool,
) -> Option<BTreeMap<usize, f64>> {
    let empty = || Some(BTreeMap::new());
    if rows.len() < 2 {
        return empty();
    }
    let width = rows[0].1.len();
    if rows.iter().any(|(_, v)| v.len() != width) {
        return empty();
    }
    let mut mean = vec![0.0; width];
    for (_, row) in rows.iter() {
        if cancelled() {
            return None;
        }
        for (m, x) in mean.iter_mut().zip(row) {
            *m += x / rows.len() as f64;
        }
    }
    // Reuse the centroid buffers for centering instead of retaining a second
    // library-sized matrix. The original shared embeddings are immutable.
    for (_, row) in rows.iter_mut() {
        if cancelled() {
            return None;
        }
        for (value, mean) in row.iter_mut().zip(&mean) {
            *value -= mean;
        }
    }
    let gram = rows.len() < width;
    let size = rows.len().min(width);
    let mut matrix = DMatrix::<f64>::zeros(size, size);
    if gram {
        for i in 0..size {
            if cancelled() {
                return None;
            }
            for j in 0..=i {
                matrix[(i, j)] = rows[i].1.iter().zip(&rows[j].1).map(|(a, b)| a * b).sum();
            }
        }
    } else {
        for (_, row) in rows.iter() {
            if cancelled() {
                return None;
            }
            for i in 0..size {
                for j in 0..=i {
                    matrix[(i, j)] += row[i] * row[j];
                }
            }
        }
    }
    if cancelled() {
        return None;
    }
    // A finite iteration limit also bounds the non-interruptible solver phase.
    let Some(eigen) = SymmetricEigen::try_new(matrix, 1e-12, 10_000) else {
        return empty();
    };
    if cancelled() {
        return None;
    }
    let component =
        (0..size).max_by(|a, b| eigen.eigenvalues[*a].total_cmp(&eigen.eigenvalues[*b]))?;
    let variance = eigen.eigenvalues[component];
    if !variance.is_finite() || variance <= 1e-20 {
        return empty();
    }
    let mut loading = if gram {
        let mut loading = vec![0.0; width];
        for (i, (_, row)) in rows.iter().enumerate() {
            for (v, x) in loading.iter_mut().zip(row) {
                *v += x * eigen.eigenvectors[(i, component)] / variance.sqrt();
            }
        }
        loading
    } else {
        eigen
            .eigenvectors
            .column(component)
            .iter()
            .copied()
            .collect()
    };
    let pivot = (0..width).max_by(|a, b| {
        loading[*a]
            .abs()
            .total_cmp(&loading[*b].abs())
            .then_with(|| b.cmp(a))
    })?;
    if loading[pivot] < 0.0 {
        for x in &mut loading {
            *x = -*x;
        }
    }
    let mut scores = BTreeMap::new();
    for (id, row) in rows.iter() {
        if cancelled() {
            return None;
        }
        let score: f64 = row.iter().zip(&loading).map(|(a, b)| a * b).sum();
        if !score.is_finite() {
            return empty();
        }
        scores.insert(*id, score);
    }
    Some(scores)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_degenerate_and_missing_embeddings_fall_back_to_time() {
        let cases = [
            vec![vec![1.0], vec![1.0]],
            vec![vec![f32::NAN], vec![1.0]],
            vec![vec![1.0, 2.0], vec![1.0]],
        ];
        for vectors in cases {
            assert_eq!(
                order(
                    [
                        (1, Some(10), Some(vectors[0].as_slice())),
                        (2, Some(1), Some(vectors[1].as_slice())),
                        (3, None, None)
                    ]
                    .into_iter(),
                    GroupSort::Similarity,
                    || false
                ),
                Some(vec![2, 1, 3])
            );
        }
        assert_eq!(
            order(
                [(2, Some(3), None), (1, Some(3), None)].into_iter(),
                GroupSort::Time,
                || false
            ),
            Some(vec![1, 2])
        );
    }
    #[test]
    fn group_order_cancels_during_aggregation_and_matrix_work() {
        use std::cell::Cell;
        let count = Cell::new(0);
        assert!(
            order(
                (0..100_000).map(|i| (i, Some(i as i64), None)),
                GroupSort::Time,
                || {
                    count.set(count.get() + 1);
                    count.get() > 5
                }
            )
            .is_none()
        );
        let mut rows: Vec<_> = (0..500)
            .map(|i| (i, (0..20).map(|j| ((i + j) % 17) as f64).collect()))
            .collect();
        let count = Cell::new(0);
        assert!(
            principal_scores(&mut rows, &|| {
                count.set(count.get() + 1);
                count.get() > 1010
            })
            .is_none()
        );
    }
}
