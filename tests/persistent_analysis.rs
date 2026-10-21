use photosort::core::{
    analysis::{Analysis, Thresholds},
    cache::{self, Limits},
    catalog::Catalog,
    jobs::{AnalysisProfile, AnalysisStats, Event, Runtime},
    metadata,
    model::{Media, MediaId},
    pipeline::{Pipeline, Stats},
    subjects::store::Store,
};
use std::{
    collections::HashMap,
    fs,
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

fn scan(root: &Path) -> Arc<Vec<Media>> {
    Arc::new(Catalog::scan(root, || false, |_| {}).unwrap().media)
}
fn run(
    rt: &Runtime,
    media: Arc<Vec<Media>>,
    profile: AnalysisProfile,
    thresholds: Thresholds,
) -> (AnalysisStats, HashMap<MediaId, Analysis>) {
    let accepted = rt
        .analyze_with_thresholds(media, profile, thresholds)
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut values = HashMap::new();
    loop {
        assert!(Instant::now() < deadline, "Analysis timed out");
        match rt.events.recv_timeout(Duration::from_secs(1)) {
            Ok(e) => match e.event {
                Event::Analyzed {
                    run, id, analysis, ..
                } if run == accepted => {
                    // Only this helper's accepted generation belongs to the result.
                    values.insert(id, analysis);
                }
                Event::AnalysisDone { run, stats } if run == accepted => return (stats, values),
                Event::AnalysisError { error, .. } | Event::Error(error) => panic!("{error}"),
                Event::ModelsNeeded { .. } => panic!("Required models unavailable"),
                _ => {}
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => panic!("{e}"),
        }
    }
}
fn usage(rt: &Runtime) -> Stats {
    // Subject/group completion may have already queued automatic cache trimming.
    wait_cache(rt);
    while rt.events.try_recv().is_ok() {}
    assert!(rt.cache.request(false));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(Instant::now() < deadline);
        if let Ok(e) = rt.events.recv_timeout(Duration::from_millis(100))
            && let Event::CacheStats(stats) = e.event
        {
            return stats;
        }
    }
}
fn wait_cache(rt: &Runtime) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while rt.cache.status().busy {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn create(root: &Path) {
    image::RgbImage::from_fn(640, 480, |x, y| {
        let v = if (x + y) % 2 == 0 { 127 } else { 129 };
        image::Rgb([v, v, v])
    })
    .save(root.join("photo.png"))
    .unwrap();
}
#[test]
fn restart_and_folder_return_reclassify_without_pixels_measurements_or_content_reads() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    create(root.path());
    let media = scan(root.path());
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    let (cold, first) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    assert_eq!(
        (
            cold.decodes,
            cold.technical_measurements,
            cold.content_hashes
        ),
        (1, 1, 1)
    );
    assert_eq!(first.values().next().unwrap().issues, ["Possibly soft"]);
    rt.shutdown();
    // No cached PNG can conceal a repeated pixel load on reopening.
    for e in fs::read_dir(cache.path()).unwrap().flatten() {
        if e.path().extension().is_some_and(|v| v == "png") {
            fs::remove_file(e.path()).unwrap();
        }
    }
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    for threshold in [10.0, 100.0] {
        rt.next_epoch();
        let (warm, results) = run(
            &rt,
            media.clone(),
            AnalysisProfile::Technical,
            Thresholds {
                blur: threshold,
                ..Default::default()
            },
        );
        assert_eq!(
            (
                warm.decodes,
                warm.technical_measurements,
                warm.quality_measurements,
                warm.content_hashes,
                warm.model_initializations,
                warm.model_inferences
            ),
            (0, 0, 0, 0, 0, 0)
        );
        assert_eq!(
            results.values().next().unwrap().issues.is_empty(),
            threshold == 10.0
        );
        let mut expected = first.values().next().unwrap().clone();
        expected.reclassify(Thresholds {
            blur: threshold,
            ..Default::default()
        });
        assert_eq!(
            serde_json::to_value(&expected).unwrap(),
            serde_json::to_value(results.values().next().unwrap()).unwrap()
        );
    }
    let stats = usage(&rt);
    assert_eq!(
        (stats.disk_hits, stats.resident_bytes, stats.detail_bytes),
        (0, 0, 0)
    );
    rt.shutdown();
}
#[test]
fn corrupt_records_recompute_and_source_replacement_and_sidecar_edits_invalidate() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    create(root.path());
    let media = scan(root.path());
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    run(
        &rt,
        media.clone(),
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    rt.shutdown();
    let key = Store::measurements_key(&Pipeline::key(&media[0]).unwrap());
    fs::write(
        cache.path().join(format!("{key}.bin")),
        b"broken binary envelope",
    )
    .unwrap();
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    let (repaired, _) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    assert_eq!(
        (repaired.technical_measurements, repaired.content_hashes),
        (1, 1)
    );
    metadata::write_sidecar(&media[0].path, Some(4), Some(6)).unwrap();
    let (sidecar, _) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    assert_eq!(
        (sidecar.technical_measurements, sidecar.content_hashes),
        (2, 2)
    );
    let value = Store::new(cache.path().into(), true)
        .unwrap()
        .measurements(&Store::measurements_key(&Pipeline::key(&media[0]).unwrap()))
        .unwrap();
    assert_eq!((value.width, value.height), (480, 640));
    let old = fs::metadata(&media[0].path).unwrap();
    let replacement = root.path().join("replacement.png");
    fs::copy(&media[0].path, &replacement).unwrap();
    fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(old.modified().unwrap()))
        .unwrap();
    fs::rename(replacement, &media[0].path).unwrap();
    let (replaced, _) = run(
        &rt,
        scan(root.path()),
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    assert_eq!(
        (replaced.technical_measurements, replaced.content_hashes),
        (3, 3)
    );
    rt.shutdown();
}
#[test]
fn disabled_persistence_and_explicit_clear_do_not_resurrect_measurements() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    create(root.path());
    let media = scan(root.path());
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    rt.cache.configure(Limits {
        disk_mib: 0,
        ..Default::default()
    });
    wait_cache(&rt);
    let (cold, _) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    let (warm, _) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    assert_eq!(cold, warm);
    assert_eq!(
        cache::maintain(cache.path(), None, || false).unwrap().files,
        0
    );
    rt.shutdown();
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    let (cold, _) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    assert_eq!(cold.technical_measurements, 1);
    assert!(rt.cache.request(true));
    wait_cache(&rt);
    let (cleared, _) = run(
        &rt,
        media,
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    assert_eq!(
        (cleared.technical_measurements, cleared.content_hashes),
        (2, 2)
    );
    rt.shutdown();
}
#[test]
fn evidence_validation_and_recipes_separate_measurements_from_ai_and_settings() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    create(root.path());
    let media = scan(root.path());
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    run(
        &rt,
        media.clone(),
        AnalysisProfile::Technical,
        Thresholds::default(),
    );
    rt.shutdown();
    let store = Store::new(cache.path().into(), true).unwrap();
    let source = Pipeline::key(&media[0]).unwrap();
    let key = Store::measurements_key(&source);
    let value = store.measurements(&key).unwrap();
    assert!(value.valid());
    for bad in 0..6 {
        let mut v = value.clone();
        match bad {
            0 => v.analysis.quality_sharpness = None,
            1 => v.width = 0,
            2 => v.analysis.issues.push("stale decision".into()),
            3 => v.analysis.aesthetic = Some(0.9),
            4 => v.content_hash = "bad".into(),
            _ => v.analysis.brightness = f64::NAN,
        }
        assert!(!v.valid());
        assert!(store.put_measurements(&key, &v).is_err());
    }
    let a = Store::aesthetic_key(&source, "model-a");
    let b = Store::aesthetic_key(&source, "model-b");
    assert_ne!(a, b);
    assert_ne!(key, a);
    store.put_aesthetic(&a, 0.75).unwrap();
    assert_eq!(store.aesthetic(&a), Some(0.75));
    assert_eq!(store.aesthetic(&b), None);
    assert!(store.measurements(&key).is_some());
    assert!(store.put_aesthetic(&a, f32::NAN).is_err());
}
#[test]
#[cfg(feature = "onnx")]
#[ignore = "requires prepared native ONNX models and all seven original samples"]
fn native_seven_sample_quality_restart_reuses_models_pixels_hashes_and_measurements() {
    let sample = std::path::PathBuf::from(
        std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"),
    );
    let originals = scan(&sample);
    assert_eq!(originals.len(), 7);
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let hashes: Vec<_> = originals
        .iter()
        .map(|m| photosort::core::operations::content_hash(&m.path, || false).unwrap())
        .collect();
    for m in originals.iter() {
        fs::copy(&m.path, root.path().join(m.path.file_name().unwrap())).unwrap();
    }
    let media = scan(root.path());
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    let start = Instant::now();
    let (cold, expected) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Quality,
        Thresholds::default(),
    );
    let cold_time = start.elapsed();
    assert_eq!(expected.len(), 7);
    assert_eq!((cold.technical_measurements, cold.content_hashes), (7, 7));
    assert!(cold.model_inferences > 0);
    rt.shutdown();
    for e in fs::read_dir(cache.path()).unwrap().flatten() {
        if e.path().extension().is_some_and(|v| v == "png") {
            fs::remove_file(e.path()).unwrap();
        }
    }
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    let start = Instant::now();
    let (warm, actual) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Quality,
        Thresholds::default(),
    );
    let warm_time = start.elapsed();
    assert_eq!(
        (
            warm.decodes,
            warm.technical_measurements,
            warm.content_hashes,
            warm.model_inferences,
            warm.model_initializations
        ),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(
        serde_json::to_value(expected).unwrap(),
        serde_json::to_value(actual).unwrap()
    );
    let (subjects, _) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Subjects,
        Thresholds::default(),
    );
    assert_eq!(warm, subjects);
    // General regional crops may be new; a second restart must reuse those too.
    run(
        &rt,
        media.clone(),
        AnalysisProfile::Visual,
        Thresholds::default(),
    );
    rt.shutdown();
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    for profile in [
        AnalysisProfile::Visual,
        AnalysisProfile::Mixed,
        AnalysisProfile::Quality,
        AnalysisProfile::Technical,
    ] {
        let (stats, values) = run(&rt, media.clone(), profile, Thresholds::default());
        assert_eq!(values.len(), 7);
        assert_eq!(
            (
                stats.decodes,
                stats.technical_measurements,
                stats.content_hashes,
                stats.model_inferences,
                stats.model_initializations
            ),
            (0, 0, 0, 0, 0)
        );
    }
    let stats = usage(&rt);
    assert_eq!(
        (stats.disk_hits, stats.resident_bytes, stats.detail_bytes),
        (0, 0, 0)
    );
    rt.shutdown();
    for (m, hash) in originals.iter().zip(hashes) {
        assert_eq!(
            photosort::core::operations::content_hash(&m.path, || false).unwrap(),
            hash
        );
    }
    eprintln!(
        "Seven-sample Quality: cold {cold_time:?}, reopened {warm_time:?}; cold {cold:?}, warm {warm:?}. All original hashes unchanged."
    );
}

#[test]
fn reopened_measurements_preserve_float_bits_at_threshold_boundaries() {
    let root = tempfile::tempdir().unwrap();
    let key = Store::measurements_key("roundtrip-source");
    let values = [
        3878.7204020833333,
        0.1,
        0.9999999999999999,
        3432.906535723644,
    ];
    for sharpness in values {
        let mut measured = photosort::core::analysis::Measurements {
            analysis: Analysis {
                sharpness,
                quality_sharpness: Some(sharpness),
                brightness: 100.0,
                score: 50.0,
                ..Default::default()
            },
            width: 1,
            height: 1,
            content_hash: "a".repeat(64),
        };
        Store::new(root.path().into(), true)
            .unwrap()
            .put_measurements(&key, &measured)
            .unwrap();
        let mut restored = Store::new(root.path().into(), true)
            .unwrap()
            .measurements(&key)
            .unwrap();
        assert_eq!(restored.analysis.sharpness.to_bits(), sharpness.to_bits());
        let threshold = Thresholds {
            blur: sharpness,
            ..Default::default()
        };
        measured.analysis.reclassify(threshold);
        restored.analysis.reclassify(threshold);
        assert_eq!(restored.analysis.issues, measured.analysis.issues);
    }
}
#[test]
fn shared_face_cache_and_legacy_evidence_do_not_request_pixels_or_initialize_models() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    let mut models = photosort::core::models::Models::default();
    let legacy = photosort::core::faces::Faces::default();
    for previous in [Some(&legacy), None] {
        let faces = photosort::core::faces::cached_lazy(
            &mut models,
            &store,
            "same-photo",
            previous,
            || panic!("Cache hit requested pixels"),
            || false,
        )
        .unwrap();
        assert_eq!(faces.count, 0);
    }
    assert!(
        photosort::core::faces::cached_lazy(
            &mut models,
            &store,
            "same-photo",
            None,
            || panic!("Cancelled work requested pixels"),
            || true
        )
        .is_err()
    );
    assert_eq!((models.initializations, models.inferences), (0, 0));
}

#[test]
#[cfg(feature = "onnx")]
#[ignore = "requires prepared native Quality and face models"]
fn native_portrait_quality_restart_preserves_face_evidence_without_pixel_requests() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let demo = image::load_from_memory(include_bytes!("fixtures/recognition/sface-demo.jpg"))
        .unwrap()
        .to_rgba8();
    image::imageops::crop_imm(&demo, 0, 12, 512, 488)
        .to_image()
        .save(root.path().join("portrait.png"))
        .unwrap();
    let media = scan(root.path());
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    let (cold, values) = run(
        &rt,
        media.clone(),
        AnalysisProfile::Quality,
        Thresholds::default(),
    );
    assert!(values.values().next().unwrap().faces.unwrap().count > 0);
    assert!(cold.model_inferences > 0);
    rt.shutdown();
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    let (warm, reopened) = run(&rt, media, AnalysisProfile::Quality, Thresholds::default());
    assert_eq!(
        (
            warm.decodes,
            warm.technical_measurements,
            warm.content_hashes,
            warm.model_initializations,
            warm.model_inferences
        ),
        (0, 0, 0, 0, 0)
    );
    assert_eq!(
        serde_json::to_value(values).unwrap(),
        serde_json::to_value(reopened).unwrap()
    );
    let stats = usage(&rt);
    assert_eq!(
        (stats.disk_hits, stats.resident_bytes, stats.detail_bytes),
        (0, 0, 0)
    );
    rt.shutdown();
}
