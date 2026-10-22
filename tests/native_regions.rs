#![cfg(feature = "native-imaging")]
#[path = "../src/test_support.rs"]
mod test_support;
use image::{DynamicImage, Rgba, RgbaImage};
use photosort::core::{
    catalog::Catalog,
    inspection::{Region, Request},
    metadata,
    model::{Media, Step},
    pipeline::{Pipeline, orient},
};
use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};
fn source(root: &Path) -> (Media, RgbaImage) {
    let pixels = RgbaImage::from_fn(2500, 1400, |x, y| {
        Rgba([(x % 251) as u8, (y % 251) as u8, ((x + y) % 251) as u8, 255])
    });
    pixels.save(root.join("source.png")).unwrap();
    let media = Catalog::scan(root, || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    (media, pixels)
}
fn request(media: &Media, x: u32, y: u32, width: u32, height: u32, edge: u32) -> Request {
    Request {
        slot: 0,
        media: media.clone(),
        region: Region {
            x,
            y,
            width,
            height,
        },
        output_edge: edge,
    }
}
#[test]
fn oriented_native_pixels_and_nearby_pans_share_one_bounded_tile_across_workflows() {
    let root = tempfile::tempdir().unwrap();
    let (media, pixels) = source(root.path());
    let original = std::fs::read(&media.path).unwrap();
    let mut pipeline = Pipeline::new(4 * 1024 * 1024, root.path().join("cache"));
    for orientation in 1..=8 {
        metadata::write_sidecar(&media.path, None, Some(orientation)).unwrap();
        let expected = orient(DynamicImage::ImageRgba8(pixels.clone()), orientation).into_rgba8();
        let before = pipeline.stats.native_regions;
        for step in Step::ALL {
            for delta in [0, 9, 21] {
                let mut r = request(&media, 1100 + delta, 650 + delta, 140, 100, 512);
                r.slot = step.index() as u64;
                let actual = pipeline.inspect(&r).unwrap();
                assert_eq!(
                    *actual,
                    image::imageops::crop_imm(&expected, r.region.x, r.region.y, 140, 100)
                        .to_image(),
                    "orientation {orientation}, pan {delta}"
                );
            }
        }
        assert_eq!(pipeline.stats.native_regions, before + 1);
        assert_eq!(pipeline.stats.metadata_reads, u64::from(orientation));
        assert!(pipeline.stats.detail_bytes <= 8 * 1024 * 1024);
    }
    assert_eq!(std::fs::read(&media.path).unwrap(), original);
}
#[test]
fn clipped_cached_tiles_check_bounds_and_scaled_views_store_only_output_pixels() {
    let root = tempfile::tempdir().unwrap();
    let (media, pixels) = source(root.path());
    let mut pipeline = Pipeline::new(4 * 1024 * 1024, root.path().join("cache"));
    let good = request(&media, 2490, 1390, 10, 10, 512);
    assert_eq!(
        *pipeline.inspect(&good).unwrap(),
        image::imageops::crop_imm(&pixels, 2490, 1390, 10, 10).to_image()
    );
    let decodes = pipeline.stats.decodes;
    for invalid in [
        request(&media, 2490, 1390, 20, 10, 512),
        request(&media, 2490, 1390, 10, 20, 512),
        request(&media, u32::MAX, 0, 2, 1, 512),
    ] {
        assert!(pipeline.inspect(&invalid).is_err());
    }
    assert_eq!(
        pipeline.stats.decodes, decodes,
        "invalid cached bounds must not decode again"
    );
    pipeline.clear_memory();
    let full = request(&media, 0, 0, 2500, 1400, 1000);
    let output = pipeline.inspect(&full).unwrap();
    assert_eq!(output.dimensions(), (1000, 560));
    assert_eq!(pipeline.stats.detail_bytes, 1000 * 560 * 4);
    let decodes = pipeline.stats.decodes;
    assert_eq!(*pipeline.inspect(&full).unwrap(), *output);
    assert_eq!(pipeline.stats.decodes, decodes);
}
#[test]
fn region_cancellation_and_eviction_preserve_the_shared_budget_and_retry() {
    let root = tempfile::tempdir().unwrap();
    let (media, _) = source(root.path());
    let mut pipeline = Pipeline::new(512 * 1024, root.path().join("cache"));
    let first = request(&media, 1100, 650, 140, 100, 512);
    assert!(pipeline.inspect_with_warning(&first, &|| true).is_err());
    assert_eq!(pipeline.stats.native_regions, 0);
    pipeline.metadata(&media).unwrap();
    let calls = AtomicUsize::new(0);
    assert!(
        pipeline
            .inspect_with_warning(&first, &|| calls.fetch_add(1, Ordering::Relaxed) >= 5)
            .is_err()
    );
    assert_eq!(pipeline.stats.detail_bytes, 0);
    assert_eq!(
        pipeline.stats.native_regions, 1,
        "cancellation must reach native region preparation"
    );
    pipeline.inspect(&first).unwrap();
    assert_eq!(pipeline.stats.detail_bytes, 512 * 512 * 4);
    let renders = pipeline.stats.native_regions;
    pipeline
        .inspect(&request(&media, 1600, 650, 140, 100, 512))
        .unwrap();
    assert_eq!(pipeline.stats.detail_bytes, 512 * 512 * 4);
    pipeline.inspect(&first).unwrap();
    assert_eq!(pipeline.stats.native_regions, renders + 2);
    pipeline.clear_memory();
    assert_eq!(pipeline.stats.detail_bytes, 0);
}
#[test]
fn sixteen_bit_inspection_matches_a_full_precision_decode_for_all_orientations() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("sixteen.png");
    let image = image::ImageBuffer::<Rgba<u16>, Vec<u16>>::from_fn(1100, 700, |x, y| {
        Rgba([
            (x * 31) as u16,
            (y * 71) as u16,
            ((x + y) * 29) as u16,
            65535,
        ])
    });
    DynamicImage::ImageRgba16(image).save(&path).unwrap();
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let full = photosort_imaging_vips::render(&path, Default::default(), || false).unwrap();
    let photosort_imaging_vips::Pixels::U16(values) = full.pixels else {
        panic!("native precision lost")
    };
    let original = DynamicImage::ImageRgba16(
        image::ImageBuffer::from_raw(full.width, full.height, values).unwrap(),
    );
    let mut pipeline = Pipeline::new(16 * 1024 * 1024, root.path().join("cache"));
    for orientation in 1..=8 {
        metadata::write_sidecar(&path, None, Some(orientation)).unwrap();
        let expected = orient(original.clone(), orientation)
            .crop_imm(520, 530, 100, 100)
            .into_rgba8();
        assert_eq!(
            *pipeline
                .inspect(&request(&media, 520, 530, 100, 100, 512))
                .unwrap(),
            expected
        );
    }
}

#[test]
fn cropped_rotation_preserves_embedded_color_and_straight_alpha() {
    use image::ImageEncoder;
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("linear.png");
    let mut profile = moxcms::ColorProfile::new_srgb();
    profile.cicp = None;
    let curve = moxcms::ToneReprCurve::Parametric(vec![1.]);
    profile.red_trc = Some(curve.clone());
    profile.green_trc = Some(curve.clone());
    profile.blue_trc = Some(curve);
    let mut encoder = image::codecs::png::PngEncoder::new(std::fs::File::create(&path).unwrap());
    encoder.set_icc_profile(profile.encode().unwrap()).unwrap();
    encoder
        .write_image(
            &[128, 128, 128, 128].repeat(1100 * 700),
            1100,
            700,
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let mut pipeline = Pipeline::new(8 * 1024 * 1024, root.path().join("cache"));
    for orientation in [1, 2, 6, 8] {
        metadata::write_sidecar(&path, None, Some(orientation)).unwrap();
        let image = pipeline
            .inspect(&request(&media, 520, 530, 100, 100, 512))
            .unwrap();
        for p in image.pixels() {
            assert_eq!(p[3], 128);
            assert!(p.0[..3].iter().all(|v| (187..=189).contains(v)), "{p:?}");
        }
    }
}
#[test]
fn image_actor_discards_old_epochs_and_reuses_inspection_across_workflows() {
    use photosort::core::jobs::{Event, Runtime};
    use std::time::{Duration, Instant};
    let root = tempfile::tempdir().unwrap();
    let (media, pixels) = source(root.path());
    let mut runtime = Runtime::with_cache(|| {}, root.path().join("cache"));
    runtime.inspect(vec![request(&media, 0, 0, 2500, 1400, 2500)]);
    let epoch = runtime.next_epoch();
    let mut before = None;
    for step in Step::ALL {
        let mut wanted = request(&media, 1100 + step.index() as u32, 650, 140, 100, 512);
        wanted.slot = step.index() as u64;
        let submitted = Instant::now();
        runtime.inspect(vec![wanted.clone()]);
        assert!(
            submitted.elapsed() < Duration::from_secs(1),
            "UI request performed blocking image work"
        );
        let deadline = Instant::now() + test_support::worker_patience();
        loop {
            assert!(
                Instant::now() < deadline,
                "Latest inspection did not complete"
            );
            let Ok(event) = runtime.events.recv_timeout(Duration::from_millis(100)) else {
                continue;
            };
            if event.epoch != epoch {
                continue;
            }
            if let Event::Inspected {
                request,
                result,
                stats,
                ..
            } = event.event
            {
                assert_eq!(request, wanted);
                assert_eq!(
                    *result.unwrap(),
                    image::imageops::crop_imm(&pixels, wanted.region.x, 650, 140, 100).to_image()
                );
                let counts = (stats.native_regions, stats.metadata_reads);
                if let Some(before) = before {
                    assert_eq!(counts, before, "workflow rebuilt the shared tile");
                }
                before = Some(counts);
                break;
            }
        }
    }
    runtime.inspect(Vec::new());
    runtime.shutdown();
}

#[test]
fn rotated_fit_views_resize_before_materializing_and_keep_exact_orientation() {
    let root = tempfile::tempdir().unwrap();
    let (media, _) = source(root.path());
    let base = photosort_imaging_vips::render_region(
        &media.path,
        photosort_imaging_vips::Request {
            edge: Some(1000),
            region: Some(photosort_imaging_vips::Region {
                x: 0,
                y: 0,
                width: 2500,
                height: 1400,
            }),
        },
        1,
        0,
        || false,
    )
    .unwrap();
    let photosort_imaging_vips::Pixels::U8(pixels) = base.pixels else {
        panic!("scaled output should be 8-bit")
    };
    let base =
        DynamicImage::ImageRgba8(RgbaImage::from_raw(base.width, base.height, pixels).unwrap());
    let mut pipeline = Pipeline::new(4 * 1024 * 1024, root.path().join("cache"));
    for orientation in 1..=8 {
        metadata::write_sidecar(&media.path, None, Some(orientation)).unwrap();
        let (w, h) = if orientation >= 5 {
            (1400, 2500)
        } else {
            (2500, 1400)
        };
        let actual = pipeline
            .inspect(&request(&media, 0, 0, w, h, 1000))
            .unwrap();
        assert_eq!(*actual, orient(base.clone(), orientation).into_rgba8());
        assert!(pipeline.stats.detail_bytes <= 8 * 1024 * 1024);
    }
}
