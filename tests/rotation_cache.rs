use photosort::core::{
    cache,
    catalog::Catalog,
    metadata,
    model::Media,
    models::{Models, RotationSuggestion},
    pipeline::Pipeline,
    rotation::Detector,
    subjects::store::Store,
};
use std::{
    fs,
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

fn suggestion() -> RotationSuggestion {
    RotationSuggestion {
        turns: 2,
        confidence: 0.7345679,
        uncertain: true,
        views: 3,
    }
}
fn fixture(root: &Path) -> (Media, Detector) {
    // Invalid image/model bytes prove a cache hit cannot silently load either.
    fs::write(root.join("photo.png"), b"undecodable photo").unwrap();
    fs::write(root.join("model.onnx"), b"not a native model").unwrap();
    let media = Catalog::scan(root, || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    (media, Detector::new(root.join("model.onnx")).unwrap())
}
#[test]
fn rotation_cache_restart_preserves_uncertainty_without_pixels_metadata_or_models() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let (media, detector) = fixture(root.path());
    let key = detector.cache_key(&media).unwrap();
    let expected = suggestion();
    Store::new(cache.path().into(), true)
        .unwrap()
        .put_rotation(&key, &expected)
        .unwrap();
    for _ in 0..3 {
        let store = Store::new(cache.path().into(), true).unwrap();
        let mut pipeline = Pipeline::new(1024, cache.path().into());
        let mut models = Models::default();
        assert_eq!(
            detector
                .detect(&media, &mut pipeline, &mut models, &store, &|| false)
                .unwrap(),
            expected
        );
        assert_eq!(
            (
                pipeline.stats.decodes,
                pipeline.stats.metadata_reads,
                pipeline.stats.disk_hits
            ),
            (0, 0, 0)
        );
        assert_eq!(
            (pipeline.stats.resident_bytes, pipeline.stats.detail_bytes),
            (0, 0)
        );
        assert_eq!((models.initializations, models.inferences), (0, 0));
        assert!(
            store.aesthetic(&key).is_none(),
            "Binary evidence kinds must remain distinct"
        );
    }
}
#[test]
fn rotation_cache_validates_semantics_and_rejects_corrupt_evidence() {
    let cache = tempfile::tempdir().unwrap();
    let key = Store::rotation_key("source", "model");
    let store = Store::new(cache.path().into(), true).unwrap();
    for turns in 0..=4 {
        for views in 0..=4 {
            for uncertain in [false, true] {
                let value = RotationSuggestion {
                    turns,
                    views,
                    uncertain,
                    confidence: 0.7,
                };
                let valid = turns < 4
                    && match views {
                        1 => turns == 0 && !uncertain,
                        2 => turns != 0 && !uncertain,
                        3 => !uncertain || turns != 0,
                        _ => false,
                    };
                assert_eq!(store.put_rotation(&key, &value).is_ok(), valid);
            }
        }
    }
    for confidence in [f32::NAN, f32::INFINITY, -0.1, 1.1] {
        assert!(
            store
                .put_rotation(
                    &key,
                    &RotationSuggestion {
                        confidence,
                        ..suggestion()
                    }
                )
                .is_err()
        );
    }
    store.put_rotation(&key, &suggestion()).unwrap();
    let path = cache.path().join(format!("{key}.bin"));
    let mut bytes = fs::read(&path).unwrap();
    *bytes.last_mut().unwrap() ^= 1;
    fs::write(&path, bytes).unwrap();
    let reopened = Store::new(cache.path().into(), true).unwrap();
    assert!(reopened.rotation(&key).is_none());
    reopened.put_rotation(&key, &suggestion()).unwrap();
    assert_eq!(reopened.rotation(&key), Some(suggestion()));
    assert_ne!(key, Store::rotation_key("changed source", "model"));
    assert_ne!(key, Store::rotation_key("source", "changed model"));
    assert_ne!(key, Store::aesthetic_key("source", "model"));
}
#[test]
fn rotation_cache_tracks_sidecars_photo_identity_and_model_replacements() {
    let root = tempfile::tempdir().unwrap();
    let (media, detector) = fixture(root.path());
    let first = detector.cache_key(&media).unwrap();
    metadata::write_sidecar(&media.path, Some(3), Some(6)).unwrap();
    let sidecar = detector.cache_key(&media).unwrap();
    assert_ne!(first, sidecar);
    fs::remove_file(metadata::sidecar(&media.path)).unwrap();
    assert_eq!(first, detector.cache_key(&media).unwrap());
    fn replace(path: &Path) {
        let old = fs::metadata(path).unwrap();
        let replacement = path.with_extension("replacement");
        fs::copy(path, &replacement).unwrap();
        fs::File::options()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(old.modified().unwrap()))
            .unwrap();
        fs::rename(replacement, path).unwrap();
    }
    replace(&media.path);
    assert_ne!(
        first,
        detector.cache_key(&media).unwrap(),
        "Same size and mtime cannot hide a replacement"
    );
    let key = detector.cache_key(&media).unwrap();
    replace(&root.path().join("model.onnx"));
    let updated = Detector::new(root.path().join("model.onnx")).unwrap();
    assert_ne!(key, updated.cache_key(&media).unwrap());
}
#[test]
fn rotation_cache_cancellation_and_changed_inputs_reject_hits_before_publication() {
    for change in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let (media, detector) = fixture(root.path());
        let store = Store::new(cache.path().into(), true).unwrap();
        store
            .put_rotation(&detector.cache_key(&media).unwrap(), &suggestion())
            .unwrap();
        let calls = AtomicUsize::new(0);
        let mut pipeline = Pipeline::new(1024, cache.path().into());
        let mut models = Models::default();
        let result = detector.detect(&media, &mut pipeline, &mut models, &store, &|| {
            calls.fetch_add(1, Ordering::Relaxed);
            if change == 0 {
                return true;
            }
            if calls.load(Ordering::Relaxed) == 2 {
                match change {
                    1 => return true,
                    2 => fs::write(&media.path, b"changed source").unwrap(),
                    _ => fs::write(root.path().join("model.onnx"), b"changed model").unwrap(),
                }
            }
            false
        });
        let error = result.unwrap_err().to_string();
        assert!(
            error.contains("cancelled") || error.contains("changed"),
            "{error}"
        );
        assert_eq!(
            (
                models.initializations,
                models.inferences,
                pipeline.stats.decodes,
                pipeline.stats.metadata_reads
            ),
            (0, 0, 0, 0)
        );
    }
}
#[test]
fn rotation_cache_obeys_persistence_budget_and_active_working_set_ownership() {
    let cache = tempfile::tempdir().unwrap();
    let key = Store::rotation_key("source", "model");
    let store = Store::new(cache.path().into(), true).unwrap();
    store.put_rotation(&key, &suggestion()).unwrap();
    let private = Store::new(cache.path().into(), false).unwrap();
    assert!(
        private.rotation(&key).is_none(),
        "Disabled persistence cannot revive disk records"
    );
    let private_key = Store::rotation_key("private source", "model");
    private.put_rotation(&private_key, &suggestion()).unwrap();
    assert_eq!(private.rotation(&private_key), Some(suggestion()));
    assert!(!cache.path().join(format!("{private_key}.bin")).exists());
    fs::write(cache.path().join("foreign.txt"), b"keep").unwrap();
    cache::maintain(cache.path(), Some(0), || false).unwrap();
    assert_eq!(
        store.rotation(&key),
        Some(suggestion()),
        "Active snapshots survive cache trimming"
    );
    assert!(
        Store::new(cache.path().into(), true)
            .unwrap()
            .rotation(&key)
            .is_none()
    );
    assert!(cache.path().join("foreign.txt").exists());
}

#[test]
#[cfg(feature = "onnx")]
#[ignore = "requires all seven reference photos and installed native orientation model"]
fn native_rotation_cache_reuses_seven_samples_across_runs_restart_and_invalidation() {
    use photosort::core::{
        cache::Limits,
        jobs::{AnalysisStats, Event, Runtime},
        model::MediaId,
        models::orientation_model,
        operations::content_hash,
        pipeline::Stats,
    };
    use std::{
        collections::HashMap,
        path::PathBuf,
        sync::Arc,
        time::{Duration, Instant},
    };
    fn wait_cache(rt: &Runtime) {
        let deadline = Instant::now() + crate::test_support::worker_patience();
        while rt.cache.status().busy {
            assert!(Instant::now() < deadline);
            std::thread::sleep(Duration::from_millis(5));
        }
    }
    fn stats(rt: &Runtime) -> Stats {
        wait_cache(rt);
        while rt.events.try_recv().is_ok() {}
        assert!(rt.cache.request(false));
        let deadline = Instant::now() + crate::test_support::worker_patience();
        loop {
            assert!(Instant::now() < deadline);
            if let Ok(e) = rt.events.recv_timeout(Duration::from_millis(100))
                && let Event::CacheStats(stats) = e.event
            {
                return stats;
            }
        }
    }
    fn run(
        rt: &Runtime,
        root: &Path,
        model: &Path,
    ) -> (AnalysisStats, HashMap<MediaId, RotationSuggestion>) {
        let media = Arc::new(Catalog::scan(root, || false, |_| {}).unwrap().media);
        let run = rt.orient(media, model.into()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(180);
        let mut values = HashMap::new();
        loop {
            assert!(Instant::now() < deadline, "Rotation worker timed out");
            match rt.events.recv_timeout(Duration::from_millis(250)) {
                Ok(envelope) => match envelope.event {
                    Event::Rotation {
                        run: found,
                        id,
                        suggestion,
                    } if found == run => {
                        values.insert(id, suggestion);
                    }
                    Event::AnalysisDone { run: found, stats } if found == run => {
                        return (stats, values);
                    }
                    Event::AnalysisError { error, .. } | Event::Error(error) => panic!("{error}"),
                    Event::ModelsNeeded { .. } => {
                        panic!("Install the native orientation model/runtime")
                    }
                    _ => {}
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
    let samples =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let originals = Catalog::scan(&samples, || false, |_| {}).unwrap().media;
    assert_eq!(originals.len(), 7);
    let hashes: Vec<_> = originals
        .iter()
        .map(|m| content_hash(&m.path, || false).unwrap())
        .collect();
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    for media in &originals {
        fs::copy(
            &media.path,
            root.path().join(media.path.file_name().unwrap()),
        )
        .unwrap();
    }
    let model = orientation_model().expect("Install orientation model");
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    let start = Instant::now();
    let (cold, expected) = run(&rt, root.path(), &model);
    let cold_time = start.elapsed();
    assert_eq!(expected.len(), 7);
    assert_eq!(
        (
            cold.decodes,
            cold.model_initializations,
            cold.model_inferences
        ),
        (7, 1, 7)
    );
    let cold_pipeline = stats(&rt);
    assert_eq!(cold_pipeline.metadata_reads, 7);
    for new_epoch in [false, true] {
        if new_epoch {
            rt.next_epoch();
        }
        let (warm, actual) = run(&rt, root.path(), &model);
        assert_eq!(actual, expected);
        assert_eq!(warm, cold);
        assert_eq!(stats(&rt).metadata_reads, cold_pipeline.metadata_reads);
    }
    rt.shutdown();
    // Remove proxy files so warm runs cannot conceal a second pixel load.
    for entry in fs::read_dir(cache.path()).unwrap().flatten() {
        if entry.path().extension().is_some_and(|v| v == "png") {
            fs::remove_file(entry.path()).unwrap();
        }
    }
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    let start = Instant::now();
    let (warm, actual) = run(&rt, root.path(), &model);
    let warm_time = start.elapsed();
    assert_eq!(actual, expected);
    assert_eq!(
        (
            warm.decodes,
            warm.model_initializations,
            warm.model_inferences
        ),
        (0, 0, 0)
    );
    let empty = stats(&rt);
    assert_eq!(
        (
            empty.metadata_reads,
            empty.disk_hits,
            empty.resident_bytes,
            empty.detail_bytes
        ),
        (0, 0, 0, 0)
    );
    let media = Catalog::scan(root.path(), || false, |_| {}).unwrap().media;
    metadata::write_sidecar(&media[0].path, Some(3), Some(6)).unwrap();
    let (changed, _) = run(&rt, root.path(), &model);
    assert_eq!(changed.decodes, 1);
    assert_eq!(changed.model_initializations, 1);
    assert!((1..=3).contains(&changed.model_inferences));
    assert_eq!(stats(&rt).metadata_reads, 1);
    let detector = Detector::new(model.clone()).unwrap();
    let corrupt_key = detector.cache_key(&media[1]).unwrap();
    fs::write(
        cache.path().join(format!("{corrupt_key}.bin")),
        b"corrupt cache",
    )
    .unwrap();
    rt.next_epoch();
    let (repaired, _) = run(&rt, root.path(), &model);
    assert_eq!(repaired.decodes, changed.decodes + 1);
    assert!(repaired.model_inferences > changed.model_inferences);
    assert!(
        Store::new(cache.path().into(), true)
            .unwrap()
            .rotation(&corrupt_key)
            .is_some()
    );
    wait_cache(&rt);
    assert!(rt.cache.request(true));
    wait_cache(&rt);
    let (cleared, _) = run(&rt, root.path(), &model);
    assert_eq!(cleared.decodes, repaired.decodes + 7);
    assert!(cleared.model_inferences >= repaired.model_inferences + 7);
    rt.cache.configure(Limits {
        disk_mib: 0,
        ..Default::default()
    });
    wait_cache(&rt);
    rt.next_epoch();
    let (private_cold, private_expected) = run(&rt, root.path(), &model);
    let (private_warm, private_actual) = run(&rt, root.path(), &model);
    assert!(private_cold.model_inferences >= cleared.model_inferences + 7);
    assert_eq!(private_warm, private_cold);
    assert_eq!(private_actual, private_expected);
    assert_eq!(
        cache::maintain(cache.path(), None, || false).unwrap().files,
        0
    );
    rt.shutdown();
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    rt.cache.configure(Limits {
        disk_mib: 0,
        ..Default::default()
    });
    wait_cache(&rt);
    let (private_restart, _) = run(&rt, root.path(), &model);
    assert_eq!(private_restart.decodes, 7);
    assert_eq!(private_restart.model_initializations, 1);
    assert!(private_restart.model_inferences >= 7);
    rt.shutdown();
    for (media, hash) in originals.iter().zip(hashes) {
        assert_eq!(content_hash(&media.path, || false).unwrap(), hash);
    }
    eprintln!(
        "Rotation cache: cold {cold_time:?}, restarted {warm_time:?}; cold {cold:?}; restarted {warm:?}; warm metadata reads 0; invalidation/corruption/clear/zero-persistence passed"
    );
}

#[test]
fn working_image_rotation_recipe_does_not_reuse_old_analysis_predictions() {
    let cache = tempfile::tempdir().unwrap();
    let store = Store::new(cache.path().into(), true).unwrap();
    let old = blake3::hash(
        b"rotation-v1-neutral1024-triangle416-crop384-imagenet-consensus021:source:model",
    )
    .to_hex()
    .to_string();
    let current = Store::rotation_key("source", "model");
    store.put_rotation(&old, &suggestion()).unwrap();
    assert_ne!(old, current);
    assert!(store.rotation(&current).is_none());
    store.put_rotation(&current, &suggestion()).unwrap();
    assert_eq!(store.rotation(&current), Some(suggestion()));
}
