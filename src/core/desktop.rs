//! Shared, lazy desktop handoff. Never decodes photos or initializes models.
mod terminal;

use super::model::{Media, MediaKind};
use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Reveal,
    Open,
    OpenFolder,
    OpenVideo,
    Terminal,
}
impl Action {
    pub fn label(self) -> &'static str {
        match self {
            Self::Reveal if cfg!(target_os = "macos") => "Show in Finder",
            Self::Reveal if cfg!(target_os = "windows") => "Show in Explorer",
            Self::Reveal => "Show in file manager",
            Self::Open => "Open in default app",
            Self::OpenFolder => "Open folder",
            Self::OpenVideo => "Open in external player",
            Self::Terminal => "Open terminal here",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub action: Action,
    pub path: PathBuf,
}
impl Request {
    /// Capture the clicked catalog entry, not a later selection or a staged path.
    pub fn for_media(media: &Media, action: Action) -> Result<Self> {
        ensure!(
            action != Action::OpenVideo
                || (media.kind == MediaKind::Video
                    && MediaKind::from_path(&media.path) == Some(MediaKind::Video)),
            "External playback requires a video file"
        );
        Self::for_path(&media.path, action)
    }
    /// Capture a scanned source file or directory. Filesystem checks stay on the worker.
    pub fn for_path(path: &std::path::Path, action: Action) -> Result<Self> {
        ensure!(
            path.is_absolute(),
            "Desktop actions require an absolute local path"
        );
        ensure!(
            action != Action::OpenVideo || MediaKind::from_path(path) == Some(MediaKind::Video),
            "External playback requires a video file"
        );
        Ok(Self {
            action,
            path: path.to_owned(),
        })
    }
    fn verified(&self) -> Result<Self> {
        ensure!(
            self.path.is_absolute(),
            "Desktop actions require an absolute local path"
        );
        let metadata = std::fs::metadata(&self.path)
            .with_context(|| format!("File is unavailable: {}", self.path.display()))?;
        ensure!(
            self.action != Action::OpenFolder || metadata.is_dir(),
            "Folder is unavailable: {}",
            self.path.display()
        );
        ensure!(
            metadata.is_file() || (metadata.is_dir() && self.action != Action::OpenVideo),
            "File is unavailable: {}",
            self.path.display()
        );
        ensure!(
            self.action != Action::OpenVideo
                || MediaKind::from_path(&self.path) == Some(MediaKind::Video),
            "External playback requires a video file"
        );
        let path = if self.action == Action::Terminal && metadata.is_file() {
            self.path
                .parent()
                .context("File has no containing folder")?
                .to_owned()
        } else {
            self.path.clone()
        };
        Ok(Self {
            action: self.action,
            path,
        })
    }
}
pub struct Completion {
    pub request: Request,
    pub result: Result<(), String>,
    epoch: u64,
}
type Launcher = Arc<dyn Fn(&Request) -> Result<()> + Send + Sync>;
pub struct Worker {
    generation: Arc<AtomicU64>,
    stopped: Arc<AtomicBool>,
    result: Arc<Mutex<Option<Completion>>>,
    thread: Option<thread::JoinHandle<()>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    launch: Launcher,
    terminals: Arc<terminal::Children>,
}
impl Worker {
    pub fn new(generation: Arc<AtomicU64>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let terminals = Arc::new(terminal::Children::default());
        let launcher = terminals.clone();
        let mut worker = Self::with_launcher(
            generation,
            wake,
            Arc::new(move |request| {
                match request.action {
                    Action::Reveal => opener::reveal(&request.path)?,
                    Action::Open | Action::OpenFolder | Action::OpenVideo => {
                        opener::open(&request.path)?
                    }
                    Action::Terminal => launcher.open(&request.path)?,
                }
                Ok(())
            }),
        );
        worker.terminals = terminals;
        worker
    }
    pub(crate) fn with_launcher(
        generation: Arc<AtomicU64>,
        wake: Arc<dyn Fn() + Send + Sync>,
        launch: Launcher,
    ) -> Self {
        Self {
            generation,
            stopped: Arc::default(),
            result: Arc::default(),
            thread: None,
            wake,
            launch,
            terminals: Arc::default(),
        }
    }
    pub fn running(&self) -> bool {
        self.thread
            .as_ref()
            .is_some_and(|thread| !thread.is_finished())
    }
    pub fn start(&mut self, epoch: u64, request: Request) -> Result<()> {
        ensure!(
            !self.stopped.load(Ordering::Acquire),
            "Desktop service has stopped"
        );
        ensure!(
            self.generation.load(Ordering::Acquire) == epoch,
            "Library changed before desktop request"
        );
        ensure!(!self.running(), "A desktop request is already in progress");
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.result.lock().unwrap().take();
        let (generation, stopped, result, wake, launch) = (
            self.generation.clone(),
            self.stopped.clone(),
            self.result.clone(),
            self.wake.clone(),
            self.launch.clone(),
        );
        self.thread = Some(
            thread::Builder::new()
                .name("photosort-desktop".into())
                .spawn(move || {
                    let cancelled = || {
                        stopped.load(Ordering::Acquire)
                            || generation.load(Ordering::Acquire) != epoch
                    };
                    if cancelled() {
                        return;
                    }
                    let outcome = (|| {
                        let target = request.verified()?;
                        ensure!(!cancelled(), "Desktop request cancelled");
                        launch(&target)
                    })()
                    .map_err(|e| format!("{e:#}"));
                    if cancelled() {
                        return;
                    }
                    *result.lock().unwrap() = Some(Completion {
                        request,
                        result: outcome,
                        epoch,
                    });
                    wake();
                })
                .context("Could not start desktop handoff")?,
        );
        Ok(())
    }
    /// Reap finished terminal children without waiting or blocking the UI.
    pub fn poll_terminals(&self) -> bool {
        self.terminals.poll()
    }
    pub fn take(&self) -> Option<Completion> {
        self.result.lock().unwrap().take().filter(|result| {
            !self.stopped.load(Ordering::Acquire)
                && result.epoch == self.generation.load(Ordering::Acquire)
        })
    }
    pub fn shutdown(&mut self) {
        self.stopped.store(true, Ordering::Release);
        self.result.lock().unwrap().take();
        // A handed-off native operation cannot be recalled. Detach this
        // single bounded call so a hung desktop/DBus service cannot block
        // application shutdown; it owns no photos, app state or worker queue.
        if let Some(thread) = self.thread.take()
            && thread.is_finished()
        {
            let _ = thread.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{atomic::AtomicUsize, mpsc},
        time::{Duration, Instant},
    };
    fn wait(worker: &Worker) {
        let end = Instant::now() + Duration::from_secs(5);
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
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
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
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
        worker.shutdown();
        assert!(worker.start(0, request).is_err());
        release.send(()).unwrap();
        finished.recv_timeout(Duration::from_secs(5)).unwrap();
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
}
