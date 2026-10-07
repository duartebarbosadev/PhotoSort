//! One lazy playback owner, independent of image/model/file queues.
#[cfg(target_os = "macos")]
mod native;
#[cfg(all(feature = "portable-video", not(target_os = "macos")))]
mod portable;
use super::model::{Fingerprint, Media, MediaId, MediaKind};
use anyhow::Result;
use image::RgbaImage;
#[cfg(target_os = "macos")]
pub use native::thumbnail;
#[cfg(all(feature = "portable-video", not(target_os = "macos")))]
pub use portable::thumbnail;
use std::{
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Clone)]
pub struct Frame {
    pub serial: u64,
    pub pixels: Arc<RgbaImage>,
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub id: Option<MediaId>,
    pub loading: bool,
    pub ready: bool,
    pub playing: bool,
    pub play_requested: bool,
    pub ended: bool,
    pub position: f64,
    /// Latest requested position until the backend finishes that seek.
    pub seek_target: Option<f64>,
    pub duration: f64,
    pub volume: f32,
    pub frame: Option<Frame>,
    pub error: Option<String>,
}
#[derive(Default)]
struct Request {
    media: Option<Media>,
    revision: u64,
    playing: bool,
    seek: Option<(u64, f64)>,
    volume: f32,
}
struct Shared {
    request: Request,
    snapshot: Snapshot,
}
pub(crate) struct Decoded {
    position: f64,
    duration: f64,
    ready: bool,
    playing: bool,
    ended: bool,
    seeking: bool,
    frame: Option<RgbaImage>,
}
pub(crate) trait Backend {
    fn control(&mut self, playing: bool, seek: Option<f64>, volume: f32);
    fn poll(&mut self) -> Result<Decoded>;
}
type Factory = Arc<dyn Fn(&Path, &dyn Fn() -> bool) -> Result<Box<dyn Backend>> + Send + Sync>;
fn open(path: &Path, cancelled: &dyn Fn() -> bool) -> Result<Box<dyn Backend>> {
    #[cfg(target_os = "macos")]
    {
        Ok(Box::new(native::Player::open(path, cancelled)?))
    }
    #[cfg(all(feature = "portable-video", not(target_os = "macos")))]
    {
        Ok(Box::new(portable::Player::open(path, cancelled)?))
    }
    #[cfg(all(not(feature = "portable-video"), not(target_os = "macos")))]
    {
        let _ = (path, cancelled);
        anyhow::bail!(
            "In-app playback is not available on this platform yet. Use the external player."
        )
    }
}
pub fn supported() -> bool {
    cfg!(any(target_os = "macos", feature = "portable-video"))
}
/// Commands and frames replace previous values; UI rate cannot create an unbounded queue.
pub struct Player {
    shared: Arc<(Mutex<Shared>, Condvar)>,
    generation: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<thread::JoinHandle<()>>>,
    wake: Arc<dyn Fn() + Send + Sync>,
    factory: Factory,
}
impl Player {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        Self::with_factory(wake, Arc::new(open))
    }
    fn with_factory(wake: Arc<dyn Fn() + Send + Sync>, factory: Factory) -> Self {
        Self {
            shared: Arc::new((
                Mutex::new(Shared {
                    request: Request {
                        volume: 0.5,
                        ..Default::default()
                    },
                    snapshot: Snapshot {
                        volume: 0.5,
                        ..Default::default()
                    },
                }),
                Condvar::new(),
            )),
            generation: Arc::new(AtomicU64::new(0)),
            stop: Arc::new(AtomicBool::new(false)),
            thread: Mutex::new(None),
            wake,
            factory,
        }
    }
    pub fn snapshot(&self) -> Snapshot {
        let s = self.shared.0.lock().unwrap();
        let mut snapshot = s.snapshot.clone();
        snapshot.play_requested = s.request.playing;
        snapshot.seek_target = s.request.seek.map(|(_, target)| target);
        snapshot
    }
    pub fn select(&self, media: Option<&Media>) {
        let media = media.filter(|m| m.kind == MediaKind::Video);
        let mut s = self.shared.0.lock().unwrap();
        if self.stop.load(Ordering::Acquire) {
            return;
        }
        if s.request
            .media
            .as_ref()
            .map(|m| (&m.path, m.fingerprint, m.id))
            == media.map(|m| (&m.path, m.fingerprint, m.id))
        {
            return;
        }
        s.request.media = media.cloned();
        s.request.playing = false;
        s.request.seek = None;
        s.request.revision += 1;
        s.snapshot = Snapshot {
            id: media.map(|m| m.id),
            loading: media.is_some(),
            volume: s.request.volume,
            ..Default::default()
        };
        self.generation.fetch_add(1, Ordering::AcqRel);
        drop(s);
        if media.is_some() {
            self.start();
        }
        self.shared.1.notify_one();
    }
    pub fn toggle(&self) {
        let mut s = self.shared.0.lock().unwrap();
        if s.request.media.is_none() || s.snapshot.error.is_some() {
            return;
        }
        s.request.revision += 1;
        if s.snapshot.ended {
            s.request.seek = Some((s.request.revision, 0.0));
            s.request.playing = true;
            s.snapshot.ended = false;
        } else {
            s.request.playing = !s.request.playing;
        }
        drop(s);
        self.shared.1.notify_one();
    }
    pub fn pause(&self) {
        let mut s = self.shared.0.lock().unwrap();
        if s.request.playing {
            s.request.playing = false;
            s.request.revision += 1;
            s.snapshot.playing = false;
        }
        drop(s);
        self.shared.1.notify_one();
    }
    pub fn seek(&self, seconds: f64) {
        self.request_seek(seconds, false);
    }
    pub fn seek_relative(&self, seconds: f64) {
        self.request_seek(seconds, true);
    }
    fn request_seek(&self, seconds: f64, relative: bool) {
        let mut s = self.shared.0.lock().unwrap();
        if !seconds.is_finite()
            || s.request.media.is_none()
            || !s.snapshot.ready
            || s.snapshot.error.is_some()
            || self.stop.load(Ordering::Acquire)
        {
            return;
        }
        let base = if relative {
            s.request
                .seek
                .map_or(s.snapshot.position, |(_, target)| target)
        } else {
            0.0
        };
        let target = (base + seconds).clamp(0.0, s.snapshot.duration.max(0.0));
        if s.request.seek.is_some_and(|(_, old)| old == target) {
            return;
        }
        s.request.revision += 1;
        s.request.seek = Some((s.request.revision, target));
        s.snapshot.ended = false;
        drop(s);
        self.shared.1.notify_one();
    }
    pub fn volume(&self, volume: f32) {
        if !volume.is_finite() {
            return;
        }
        let mut s = self.shared.0.lock().unwrap();
        s.request.volume = volume.clamp(0.0, 1.0);
        s.snapshot.volume = s.request.volume;
        s.request.revision += 1;
        drop(s);
        self.shared.1.notify_one();
    }
    fn start(&self) {
        let mut thread = self.thread.lock().unwrap();
        if thread.is_some() || self.stop.load(Ordering::Acquire) {
            return;
        }
        let shared = self.shared.clone();
        let generation = self.generation.clone();
        let stop = self.stop.clone();
        let wake = self.wake.clone();
        let factory = self.factory.clone();
        *thread = Some(
            thread::Builder::new()
                .name("photosort-playback".into())
                .spawn(move || {
                    let mut backend: Option<Box<dyn Backend>> = None;
                    let mut loaded = u64::MAX;
                    let mut applied = u64::MAX;
                    let mut applied_seek = None;
                    let mut seeking: Option<Instant> = None;
                    let mut serial = 0;
                    let mut checked = Instant::now();
                    let mut opening = Instant::now();
                    while !stop.load(Ordering::Acquire) {
                        let (media, revision, playing, seek, volume, current) = {
                            let s = shared.0.lock().unwrap();
                            (
                                s.request.media.clone(),
                                s.request.revision,
                                s.request.playing,
                                s.request.seek,
                                s.request.volume,
                                generation.load(Ordering::Acquire),
                            )
                        };
                        let cancelled = || {
                            stop.load(Ordering::Acquire)
                                || generation.load(Ordering::Acquire) != current
                        };
                        if current != loaded {
                            backend = None;
                            loaded = current;
                            applied = u64::MAX;
                            applied_seek = None;
                            seeking = None;
                            opening = Instant::now();
                            if let Some(media) = &media {
                                let result = (|| {
                                    anyhow::ensure!(
                                        Fingerprint::read(&media.path)? == media.fingerprint,
                                        "Video changed; reopen the folder"
                                    );
                                    factory(&media.path, &cancelled)
                                })();
                                if cancelled() {
                                    continue;
                                }
                                match result {
                                    Ok(value) => backend = Some(value),
                                    Err(e) => {
                                        let mut s = shared.0.lock().unwrap();
                                        if generation.load(Ordering::Acquire) == current {
                                            s.snapshot.loading = false;
                                            s.snapshot.error = Some(format!("{e:#}"));
                                        }
                                        drop(s);
                                        wake();
                                    }
                                }
                            }
                        }
                        if let Some(player) = &mut backend {
                            // Keep only the latest destination while the backend finishes its
                            // current seek. Transport/volume controls still apply immediately.
                            let target = seek.filter(|(revision, _)| {
                                seeking.is_none() && Some(*revision) != applied_seek
                            });
                            if revision != applied || target.is_some() {
                                player.control(playing, target.map(|(_, v)| v), volume);
                                if let Some((revision, _)) = target {
                                    applied_seek = Some(revision);
                                    seeking = Some(Instant::now());
                                }
                                applied = revision;
                            }
                            let result = (|| {
                                if checked.elapsed() >= Duration::from_millis(250) {
                                    checked = Instant::now();
                                    let m = media.as_ref().unwrap();
                                    anyhow::ensure!(
                                        Fingerprint::read(&m.path)? == m.fingerprint,
                                        "Video changed; reopen the folder"
                                    );
                                }
                                let value = player.poll()?;
                                anyhow::ensure!(
                                    value.ready || opening.elapsed() < Duration::from_secs(15),
                                    "Video preparation timed out"
                                );
                                anyhow::ensure!(
                                    !value.seeking
                                        || seeking.is_none_or(
                                            |start| start.elapsed() < Duration::from_secs(15)
                                        ),
                                    "Video seeking timed out"
                                );
                                Ok(value)
                            })();
                            if cancelled() {
                                continue;
                            }
                            let mut s = shared.0.lock().unwrap();
                            let mut changed = false;
                            let mut failed = false;
                            if generation.load(Ordering::Acquire) == current {
                                match result {
                                    Ok(value) => {
                                        if !value.seeking {
                                            seeking = None;
                                        }
                                        // A new request may arrive during poll. Do not publish an
                                        // obsolete frame/EOF or clear that newer destination.
                                        let pending = s.request.seek.map(|(revision, _)| revision);
                                        if value.seeking
                                            || (pending.is_some() && pending != applied_seek)
                                        {
                                            // Keep the last accepted frame while in flight.
                                        } else {
                                            changed = s.request.seek.take().is_some();
                                            let snapshot = &mut s.snapshot;
                                            changed |= value.frame.is_some()
                                                || snapshot.ready != value.ready
                                                || snapshot.playing != value.playing
                                                || snapshot.ended != value.ended
                                                || (snapshot.position - value.position).abs()
                                                    > 0.015
                                                || snapshot.duration != value.duration;
                                            snapshot.loading = !value.ready;
                                            snapshot.ready = value.ready;
                                            snapshot.playing = value.playing;
                                            snapshot.ended = value.ended;
                                            snapshot.position = value.position;
                                            snapshot.duration = value.duration;
                                            if let Some(pixels) = value.frame {
                                                serial += 1;
                                                snapshot.frame = Some(Frame {
                                                    serial,
                                                    pixels: Arc::new(pixels),
                                                });
                                            }
                                            if value.ended
                                                && s.request.revision == revision
                                                && s.request.playing
                                            {
                                                s.request.playing = false;
                                                s.request.revision += 1;
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        changed = true;
                                        s.snapshot.error = Some(format!("{e:#}"));
                                        s.snapshot.loading = false;
                                        s.snapshot.playing = false;
                                        s.request.playing = false;
                                        s.request.seek = None;
                                        failed = true;
                                    }
                                }
                            }
                            drop(s);
                            // Releasing native media may do work; never hold the UI snapshot lock.
                            if failed {
                                backend = None;
                            }
                            if changed {
                                wake();
                            }
                        }
                        let lock = shared.0.lock().unwrap();
                        if stop.load(Ordering::Acquire)
                            || generation.load(Ordering::Acquire) != current
                            || lock.request.revision != revision
                        {
                            continue;
                        }
                        let delay = if media.is_none() || backend.is_none() {
                            Duration::from_secs(3600)
                        } else if playing
                            || lock.request.seek.is_some()
                            || lock.snapshot.loading
                            || lock.snapshot.frame.is_none()
                        {
                            Duration::from_millis(33)
                        } else {
                            Duration::from_millis(250)
                        };
                        let _ = shared.1.wait_timeout(lock, delay).unwrap();
                    }
                })
                .expect("Start playback owner"),
        );
    }
    pub fn shutdown(&self) {
        {
            // Pair the predicate change with the condition-variable mutex, so
            // shutdown cannot notify between the owner's check and its wait.
            let _guard = self.shared.0.lock().unwrap();
            self.stop.store(true, Ordering::Release);
            self.generation.fetch_add(1, Ordering::AcqRel);
        }
        self.shared.1.notify_all();
        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
        self.shared.0.lock().unwrap().snapshot = Snapshot::default();
    }
}
impl Drop for Player {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(all(test, any(target_os = "macos", feature = "portable-video")))]
pub(crate) fn test_player() -> Player {
    tests::player().0
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    #[derive(Default)]
    pub(super) struct Counts {
        opened: AtomicUsize,
        closed: AtomicUsize,
        polls: AtomicUsize,
    }
    struct Fake {
        counts: Arc<Counts>,
        playing: bool,
        position: f64,
        changed: bool,
        color: u8,
    }
    impl Backend for Fake {
        fn control(&mut self, playing: bool, seek: Option<f64>, _: f32) {
            self.playing = playing;
            if let Some(v) = seek {
                self.position = v;
                self.changed = true;
            }
        }
        fn poll(&mut self) -> Result<Decoded> {
            self.counts.polls.fetch_add(1, Ordering::Relaxed);
            if self.playing {
                self.position = (self.position + 0.1).min(30.0);
                self.changed = true;
            }
            let frame = std::mem::take(&mut self.changed)
                .then(|| RgbaImage::from_pixel(4, 2, image::Rgba([self.color, 0, 0, 255])));
            Ok(Decoded {
                position: self.position,
                duration: 30.0,
                ready: true,
                playing: self.playing && self.position < 30.0,
                ended: self.position >= 30.0,
                seeking: false,
                frame,
            })
        }
    }
    impl Drop for Fake {
        fn drop(&mut self) {
            self.counts.closed.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(super) fn player() -> (Player, Arc<Counts>) {
        let counts = Arc::new(Counts::default());
        let state = counts.clone();
        let player = Player::with_factory(
            Arc::new(|| {}),
            Arc::new(move |path, _| {
                state.opened.fetch_add(1, Ordering::Relaxed);
                Ok(Box::new(Fake {
                    counts: state.clone(),
                    playing: false,
                    position: 0.0,
                    changed: true,
                    color: path.file_name().unwrap().as_encoded_bytes()[0],
                }))
            }),
        );
        (player, counts)
    }
    fn media(root: &Path, name: &str) -> Media {
        let path = root.join(name);
        std::fs::write(&path, "fixture").unwrap();
        Media {
            id: MediaId::for_path(&path),
            fingerprint: Fingerprint::read(&path).unwrap(),
            path,
            kind: MediaKind::Video,
        }
    }
    fn wait(player: &Player, condition: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let until = Instant::now() + Duration::from_secs(3);
        loop {
            let s = player.snapshot();
            if condition(&s) {
                return s;
            }
            assert!(Instant::now() < until, "Playback worker stalled");
            thread::sleep(Duration::from_millis(2));
        }
    }
    #[derive(Default)]
    struct DelayedState {
        targets: Vec<f64>,
        complete: bool,
        playing: bool,
        volume: f32,
        opened: usize,
    }
    struct Delayed {
        state: Arc<Mutex<DelayedState>>,
        target: Option<f64>,
        position: f64,
    }
    impl Backend for Delayed {
        fn control(&mut self, playing: bool, seek: Option<f64>, volume: f32) {
            let mut state = self.state.lock().unwrap();
            state.playing = playing;
            state.volume = volume;
            if let Some(target) = seek {
                assert!(self.target.is_none(), "Do not interrupt an in-flight seek");
                self.target = Some(target);
                state.targets.push(target);
            }
        }
        fn poll(&mut self) -> Result<Decoded> {
            let mut state = self.state.lock().unwrap();
            if state.complete && self.target.is_some() {
                self.position = self.target.take().unwrap();
                state.complete = false;
            }
            Ok(Decoded {
                position: self.position,
                duration: 120.0,
                ready: true,
                playing: state.playing,
                // Deliberately expose stale EOF and pixels during the seek.
                ended: self.target.is_some(),
                seeking: self.target.is_some(),
                frame: Some(RgbaImage::from_pixel(
                    2,
                    2,
                    image::Rgba([self.position as u8, 0, 0, 255]),
                )),
            })
        }
    }
    #[test]
    fn delayed_seeks_accumulate_coalesce_and_discard_obsolete_pixels_and_eof() {
        let root = tempfile::tempdir().unwrap();
        let a = media(root.path(), "a.mp4");
        let b = media(root.path(), "b.mp4");
        let state = Arc::new(Mutex::new(DelayedState::default()));
        let shared = state.clone();
        let player = Player::with_factory(
            Arc::new(|| {}),
            Arc::new(move |_, _| {
                shared.lock().unwrap().opened += 1;
                Ok(Box::new(Delayed {
                    state: shared.clone(),
                    target: None,
                    position: 0.0,
                }))
            }),
        );
        player.select(Some(&a));
        wait(&player, |s| s.ready);
        player.seek_relative(10.0);
        wait(&player, |_| state.lock().unwrap().targets.len() == 1);
        player.seek_relative(10.0);
        player.seek_relative(10.0);
        assert_eq!(player.snapshot().seek_target, Some(30.0));
        player.seek(f64::NAN);
        player.seek_relative(f64::INFINITY);
        assert_eq!(player.snapshot().seek_target, Some(30.0));
        player.toggle();
        player.volume(0.25);
        wait(&player, |_| {
            let s = state.lock().unwrap();
            s.playing && s.volume == 0.25
        });
        let snapshot = player.snapshot();
        assert_eq!(snapshot.position, 0.0);
        assert!(!snapshot.ended);
        assert!(snapshot.play_requested);
        assert_eq!(state.lock().unwrap().targets, vec![10.0]);
        state.lock().unwrap().complete = true;
        wait(&player, |_| state.lock().unwrap().targets.len() == 2);
        let snapshot = player.snapshot();
        assert_eq!(
            snapshot.position, 0.0,
            "Old seek completion must not publish"
        );
        assert_eq!(snapshot.frame.unwrap().pixels.get_pixel(0, 0).0[0], 0);
        assert_eq!(snapshot.seek_target, Some(30.0));
        assert_eq!(state.lock().unwrap().targets, vec![10.0, 30.0]);
        state.lock().unwrap().complete = true;
        let snapshot = wait(&player, |s| s.seek_target.is_none() && s.position == 30.0);
        assert_eq!(snapshot.frame.unwrap().pixels.get_pixel(0, 0).0[0], 30);
        assert_eq!(state.lock().unwrap().opened, 1);
        player.seek_relative(10.0);
        wait(&player, |_| state.lock().unwrap().targets.len() == 3);
        assert_eq!(
            player.snapshot().seek_target,
            Some(40.0),
            "Use settled position for next skip"
        );
        player.select(Some(&b));
        let snapshot = wait(&player, |s| s.ready && s.id == Some(b.id));
        assert!(snapshot.seek_target.is_none());
        assert_eq!(snapshot.position, 0.0);
        assert_eq!(state.lock().unwrap().opened, 2);
        player.shutdown();
    }

    #[test]
    fn lazy_shared_player_reuses_selection_coalesces_seeks_and_retains_one_frame() {
        let root = tempfile::tempdir().unwrap();
        let m = media(root.path(), "a.mp4");
        let (player, counts) = player();
        assert!(player.thread.lock().unwrap().is_none());
        player.select(None);
        assert!(player.thread.lock().unwrap().is_none());
        player.select(Some(&m));
        let first = wait(&player, |s| s.frame.is_some());
        assert!(!first.playing);
        for _ in 0..20 {
            player.select(Some(&m));
        }
        assert_eq!(counts.opened.load(Ordering::Relaxed), 1);
        let old = Arc::downgrade(&first.frame.as_ref().unwrap().pixels);
        drop(first);
        for n in 0..2000 {
            player.seek(n as f64 / 100.0);
        }
        wait(&player, |s| (s.position - 19.99).abs() < 0.0001);
        assert!(old.upgrade().is_none());
        player.seek(f64::NAN);
        player.volume(f32::NAN);
        assert_eq!(player.snapshot().volume, 0.5);
        player.volume(10.0);
        assert_eq!(player.snapshot().volume, 1.0);
        player.toggle();
        wait(&player, |s| s.playing);
        player.pause();
        wait(&player, |s| !s.playing);
        player.select(None);
        assert!(player.snapshot().frame.is_none());
        player.shutdown();
        assert_eq!(
            counts.opened.load(Ordering::Relaxed),
            counts.closed.load(Ordering::Relaxed)
        );
    }
    #[test]
    fn blocked_open_cancels_on_selection_change_and_shutdown_without_publishing_stale_errors() {
        let root = tempfile::tempdir().unwrap();
        let a = media(root.path(), "a.mp4");
        let b = media(root.path(), "b.mp4");
        let entered = Arc::new(AtomicUsize::new(0));
        let signal = entered.clone();
        let player = Player::with_factory(
            Arc::new(|| {}),
            Arc::new(move |_, cancelled| {
                signal.fetch_add(1, Ordering::Release);
                while !cancelled() {
                    thread::sleep(Duration::from_millis(1));
                }
                anyhow::bail!("Cancelled open")
            }),
        );
        player.select(Some(&a));
        let until = Instant::now() + Duration::from_secs(2);
        while entered.load(Ordering::Acquire) < 1 {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
        player.select(Some(&b));
        while entered.load(Ordering::Acquire) < 2 {
            assert!(Instant::now() < until);
            thread::sleep(Duration::from_millis(1));
        }
        let snapshot = player.snapshot();
        assert_eq!(snapshot.id, Some(b.id));
        assert!(snapshot.error.is_none());
        assert!(snapshot.frame.is_none());
        let start = Instant::now();
        player.shutdown();
        assert!(start.elapsed() < Duration::from_millis(500));
        player.select(Some(&a));
        assert!(player.snapshot().id.is_none());
    }
    #[test]
    fn changed_source_stops_playback_and_releases_native_owner() {
        let root = tempfile::tempdir().unwrap();
        let m = media(root.path(), "a.mp4");
        let (player, counts) = player();
        player.select(Some(&m));
        wait(&player, |s| s.ready);
        player.toggle();
        wait(&player, |s| s.playing);
        std::fs::write(&m.path, "changed content").unwrap();
        let error = wait(&player, |s| s.error.is_some());
        assert!(!error.playing);
        player.shutdown();
        assert_eq!(counts.closed.load(Ordering::Relaxed), 1);
    }
    #[cfg(all(feature = "portable-video", not(target_os = "macos")))]
    #[test]
    fn portable_decoder_uses_one_owner_and_discards_old_frames_on_navigation() {
        gst::init().unwrap();
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video");
        let catalog = crate::core::catalog::Catalog::scan(&root, || false, |_| {}).unwrap();
        let opened = Arc::new(AtomicUsize::new(0));
        let count = opened.clone();
        let player = Player::with_factory(
            Arc::new(|| {}),
            Arc::new(move |path, cancelled| {
                count.fetch_add(1, Ordering::Relaxed);
                let audio = gst::ElementFactory::make("fakesink")
                    .property("sync", true)
                    .build()?;
                Ok(Box::new(portable::Player::open_with_audio(
                    path,
                    cancelled,
                    Some(audio),
                )?))
            }),
        );
        let a = catalog
            .media
            .iter()
            .find(|m| m.name() == "motion.mp4")
            .unwrap();
        let b = catalog
            .media
            .iter()
            .find(|m| m.name() == "rotated.mp4")
            .unwrap();
        player.select(Some(a));
        let first = wait(&player, |s| s.ready && s.frame.is_some());
        let old = Arc::downgrade(&first.frame.as_ref().unwrap().pixels);
        let serial = first.frame.as_ref().unwrap().serial;
        drop(first);
        for _ in 0..100 {
            player.select(Some(a));
        }
        thread::sleep(Duration::from_millis(300));
        assert_eq!(player.snapshot().frame.unwrap().serial, serial);
        assert_eq!(opened.load(Ordering::Relaxed), 1);
        player.seek(1.5);
        wait(&player, |s| {
            s.frame.as_ref().is_some_and(|f| f.serial > serial) && (s.position - 1.5).abs() < 0.08
        });
        assert!(old.upgrade().is_none());
        player.toggle();
        wait(&player, |s| s.playing && s.position > 1.7);
        player.select(Some(b));
        let next = wait(&player, |s| s.id == Some(b.id) && s.frame.is_some());
        assert_eq!(next.frame.unwrap().pixels.dimensions(), (180, 320));
        assert!(!next.playing);
        assert_eq!(opened.load(Ordering::Relaxed), 2);
        player.select(None);
        assert!(player.snapshot().frame.is_none());
        let start = Instant::now();
        player.shutdown();
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
