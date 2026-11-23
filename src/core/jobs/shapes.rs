//! Reads the open folder's photo shapes (size and orientation) ahead of
//! their thumbnails, on a thread of its own, so a grid lays its tiles out in
//! their real shape from the start instead of as 4:3 placeholders that
//! reshape as each thumbnail arrives. A shape is the photo's cached metadata
//! when it was seen before, and otherwise its header alone: a few kilobytes,
//! no pixels, an order of magnitude faster than the thumbnail. The metadata
//! it reads is cached on disk, so the thumbnail's own read finds it there.
use super::{Event, Publisher};
use crate::core::{
    cache::Writes,
    model::{Media, MediaId, MediaKind},
    pipeline::Pipeline,
};
use std::{
    path::PathBuf,
    sync::{Arc, Condvar, Mutex},
    thread,
};

/// A photo's stored width, height and EXIF orientation.
pub type Shape = (u32, u32, u8);

/// Photos per batch while the first screenful is read, so a board waiting
/// for it draws as soon as its shapes are in.
const FIRST: usize = 8;
/// Photos read in small batches: more than a screen of small tiles.
const SCREEN: usize = 64;
/// Photos in every later batch, so the window is not woken per photo.
const LATER: usize = 32;

#[derive(Default)]
struct Queue {
    epoch: u64,
    media: Vec<Media>,
    next: usize,
    stop: bool,
}

#[derive(Default)]
struct Shared {
    queue: Mutex<Queue>,
    work: Condvar,
}

pub struct Shapes {
    shared: Arc<Shared>,
    thread: Option<thread::JoinHandle<()>>,
}

impl Shapes {
    pub(super) fn new(publisher: Publisher, cache: PathBuf, writes: Arc<Writes>) -> Self {
        let shared = Arc::new(Shared::default());
        let thread = {
            let shared = shared.clone();
            thread::Builder::new()
                .name("photosort-shapes".into())
                .spawn(move || run(&shared, &publisher, cache, writes))
                .expect("shape thread starts")
        };
        Self {
            shared,
            thread: Some(thread),
        }
    }

    /// Reads these photos' shapes, in this order, instead of any still
    /// waiting. Videos are left to their thumbnails.
    pub(super) fn read(&self, epoch: u64, media: Vec<Media>) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.epoch = epoch;
        queue.media = media
            .into_iter()
            .filter(|m| m.kind != MediaKind::Video)
            .collect();
        queue.next = 0;
        drop(queue);
        self.shared.work.notify_one();
    }

    /// Drops what is waiting, for another folder.
    pub(super) fn clear(&self) {
        let mut queue = self.shared.queue.lock().unwrap();
        queue.media.clear();
        queue.next = 0;
    }

    pub(super) fn shutdown(&mut self) {
        self.shared.queue.lock().unwrap().stop = true;
        self.shared.work.notify_all();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn run(shared: &Shared, publisher: &Publisher, cache: PathBuf, writes: Arc<Writes>) {
    let mut pipeline = Pipeline::new(0, cache);
    pipeline.share_writes(writes);
    pipeline.limit_threads(1);
    // Only the shape is wanted; the image worker names places.
    pipeline.skip_places();
    let mut batch: Vec<(MediaId, Shape)> = Vec::new();
    let mut batch_epoch = 0;
    let mut sent = 0;
    loop {
        let (epoch, media, last) = {
            let mut queue = shared.queue.lock().unwrap();
            while !queue.stop && queue.next >= queue.media.len() {
                queue = shared.work.wait(queue).unwrap();
            }
            if queue.stop {
                return;
            }
            if queue.next == 0 {
                // A new list: its first batch is small again. Shapes read
                // for the same folder still go out with it.
                sent = 0;
                if batch_epoch != queue.epoch {
                    batch.clear();
                    batch_epoch = queue.epoch;
                }
            }
            let media = queue.media[queue.next].clone();
            queue.next += 1;
            (queue.epoch, media, queue.next == queue.media.len())
        };
        let cancelled = || publisher.cancelled(epoch);
        if cancelled() {
            continue;
        }
        let read = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            pipeline.metadata_cancellable(&media, &cancelled)
        }));
        if let Ok(Ok(metadata)) = read {
            batch.push((
                media.id,
                (metadata.width, metadata.height, metadata.orientation),
            ));
        }
        let size = if sent < SCREEN { FIRST } else { LATER };
        if !batch.is_empty() && (batch.len() >= size || last) {
            sent += batch.len();
            publisher.send(batch_epoch, Event::Shapes(std::mem::take(&mut batch)));
        }
    }
}
