//! Decodes and measures the photos an analysis reaches next, several at a
//! time, next to the image worker. Each helper thread owns its own pipeline
//! and keeps nothing in memory; the disk cache is shared as it is with the
//! preview builders. A helper hands back the analysis image and its
//! measurements. The image worker still takes the photos in order, runs the
//! models, stores the evidence and publishes, so results arrive in the same
//! order and cancellation still retires the whole run.
//!
//! Helpers only ever do what the image worker would have done itself, with
//! the same recipe and decode allowance. Whatever they skip (cached
//! measurements, a photo too large to decode next to others, memory
//! pressure, libvips formats) or fail at is done by the image worker the way
//! it always was, so a result never depends on whether a photo was prepared
//! ahead.
use crate::core::{
    analysis::{self, Measurements, Thresholds},
    cache::{Limits, Writes},
    memory::{GIB, Machine, Pressure},
    model::{Media, MediaKind},
    operations,
    pipeline::{Asset, Pipeline},
    processing::{Backend, RunConfig},
    subjects::store::Store,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, RecvTimeoutError, Sender, channel},
    },
    thread,
    time::Duration,
};

/// Never more helpers than this, however many cores: past it, memory
/// bandwidth rather than cores limits decoding.
const MAX_HELPERS: usize = 16;
/// What a helper assumes a decode needs per source pixel: the decoded
/// pixels, an oriented copy and the downscale's working rows, with room to
/// spare for 16-bit sources.
const BYTES_PER_PIXEL: u64 = 16;
/// How long the image worker waits for the photo it needs before it looks
/// at inspection, preview and cache requests again.
const PATIENCE: Duration = Duration::from_millis(10);

pub(super) type Cancelled = Arc<dyn Fn() -> bool + Send + Sync>;

/// One photo decoded and measured ahead.
pub(super) struct Measured {
    pub source_key: String,
    pub asset: Asset,
    pub measurements: Measurements,
    /// Source decodes this took, for the run's statistics.
    pub decodes: u64,
}

struct Job {
    token: u64,
    index: usize,
    media: Media,
    store: Arc<Store>,
    limits: Option<Limits>,
    cancelled: Cancelled,
    /// The largest decode, by `BYTES_PER_PIXEL`, a helper takes on.
    room: u64,
}

struct Done {
    token: u64,
    index: usize,
    measured: Option<Box<Measured>>,
}

/// The run being prepared ahead.
struct Run {
    token: u64,
    media: Arc<Vec<Media>>,
    store: Arc<Store>,
    limits: Option<Limits>,
    cancelled: Cancelled,
    helpers: usize,
    room: u64,
    /// The next index that may be handed to a helper.
    next: usize,
    busy: HashSet<usize>,
    ready: HashMap<usize, Option<Box<Measured>>>,
}

/// What `Ahead::take` found for a photo.
pub(super) enum Taken {
    /// A helper finished it: measured, or `None` when it left the photo to
    /// the image worker.
    Ready(Option<Box<Measured>>),
    /// A helper is still on it; look at other requests and ask again.
    Waiting,
    /// No helper was given it.
    Absent,
}

pub(super) struct Ahead {
    cache: PathBuf,
    writes: Arc<Writes>,
    jobs: Option<Sender<Job>>,
    queue: Arc<Mutex<Receiver<Job>>>,
    done_tx: Sender<Done>,
    done: Receiver<Done>,
    threads: Vec<thread::JoinHandle<()>>,
    token: u64,
    /// The token of the run being prepared, 0 for none. Helpers skip jobs
    /// of any other.
    live: Arc<AtomicU64>,
    run: Option<Run>,
}

/// Helpers for a run: the processing threads the user allows, halved when
/// models share the CPU with them, and no more than memory affords at about
/// a gigabyte each.
pub(super) fn helpers(config: RunConfig, models: bool) -> usize {
    let threads = config.threads.max(1);
    let threads = if models && config.backend == Backend::Cpu {
        threads / 2
    } else {
        threads
    };
    let memory = usize::try_from(Machine::current().quiet_allowance() / GIB).unwrap_or(usize::MAX);
    threads.min(memory).clamp(1, MAX_HELPERS)
}

impl Ahead {
    pub(super) fn new(cache: PathBuf, writes: Arc<Writes>) -> Self {
        let (jobs, queue) = channel();
        let (done_tx, done) = channel();
        Self {
            cache,
            writes,
            jobs: Some(jobs),
            queue: Arc::new(Mutex::new(queue)),
            done_tx,
            done,
            threads: Vec::new(),
            token: 0,
            live: Arc::default(),
            run: None,
        }
    }

    /// Starts preparing `media` ahead of the image worker, replacing any
    /// earlier run. `cancelled` is the run's own cancellation.
    pub(super) fn begin(
        &mut self,
        media: Arc<Vec<Media>>,
        store: Arc<Store>,
        limits: Option<Limits>,
        cancelled: Cancelled,
        helpers: usize,
    ) {
        self.end();
        self.token = self.token.wrapping_add(1).max(1);
        while self.threads.len() < helpers {
            let live = self.live.clone();
            let queue = self.queue.clone();
            let done = self.done_tx.clone();
            let cache = self.cache.clone();
            let writes = self.writes.clone();
            match thread::Builder::new()
                .name(format!("analysis-helper-{}", self.threads.len()))
                .spawn(move || work(&live, &queue, &done, cache, writes))
            {
                Ok(thread) => self.threads.push(thread),
                Err(error) => {
                    tracing::warn!(%error, "analysis helper not started");
                    break;
                }
            }
        }
        let helpers = helpers.min(self.threads.len());
        if helpers == 0 {
            return;
        }
        // Each helper and the image worker may decode at once; together
        // they stay within what one decode may use without asking.
        let room = Machine::current().quiet_allowance() / (helpers as u64 + 1);
        self.live.store(self.token, Ordering::Release);
        self.run = Some(Run {
            token: self.token,
            media,
            store,
            limits,
            cancelled,
            helpers,
            room,
            next: 0,
            busy: HashSet::new(),
            ready: HashMap::new(),
        });
    }

    /// Stops preparing ahead. Helpers drop what they are on as soon as the
    /// run reports cancellation; anything they still finish is discarded.
    pub(super) fn end(&mut self) {
        self.live.store(0, Ordering::Release);
        self.run = None;
        while self.done.try_recv().is_ok() {}
    }

    /// The photo at `index`, after handing the photos after it to helpers.
    pub(super) fn take(&mut self, index: usize) -> Taken {
        let Some(jobs) = &self.jobs else {
            return Taken::Absent;
        };
        let Some(run) = &mut self.run else {
            return Taken::Absent;
        };
        // The image worker is on `index`; helpers work on what follows.
        run.next = run.next.max(index + 1);
        while run.busy.len() < run.helpers
            && run.busy.len() + run.ready.len() < run.helpers * 2
            && run.next < run.media.len()
        {
            let media = &run.media[run.next];
            if media.kind != MediaKind::Video {
                let job = Job {
                    token: run.token,
                    index: run.next,
                    media: media.clone(),
                    store: run.store.clone(),
                    limits: run.limits,
                    cancelled: run.cancelled.clone(),
                    room: run.room,
                };
                if jobs.send(job).is_err() {
                    break;
                }
                run.busy.insert(run.next);
            }
            run.next += 1;
        }
        let accept = |done: Done, run: &mut Run| {
            if done.token == run.token {
                run.busy.remove(&done.index);
                run.ready.insert(done.index, done.measured);
            }
        };
        while let Ok(done) = self.done.try_recv() {
            accept(done, run);
        }
        // Photos before `index` were done by the image worker meanwhile.
        run.ready.retain(|&i, _| i >= index);
        if let Some(measured) = run.ready.remove(&index) {
            return Taken::Ready(measured);
        }
        if !run.busy.contains(&index) {
            return Taken::Absent;
        }
        match self.done.recv_timeout(PATIENCE) {
            Ok(done) => accept(done, run),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return Taken::Absent,
        }
        match run.ready.remove(&index) {
            Some(measured) => Taken::Ready(measured),
            None => Taken::Waiting,
        }
    }
}

impl Drop for Ahead {
    fn drop(&mut self) {
        self.end();
        // Helpers skip what is still queued and stop once the queue closes.
        // A decode in progress ends at its next cancellation check when the
        // runtime is stopping, and otherwise finishes first.
        self.jobs = None;
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn work(
    live: &AtomicU64,
    queue: &Mutex<Receiver<Job>>,
    done: &Sender<Done>,
    cache: PathBuf,
    writes: Arc<Writes>,
) {
    let mut pipeline = Pipeline::new(0, cache);
    pipeline.share_writes(writes);
    // Helpers already run one per core.
    pipeline.limit_threads(1);
    loop {
        let job = queue.lock().unwrap().recv();
        let Ok(job) = job else {
            return;
        };
        if job.token != live.load(Ordering::Acquire) {
            continue;
        }
        // A panic leaves the photo to the image worker, which reports it.
        let measured = crate::core::crash::catch(|| prepare(&mut pipeline, &job))
            .ok()
            .flatten()
            .map(Box::new);
        pipeline.clear_memory();
        let reply = Done {
            token: job.token,
            index: job.index,
            measured,
        };
        if done.send(reply).is_err() {
            return;
        }
    }
}

/// `ImageWorker::measurements` for one photo, or `None` to leave it to the
/// image worker.
fn prepare(pipeline: &mut Pipeline, job: &Job) -> Option<Measured> {
    let cancelled = || (job.cancelled)();
    if cancelled() || Pressure::current() > Pressure::Normal || !Pipeline::shared_decode(&job.media)
    {
        return None;
    }
    if let Some(limits) = job.limits {
        pipeline.configure_cache(limits);
    }
    let source_key = Pipeline::key(&job.media).ok()?;
    if (job.store)
        .measurements(&Store::measurements_key(&source_key))
        .is_some()
    {
        return None;
    }
    let metadata = pipeline.metadata_cancellable(&job.media, &cancelled).ok()?;
    let pixels = u64::from(metadata.width) * u64::from(metadata.height);
    if pixels.saturating_mul(BYTES_PER_PIXEL) > job.room {
        return None;
    }
    let before = pipeline.stats.decodes;
    let asset = pipeline.analysis(&job.media, cancelled).ok()?;
    let mut technical = analysis::analyze_cancellable(&asset, Thresholds::default(), cancelled)?;
    technical.issues.clear();
    let content_hash = operations::content_hash(&job.media.path, cancelled).ok()?;
    if cancelled() || Pipeline::key(&job.media).ok()? != source_key {
        return None;
    }
    Some(Measured {
        source_key,
        measurements: Measurements {
            analysis: technical,
            width: asset.pixels.width(),
            height: asset.pixels.height(),
            content_hash,
        },
        asset,
        decodes: pipeline.stats.decodes - before,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Fingerprint, MediaId};
    use std::{path::Path, sync::atomic::AtomicBool, time::Instant};

    /// BMP decodes outside libvips in every build, so helpers take it.
    fn photos(dir: &Path, count: u8) -> Arc<Vec<Media>> {
        Arc::new(
            (0..count)
                .map(|n| {
                    let path = dir.join(format!("{n}.bmp"));
                    image::RgbImage::from_fn(400, 300, |x, y| {
                        image::Rgb([(x ^ y) as u8, (x * 3) as u8, n.wrapping_mul(40)])
                    })
                    .save(&path)
                    .unwrap();
                    Media {
                        id: MediaId::for_path(&path),
                        fingerprint: Fingerprint::read(&path).unwrap(),
                        path,
                        kind: MediaKind::Photo,
                    }
                })
                .collect(),
        )
    }

    /// `take`, asking again while a helper is still on the photo.
    fn take(ahead: &mut Ahead, index: usize) -> Taken {
        let deadline = Instant::now() + crate::test_support::worker_patience();
        loop {
            match ahead.take(index) {
                Taken::Waiting if Instant::now() < deadline => {}
                taken => return taken,
            }
        }
    }

    #[test]
    fn helpers_measure_exactly_what_the_image_worker_would() {
        let (dir, cache, own_cache, evidence) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let media = photos(dir.path(), 6);
        let store = Arc::new(Store::new(evidence.path().into(), true).unwrap());
        let mut ahead = Ahead::new(cache.path().into(), Arc::default());
        ahead.begin(media.clone(), store, None, Arc::new(|| false), 3);
        assert!(
            matches!(take(&mut ahead, 0), Taken::Absent),
            "the image worker takes the first photo itself"
        );
        let mut own = Pipeline::new(64 << 20, own_cache.path().into());
        for (index, photo) in media.iter().enumerate().skip(1) {
            let Taken::Ready(Some(measured)) = take(&mut ahead, index) else {
                panic!("photo {index} was not prepared ahead");
            };
            let asset = own.analysis(photo, || false).unwrap();
            let mut technical = analysis::analyze(&asset, Thresholds::default());
            technical.issues.clear();
            assert_eq!(measured.source_key, Pipeline::key(photo).unwrap());
            assert_eq!(*measured.asset.pixels, *asset.pixels);
            assert_eq!(
                (
                    measured.asset.metadata.width,
                    measured.asset.metadata.height
                ),
                (asset.metadata.width, asset.metadata.height)
            );
            assert_eq!(
                serde_json::to_string(&measured.measurements.analysis).unwrap(),
                serde_json::to_string(&technical).unwrap()
            );
            assert_eq!(
                (measured.measurements.width, measured.measurements.height),
                asset.pixels.dimensions()
            );
            assert_eq!(
                measured.measurements.content_hash,
                operations::content_hash(&photo.path, || false).unwrap()
            );
            assert_eq!(measured.decodes, 1);
        }
        ahead.end();
    }

    #[test]
    fn cached_or_cancelled_photos_are_left_to_the_image_worker() {
        let (dir, cache, evidence) = (
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
            tempfile::tempdir().unwrap(),
        );
        let media = photos(dir.path(), 4);
        let store = Arc::new(Store::new(evidence.path().into(), true).unwrap());
        let mut ahead = Ahead::new(cache.path().into(), Arc::default());

        // Measured before: nothing to prepare.
        let mut own = Pipeline::new(64 << 20, cache.path().into());
        for photo in media.iter() {
            let asset = own.analysis(photo, || false).unwrap();
            let mut technical = analysis::analyze(&asset, Thresholds::default());
            technical.issues.clear();
            let source = Pipeline::key(photo).unwrap();
            let value = Measurements {
                analysis: technical,
                width: asset.pixels.width(),
                height: asset.pixels.height(),
                content_hash: operations::content_hash(&photo.path, || false).unwrap(),
            };
            store
                .put_measurements(&Store::measurements_key(&source), &value)
                .unwrap();
        }
        ahead.begin(media.clone(), store.clone(), None, Arc::new(|| false), 2);
        // The image worker is on the first photo; helpers get the others.
        assert!(matches!(take(&mut ahead, 0), Taken::Absent));
        for index in 1..media.len() {
            assert!(matches!(take(&mut ahead, index), Taken::Ready(None)));
        }

        // A cancelled run: helpers drop everything.
        let fresh = Arc::new(Store::new(evidence.path().join("fresh"), true).unwrap());
        let cancelled = Arc::new(AtomicBool::new(true));
        let flag = cancelled.clone();
        ahead.begin(
            media.clone(),
            fresh,
            None,
            Arc::new(move || flag.load(Ordering::Relaxed)),
            2,
        );
        assert!(matches!(take(&mut ahead, 0), Taken::Absent));
        for index in 1..media.len() {
            assert!(matches!(
                take(&mut ahead, index),
                Taken::Ready(None) | Taken::Absent
            ));
        }
        ahead.end();
        assert!(matches!(ahead.take(1), Taken::Absent));
    }
}
