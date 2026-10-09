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
mod tests;
