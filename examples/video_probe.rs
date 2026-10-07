#[cfg(target_os = "macos")]
fn main() {
    use photosort::core::{catalog::Catalog, playback::Player};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    fn wait(
        player: &Player,
        condition: impl Fn(&photosort::core::playback::Snapshot) -> bool,
    ) -> photosort::core::playback::Snapshot {
        let deadline = Instant::now() + Duration::from_secs(12);
        loop {
            let snapshot = player.snapshot();
            assert!(snapshot.error.is_none(), "{:?}", snapshot.error);
            if condition(&snapshot) {
                return snapshot;
            }
            assert!(
                Instant::now() < deadline,
                "Timed out: ready {} loading {} playing {} pos {} duration {} frame {}",
                snapshot.ready,
                snapshot.loading,
                snapshot.playing,
                snapshot.position,
                snapshot.duration,
                snapshot.frame.is_some()
            );
            pump_main();
        }
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video");
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let media = |name: &str| catalog.media.iter().find(|m| m.name() == name).unwrap();
    let hashes: Vec<_> = catalog
        .media
        .iter()
        .map(|m| photosort::core::operations::content_hash(&m.path, || false).unwrap())
        .collect();
    let cache = tempfile::tempdir().unwrap();
    let mut pipeline =
        photosort::core::pipeline::Pipeline::new(32 * 1024 * 1024, cache.path().into());
    for (name, dimensions) in [
        ("motion.mp4", (320, 180)),
        ("rotated.mp4", (180, 320)),
        ("large.mp4", (1280, 720)),
    ] {
        let poster = pipeline.load(media(name)).unwrap();
        assert_eq!(poster.pixels.dimensions(), dimensions);
        let expected_source = if name == "large.mp4" {
            (3840, 2160)
        } else {
            dimensions
        };
        assert_eq!(
            (poster.metadata.width, poster.metadata.height),
            expected_source
        );
        let again = pipeline.load(media(name)).unwrap();
        assert!(Arc::ptr_eq(&poster.pixels, &again.pixels));
    }
    assert_eq!(pipeline.stats.decodes, 3);
    assert_eq!(pipeline.stats.metadata_reads, 3);
    // Reopen disk posters with a fresh owner: geometry remains source-sized,
    // without another metadata read or decode. Repeat a metadata-only request.
    let mut restarted =
        photosort::core::pipeline::Pipeline::new(32 * 1024 * 1024, cache.path().into());
    for (name, expected) in [
        ("motion.mp4", (320, 180)),
        ("rotated.mp4", (180, 320)),
        ("large.mp4", (3840, 2160)),
    ] {
        let poster = restarted.load(media(name)).unwrap();
        assert_eq!((poster.metadata.width, poster.metadata.height), expected);
        let meta = restarted.metadata(media(name)).unwrap();
        assert_eq!((meta.width, meta.height), expected);
    }
    assert_eq!(restarted.stats.decodes, 0);
    assert_eq!(restarted.stats.metadata_reads, 0);
    assert_eq!(restarted.stats.disk_hits, 3);
    assert_eq!(restarted.stats.metadata_disk_hits, 3);
    let player = Player::new(Arc::new(|| {}));
    player.volume(0.0);
    player.select(Some(media("motion.mp4")));
    let start = wait(&player, |s| s.ready && s.frame.is_some());
    assert!(!start.playing);
    assert!((2.9..3.2).contains(&start.duration));
    assert_eq!(start.frame.unwrap().pixels.dimensions(), (320, 180));
    player.seek(0.0);
    wait(&player, |s| s.seek_target.is_none() && s.position < 0.08);
    player.seek_relative(0.4);
    player.seek_relative(0.4);
    player.seek_relative(0.4);
    wait(&player, |s| {
        s.seek_target.is_none() && (s.position - 1.2).abs() < 0.08
    });
    for i in 0..100 {
        player.seek(i as f64 / 50.0);
    }
    wait(&player, |s| {
        s.seek_target.is_none() && (s.position - 1.98).abs() < 0.08
    });
    player.seek(100.0);
    wait(&player, |s| s.seek_target.is_none() && s.ended);
    player.seek(1.5);
    let sought = wait(&player, |s| {
        s.seek_target.is_none() && (s.position - 1.5).abs() < 0.08 && s.frame.is_some()
    });
    assert!(!sought.playing);
    player.toggle();
    wait(&player, |s| s.playing && s.position > 1.7);
    player.pause();
    let paused = wait(&player, |s| !s.playing);
    std::thread::sleep(Duration::from_millis(200));
    assert!((player.snapshot().position - paused.position).abs() < 0.1);
    player.seek(2.8);
    player.toggle();
    wait(&player, |s| s.ended);
    player.toggle();
    wait(&player, |s| s.playing && s.position < 1.0);
    player.select(Some(media("rotated.mp4")));
    let rotated = wait(&player, |s| s.ready && s.frame.is_some());
    assert!(!rotated.playing);
    assert_eq!(rotated.frame.unwrap().pixels.dimensions(), (180, 320));
    player.select(Some(media("large.mp4")));
    let large = wait(&player, |s| s.ready && s.frame.is_some());
    let pixels = &large.frame.unwrap().pixels;
    assert_eq!(pixels.dimensions(), (1280, 720));
    let blue = pixels.get_pixel(640, 360).0;
    assert!(blue[2] > 200 && blue[0] < 30 && blue[1] < 30, "{blue:?}");
    println!(
        "Native playback passed: paused open, accumulated/coalesced seeking, end clamp, playback, EOF/replay, rotated frames and bounded 4K output"
    );
    player.select(None);
    assert!(player.snapshot().frame.is_none());
    let start = Instant::now();
    player.shutdown();
    assert!(start.elapsed() < Duration::from_secs(2));
    for (m, hash) in catalog.media.iter().zip(hashes) {
        assert_eq!(
            photosort::core::operations::content_hash(&m.path, || false).unwrap(),
            hash
        );
    }
}

#[cfg(target_os = "macos")]
fn pump_main() {
    use std::ffi::c_void;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFRunLoopDefaultMode: *const c_void;
        fn CFRunLoopRunInMode(mode: *const c_void, seconds: f64, return_after_source: u8) -> i32;
    }
    unsafe {
        CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.01, 0);
    }
}
#[cfg(not(target_os = "macos"))]
fn main() {
    println!("Native video probe requires macOS");
}
