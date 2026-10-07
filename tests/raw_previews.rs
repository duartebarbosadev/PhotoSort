//! Native RAW preview/detail contracts. Only independent copies receive sidecars or corruptions.
#![cfg(feature = "raw")]
use photosort::core::{
    catalog::Catalog,
    inspection::{Region, Request},
    metadata,
    model::*,
    operations,
    pipeline::Pipeline,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR/arw_sample.ARW"]
fn shared_raw_preview_geometry_recipes_reuse_and_fallbacks() -> anyhow::Result<()> {
    let source =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap()).join("arw_sample.ARW");
    let original_hash = operations::content_hash(&source, || false)?;
    let temp = tempfile::tempdir()?;
    let normal = temp.path().join("normal.ARW");
    std::fs::copy(&source, &normal)?;
    let normal = normal.canonicalize()?;
    let bytes = std::fs::read(&normal)?;
    assert_eq!(&bytes[..4], b"II*\0");
    let u16_at = |at: usize| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let mut missing = bytes.clone();
    let mut corrupt = bytes.clone();
    let mut ifd = u32_at(4) as usize;
    let mut previews = 0;
    // TIFF's top-level IFD chain contains both this fixture's embedded JPEGs.
    while ifd != 0 {
        let count = u16_at(ifd) as usize;
        let mut jpeg = None;
        let mut length = None;
        for n in 0..count {
            let at = ifd + 2 + n * 12;
            match u16_at(at) {
                0x201 => {
                    jpeg = Some(u32_at(at + 8) as usize);
                    missing[at + 8..at + 12].fill(0);
                }
                0x202 => {
                    length = Some(u32_at(at + 8) as usize);
                    missing[at + 8..at + 12].fill(0);
                }
                _ => {}
            }
        }
        if let (Some(start), Some(length)) = (jpeg, length) {
            assert!(length > 2 && start + length <= corrupt.len());
            corrupt[start..start + length].fill(0);
            previews += 1;
        }
        ifd = u32_at(ifd + 2 + count * 12) as usize;
    }
    assert_eq!(previews, 2, "Fixture changed; inspect its TIFF structure");
    std::fs::write(temp.path().join("missing.ARW"), missing)?;
    std::fs::write(temp.path().join("corrupt.ARW"), corrupt)?;
    let media = Catalog::scan(temp.path(), || false, |_| {})?.media;
    for photo in &media {
        let fallback = photo.path != normal;
        for orientation in [1, 2, 6, 8] {
            metadata::write_sidecar(&photo.path, Some(4), Some(orientation))?;
            for display_first in [false, true] {
                let cache = tempfile::tempdir()?;
                let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
                if display_first {
                    pipeline.load(photo)?;
                }
                let analysis = pipeline.analysis(photo, || false)?;
                let display = pipeline.load(photo)?;
                assert_eq!(
                    (display.metadata.width, display.metadata.height),
                    (6024, 4024)
                );
                assert_eq!(display.metadata.orientation, orientation);
                assert_eq!(display.metadata.rating, 4);
                assert!(display.pixels.width().max(display.pixels.height()) <= 2048);
                assert!(analysis.pixels.width().max(analysis.pixels.height()) <= 1024);
                assert_eq!(
                    (pipeline.stats.decodes, pipeline.stats.metadata_reads),
                    (1, 1)
                );
                assert_eq!(
                    (pipeline.stats.raw_embedded, pipeline.stats.raw_fallback),
                    if fallback { (0, 1) } else { (1, 0) },
                    "{}",
                    photo.name()
                );
                for _ in Step::ALL {
                    assert!(Arc::ptr_eq(&display.pixels, &pipeline.load(photo)?.pixels));
                    assert!(Arc::ptr_eq(
                        &analysis.pixels,
                        &pipeline.analysis(photo, || false)?.pixels
                    ));
                }
                let mut reopened = Pipeline::new(128 * 1024 * 1024, cache.path().into());
                assert_eq!(reopened.load(photo)?.pixels, display.pixels);
                assert_eq!(reopened.analysis(photo, || false)?.pixels, analysis.pixels);
                assert_eq!((reopened.stats.decodes, reopened.stats.disk_hits), (0, 2));
                let (width, height) = if orientation >= 5 {
                    (4024, 6024)
                } else {
                    (6024, 4024)
                };
                let request = Request {
                    slot: 0,
                    media: photo.clone(),
                    region: Region {
                        x: width - 32,
                        y: height - 32,
                        width: 32,
                        height: 32,
                    },
                    output_edge: 32,
                };
                let detail = pipeline.inspect(&request)?;
                assert_eq!(detail.dimensions(), (32, 32));
                assert_eq!(
                    pipeline.stats.decodes, 2,
                    "Full RAW rendering is deferred until inspection"
                );
                assert_eq!(pipeline.inspect(&request)?, detail);
                assert_eq!(pipeline.stats.decodes, 2);
                assert_eq!(
                    pipeline.analysis(photo, || false)?.pixels,
                    analysis.pixels,
                    "Inspecting full RAW must not change the analysis recipe"
                );
                assert_eq!(
                    (
                        pipeline.metadata(photo)?.width,
                        pipeline.metadata(photo)?.height
                    ),
                    (6024, 4024)
                );
            }
        }
    }
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let photo = media.iter().find(|m| m.path == normal).unwrap();
    let polls = AtomicUsize::new(0);
    assert!(
        pipeline
            .load_cancellable(photo, || polls.fetch_add(1, Ordering::Relaxed) >= 6)
            .is_err()
    );
    assert_eq!(pipeline.stats.resident_bytes, 0);
    assert_eq!(pipeline.stats.detail_bytes, 0);
    pipeline.load(photo)?;
    let before = pipeline.stats.decodes;
    metadata::write_sidecar(&photo.path, Some(2), Some(3))?;
    let replaced = pipeline.load(photo)?;
    assert_eq!(replaced.metadata.orientation, 3);
    assert_eq!(replaced.metadata.rating, 2);
    assert_eq!(
        pipeline.stats.decodes,
        before + 1,
        "Sidecar change must reject old prepared pixels"
    );
    assert_eq!(operations::content_hash(&source, || false)?, original_hash);
    Ok(())
}

#[test]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR/arw_sample.ARW"]
fn raw_analysis_only_geometry_survives_metadata_only_restart() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap());
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    let photo = catalog
        .media
        .iter()
        .find(|m| m.name() == "arw_sample.ARW")
        .unwrap();
    let cache = tempfile::tempdir()?;
    let mut first = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let analysis = first.analysis(photo, || false)?;
    assert_eq!(
        (analysis.metadata.width, analysis.metadata.height),
        (6024, 4024)
    );
    drop(first);
    let mut reopened = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let metadata = reopened.metadata(photo)?;
    assert_eq!(
        serde_json::to_value(metadata)?,
        serde_json::to_value(analysis.metadata)?,
        "Metadata-only cache hits must retain decoded RAW geometry"
    );
    assert_eq!(
        (
            reopened.stats.decodes,
            reopened.stats.metadata_reads,
            reopened.stats.metadata_disk_hits
        ),
        (0, 0, 1)
    );
    assert_eq!(
        (reopened.stats.resident_bytes, reopened.stats.detail_bytes),
        (0, 0)
    );
    Ok(())
}
