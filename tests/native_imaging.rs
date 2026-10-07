#![cfg(feature = "native-imaging")]
use image::{ImageEncoder, Rgba, RgbaImage};
use photosort::core::{
    catalog::Catalog,
    inspection::{Region, Request},
    model::Step,
    pipeline::Pipeline,
};
use std::{fs::File, path::Path, sync::Arc};
fn image(path: &Path, width: u32, height: u32) {
    RgbaImage::from_fn(width, height, |x, y| {
        Rgba([(x % 255) as u8, (y % 255) as u8, ((x + y) % 255) as u8, 255])
    })
    .save(path)
    .unwrap();
}
fn request(media: &photosort::core::model::Media) -> Request {
    Request {
        slot: 0,
        media: media.clone(),
        region: Region {
            x: 100,
            y: 50,
            width: 100,
            height: 100,
        },
        output_edge: 100,
    }
}
#[test]
fn all_workflows_share_bounded_working_pixels_and_defer_originals() {
    let root = tempfile::tempdir().unwrap();
    image(&root.path().join("large.png"), 3000, 1500);
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let disk = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(32 * 1024 * 1024, disk.path().into());
    let preview = pipeline.load(&media).unwrap();
    let analysis = pipeline.analysis(&media, || false).unwrap();
    assert_eq!(preview.pixels.dimensions(), (2048, 1024));
    assert_eq!(analysis.pixels.dimensions(), (1024, 512));
    assert_eq!(
        (preview.metadata.width, preview.metadata.height),
        (3000, 1500)
    );
    assert_eq!(
        (
            pipeline.stats.decodes,
            pipeline.stats.native_renders,
            pipeline.stats.metadata_reads
        ),
        (1, 1, 1)
    );
    assert_eq!(pipeline.stats.detail_bytes, 2048 * 1024 * 4);
    for _ in Step::ALL {
        assert!(Arc::ptr_eq(
            &preview.pixels,
            &pipeline.load(&media).unwrap().pixels
        ));
        assert!(Arc::ptr_eq(
            &analysis.pixels,
            &pipeline.analysis(&media, || false).unwrap().pixels
        ));
        let original = pipeline.inspect(&request(&media)).unwrap();
        assert_eq!(original.get_pixel(0, 0).0, [100, 50, 150, 255]);
    }
    assert_eq!(
        (
            pipeline.stats.decodes,
            pipeline.stats.native_renders,
            pipeline.stats.metadata_reads
        ),
        (2, 2, 1)
    );
}
#[test]
fn original_first_cannot_change_analysis_or_evict_a_useful_working_image() {
    let root = tempfile::tempdir().unwrap();
    image(&root.path().join("large.png"), 3000, 1500);
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let mut results = Vec::new();
    for original_first in [false, true] {
        let disk = tempfile::tempdir().unwrap();
        let mut pipeline = Pipeline::new(32 * 1024 * 1024, disk.path().into());
        if original_first {
            pipeline.inspect(&request(&media)).unwrap();
        }
        results.push(pipeline.analysis(&media, || false).unwrap().pixels);
        pipeline.load(&media).unwrap();
        assert_eq!(
            pipeline.stats.native_renders,
            if original_first { 2 } else { 1 }
        );
    }
    assert_eq!(*results[0], *results[1]);
    let disk = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(5 * 1024 * 1024, disk.path().into());
    pipeline.load(&media).unwrap();
    let working = pipeline.stats.detail_bytes;
    pipeline.inspect(&request(&media)).unwrap();
    assert_eq!(pipeline.stats.detail_bytes, working + 512 * 512 * 4);
    pipeline.analysis(&media, || false).unwrap();
    assert_eq!(
        pipeline.stats.native_renders, 2,
        "oversized original evicted reusable working pixels"
    );
}
#[test]
fn color_fallback_warning_survives_both_proxy_caches_and_requires_its_status_record() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("invalid-profile.png");
    let mut encoder = image::codecs::png::PngEncoder::new(File::create(&path).unwrap());
    encoder.set_icc_profile(vec![0; 128]).unwrap();
    encoder
        .write_image(
            &[128, 128, 128, 255].repeat(200 * 200),
            200,
            200,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let disk = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(8 * 1024 * 1024, disk.path().into());
    let warning = pipeline
        .load(&media)
        .unwrap()
        .decoder_warning
        .expect("loader discarded a profile");
    assert!(warning.contains("iCCP"));
    assert_eq!(
        pipeline
            .analysis(&media, || false)
            .unwrap()
            .decoder_warning
            .as_deref(),
        Some(warning.as_str())
    );
    let (_, original_warning) = pipeline
        .inspect_with_warning(&request(&media), &|| false)
        .unwrap();
    assert_eq!(original_warning.as_deref(), Some(warning.as_str()));
    assert_eq!(pipeline.stats.native_renders, 1);
    let mut reopened = Pipeline::new(8 * 1024 * 1024, disk.path().into());
    for asset in [
        reopened.load(&media).unwrap(),
        reopened.analysis(&media, || false).unwrap(),
    ] {
        assert_eq!(asset.decoder_warning.as_deref(), Some(warning.as_str()));
    }
    assert_eq!(reopened.stats.native_renders, 0);
    std::fs::remove_file(
        disk.path()
            .join(format!("{}.json", Pipeline::key(&media).unwrap())),
    )
    .unwrap();
    let mut reopened = Pipeline::new(8 * 1024 * 1024, disk.path().into());
    assert_eq!(
        reopened.load(&media).unwrap().decoder_warning.as_deref(),
        Some(warning.as_str())
    );
    assert_eq!(
        reopened.stats.native_renders, 1,
        "pixels without their warning record must be rebuilt"
    );
}
#[test]
#[ignore = "Uses seven original samples; set PHOTOSORT_SAMPLE_DIR"]
fn original_samples_use_shared_pipeline_and_restart_without_decoding() {
    let root = std::path::PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap());
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    assert_eq!(catalog.media.len(), 7);
    let hashes: Vec<_> = catalog
        .media
        .iter()
        .map(|m| photosort::core::operations::content_hash(&m.path, || false).unwrap())
        .collect();
    let disk = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, disk.path().into());
    for media in &catalog.media {
        let preview = pipeline.load(media).unwrap();
        let analysis = pipeline.analysis(media, || false).unwrap();
        assert!(preview.pixels.width() <= 2048 && preview.pixels.height() <= 2048);
        assert!(analysis.pixels.width() <= 1024 && analysis.pixels.height() <= 1024);
    }
    assert_eq!(pipeline.stats.native_renders, 6);
    let mut reopened = Pipeline::new(128 * 1024 * 1024, disk.path().into());
    for (media, hash) in catalog.media.iter().zip(hashes) {
        reopened.load(media).unwrap();
        reopened.analysis(media, || false).unwrap();
        assert_eq!(
            photosort::core::operations::content_hash(&media.path, || false).unwrap(),
            hash
        );
    }
    assert_eq!((reopened.stats.decodes, reopened.stats.disk_hits), (0, 14));
}
