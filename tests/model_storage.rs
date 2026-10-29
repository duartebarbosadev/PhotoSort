use crate::test_support;
use photosort::core::model_assets::{self, Health, Id, Work, Worker};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("models")).unwrap();
    std::fs::write(
        dir.path().join("models").join(Id::Detector.asset().name),
        b"managed",
    )
    .unwrap();
    dir
}
fn wait(worker: &Worker) -> model_assets::State {
    let deadline = Instant::now() + test_support::worker_patience();
    loop {
        let expired = Instant::now() >= deadline;
        let state = worker.state();
        if !state.running {
            return state;
        }
        assert!(!expired);
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn removal_requires_the_reviewed_identity_and_root_and_reports_actual_bytes() {
    let dir = fixture();
    let other = fixture();
    let copy = model_assets::review_managed(dir.path(), Id::Detector)
        .unwrap()
        .unwrap();
    assert_eq!(copy.bytes(), 7);
    assert_eq!(copy.id(), Id::Detector);
    let report =
        model_assets::remove_reviewed(other.path(), std::slice::from_ref(&copy), &|| false)
            .unwrap();
    assert_eq!(report.errors.len(), 1);
    assert!(copy.path().exists());
    std::fs::write(copy.path(), b"replacement").unwrap();
    let report =
        model_assets::remove_reviewed(dir.path(), std::slice::from_ref(&copy), &|| false).unwrap();
    assert_eq!(report.errors.len(), 1);
    assert!(report.removed.is_empty());
    assert_eq!(std::fs::read(copy.path()).unwrap(), b"replacement");
    let current = model_assets::review_managed(dir.path(), Id::Detector)
        .unwrap()
        .unwrap();
    let report = model_assets::remove_reviewed(dir.path(), &[current], &|| false).unwrap();
    assert_eq!(report.bytes, 11);
    assert_eq!(report.removed, [copy.path().to_path_buf()]);
    assert!(report.errors.is_empty());
}
#[test]
fn cancellation_preserves_reviewed_copy_and_never_removes_unreviewed_files() {
    let dir = fixture();
    std::fs::write(dir.path().join("models/custom.onnx"), b"custom").unwrap();
    let copy = model_assets::review_managed(dir.path(), Id::Detector)
        .unwrap()
        .unwrap();
    let report =
        model_assets::remove_reviewed(dir.path(), std::slice::from_ref(&copy), &|| true).unwrap();
    assert!(report.cancelled && report.removed.is_empty());
    assert!(copy.path().is_file());
    model_assets::remove_reviewed(dir.path(), &[copy], &|| false).unwrap();
    assert_eq!(
        std::fs::read(dir.path().join("models/custom.onnx")).unwrap(),
        b"custom"
    );
}
#[cfg(unix)]
#[test]
fn replacement_links_and_linked_managed_roots_cannot_remove_external_models() {
    let dir = fixture();
    let outside = fixture();
    let copy = model_assets::review_managed(dir.path(), Id::Detector)
        .unwrap()
        .unwrap();
    let source = outside
        .path()
        .join("models")
        .join(Id::Detector.asset().name);
    std::fs::remove_file(copy.path()).unwrap();
    std::os::unix::fs::symlink(&source, copy.path()).unwrap();
    let report = model_assets::remove_reviewed(dir.path(), &[copy], &|| false).unwrap();
    assert_eq!(report.errors.len(), 1);
    assert!(model_assets::review_managed(dir.path(), Id::Detector).is_err());
    let snapshot = model_assets::Inspector::default()
        .inspect(&[dir.path().into()], &|| false)
        .unwrap();
    assert!(snapshot.rows.iter().all(|row| row.managed.is_none()));
    let linked = outside.path().join("linked");
    std::os::unix::fs::symlink(dir.path(), &linked).unwrap();
    assert!(model_assets::review_managed(&linked, Id::Detector).is_err());
    assert_eq!(std::fs::read(source).unwrap(), b"managed");
}
#[test]
fn worker_removes_only_selected_copy_and_publishes_fallback_or_missing_state() {
    for fallback in [false, true] {
        let dir = fixture();
        let bundle = fixture();
        let mut roots = vec![dir.path().into()];
        if fallback {
            roots.push(bundle.path().into());
        }
        let mut worker = Worker::new(roots, Arc::new(|| {}));
        worker.start(Work::Inspect).unwrap();
        let before = wait(&worker).snapshot.unwrap();
        let copy = before
            .rows
            .iter()
            .find(|row| row.id == Id::Detector)
            .unwrap()
            .managed
            .clone()
            .unwrap();
        worker.start(Work::Remove(copy)).unwrap();
        let after = wait(&worker);
        assert!(after.error.is_none(), "{:?}", after.error);
        assert!(!after.mutating && after.progress.is_none());
        assert!(after.message.starts_with("Removed managed copy"));
        let snapshot = after.snapshot.unwrap();
        let row = snapshot
            .rows
            .iter()
            .find(|row| row.id == Id::Detector)
            .unwrap();
        assert!(row.managed.is_none());
        if fallback {
            assert!(
                matches!(&row.health, Health::Invalid(path, _) if path.starts_with(bundle.path()))
            );
        } else {
            assert!(matches!(row.health, Health::Missing));
        }
        assert_eq!(
            std::fs::read(bundle.path().join("models").join(Id::Detector.asset().name)).unwrap(),
            b"managed"
        );
        worker.shutdown();
    }
}
#[test]
fn worker_failed_removal_invalidates_stale_readiness_until_refresh() {
    let dir = fixture();
    let mut worker = Worker::new(vec![dir.path().into()], Arc::new(|| {}));
    worker.start(Work::Inspect).unwrap();
    let snapshot = wait(&worker).snapshot.unwrap();
    let copy = snapshot
        .rows
        .iter()
        .find(|row| row.id == Id::Detector)
        .unwrap()
        .managed
        .clone()
        .unwrap();
    std::fs::write(copy.path(), b"new-model").unwrap();
    worker.start(Work::Remove(copy.clone())).unwrap();
    let after = wait(&worker);
    assert!(after.error.unwrap().contains("changed since review"));
    assert!(after.snapshot.is_none() && !after.mutating);
    worker.start(Work::Inspect).unwrap();
    let refreshed = wait(&worker).snapshot.unwrap();
    assert_eq!(
        refreshed
            .rows
            .iter()
            .find(|row| row.id == Id::Detector)
            .unwrap()
            .managed
            .as_ref()
            .unwrap()
            .bytes(),
        9
    );
    assert_eq!(std::fs::read(copy.path()).unwrap(), b"new-model");
    worker.shutdown();
}
