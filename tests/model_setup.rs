use crate::test_support;
use photosort::core::{
    model_assets::{self, Health, Id, Work, Worker},
    models,
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
fn wait(worker: &Worker) -> model_assets::State {
    wait_within(worker, Duration::from_secs(240))
}
fn wait_within(worker: &Worker, limit: Duration) -> model_assets::State {
    let until = Instant::now() + limit;
    loop {
        let state = worker.state();
        if !state.running {
            return state;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
}
#[test]
#[ignore = "Verifies every local prepared model asset and restores a disposable copy"]
fn prepared_bundle_verifies_once_and_import_reuses_the_streamed_checksum() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("resources");
    let dir = tempfile::tempdir().unwrap();
    let mut worker = Worker::new(vec![dir.path().into(), source.clone()], Arc::new(|| {}));
    worker.start(Work::Inspect).unwrap();
    let first = wait(&worker);
    assert!(first.error.is_none(), "{:?}", first.error);
    let first = first.snapshot.unwrap();
    assert_eq!(first.rows.len(), photosort::core::model_assets::ALL.len());
    assert!(
        first
            .rows
            .iter()
            .all(|r| matches!(r.health, Health::Verified(_)))
    );
    assert_eq!(first.hash_reads, photosort::core::model_assets::ALL.len());
    worker.start(Work::Inspect).unwrap();
    let second = wait(&worker).snapshot.unwrap();
    assert_eq!(
        second.hash_reads, 6,
        "Refresh must reuse source identity checks"
    );
    worker.start(Work::Restore(Id::Detector)).unwrap();
    let restored = wait(&worker);
    assert!(restored.error.is_none(), "{:?}", restored.error);
    let restored = restored.snapshot.unwrap();
    assert_eq!(
        restored.hash_reads, 6,
        "Do not rehash a file already verified while copying"
    );
    let row = restored.rows.iter().find(|r| r.id == Id::Detector).unwrap();
    assert!(matches!(&row.health,Health::Verified(path) if path.starts_with(dir.path())));
    let source_hash = photosort::core::operations::content_hash(
        &source.join("models").join(Id::Detector.asset().name),
        || false,
    )
    .unwrap();
    assert_eq!(
        source_hash,
        photosort::core::operations::content_hash(
            &dir.path().join("models").join(Id::Detector.asset().name),
            || false
        )
        .unwrap()
    );
    let reviewed = row.managed.clone().unwrap();
    worker.start(Work::Remove(reviewed)).unwrap();
    let removed = wait(&worker);
    assert!(removed.error.is_none(), "{:?}", removed.error);
    let snapshot = removed.snapshot.unwrap();
    assert_eq!(
        snapshot.hash_reads, 6,
        "Removing a managed copy must reuse bundled verification"
    );
    let row = snapshot.rows.iter().find(|r| r.id == Id::Detector).unwrap();
    assert!(row.managed.is_none());
    assert!(matches!(&row.health, Health::Verified(path) if path.starts_with(&source)));
    assert_eq!(
        source_hash,
        photosort::core::operations::content_hash(
            &source.join("models").join(Id::Detector.asset().name),
            || false
        )
        .unwrap()
    );
    assert!(models::runtime_path().is_some());
    worker.shutdown();
}
#[test]
#[cfg(feature = "model-downloads")]
#[ignore = "Downloads the pinned 38.7 MB SFace model into a disposable directory"]
fn pinned_native_model_download_verifies_and_never_changes_bundled_assets() {
    pinned_download(Id::People);
}

#[test]
#[cfg(feature = "model-downloads")]
#[ignore = "Downloads the pinned 80.6 MB orientation model into a disposable directory"]
fn pinned_orientation_download_installs_without_a_bundle_and_reuses_verification() {
    pinned_download(Id::Rotation);
}

#[test]
#[cfg(feature = "model-downloads")]
#[ignore = "Downloads the three pinned MediaPipe models (7.3 MB) from the models-2 release"]
fn pinned_converted_face_model_downloads_install_without_a_bundle() {
    for id in [Id::Detector, Id::Landmarks, Id::Blendshapes] {
        pinned_download(id);
    }
}

#[test]
#[cfg(feature = "model-downloads")]
#[ignore = "Downloads the pinned 534.5 MB aesthetic model from the models-2 release"]
fn pinned_aesthetic_download_installs_without_a_bundle() {
    pinned_download(Id::Aesthetic);
}

#[cfg(feature = "model-downloads")]
fn pinned_download(id: Id) {
    let dir = tempfile::tempdir().unwrap();
    let mut worker = Worker::new(vec![dir.path().into()], Arc::new(|| {}));
    worker.start(Work::Inspect).unwrap();
    let before = wait(&worker);
    assert!(before.error.is_none(), "{:?}", before.error);
    assert!(matches!(
        before
            .snapshot
            .unwrap()
            .rows
            .iter()
            .find(|r| r.id == id)
            .unwrap()
            .health,
        Health::Missing
    ));
    worker.start(Work::Download(id)).unwrap();
    let state = wait_within(&worker, Duration::from_secs(1800));
    assert!(state.error.is_none(), "{:?}", state.error);
    let snapshot = state.snapshot.unwrap();
    let destination = dir.path().join("models").join(id.asset().name);
    assert!(matches!(
        &snapshot.rows.iter().find(|r| r.id == id).unwrap().health,
        Health::Verified(path) if path == &destination
    ));
    assert_eq!(
        std::fs::metadata(&destination).unwrap().len(),
        id.asset().bytes
    );
    assert_eq!(
        snapshot.hash_reads, 0,
        "The download stream already verified SHA-256"
    );
    worker.start(Work::Inspect).unwrap();
    let refreshed = wait(&worker);
    assert!(refreshed.error.is_none(), "{:?}", refreshed.error);
    assert_eq!(
        refreshed.snapshot.unwrap().hash_reads,
        0,
        "Refresh must reuse installation verification"
    );
    assert_eq!(
        std::fs::read_dir(dir.path().join("models"))
            .unwrap()
            .count(),
        1,
        "Only the requested model is installed; partial files are removed"
    );
    worker.shutdown();
}

#[test]
fn missing_explicit_orientation_model_is_reported_before_any_photo_decode() {
    use photosort::core::{
        jobs::{Event, Runtime},
        model::*,
    };
    let dir = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::with_cache(|| {}, dir.path().join("cache"));
    let media = Media {
        id: MediaId(1),
        path: dir.path().join("not-a-real-photo.jpg"),
        kind: MediaKind::Photo,
        fingerprint: Fingerprint::default(),
    };
    let run = runtime
        .orient(Arc::new(vec![media]), dir.path().join("missing.onnx"))
        .unwrap();
    let deadline = Instant::now() + test_support::worker_patience();
    loop {
        assert!(Instant::now() < deadline);
        match runtime.events.recv_timeout(Duration::from_millis(20)) {
            Ok(envelope) => match envelope.event {
                Event::ModelsNeeded { run: found, issue } => {
                    assert_eq!(found, run);
                    assert_eq!(issue.missing, vec![Id::Rotation]);
                    break;
                }
                Event::Error(error)
                | Event::AnalysisError { error, .. }
                | Event::AssetError { error, .. } => {
                    panic!("Prerequisites must fail before image work: {error}")
                }
                _ => {}
            },
            Err(_) => continue,
        }
    }
    runtime.cache.request(false);
    loop {
        assert!(Instant::now() < deadline);
        if let Ok(envelope) = runtime.events.recv_timeout(Duration::from_millis(20))
            && let Event::CacheStats(stats) = envelope.event
        {
            assert_eq!((stats.decodes, stats.metadata_reads), (0, 0));
            break;
        }
    }
    assert!(
        runtime.model_assets.state().snapshot.is_none(),
        "Preflight must not start a setup scan or download by itself"
    );
    runtime.shutdown();
}
