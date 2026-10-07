use photosort::core::{
    model::{Fingerprint, Media, MediaId, MediaKind},
    pipeline::Pipeline,
};
#[test]
fn cancelled_video_preview_starts_no_metadata_or_decoder_work() {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video/motion.mp4");
    let media = Media {
        id: MediaId::for_path(&path),
        kind: MediaKind::Video,
        fingerprint: Fingerprint::read(&path).unwrap(),
        path,
    };
    let cache = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
    assert!(pipeline.load_cancellable(&media, || true).is_err());
    assert_eq!(
        (pipeline.stats.decodes, pipeline.stats.metadata_reads),
        (0, 0)
    );
    assert_eq!(std::fs::read_dir(cache.path()).unwrap().count(), 0);
}
