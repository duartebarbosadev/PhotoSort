use super::*;
use crate::test_support::worker_patience;
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
    let until = Instant::now() + worker_patience();
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
    let until = Instant::now() + worker_patience();
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
