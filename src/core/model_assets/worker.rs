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
    /// Download and install an ONNX Runtime package into the data folder.
    InstallRuntime(runtime::Package),
    RemoveRuntime(runtime::Package),
    /// Download all: these downloads and restores, one after another. The
    /// first that fails or is cancelled ends it; what finished stays.
    All(Vec<Work>),
}
impl Work {
    /// The model this is about, for its card's progress.
    fn id(&self) -> Option<Id> {
        match self {
            Work::Import(id, _) | Work::Restore(id) | Work::Download(id) => Some(*id),
            Work::Remove(copy) => Some(copy.id()),
            Work::Inspect | Work::InstallRuntime(_) | Work::RemoveRuntime(_) | Work::All(_) => None,
        }
    }
    fn package(&self) -> Option<runtime::Package> {
        match self {
            Work::InstallRuntime(package) => Some(*package),
            _ => None,
        }
    }
    /// How many bytes its progress counts to; 0 when it has none.
    fn total(&self) -> u64 {
        match self {
            Work::InstallRuntime(package) => {
                runtime::info(*package).map_or(0, |info| info.download_bytes())
            }
            Work::Import(id, _) | Work::Restore(id) | Work::Download(id) => id.asset().bytes,
            Work::Inspect | Work::Remove(_) | Work::RemoveRuntime(_) | Work::All(_) => 0,
        }
    }
    /// Where its progress starts: what an interrupted download of the same
    /// model kept under `root`, which this one continues from.
    pub(super) fn kept(&self, root: Option<&PathBuf>) -> u64 {
        match (self, root) {
            (Work::Download(id), Some(root)) => {
                partial_bytes(root, id.asset().name).min(id.asset().bytes)
            }
            _ => 0,
        }
    }
    /// What the panel says when this was cancelled before it changed anything.
    fn cancelled(&self) -> &'static str {
        match self {
            Work::Inspect => "Check cancelled · the list below may be out of date",
            Work::Download(_) => {
                "Download cancelled · the part already downloaded is kept, so the next download continues from there"
            }
            Work::InstallRuntime(_) => "Download cancelled · nothing was installed",
            Work::Import(..) | Work::Restore(_) => "Cancelled · the model was not installed",
            Work::Remove(_) | Work::RemoveRuntime(_) => "Cancelled · nothing was removed",
            Work::All(_) => "Download cancelled · nothing was installed",
        }
    }
}
#[derive(Clone)]
pub struct Progress {
    pub id: Option<Id>,
    /// The runtime package being installed, when that is the work.
    pub package: Option<runtime::Package>,
    pub bytes: u64,
    pub total: u64,
    /// For Download all: which of its downloads this is, and of how many.
    pub step: Option<(usize, usize)>,
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
/// How far Download all got before it stopped: "1 of 3 finished and stays
/// installed", "2 of 3 finished and stay installed".
pub(super) fn finished_text(finished: usize, count: usize) -> String {
    format!(
        "{finished} of {count} finished and {} installed",
        if finished == 1 { "stays" } else { "stay" }
    )
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
                        let stopped = || stop.load(Ordering::Acquire);
                        let cancelled = || stopped() || request.cancelled.load(Ordering::Acquire);
                        let last = Mutex::new(Instant::now() - Duration::from_secs(1));
                        // One step: an operation, or one download of Download all.
                        let perform = |inspector: &mut Inspector, work: Work, step: Option<(usize, usize)>| -> Result<Option<String>> {
                            let root = roots.first().context("No managed model directory")?;
                            let (id, package) = (work.id(), work.package());
                            let report = |bytes, total| {
                                let mut last = last.lock().unwrap();
                                if last.elapsed() < Duration::from_millis(75) { return; }
                                *last = Instant::now();
                                state.lock().unwrap().progress = Some(Progress { id, package, bytes, total, step });
                                wake();
                            };
                            let progress = |bytes| report(bytes, id.map_or(0, |id| id.asset().bytes));
                            Ok(match work {
                                Work::Inspect | Work::All(_) => None,
                                Work::Remove(copy) => {
                                    let report = remove_reviewed(root, &[copy], &cancelled)?;
                                    ensure!(report.errors.is_empty(), "{}", report.errors.join("\n"));
                                    Some(if report.removed.is_empty() { "No model copies removed".into() }
                                        else { format!("Removed managed copy · {:.1} MB released", report.bytes as f64 / 1e6) })
                                }
                                Work::Import(id, source) => {
                                    let installed = import(id.asset(), &source, root, &cancelled, &progress)?;
                                    inspector.remember_install(id.asset(), &installed);
                                    None
                                }
                                Work::Restore(id) => {
                                    let source = roots.iter().skip(1)
                                        .map(|r| r.join("models").join(id.asset().name))
                                        .find(|p| p.is_file()).context("No bundled copy is available")?;
                                    let installed = import(id.asset(), &source, root, &cancelled, &progress)?;
                                    inspector.remember_install(id.asset(), &installed);
                                    None
                                }
                                Work::InstallRuntime(package) => {
                                    runtime::install(root, package, &cancelled, &report)?;
                                    Some(match package {
                                        runtime::Package::Cpu => "ONNX Runtime installed".into(),
                                        runtime::Package::Cuda => "NVIDIA GPU support installed · restart PhotoSort to use it".into(),
                                    })
                                }
                                Work::RemoveRuntime(package) => {
                                    runtime::remove(root, package)?;
                                    Some("Removed · restart PhotoSort to release the files in use".into())
                                }
                                Work::Download(id) => {
                                    let installed = download::run(id.asset(), root, &cancelled, &progress)?;
                                    inspector.remember_install(id.asset(), &installed);
                                    None
                                }
                            })
                        };
                        let (inspecting, cancelled_text) = (matches!(request.work, Work::Inspect), request.work.cancelled());
                        // What a failed or cancelled download kept changes its
                        // card, and so do the steps of Download all that finished.
                        let downloads = matches!(request.work, Work::Download(_) | Work::All(_));
                        let mut completion = "Model check complete".to_owned();
                        // Download all: how many of its steps finished.
                        let mut finished = None;
                        let changed = (|| -> Result<()> {
                            match request.work {
                                Work::All(steps) => {
                                    let count = steps.len();
                                    finished = Some((0, count));
                                    for (index, step) in steps.into_iter().enumerate() {
                                        ensure!(!cancelled(), "Cancelled");
                                        let name = step.id().map_or("the AI runtime", |id| id.asset().title);
                                        {
                                            let mut state = state.lock().unwrap();
                                            state.message = format!("Downloading {} of {count} · {name}…", index + 1);
                                            state.progress = Some(Progress { id: step.id(), package: step.package(), bytes: step.kept(roots.first()), total: step.total(), step: Some((index + 1, count)) });
                                        }
                                        wake();
                                        perform(&mut inspector, step, Some((index + 1, count))).with_context(|| format!("Download all stopped at {name}"))?;
                                        finished = Some((index + 1, count));
                                        // Each card says "Installed" as soon as it is,
                                        // not when the whole queue ends.
                                        if let Ok(snapshot) = inspector.inspect(&roots, &cancelled) {
                                            state.lock().unwrap().snapshot = Some(Arc::new(snapshot));
                                            wake();
                                        }
                                    }
                                    completion = format!("Downloaded and verified · {}", if count == 1 { "1 item".to_owned() } else { format!("{count} items") });
                                }
                                work => {
                                    if let Some(text) = perform(&mut inspector, work, None)? {
                                        completion = text;
                                    }
                                }
                            }
                            Ok(())
                        })();
                        // A cancel that comes after the change was made is too
                        // late to undo it: the list is checked again all the
                        // same, and only quitting interrupts that. A check that
                        // was itself cancelled leaves the list as it was.
                        let done = changed.is_ok() && !inspecting;
                        let result = match changed {
                            Ok(()) if inspecting => inspector.inspect(&roots, &cancelled),
                            Ok(()) => inspector.inspect(&roots, &stopped),
                            Err(error) => Err(error),
                        };
                        let refreshed = (result.is_err() && downloads && !stopped())
                            .then(|| inspector.inspect(&roots, &stopped).ok())
                            .flatten();
                        let mut current = state.lock().unwrap();
                        current.running = false;
                        current.mutating = false;
                        current.progress = None;
                        if let Some(snapshot) = refreshed {
                            current.snapshot = Some(Arc::new(snapshot));
                        }
                        match result {
                            Ok(snapshot) => {
                                current.snapshot = Some(Arc::new(snapshot));
                                current.error = None;
                                current.message = completion;
                            }
                            Err(_) if cancelled() && !done => {
                                current.error = None;
                                current.message = match finished {
                                    Some((n, count)) if n > 0 => format!("Download cancelled · {}", finished_text(n, count)),
                                    _ => cancelled_text.into(),
                                };
                            }
                            Err(error) => {
                                current.error = Some(format!("{error:#}"));
                                current.message = match finished {
                                    Some((n, count)) if n > 0 => format!("Download all stopped · {}", finished_text(n, count)),
                                    _ => "Model operation did not complete".into(),
                                };
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
                Work::InstallRuntime(..) => "Downloading and verifying the AI runtime…",
                Work::RemoveRuntime(..) => "Removing the AI runtime…",
                Work::All(..) => "Downloading and verifying…",
            }
            .into();
            state.progress = Some(Progress {
                // A removal's card shows no transfer.
                id: work.id().filter(|_| !matches!(work, Work::Remove(_))),
                package: work.package(),
                bytes: work.kept(self.roots.first()),
                total: work.total(),
                step: None,
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
