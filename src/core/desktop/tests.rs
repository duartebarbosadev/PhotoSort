use super::*;
use crate::test_support::worker_patience;
use std::{
    sync::{atomic::AtomicUsize, mpsc},
    time::{Duration, Instant},
};
fn wait(worker: &Worker) {
    let end = Instant::now() + worker_patience();
    while worker.running() {
        assert!(Instant::now() < end);
        thread::sleep(Duration::from_millis(2));
    }
}
fn photo(root: &std::path::Path) -> Media {
    let path = root.join("-photo '旅行' #1.jpg");
    std::fs::write(&path, b"fixture").unwrap();
    Media {
        id: super::super::model::MediaId::for_path(&path),
        kind: MediaKind::Photo,
        fingerprint: super::super::model::Fingerprint::read(&path).unwrap(),
        path,
    }
}
#[test]
fn open_folder_checks_directory_on_worker_without_creating_or_opening_files() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("cache ' café #1");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("keep.txt");
    std::fs::write(&file, b"unchanged").unwrap();
    let missing = dir.path().join("absent");
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let expected = folder.clone();
    let mut worker = Worker::with_launcher(
        Arc::default(),
        Arc::new(|| {}),
        Arc::new(move |r| {
            assert_eq!(r.path, expected);
            assert_eq!(r.action, Action::OpenFolder);
            count.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }),
    );
    for (path, valid) in [(&folder, true), (&file, false), (&missing, false)] {
        worker
            .start(0, Request::for_path(path, Action::OpenFolder).unwrap())
            .unwrap();
        wait(&worker);
        assert_eq!(worker.take().unwrap().result.is_ok(), valid);
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert!(!missing.exists());
    assert_eq!(std::fs::read(file).unwrap(), b"unchanged");
}
#[test]
fn exact_clicked_path_is_passed_once_without_photo_processing() {
    let dir = tempfile::tempdir().unwrap();
    let media = photo(dir.path());
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    let path = media.path.clone();
    let mut worker = Worker::with_launcher(
        Arc::default(),
        Arc::new(|| {}),
        Arc::new(move |request| {
            assert_eq!(request.path, path);
            assert_eq!(request.action, Action::Reveal);
            counter.fetch_add(1, Ordering::Relaxed);
            Ok(())
        }),
    );
    assert!(worker.thread.is_none());
    worker
        .start(0, Request::for_media(&media, Action::Reveal).unwrap())
        .unwrap();
    wait(&worker);
    assert!(worker.take().unwrap().result.is_ok());
    assert!(worker.take().is_none());
    assert_eq!(calls.load(Ordering::Relaxed), 1);
    assert_eq!(std::fs::read(&media.path).unwrap(), b"fixture");
}
#[test]
fn scanned_documents_and_folders_use_the_same_bounded_desktop_owner() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("notes ' café $(literal).txt");
    std::fs::write(&path, "unchanged document").unwrap();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let sent = requests.clone();
    let mut worker = Worker::with_launcher(
        Arc::default(),
        Arc::new(|| {}),
        Arc::new(move |request| {
            sent.lock().unwrap().push(request.clone());
            Ok(())
        }),
    );
    for path in [path.as_path(), dir.path()] {
        for action in [Action::Open, Action::Reveal] {
            let request = Request::for_path(path, action).unwrap();
            worker.start(0, request.clone()).unwrap();
            wait(&worker);
            assert!(worker.take().unwrap().result.is_ok());
            assert_eq!(requests.lock().unwrap().last(), Some(&request));
        }
    }
    assert_eq!(requests.lock().unwrap().len(), 4);
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "unchanged document"
    );
    assert!(Request::for_path(std::path::Path::new("relative.txt"), Action::Open).is_err());
    assert!(
        Request::for_path(
            std::path::Path::new("https://example.invalid"),
            Action::Open
        )
        .is_err()
    );
    worker.shutdown();
}

#[test]
fn terminal_resolves_files_to_their_original_folder_once_on_the_worker() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("photos ' café $(literal) & %PATH%");
    std::fs::create_dir(&nested).unwrap();
    let file = nested.join("photo.jpg");
    std::fs::write(&file, b"unchanged").unwrap();
    let launches = Arc::new(Mutex::new(Vec::new()));
    let calls = launches.clone();
    let mut worker = Worker::with_launcher(
        Arc::default(),
        Arc::new(|| {}),
        Arc::new(move |r| {
            calls
                .lock()
                .unwrap()
                .push((r.clone(), thread::current().id()));
            Ok(())
        }),
    );
    for path in [&file, &nested] {
        let request = Request::for_path(path, Action::Terminal).unwrap();
        worker.start(0, request.clone()).unwrap();
        wait(&worker);
        let completed = worker.take().unwrap();
        assert!(completed.result.is_ok());
        assert_eq!(completed.request, request);
    }
    let calls = launches.lock().unwrap();
    assert_eq!(calls.len(), 2);
    for (request, thread) in calls.iter() {
        assert_eq!(request.path, nested);
        assert_eq!(request.action, Action::Terminal);
        assert_ne!(*thread, thread::current().id());
    }
    drop(calls);
    std::fs::remove_file(&file).unwrap();
    worker
        .start(0, Request::for_path(&file, Action::Terminal).unwrap())
        .unwrap();
    wait(&worker);
    assert!(worker.take().unwrap().result.is_err());
    assert_eq!(launches.lock().unwrap().len(), 2);
    assert!(!worker.poll_terminals());
}

#[test]
fn missing_files_and_invalid_targets_never_launch_an_application() {
    let dir = tempfile::tempdir().unwrap();
    let mut media = photo(dir.path());
    assert!(Request::for_media(&media, Action::OpenVideo).is_err());
    let request = Request::for_media(&media, Action::Reveal).unwrap();
    std::fs::remove_file(&media.path).unwrap();
    let mut worker = Worker::with_launcher(
        Arc::default(),
        Arc::new(|| {}),
        Arc::new(|_| panic!("Unexpected desktop launch")),
    );
    worker.start(0, request).unwrap();
    wait(&worker);
    assert!(
        worker
            .take()
            .unwrap()
            .result
            .unwrap_err()
            .contains("File is unavailable")
    );
    media.path = "https://example.invalid/photo.jpg".into();
    assert!(Request::for_media(&media, Action::Reveal).is_err());
    worker
        .start(
            0,
            Request {
                path: dir.path().into(),
                action: Action::OpenVideo,
            },
        )
        .unwrap();
    wait(&worker);
    assert!(worker.take().unwrap().result.is_err());
}
#[test]
fn one_inflight_handoff_is_bounded_and_obsolete_failures_are_discarded() {
    let dir = tempfile::tempdir().unwrap();
    let media = photo(dir.path());
    let request = Request::for_media(&media, Action::Reveal).unwrap();
    let generation = Arc::new(AtomicU64::new(0));
    let (started, ready) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let blocked = Mutex::new(blocked);
    let mut worker = Worker::with_launcher(
        generation.clone(),
        Arc::new(|| {}),
        Arc::new(move |_| {
            started.send(()).unwrap();
            blocked.lock().unwrap().recv().unwrap();
            anyhow::bail!("Old folder error")
        }),
    );
    worker.start(0, request.clone()).unwrap();
    ready.recv_timeout(worker_patience()).unwrap();
    assert!(worker.start(0, request.clone()).is_err());
    generation.store(1, Ordering::Release);
    assert!(worker.start(0, request).is_err());
    release.send(()).unwrap();
    wait(&worker);
    assert!(worker.take().is_none());
}
#[test]
fn shutdown_does_not_wait_for_a_stalled_file_manager_or_publish_its_result() {
    let dir = tempfile::tempdir().unwrap();
    let media = photo(dir.path());
    let request = Request::for_media(&media, Action::Reveal).unwrap();
    let (started, ready) = mpsc::channel();
    let (release, blocked) = mpsc::channel();
    let (done, finished) = mpsc::channel();
    let blocked = Mutex::new(blocked);
    let mut worker = Worker::with_launcher(
        Arc::default(),
        Arc::new(|| panic!("Wake after shutdown")),
        Arc::new(move |_| {
            started.send(()).unwrap();
            blocked.lock().unwrap().recv().unwrap();
            done.send(()).unwrap();
            Ok(())
        }),
    );
    worker.start(0, request.clone()).unwrap();
    ready.recv_timeout(worker_patience()).unwrap();
    worker.shutdown();
    assert!(worker.start(0, request).is_err());
    release.send(()).unwrap();
    finished.recv_timeout(worker_patience()).unwrap();
    assert!(worker.take().is_none());
}
#[test]
fn video_handoff_preserves_literal_paths_and_reports_native_failures() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("movie $(touch sentinel).mp4");
    std::fs::write(&path, b"video fixture").unwrap();
    let media = Media {
        id: super::super::model::MediaId::for_path(&path),
        kind: MediaKind::Video,
        fingerprint: super::super::model::Fingerprint::read(&path).unwrap(),
        path: path.clone(),
    };
    let mut worker = Worker::with_launcher(
        Arc::default(),
        Arc::new(|| {}),
        Arc::new(move |request| {
            assert_eq!(request.path, path);
            assert_eq!(request.action, Action::OpenVideo);
            anyhow::bail!("No player registered")
        }),
    );
    worker
        .start(0, Request::for_media(&media, Action::OpenVideo).unwrap())
        .unwrap();
    wait(&worker);
    assert!(
        worker
            .take()
            .unwrap()
            .result
            .unwrap_err()
            .contains("No player registered")
    );
    assert!(!dir.path().join("sentinel").exists());
}
