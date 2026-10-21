use photosort::core::{
    cache::{self, Limits},
    catalog::Catalog,
    jobs::{AnalysisProfile, Event, Runtime},
    pipeline::Pipeline,
    preferences::Preferences,
    startup::Options,
};
use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};
fn artifact(dir: &Path, n: usize, suffix: &str, bytes: &[u8]) -> std::path::PathBuf {
    let p = dir.join(format!("{n:064x}.{suffix}"));
    std::fs::write(&p, bytes).unwrap();
    p
}
#[test]
fn disk_maintenance_owns_only_generated_artifacts_and_preserves_data() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    std::fs::create_dir(&cache).unwrap();
    artifact(&cache, 1, "png", b"1234");
    artifact(&cache, 1, "json", b"12");
    artifact(&cache, 2, "tmp.png", b"123");
    artifact(&cache, 3, "png", b"");
    artifact(&cache, 4, "bin", b"12345");
    let sentinels = [
        dir.path().join("sessions.json"),
        dir.path().join("views.json"),
        dir.path().join("photo.png"),
        cache.join("notes.json"),
        cache.join(format!("{:064x}.xmp", 5)),
    ];
    for path in &sentinels {
        std::fs::write(path, b"preserve").unwrap();
    }
    std::fs::create_dir(cache.join("nested")).unwrap();
    let nested = artifact(&cache.join("nested"), 4, "png", b"nested");
    let usage = cache::maintain(&cache, None, || false).unwrap();
    assert_eq!((usage.disk_bytes, usage.files), (14, 5));
    let pruned = cache::maintain(&cache, Some(4), || false).unwrap();
    assert!(pruned.disk_bytes <= 4);
    let empty = cache::maintain(&cache, Some(0), || false).unwrap();
    assert_eq!((empty.disk_bytes, empty.files), (0, 0));
    for path in sentinels {
        assert_eq!(std::fs::read(path).unwrap(), b"preserve");
    }
    assert_eq!(std::fs::read(nested).unwrap(), b"nested");
}
#[cfg(unix)]
#[test]
fn maintenance_never_follows_symlinks_or_accepts_a_symlink_root() {
    let dir = tempfile::tempdir().unwrap();
    let cache = dir.path().join("cache");
    std::fs::create_dir(&cache).unwrap();
    let photo = artifact(dir.path(), 1, "png", b"original");
    let link = cache.join(format!("{:064x}.png", 1));
    std::os::unix::fs::symlink(&photo, &link).unwrap();
    assert_eq!(cache::maintain(&cache, Some(0), || false).unwrap().files, 0);
    assert!(link.is_symlink());
    assert_eq!(std::fs::read(&photo).unwrap(), b"original");
    let alias = dir.path().join("alias");
    std::os::unix::fs::symlink(&cache, &alias).unwrap();
    assert!(cache::maintain(&alias, Some(0), || false).is_err());
}
#[test]
fn maintenance_cancels_and_missing_directory_is_a_noop() {
    let dir = tempfile::tempdir().unwrap();
    artifact(dir.path(), 1, "png", b"data");
    assert!(
        cache::maintain(dir.path(), Some(0), || true)
            .unwrap_err()
            .to_string()
            .contains("cancelled")
    );
    assert_eq!(
        cache::maintain(dir.path(), None, || false).unwrap().files,
        1
    );
    assert_eq!(
        cache::maintain(&dir.path().join("absent"), Some(0), || false)
            .unwrap()
            .files,
        0
    );
}
#[test]
fn cache_preferences_are_bounded_backward_compatible_and_cli_parses() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefs.json");
    std::fs::write(&path, b"[1,{}]").unwrap();
    let mut p = Preferences::load(&path).unwrap();
    assert_eq!(p.cache, Limits::default());
    p.cache = Limits {
        previews_mib: 0,
        originals_mib: u32::MAX,
        disk_mib: u32::MAX,
    };
    p.save(&path).unwrap();
    let loaded = Preferences::load(&path).unwrap();
    assert_eq!(
        loaded.cache,
        Limits {
            previews_mib: 32,
            originals_mib: 2048,
            disk_mib: 32768
        }
    );
    assert!(
        Options::parse(["--clear-cache".into(), "--last-folder".into()])
            .unwrap()
            .clear_cache
    );
}
#[test]
fn lowered_memory_limits_evict_without_redecoding_and_zero_disk_disables_writes() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("photos");
    std::fs::create_dir(&library).unwrap();
    for i in 0..3 {
        image::RgbaImage::new(2400, 2400)
            .save(library.join(format!("{i}.png")))
            .unwrap();
    }
    let media = Catalog::scan(&library, || false, |_| {}).unwrap().media;
    let disk = dir.path().join("cache");
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, disk.clone());
    for m in &media {
        pipeline.load(m).unwrap();
    }
    let decodes = pipeline.stats.decodes;
    assert_eq!(decodes, 3);
    pipeline.configure_cache(Limits {
        previews_mib: 32,
        originals_mib: 64,
        disk_mib: 0,
    });
    assert!(
        pipeline.stats.resident_bytes <= 32 * 1024 * 1024
            && pipeline.stats.detail_bytes <= 64 * 1024 * 1024
    );
    assert_eq!(pipeline.stats.decodes, decodes);
    pipeline.clear_memory();
    pipeline.maintain_cache(true, || false).unwrap();
    pipeline.load(&media[0]).unwrap();
    assert_eq!(cache::maintain(&disk, None, || false).unwrap().files, 0);
}
fn wait_cache(runtime: &Runtime) -> cache::Status {
    let until = Instant::now() + Duration::from_secs(10);
    loop {
        let status = runtime.cache.status();
        if !status.busy {
            return status;
        }
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(5));
    }
}
fn analyze(
    runtime: &Runtime,
    media: Arc<Vec<photosort::core::model::Media>>,
) -> photosort::core::jobs::AnalysisStats {
    let run = runtime.analyze(media, AnalysisProfile::Technical).unwrap();
    loop {
        match runtime
            .events
            .recv_timeout(Duration::from_secs(10))
            .unwrap()
            .event
        {
            Event::AnalysisDone { run: got, stats } if got == run => return stats,
            Event::AnalysisError { error, .. } | Event::Error(error) => panic!("{error}"),
            _ => {}
        }
    }
}
#[test]
fn worker_cache_controls_reuse_expensive_work_and_survive_folder_epochs() {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("photos");
    std::fs::create_dir(&library).unwrap();
    image::RgbaImage::new(640, 640)
        .save(library.join("sample.png"))
        .unwrap();
    let catalog = Catalog::scan(&library, || false, |_| {}).unwrap();
    let media = Arc::new(catalog.media);
    let runtime = Runtime::with_cache(|| {}, dir.path().join("cache"));
    let first = analyze(&runtime, media.clone());
    assert_eq!(first.decodes, 1);
    runtime.cache.configure(Limits {
        previews_mib: 64,
        ..Default::default()
    });
    let status = wait_cache(&runtime);
    assert!(status.usage.unwrap().disk_bytes > 0);
    assert_eq!(
        first,
        analyze(&runtime, media.clone()),
        "Limit changes must reuse measurements, pixels and models"
    );
    assert!(runtime.cache.request(true));
    runtime.next_epoch();
    let usage = wait_cache(&runtime).usage.unwrap();
    assert_eq!(
        (
            usage.disk_bytes,
            usage.files,
            usage.preview_bytes,
            usage.original_bytes,
            usage.metadata_entries,
            usage.analysis_entries
        ),
        (0, 0, 0, 0, 0, 0)
    );
    let after = analyze(&runtime, media);
    assert_eq!(after.decodes, first.decodes + 1);
    assert_eq!(
        after.technical_measurements,
        first.technical_measurements + 1
    );
    assert_eq!(after.model_initializations, 0);
    assert_eq!(after.model_inferences, 0);
}
