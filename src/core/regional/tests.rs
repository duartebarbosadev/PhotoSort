use super::*;
use std::cell::Cell;

fn exhaustive(a: &Features, b: &Features, eps: f32) -> Result<(bool, usize)> {
    let regions = if a.rows.len() == b.rows.len() && a.rows.len() > 1 {
        a.rows.len()
    } else {
        a.rows.len() * b.rows.len()
    };
    Ok((a.distance(b)? <= eps, regions))
}
fn random(state: &mut u64) -> f32 {
    *state = state
        .wrapping_mul(6364136223846793005)
        .wrapping_add(1442695040888963407);
    ((*state >> 32) as u32 as f64 / u32::MAX as f64 * 2. - 1.) as f32
}
fn evidence(photo: usize) -> Vec<Vec<f32>> {
    let mut state = (photo / 4 + 1) as u64;
    (0..6)
        .map(|_| {
            (0..384)
                .map(|i| random(&mut state) + (photo % 4) as f32 * ((i % 7) as f32 - 3.) * 0.0001)
                .collect()
        })
        .collect()
}
fn inputs(count: usize) -> Vec<Input> {
    (0..count)
        .map(|i| Input {
            key: i.to_string(),
            portrait: false,
        })
        .collect()
}
fn check_pair(a: &Features, b: &Features) {
    let distance = a.distance(b).unwrap();
    for eps in [
        0.,
        0.02,
        DEFAULT_EPS,
        0.2,
        1.,
        2.,
        distance.next_down().max(0.),
        distance,
        distance.next_up(),
    ] {
        let (close, count) = a.within_distance(b, eps).unwrap();
        let (expected, full) = exhaustive(a, b, eps).unwrap();
        assert_eq!(
            close, expected,
            "distance {distance}, eps {eps}, evaluated {count}/{full}"
        );
        assert!((1..=full).contains(&count));
    }
}

#[test]
fn early_rejection_matches_exhaustive_at_adjacent_float_thresholds_and_degenerate_inputs() {
    let mut state = 91;
    for width in [1, 2, 17, 384, 1025] {
        for rows in [1, 2, 6, 17] {
            for case in 0..12 {
                let raw: Vec<Vec<f32>> = (0..rows)
                    .map(|_| (0..width).map(|_| random(&mut state)).collect())
                    .collect();
                let mut changed = raw.clone();
                for (region, row) in changed.iter_mut().enumerate() {
                    for x in row {
                        *x = match case {
                            0 => *x,
                            1 => -*x,
                            2 => 0.,
                            3 if region > 0 => 0.,
                            4 => f32::from_bits(1),
                            5 => *x * f32::MAX,
                            6 => *x * f32::MIN_POSITIVE,
                            _ => *x + random(&mut state) * (case - 6) as f32 / 5.,
                        };
                    }
                }
                let a = Features::new(raw).unwrap();
                let b = Features::new(changed).unwrap();
                check_pair(&a, &b);
                check_pair(&b, &a);
                check_pair(&b, &b);
            }
        }
    }
    let single = Features::new(vec![vec![1., 0.]]).unwrap();
    let incomplete = Features::new(vec![vec![0., 1.], vec![1., 0.], vec![0., 0.]]).unwrap();
    check_pair(&single, &incomplete);
    check_pair(&incomplete, &single);
    assert!(
        single
            .within_distance(&Features::new(vec![vec![1.]]).unwrap(), DEFAULT_EPS)
            .is_err()
    );
}

#[test]
fn early_rejection_skips_impossible_regions_without_skipping_possible_matches() {
    let a = Features::new(vec![vec![1., 0.]; 6]).unwrap();
    let b = Features::new(vec![vec![0., 1.]; 6]).unwrap();
    assert_eq!(a.within_distance(&b, DEFAULT_EPS).unwrap(), (false, 1));
    let mut rescued = vec![vec![1., 0.]; 6];
    rescued[0] = vec![0.7, (1.0f32 - 0.49).sqrt()];
    let rescued = Features::new(rescued).unwrap();
    assert_eq!(a.within_distance(&rescued, DEFAULT_EPS).unwrap(), (true, 6));
    let mut last_differs = a.rows.clone();
    last_differs[5] = vec![0., 1.];
    assert_eq!(
        a.within_distance(&Features::new(last_differs).unwrap(), DEFAULT_EPS)
            .unwrap(),
        (false, 6)
    );
}

#[test]
fn pruned_clustering_keeps_labels_loads_bounds_transitivity_and_cancellation() {
    let mut records = inputs(260);
    let mut features: Vec<_> = (0..260)
        .map(|i| Features::new(evidence(i)).unwrap())
        .collect();
    // Nonidentical chain: only adjacent endpoints are neighbors, plus zero and
    // single-row evidence to exercise fallback, deduplication and both partitions.
    for (i, angle) in [0f32, 15., 30., 45.].into_iter().enumerate() {
        let mut v = vec![0.; 384];
        v[0] = angle.to_radians().cos();
        v[1] = angle.to_radians().sin();
        features[i] = Features::new(vec![v; 6]).unwrap();
    }
    features[4] = Features::new(vec![vec![0.; 384]; 6]).unwrap();
    features[5] = features[4].clone();
    features[6] = Features::new(vec![features[7].rows[0].clone()]).unwrap();
    for record in records.iter_mut().skip(240) {
        record.portrait = true;
    }
    for eps in [0.02, DEFAULT_EPS, 0.2, 1., 2.] {
        let load = |key: &str| Ok(features[key.parse::<usize>().unwrap()].clone());
        let baseline = cluster_with(&records, eps, load, || false, exhaustive)
            .unwrap()
            .unwrap();
        let optimized = cluster(&records, eps, load, || false).unwrap().unwrap();
        assert_eq!(baseline.labels, optimized.labels);
        assert_eq!(baseline.stats.comparisons, optimized.stats.comparisons);
        assert_eq!(
            baseline.stats.evidence_loads,
            optimized.stats.evidence_loads
        );
        assert_eq!(
            baseline.stats.peak_resident_records,
            optimized.stats.peak_resident_records
        );
        assert!(optimized.stats.peak_resident_records <= 128);
        assert!(optimized.stats.region_comparisons <= baseline.stats.region_comparisons);
        if eps == DEFAULT_EPS {
            assert_eq!(optimized.labels[0], optimized.labels[3]);
            assert!(optimized.stats.region_comparisons * 4 < baseline.stats.region_comparisons);
        }
    }
    let checks = Cell::new(0);
    let load = |key: &str| Ok(features[key.parse::<usize>().unwrap()].clone());
    assert!(
        cluster(&records, DEFAULT_EPS, load, || {
            checks.set(checks.get() + 1);
            checks.get() > 600
        })
        .unwrap()
        .is_none()
    );
}

#[test]
#[ignore = "controlled release benchmark; set PHOTOSORT_REGIONAL_MODE and PHOTOSORT_REGIONAL_COUNT"]
fn regional_pruning_benchmark() -> Result<()> {
    use crate::core::subjects::store::Store;
    let mode = std::env::var("PHOTOSORT_REGIONAL_MODE")?;
    ensure!(
        matches!(mode.as_str(), "exhaustive" | "pruned"),
        "invalid mode"
    );
    let count: usize = std::env::var("PHOTOSORT_REGIONAL_COUNT")?.parse()?;
    ensure!(
        (4..=4096).contains(&count) && count.is_multiple_of(4),
        "invalid count"
    );
    let root = tempfile::tempdir()?;
    let store = Store::new(root.path().into(), false)?;
    let mut records = inputs(count);
    for (i, input) in records.iter_mut().enumerate() {
        input.key = Store::regional_key(&input.key);
        store.put_regional(
            &input.key,
            &Embedding {
                vectors: evidence(i),
            },
        )?;
    }
    let mut runs = Vec::new();
    for round in 0..4 {
        let started = std::time::Instant::now();
        let result = cluster_with(
            &records,
            DEFAULT_EPS,
            |key| Features::new(store.regional(key).unwrap().vectors),
            || false,
            if mode == "pruned" {
                Features::within_distance
            } else {
                exhaustive
            },
        )?
        .unwrap();
        let elapsed = started.elapsed().as_secs_f64();
        for i in 0..count {
            assert_eq!(result.labels[i], result.labels[i / 4 * 4]);
        }
        assert_eq!(
            result
                .labels
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len(),
            count / 4
        );
        if round > 0 {
            runs.push(serde_json::json!({
            "seconds":elapsed, "comparisons":result.stats.comparisons,
            "regions":result.stats.region_comparisons, "loads":result.stats.evidence_loads,
            "peak_descriptors":result.stats.peak_resident_records,
            "labels_hash":blake3::hash(&result.labels.iter().flat_map(|n| (*n as u64).to_le_bytes()).collect::<Vec<_>>()).to_hex().to_string()
        }));
        }
    }
    println!(
        "REGIONAL_BENCH {}",
        serde_json::json!({"mode":mode,"records":count,"runs":runs})
    );
    Ok(())
}

#[cfg(feature = "onnx")]
#[test]
#[ignore = "seven read-only samples and native model runtime required"]
fn seven_sample_regions_keep_grouping_and_reuse_image_model_work() -> Result<()> {
    use crate::core::{catalog::Catalog, models::Models, operations, pipeline::Pipeline};
    let path = std::path::PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap());
    let catalog = Catalog::scan(&path, || false, |_| {})?;
    assert_eq!(catalog.media.len(), 7);
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let mut models = Models::default();
    let mut rows = Vec::new();
    let mut hashes = Vec::new();
    let mut records = inputs(7);
    for (index, media) in catalog.media.iter().enumerate() {
        hashes.push(operations::content_hash(&media.path, || false)?);
        let asset = pipeline.analysis(media, || false)?;
        let global = models.embedding(&asset)?;
        rows.push(Features::new(
            models
                .similarity_regions(&asset, &global, || false)?
                .vectors,
        )?);
        records[index].portrait = portrait(asset.pixels.width(), asset.pixels.height());
    }
    let before = (
        pipeline.stats.decodes,
        models.initializations,
        models.inferences,
    );
    for eps in [0.02, DEFAULT_EPS, 0.2] {
        let load = |key: &str| Ok(rows[key.parse::<usize>().unwrap()].clone());
        let baseline = cluster_with(&records, eps, load, || false, exhaustive)?.unwrap();
        let pruned = cluster(&records, eps, load, || false)?.unwrap();
        assert_eq!(baseline.labels, pruned.labels);
        for a in &rows {
            for b in &rows {
                check_pair(a, b);
            }
        }
        println!(
            "seven samples eps={eps}: regional dots {} -> {}",
            baseline.stats.region_comparisons, pruned.stats.region_comparisons
        );
    }
    assert_eq!(
        before,
        (
            pipeline.stats.decodes,
            models.initializations,
            models.inferences
        )
    );
    assert_eq!(models.initializations, 1);
    for (media, hash) in catalog.media.iter().zip(hashes) {
        assert_eq!(operations::content_hash(&media.path, || false)?, hash);
    }
    Ok(())
}
