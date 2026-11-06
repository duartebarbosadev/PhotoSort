//! Builds the opened folder's display previews and grid thumbnails on disk in
//! the background, several photos at a time, so browsing and Review find them
//! ready instead of decoding each photo when it is first shown. Each thread
//! owns its own pipeline and keeps nothing in memory; the disk cache is the
//! only thing shared, and its writes are atomic. Interactive requests still go
//! to the image worker, which then reads what was built here.
use crate::core::{
    cache::Limits, memory::Pressure, model::Media, pipeline::Pipeline, raw_look::RawLook,
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
    next: usize,
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
impl Builder {
    pub fn new(cache: PathBuf, threads: usize) -> Self {
        let shared = Arc::new(Shared::default());
        let threads = (0..threads)
            .filter_map(|n| {
                let shared = shared.clone();
                let cache = cache.clone();
                thread::Builder::new()
                    .name(format!("preview-builder-{n}"))
                    .spawn(move || run(&shared, cache))
                    .ok()
            })
            .collect();
        Self { shared, threads }
    }
    /// Builds what is missing for `media`, in order. The same folder again
    /// (same photos) keeps its progress; anything else starts over.
    pub fn build(&self, media: Arc<Vec<Media>>, limits: Limits, look: RawLook) {
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
        *queue = Queue {
            media,
            next: 0,
            limits,
            look,
            written: 0,
        };
        self.shared.work.notify_all();
    }
    /// Stops building for the current folder.
    pub fn clear(&self) {
        self.build(Arc::default(), Limits::default(), RawLook::default());
    }
    /// Photos built since the app started.
    pub fn built(&self) -> u64 {
        self.shared.built.load(Ordering::Relaxed)
    }
    pub fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
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
        if queue.next < queue.media.len() && queue.written < room {
            let media = queue.media[queue.next].clone();
            queue.next += 1;
            return Some((
                shared.generation.load(Ordering::Acquire),
                media,
                queue.limits,
                queue.look,
            ));
        }
        queue = shared.work.wait(queue).unwrap();
    }
}
fn run(shared: &Shared, cache: PathBuf) {
    let mut pipeline = Pipeline::new(0, cache);
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
            continue;
        }
        pipeline.configure_cache(limits);
        pipeline.set_raw_look(look);
        let before = pipeline.disk_written();
        match pipeline.build_preview(&media, stale) {
            Ok(true) => {
                shared.built.fetch_add(1, Ordering::Relaxed);
            }
            Ok(false) => {}
            // The image worker reports failures when the photo is shown.
            Err(error) => tracing::debug!(path = ?media.path, %error, "preview not built"),
        }
        let written = pipeline.disk_written() - before;
        let mut queue = shared.queue.lock().unwrap();
        if shared.generation.load(Ordering::Acquire) == generation {
            queue.written += written;
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::model::{Fingerprint, MediaId, MediaKind};
    #[test]
    fn a_folder_gets_its_previews_built_in_the_background_once() {
        let photos = tempfile::tempdir().unwrap();
        let cache = tempfile::tempdir().unwrap();
        let media: Vec<Media> = (0..6)
            .map(|n| {
                let path = photos.path().join(format!("{n}.png"));
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
            .collect();
        let media = Arc::new(media);
        let mut builder = Builder::new(cache.path().to_owned(), 3);
        builder.build(media.clone(), Limits::default(), RawLook::Neutral);
        let deadline = std::time::Instant::now() + crate::test_support::worker_patience();
        while builder.built() < 6 && std::time::Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(builder.built(), 6);
        // The image worker's pipeline now finds every preview on disk.
        let mut pipeline = Pipeline::new(64 << 20, cache.path().to_owned());
        for photo in media.iter() {
            pipeline.load(photo).unwrap();
        }
        assert_eq!((pipeline.stats.decodes, pipeline.stats.disk_hits), (0, 6));
        // The same folder again builds nothing more.
        builder.build(media, Limits::default(), RawLook::Neutral);
        thread::sleep(Duration::from_millis(50));
        assert_eq!(builder.built(), 6);
        builder.shutdown();
    }
}
