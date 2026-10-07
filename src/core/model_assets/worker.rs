use super::*;
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
#[derive(Clone)]
pub enum Work {
    Inspect,
    Import(Id, PathBuf),
    Restore(Id),
    Download(Id),
    Remove(ManagedCopy),
}
#[derive(Clone)]
pub struct Progress {
    pub id: Option<Id>,
    pub bytes: u64,
    pub total: u64,
}
#[derive(Clone, Default)]
pub struct State {
    pub snapshot: Option<Arc<Snapshot>>,
    pub progress: Option<Progress>,
    pub running: bool,
    pub mutating: bool,
    pub error: Option<String>,
    pub message: String,
}
struct Request {
    work: Work,
    cancelled: Arc<AtomicBool>,
}
pub struct Worker {
    state: Arc<Mutex<State>>,
    roots: Vec<PathBuf>,
    wake: Arc<dyn Fn() + Send + Sync>,
    sender: Option<mpsc::SyncSender<Request>>,
    thread: Option<thread::JoinHandle<()>>,
    cancelled: Arc<AtomicBool>,
    stop: Arc<AtomicBool>,
}
impl Worker {
    pub fn new(roots: Vec<PathBuf>, wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self {
            state: Arc::default(),
            roots,
            wake,
            sender: None,
            thread: None,
            cancelled: Arc::default(),
            stop: Arc::default(),
        }
    }
    pub fn state(&self) -> State {
        self.state.lock().unwrap().clone()
    }
    pub fn start(&mut self, work: Work) -> Result<()> {
        ensure!(
            !self.stop.load(Ordering::Acquire),
            "Model worker is stopped"
        );
        ensure!(
            !self.state().running,
            "A model operation is already running"
        );
        if self.sender.is_none() {
            let (tx, rx) = mpsc::sync_channel::<Request>(1);
            let (state, roots, stop, wake) = (
                self.state.clone(),
                self.roots.clone(),
                self.stop.clone(),
                self.wake.clone(),
            );
            self.thread = Some(thread::Builder::new()
                .name("photosort-model-setup".into())
                .spawn(move || {
                    let mut inspector = Inspector::default();
                    while !stop.load(Ordering::Acquire) {
                        let Ok(request) = rx.recv_timeout(Duration::from_millis(100)) else { continue; };
                        let cancelled = || stop.load(Ordering::Acquire) || request.cancelled.load(Ordering::Acquire);
                        let id = match &request.work {
                            Work::Inspect => None,
                            Work::Import(id, _) | Work::Restore(id) | Work::Download(id) => Some(*id),
                            Work::Remove(copy) => Some(copy.id()),
                        };
                        let last = Mutex::new(Instant::now() - Duration::from_secs(1));
                        let progress = |bytes| {
                            let mut last = last.lock().unwrap();
                            if last.elapsed() < Duration::from_millis(75) { return; }
                            *last = Instant::now();
                            state.lock().unwrap().progress = Some(Progress {
                                id, bytes, total: id.map_or(0, |id| id.asset().bytes),
                            });
                            wake();
                        };
                        let mut completion = "Model check complete".to_owned();
                        let result = (|| -> Result<Snapshot> {
                            let root = roots.first().context("No managed model directory")?;
                            match request.work {
                                Work::Inspect => {}
                                Work::Remove(copy) => {
                                    let report = remove_reviewed(root, &[copy], &cancelled)?;
                                    ensure!(report.errors.is_empty(), "{}", report.errors.join("\n"));
                                    completion = if report.removed.is_empty() { "No model copies removed".into() }
                                        else { format!("Removed managed copy · {:.1} MB released", report.bytes as f64 / 1e6) };
                                }
                                Work::Import(id, source) => {
                                    let installed = import(id.asset(), &source, root, &cancelled, &progress)?;
                                    inspector.remember_install(id.asset(), &installed);
                                }
                                Work::Restore(id) => {
                                    let source = roots.iter().skip(1)
                                        .map(|r| r.join("models").join(id.asset().name))
                                        .find(|p| p.is_file()).context("No bundled copy is available")?;
                                    let installed = import(id.asset(), &source, root, &cancelled, &progress)?;
                                    inspector.remember_install(id.asset(), &installed);
                                }
                                Work::Download(id) => {
                                    #[cfg(feature = "model-downloads")]
                                    {
                                        let installed = download::run(id.asset(), root, &cancelled, &progress)?;
                                        inspector.remember_install(id.asset(), &installed);
                                    }
                                    #[cfg(not(feature = "model-downloads"))]
                                    {
                                        let _ = id;
                                        anyhow::bail!("Downloads are not included in this build; import a prepared model");
                                    }
                                }
                            }
                            inspector.inspect(&roots, &cancelled)
                        })();
                        let mut current = state.lock().unwrap();
                        current.running = false;
                        current.mutating = false;
                        current.progress = None;
                        if cancelled() {
                            current.message = "Cancelled · completed changes are retained; refresh to check".into();
                            current.error = None;
                        } else {
                            match result {
                                Ok(snapshot) => {
                                    current.snapshot = Some(Arc::new(snapshot));
                                    current.error = None;
                                    current.message = completion;
                                }
                                Err(error) => {
                                    current.error = Some(format!("{error:#}"));
                                    current.message = "Model operation did not complete".into();
                                }
                            }
                        }
                        drop(current);
                        wake();
                    }
                })?);
            self.sender = Some(tx);
        }
        self.cancelled = Arc::new(AtomicBool::new(false));
        {
            let mut state = self.state.lock().unwrap();
            state.running = true;
            state.mutating = !matches!(work, Work::Inspect);
            state.error = None;
            if matches!(work, Work::Remove(_)) {
                state.snapshot = None;
            }
            state.message = match &work {
                Work::Inspect => "Checking model files…",
                Work::Import(..) => "Importing and verifying model…",
                Work::Restore(..) => "Restoring and verifying bundled model…",
                Work::Download(..) => "Downloading and verifying model…",
                Work::Remove(..) => "Removing managed model copy…",
            }
            .into();
            state.progress = Some(Progress {
                id: match &work {
                    Work::Inspect | Work::Remove(_) => None,
                    Work::Import(id, _) | Work::Restore(id) | Work::Download(id) => Some(*id),
                },
                bytes: 0,
                total: match &work {
                    Work::Inspect | Work::Remove(_) => 0,
                    Work::Import(id, _) | Work::Restore(id) | Work::Download(id) => {
                        id.asset().bytes
                    }
                },
            });
        }
        if self
            .sender
            .as_ref()
            .unwrap()
            .try_send(Request {
                work,
                cancelled: self.cancelled.clone(),
            })
            .is_err()
        {
            let mut state = self.state.lock().unwrap();
            state.running = false;
            state.mutating = false;
            state.progress = None;
            anyhow::bail!("Model queue is unavailable");
        }
        Ok(())
    }
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn shutdown(&mut self) {
        self.cancel();
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.shutdown();
    }
}
