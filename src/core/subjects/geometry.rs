use super::{Artifact, Geometry, dot, normalized};
use nalgebra::{SMatrix, SVector};

pub fn verify_geometry(
    a: &Artifact,
    b: &Artifact,
    cancelled: impl Fn() -> bool,
) -> Option<Geometry> {
    if cancelled() {
        return None;
    }
    let evaluated = Geometry {
        evaluated: true,
        ..Default::default()
    };
    if a.subjects.len() < 4 || b.subjects.len() < 4 {
        return Some(evaluated);
    }
    let left: Vec<_> = a
        .subjects
        .iter()
        .map(|e| normalized(&e.descriptor))
        .collect();
    let right: Vec<_> = b
        .subjects
        .iter()
        .map(|e| normalized(&e.descriptor))
        .collect();
    let mut similarity = vec![vec![0.0; b.subjects.len()]; a.subjects.len()];
    for (i, left) in left.iter().enumerate() {
        if cancelled() {
            return None;
        }
        for (j, right) in right.iter().enumerate() {
            similarity[i][j] = f64::from(dot(left, right));
        }
    }
    let forward: Vec<_> = similarity
        .iter()
        .map(|row| {
            (0..row.len())
                .max_by(|&a, &b| row[a].total_cmp(&row[b]).then_with(|| b.cmp(&a)))
                .unwrap()
        })
        .collect();
    let reverse: Vec<_> = (0..b.subjects.len())
        .map(|j| {
            (0..a.subjects.len())
                .max_by(|&a, &b| {
                    similarity[a][j]
                        .total_cmp(&similarity[b][j])
                        .then_with(|| b.cmp(&a))
                })
                .unwrap()
        })
        .collect();
    let center = |bbox: [f32; 4]| {
        [
            (bbox[0] as f64 + bbox[2] as f64) / 2.0,
            (bbox[1] as f64 + bbox[3] as f64) / 2.0,
        ]
    };
    let pairs: Vec<_> = forward
        .into_iter()
        .enumerate()
        .filter(|(i, j)| reverse[*j] == *i && similarity[*i][*j] >= 0.72)
        .map(|(i, j)| (center(a.subjects[i].bbox), center(b.subjects[j].bbox)))
        .collect();
    if pairs.len() < 4
        || pairs
            .iter()
            .any(|(a, b)| a.iter().chain(b).any(|v| !v.is_finite()))
        || hull_area(pairs.iter().map(|p| p.0).collect()) < 1e-7
        || hull_area(pairs.iter().map(|p| p.1).collect()) < 1e-7
    {
        return Some(evaluated);
    }
    let mut best = Vec::new();
    let mut best_error = f64::INFINITY;
    let mut random = 0xffff_ffffu64;
    let mut limit = 2000;
    let mut iteration = 0;
    while iteration < limit {
        if cancelled() {
            return None;
        }
        iteration += 1;
        let mut sample = Vec::with_capacity(4);
        while sample.len() < 4 {
            random ^= random << 13;
            random ^= random >> 7;
            random ^= random << 17;
            let i = random as usize % pairs.len();
            if !sample.contains(&i) {
                sample.push(i);
            }
        }
        if !nondegenerate(&sample, &pairs) {
            continue;
        }
        let Some(h) = homography(sample.iter().map(|&i| pairs[i])) else {
            continue;
        };
        let (inliers, error) = inliers(&pairs, &h);
        if inliers.len() > best.len() || (inliers.len() == best.len() && error < best_error) {
            best = inliers;
            best_error = error;
            let probability = (best.len() as f64 / pairs.len() as f64).powi(4);
            if probability >= 1.0 {
                limit = iteration;
            } else if probability > 0.0 {
                limit = limit.min(
                    ((0.005f64.ln() / (1.0 - probability).ln()).ceil() as usize).max(iteration),
                );
            }
        }
    }
    if best.len() < 4 {
        return Some(evaluated);
    }
    if let Some(h) = homography(best.iter().map(|&i| pairs[i])) {
        best = inliers(&pairs, &h).0;
    }
    let coverage_a = hull_area(best.iter().map(|&i| pairs[i].0).collect());
    let coverage_b = hull_area(best.iter().map(|&i| pairs[i].1).collect());
    Some(Geometry {
        inlier_count: best.len(),
        inlier_ratio: best.len() as f64 / pairs.len() as f64,
        coverage_a,
        coverage_b,
        evaluated: true,
    })
}
// A four-point homography cannot be determined when any three sample points
// are collinear. Reject those samples before solving the normal equations.
fn nondegenerate(sample: &[usize], pairs: &[([f64; 2], [f64; 2])]) -> bool {
    for i in 0..sample.len() {
        for j in i + 1..sample.len() {
            for k in j + 1..sample.len() {
                for (a, b, c) in [
                    (pairs[sample[i]].0, pairs[sample[j]].0, pairs[sample[k]].0),
                    (pairs[sample[i]].1, pairs[sample[j]].1, pairs[sample[k]].1),
                ] {
                    if ((b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])).abs() < 1e-7
                    {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn homography(pairs: impl Iterator<Item = ([f64; 2], [f64; 2])>) -> Option<SVector<f64, 8>> {
    let mut ata = SMatrix::<f64, 8, 8>::zeros();
    let mut atb = SVector::<f64, 8>::zeros();
    for ([x, y], [u, v]) in pairs {
        for (row, value) in [
            ([x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y], u),
            ([0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y], v),
        ] {
            for i in 0..8 {
                atb[i] += row[i] * value;
                for j in 0..8 {
                    ata[(i, j)] += row[i] * row[j];
                }
            }
        }
    }
    let result = ata.lu().solve(&atb)?;
    result.iter().all(|x| x.is_finite()).then_some(result)
}
fn inliers(pairs: &[([f64; 2], [f64; 2])], h: &SVector<f64, 8>) -> (Vec<usize>, f64) {
    let mut indices = Vec::new();
    let mut error = 0.0;
    for (i, ([x, y], [u, v])) in pairs.iter().enumerate() {
        let divisor = h[6] * x + h[7] * y + 1.0;
        if divisor.abs() < 1e-12 {
            continue;
        }
        let e = ((h[0] * x + h[1] * y + h[2]) / divisor - u).powi(2)
            + ((h[3] * x + h[4] * y + h[5]) / divisor - v).powi(2);
        if e <= 0.12f64.powi(2) {
            indices.push(i);
            error += e;
        }
    }
    (indices, error)
}
fn hull_area(mut points: Vec<[f64; 2]>) -> f64 {
    points.sort_by(|a, b| a[0].total_cmp(&b[0]).then_with(|| a[1].total_cmp(&b[1])));
    points.dedup();
    if points.len() < 3 {
        return 0.0;
    }
    let cross = |a: [f64; 2], b: [f64; 2], c: [f64; 2]| {
        (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
    };
    // Build upper and lower chains independently to preserve the closing edge.
    let chain = |iter: Vec<[f64; 2]>| {
        let mut out = Vec::new();
        for point in iter {
            while out.len() >= 2 && cross(out[out.len() - 2], out[out.len() - 1], point) <= 0.0 {
                out.pop();
            }
            out.push(point);
        }
        out
    };
    let mut lower = chain(points.clone());
    let mut upper = chain(points.into_iter().rev().collect());
    lower.pop();
    upper.pop();
    lower.extend(upper);
    (0..lower.len())
        .map(|i| {
            let a = lower[i];
            let b = lower[(i + 1) % lower.len()];
            a[0] * b[1] - a[1] * b[0]
        })
        .sum::<f64>()
        .abs()
        / 2.0
}
