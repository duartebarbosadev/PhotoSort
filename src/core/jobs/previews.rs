//! Builds the opened folder's display previews and grid thumbnails on disk in
//! the background, several photos at a time, so browsing and Review find them
//! ready instead of decoding each photo when it is first shown. Each thread
//! owns its own pipeline and keeps nothing in memory; the disk cache is the
//! only thing shared, and its writes are atomic. Interactive requests still go
//! to the image worker, which then reads what was built here.
use crate::core::{
    cache::{Limits, Writes},
    memory::Pressure,
    model::Media,
    pipeline::Pipeline,
    raw_look::RawLook,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::Duration,
};

/// What is being built. A new folder replaces it; threads notice through
/// `generation` and drop the photo they are on.
#[derive(Default)]
struct Queue {
    media: Arc<Vec<Media>>,
    /// Indices of `media` in build order: outward from the photo on screen,
    /// which comes last.
    order: Vec<usize>,
    next: usize,
    /// Photos being built right now.
    busy: usize,
    limits: Limits,
    look: RawLook,
    /// Disk bytes all threads wrote for this folder.
    written: u64,
}
#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
    generation: AtomicU64,
    stop: AtomicBool,
    built: AtomicU64,
    /// Signalled when the queue runs out and no photo is being built.
    idle: Condvar,
}
pub struct Builder {
    shared: Arc<Shared>,
    threads: Vec<thread::JoinHandle<()>>,
}
/// Half the cores, at most four: browsing and analysis keep the rest. Test
/// runtimes build nothing in the background, so what they count is exact.
pub(super) fn thread_count() -> usize {
    if cfg!(any(test, feature = "test-isolation")) {
        0
    } else {
        (crate::core::processing::available() / 2).clamp(1, 4)
    }
}
/// `0, 1, ..` reordered to alternate outward from `start`. `start` itself
/// goes last: it is on screen, so the image worker is decoding it already.
fn outward(len: usize, start: usize) -> Vec<usize> {
    let start = start.min(len.saturating_sub(1));
    let mut order = Vec::with_capacity(len);
    for step in 1..len {
        order.extend(start.checked_add(step).filter(|&i| i < len));
        order.extend(start.checked_sub(step));
    }
    order.extend((start < len).then_some(start));
    order
}
impl Builder {
    pub fn new(cache: PathBuf, threads: usize, writes: Arc<Writes>) -> Self {
        let shared = Arc::new(Shared::default());
        // Several run next to the image worker: each takes a small share of
        // the quiet decode allowance, enough for a camera preview or a
        // half-size RAW, never a full-size decode.
        let quiet = crate::core::memory::Machine::current().quiet_allowance();
        let cap = (quiet / (threads as u64 + 2)).min(1 << 30);
        let threads = (0..threads)
            .filter_map(|n| {
                let shared = shared.clone();
                let cache = cache.clone();
                let writes = writes.clone();
                thread::Builder::new()
                    .name(format!("preview-builder-{n}"))
                    .spawn(move || run(&shared, cache, cap, writes))
                    .ok()
            })
            .collect();
        Self { shared, threads }
    }
    /// Builds what is missing for `media`, outward from `start` (the photo on
    /// screen). The same folder again (same photos) keeps its progress;
    /// anything else starts over.
    pub fn build(&self, media: Arc<Vec<Media>>, start: usize, limits: Limits, look: RawLook) {
        let mut queue = self.shared.queue.lock().unwrap();
        let same = queue.media.len() == media.len()
            && queue
                .media
                .iter()
                .zip(media.iter())
                .all(|(a, b)| a.id == b.id)
            && queue.limits == limits
            && queue.look == look;
        if same {
            return;
        }
        self.shared.generation.fetch_add(1, Ordering::AcqRel);
        let busy = queue.busy;
        *queue = Queue {
            order: outward(media.len(), start),
            media,
            next: 0,
            busy,
            limits,
            look,
            written: 0,
        };
        self.shared.work.notify_all();
    }
    /// Stops building for the current folder.
    pub fn clear(&self) {
        self.build(Arc::default(), 0, Limits::default(), RawLook::default());
    }
    /// Waits until every photo of the current folder was looked at and no
    /// thread is building one; false if `timeout` passed first.
    pub fn wait_idle(&self, timeout: Duration) -> bool {
        let queue = self.shared.queue.lock().unwrap();
        let (_queue, result) = self
            .shared
            .idle
            .wait_timeout_while(queue, timeout, |q| q.busy > 0 || q.next < q.order.len())
            .unwrap();
        !result.timed_out()
    }
    /// Photos built since the app started.
    pub fn built(&self) -> u64 {
        self.shared.built.load(Ordering::Relaxed)
    }
    pub fn shutdown(&mut self) {
        {
            // Under the queue's lock: a thread that has just seen `stop` unset
            // and is about to wait would otherwise miss this wakeup, and the
            // join below would never return.
            let _queue = self
                .shared
                .queue
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            self.shared.stop.store(true, Ordering::Release);
        }
        self.shared.work.notify_all();
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
impl Drop for Builder {
    fn drop(&mut self) {
        self.shutdown();
    }
}
/// The next photo to build, waiting while there is none. None to stop.
fn next(shared: &Shared) -> Option<(u64, Media, Limits, RawLook)> {
    let mut queue = shared.queue.lock().unwrap();
    loop {
        if shared.stop.load(Ordering::Acquire) {
            return None;
        }
        // Stop well before the limit: building more would only push out
        // previews made a moment ago.
        let room = u64::from(queue.limits.disk_mib) * crate::core::cache::MIB / 4 * 3;
        if queue.next < queue.order.len() && queue.written < room {
            let media = queue.media[queue.order[queue.next]].clone();
            queue.next += 1;
            queue.busy += 1;
            return Some((
                shared.generation.load(Ordering::Acquire),
                media,
                queue.limits,
                queue.look,
            ));
        }
        // Out of room: the rest counts as looked at.
        if queue.next < queue.order.len() {
            queue.next = queue.order.len();
        }
        if queue.busy == 0 {
            shared.idle.notify_all();
        }
        queue = shared.work.wait(queue).unwrap();
    }
}
fn run(shared: &Shared, cache: PathBuf, cap: u64, writes: Arc<Writes>) {
    let mut pipeline = Pipeline::new(0, cache);
    pipeline.share_writes(writes);
    pipeline.limit_quiet_allowance(cap);
    pipeline.limit_threads(1);
    serve(shared, |media, limits, look, stale| {
        pipeline.configure_cache(limits);
        pipeline.set_raw_look(look);
        let before = pipeline.disk_written();
        let built = pipeline.build_preview(media, stale);
        (built, pipeline.disk_written().saturating_sub(before))
    });
}
/// Whether the folder changed or the app is closing; a build checks it often.
type Stale<'a> = &'a (dyn Fn() -> bool + Sync);
/// Takes photos off the queue until told to stop. `build` makes one and
/// answers whether it built it and the disk bytes it wrote.
fn serve(
    shared: &Shared,
    mut build: impl FnMut(&Media, Limits, RawLook, Stale) -> (anyhow::Result<bool>, u64),
) {
    while let Some((generation, media, limits, look)) = next(shared) {
        // The system is short of memory: wait instead of adding to it.
        while Pressure::current() > Pressure::Normal
            && !shared.stop.load(Ordering::Acquire)
            && shared.generation.load(Ordering::Acquire) == generation
        {
            thread::sleep(Duration::from_secs(1));
        }
        let stale = || {
            shared.stop.load(Ordering::Acquire)
                || shared.generation.load(Ordering::Acquire) != generation
        };
        if stale() {
            done(shared);
            continue;
        }
        // A decoder that panics on one photo must not end this thread, nor
        // leave the photo counted as being built for the rest of the session.
        let (built, written) = crate::core::crash::catch(|| build(&media, limits, look, &stale))
            .unwrap_or_else(|error| (Err(anyhow::anyhow!(error)), 0));
        match built {
            Ok(true) => {
                shared.built.fetch_add(1, Ordering::Relaxed);
            }
            Ok(false) => {}
            // The image worker reports failures when the photo is shown.
            Err(error) => tracing::debug!(path = ?media.path, %error, "preview not built"),
        }
        {
            let mut queue = shared.queue.lock().unwrap();
            if shared.generation.load(Ordering::Acquire) == generation {
                queue.written += written;
            }
        }
        done(shared);
    }
}
/// One photo finished (built, skipped or dropped).
fn done(shared: &Shared) {
    let mut queue = shared.queue.lock().unwrap();
    queue.busy = queue.busy.saturating_sub(1);
    if queue.busy == 0 && queue.next >= queue.order.len() {
        shared.idle.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Fingerprint, MediaId, MediaKind};
    #[test]
    fn the_builder_fills_the_cache_next_to_the_image_worker() {
        assert_eq!(outward(5, 2), [3, 1, 4, 0, 2]);
        assert_eq!(outward(3, 9), [1, 0, 2]);
        let photos = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        // BMP decodes outside libvips in every build, so it is built here.
        let media: Arc<Vec<Media>> = Arc::new(
            (0..6)
                .map(|n| {
                    let path = photos.path().join(format!("{n}.bmp"));
                    image::RgbImage::from_fn(300, 200, |x, y| image::Rgb([x as u8, y as u8, n]))
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
        );
        let patience = crate::test_support::worker_patience();
        let mut builder = Builder::new(cache.path().to_owned(), 3, Arc::default());
        builder.build(media.clone(), 3, Limits::default(), RawLook::Neutral);
        // The image worker shows the same photos while the builder runs.
        let mut worker = Pipeline::new(64 << 20, cache.path().to_owned());
        let shown: Vec<_> = media.iter().map(|m| worker.load(m).unwrap()).collect();
        assert!(builder.wait_idle(patience));
        let built = builder.built();
        // Between them every photo was made; both may race for the same one.
        assert!(built + worker.stats.decodes >= 6);
        // A new session finds every one on disk, as the first one showed it.
        let mut reopened = Pipeline::new(64 << 20, cache.path().to_owned());
        for (photo, asset) in media.iter().zip(&shown) {
            assert_eq!(*reopened.load(photo).unwrap().pixels, *asset.pixels);
        }
        assert_eq!((reopened.stats.decodes, reopened.stats.disk_hits), (0, 6));
        // The same folder again builds nothing more.
        builder.build(media.clone(), 0, Limits::default(), RawLook::Neutral);
        assert!(builder.wait_idle(patience));
        assert_eq!(builder.built(), built);
        builder.shutdown();
        // A decoder that panics on a photo costs that photo only: with two
        // threads and three such photos, the rest are still built, nothing
        // stays counted as being built, and closing joins both threads.
        let shared = Arc::new(Shared::default());
        let threads = (0..2)
            .map(|_| {
                let shared = shared.clone();
                thread::spawn(move || {
                    serve(&shared, |media, _, _, _| {
                        let name = media.path.file_stem().unwrap().to_str().unwrap();
                        assert!(name.parse::<u8>().unwrap() % 2 == 0, "decoder went bad");
                        (Ok(true), 0)
                    })
                })
            })
            .collect();
        let mut builder = Builder { shared, threads };
        builder.build(media, 0, Limits::default(), RawLook::Neutral);
        assert!(builder.wait_idle(patience));
        assert_eq!(builder.built(), 3);
        assert_eq!(builder.shared.queue.lock().unwrap().busy, 0);
        builder.shutdown();
    }
}
