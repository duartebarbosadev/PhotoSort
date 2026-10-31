#![cfg(all(feature = "heif", target_os = "macos"))]
use photosort::core::{
    cache::Limits,
    catalog::Catalog,
    inspection::{Region, Request},
    metadata,
    model::*,
    pipeline::{Pipeline, orient},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
fn full(pipeline: &mut Pipeline, media: &Media, dimensions: (u32, u32)) -> Arc<image::RgbaImage> {
    pipeline
        .inspect(&Request {
            slot: 0,
            media: media.clone(),
            region: Region::full(dimensions.0, dimensions.1),
            output_edge: 4096,
        })
        .unwrap()
}
fn memory_only(path: std::path::PathBuf) -> Pipeline {
    let mut p = Pipeline::new(64 * 1024 * 1024, path);
    p.configure_cache(Limits {
        previews_mib: 64,
        originals_mib: 128,
        disk_mib: 0,
        ..Limits::default()
    });
    p
}
#[test]
fn bounded_heif_previews_preserve_geometry_alpha_orientation_and_request_order() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large.heic");
    let bytes = include_bytes!("fixtures/heif/large-alpha.heic");
    std::fs::write(&path, bytes).unwrap();
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let mut reference = memory_only(root.path().join("reference"));
    let base = full(&mut reference, &media, (3073, 2051));
    for (quarter, alpha) in [255u8, 128, 64, 0].into_iter().enumerate() {
        let p = base
            .get_pixel((3073 * (quarter as u32 * 2 + 1)) / 8, 2051 / 3)
            .0;
        assert!(p[3].abs_diff(alpha) <= 1, "{p:?}");
    }
    drop(reference);
    for orientation in 1..=8 {
        metadata::write_sidecar(&media.path, None, Some(orientation)).unwrap();
        let expected = orient(
            image::DynamicImage::ImageRgba8((*base).clone()),
            orientation,
        )
        .into_rgba8();
        let mut prior = None;
        for original_first in [false, true] {
            let mut p = memory_only(root.path().join("unused"));
            if original_first {
                assert_eq!(*full(&mut p, &media, expected.dimensions()), expected);
            }
            if orientation % 2 == 0 {
                p.load(&media).unwrap();
            }
            let analysis = p.analysis(&media, || false).unwrap();
            let display = p.load(&media).unwrap();
            assert_eq!(
                (display.metadata.width, display.metadata.height),
                (3073, 2051)
            );
            assert_eq!(display.metadata.orientation, orientation);
            assert_eq!(display.pixels.width().max(display.pixels.height()), 2048);
            assert_eq!(analysis.pixels.width().max(analysis.pixels.height()), 1024);
            assert_eq!(p.stats.decodes, 1 + u64::from(original_first));
            assert_eq!(p.stats.metadata_reads, 1);
            let reference = image::imageops::resize(
                &expected,
                display.pixels.width(),
                display.pixels.height(),
                image::imageops::FilterType::Triangle,
            );
            let mean_error = display
                .pixels
                .as_raw()
                .iter()
                .zip(reference.as_raw())
                .map(|(a, b)| u64::from(a.abs_diff(*b)))
                .sum::<u64>() as f64
                / display.pixels.as_raw().len() as f64;
            assert!(
                mean_error < 6.,
                "Orientation {orientation} changed pixels too far: {mean_error}"
            );
            if let Some((a, d)) = &prior {
                assert_eq!(analysis.pixels, *a);
                assert_eq!(display.pixels, *d);
            }
            prior = Some((analysis.pixels.clone(), display.pixels.clone()));
            assert_eq!(*full(&mut p, &media, expected.dimensions()), expected);
            assert_eq!(
                p.stats.decodes, 2,
                "Only original detail needs the full decoder"
            );
            for _ in Step::ALL {
                assert!(Arc::ptr_eq(
                    &p.load(&media).unwrap().pixels,
                    &display.pixels
                ));
                assert!(Arc::ptr_eq(
                    &p.analysis(&media, || false).unwrap().pixels,
                    &analysis.pixels
                ));
            }
            assert_eq!(p.stats.decodes, 2);
        }
    }
    assert_eq!(std::fs::read(&media.path).unwrap(), bytes);
}
#[test]
fn heif_analysis_geometry_restarts_without_pixels_and_cancelled_work_is_not_cached() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large.heic");
    std::fs::write(path, include_bytes!("fixtures/heif/large-alpha.heic")).unwrap();
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let mut p = Pipeline::new(64 * 1024 * 1024, root.path().join("cache"));
    let analysis = p.analysis(&media, || false).unwrap();
    drop(p);
    let mut reopened = Pipeline::new(64 * 1024 * 1024, root.path().join("cache"));
    assert_eq!(
        serde_json::to_value(reopened.metadata(&media).unwrap()).unwrap(),
        serde_json::to_value(&analysis.metadata).unwrap()
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
        reopened.stats.resident_bytes + reopened.stats.detail_bytes,
        0
    );
    assert_eq!(
        reopened.analysis(&media, || false).unwrap().pixels,
        analysis.pixels
    );
    assert_eq!(reopened.stats.decodes, 0);
    let mut cancelled = memory_only(root.path().join("cancelled"));
    let polls = AtomicUsize::new(0);
    assert!(
        cancelled
            .load_cancellable(&media, || polls.fetch_add(1, Ordering::Relaxed) >= 6)
            .is_err()
    );
    assert_eq!(
        cancelled.stats.resident_bytes + cancelled.stats.detail_bytes,
        0
    );
    let before = cancelled.load(&media).unwrap();
    metadata::write_sidecar(&media.path, Some(3), Some(6)).unwrap();
    let after = cancelled.load(&media).unwrap();
    assert_eq!((after.metadata.orientation, after.metadata.rating), (6, 3));
    assert!(!Arc::ptr_eq(&before.pixels, &after.pixels));
}
#[test]
fn heif_container_transforms_are_not_applied_twice() {
    let root = tempfile::tempdir().unwrap();
    let mut p = memory_only(root.path().join("cache"));
    let mut base = None;
    for o in 1..=8 {
        let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join(format!("tests/fixtures/heif/portable/orientation-{o}.heic"));
        let target = root.path().join(format!("orientation-{o}.heic"));
        std::fs::copy(source, &target).unwrap();
        let media = Media {
            id: MediaId::for_path(&target),
            fingerprint: Fingerprint::read(&target).unwrap(),
            path: target,
            kind: MediaKind::Photo,
        };
        let asset = p.load(&media).unwrap();
        assert_eq!(asset.metadata.orientation, o);
        if o == 1 {
            base = Some((*asset.pixels).clone());
        }
        assert_eq!(
            *asset.pixels,
            orient(image::DynamicImage::ImageRgba8(base.clone().unwrap()), o).into_rgba8()
        );
    }
}
