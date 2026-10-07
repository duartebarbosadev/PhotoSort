#![cfg(all(feature = "heif", not(target_os = "macos")))]
use image::DynamicImage;
use photosort::core::{
    inspection::{Region, Request},
    metadata,
    model::{Fingerprint, Media, MediaId, MediaKind, Step},
    pipeline::{Pipeline, orient},
};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
fn fixture(root: &Path, name: &str) -> Media {
    let source = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/heif/portable")
        .join(name);
    let path = root.join(name);
    std::fs::copy(source, &path).unwrap();
    Media {
        id: MediaId::for_path(&path),
        fingerprint: Fingerprint::read(&path).unwrap(),
        path,
        kind: MediaKind::Photo,
    }
}
fn detail(pipeline: &mut Pipeline, media: &Media, dimensions: (u32, u32)) -> Arc<image::RgbaImage> {
    pipeline
        .inspect(&Request {
            slot: 0,
            media: media.clone(),
            region: Region {
                x: 0,
                y: 0,
                width: dimensions.0,
                height: dimensions.1,
            },
            output_edge: dimensions.0.max(dimensions.1),
        })
        .unwrap()
}
#[test]
fn all_container_orientations_and_sidecar_overrides_share_correct_pixels() {
    let root = tempfile::tempdir().unwrap();
    let disk = root.path().join("cache");
    let mut pipeline = Pipeline::new(16 * 1024 * 1024, disk.clone());
    let media: Vec<_> = (1..=8)
        .map(|o| fixture(root.path(), &format!("orientation-{o}.heic")))
        .collect();
    let original_bytes: Vec<_> = media
        .iter()
        .map(|m| std::fs::read(&m.path).unwrap())
        .collect();
    let base = DynamicImage::ImageRgba8((*pipeline.load(&media[0]).unwrap().pixels).clone());
    assert_eq!((base.width(), base.height()), (64, 48));
    for (index, photo) in media.iter().enumerate() {
        let container = index as u8 + 1;
        let meta = pipeline.metadata(photo).unwrap();
        assert_eq!(
            (meta.orientation, meta.container_orientation),
            (container, Some(container))
        );
        assert_eq!(
            *pipeline.load(photo).unwrap().pixels,
            orient(base.clone(), container).to_rgba8()
        );
        for desired in 1..=8 {
            metadata::write_sidecar(&photo.path, None, Some(desired)).unwrap();
            let before = (pipeline.stats.decodes, pipeline.stats.metadata_reads);
            let display = pipeline.load(photo).unwrap();
            assert_eq!(
                (
                    display.metadata.orientation,
                    display.metadata.container_orientation
                ),
                (desired, Some(container))
            );
            let expected = orient(base.clone(), desired).to_rgba8();
            assert_eq!(
                *display.pixels, expected,
                "container {container}, sidecar {desired}"
            );
            for _ in Step::ALL {
                assert!(Arc::ptr_eq(
                    &display.pixels,
                    &pipeline.load(photo).unwrap().pixels
                ));
                assert_eq!(
                    *pipeline.analysis(photo, || false).unwrap().pixels,
                    expected
                );
                assert_eq!(
                    *detail(&mut pipeline, photo, expected.dimensions()),
                    expected
                );
            }
            assert_eq!(
                (pipeline.stats.decodes, pipeline.stats.metadata_reads),
                (before.0 + 1, before.1 + 1)
            );
        }
    }
    let mut reopened = Pipeline::new(16 * 1024 * 1024, disk);
    for (photo, bytes) in media.iter().zip(original_bytes) {
        let expected = orient(base.clone(), 8).to_rgba8();
        assert_eq!(*reopened.load(photo).unwrap().pixels, expected);
        assert_eq!(
            *reopened.analysis(photo, || false).unwrap().pixels,
            expected
        );
        assert_eq!(std::fs::read(&photo.path).unwrap(), bytes);
    }
    assert_eq!(
        (
            reopened.stats.decodes,
            reopened.stats.metadata_reads,
            reopened.stats.disk_hits
        ),
        (0, 0, 16)
    );
}
#[test]
fn icc_and_nclx_linear_light_become_srgb_once_with_alpha_preserved() {
    let root = tempfile::tempdir().unwrap();
    for name in ["linear-icc.heic", "linear-nclx.heic"] {
        let media = fixture(root.path(), name);
        let mut pipeline =
            Pipeline::new(4 * 1024 * 1024, root.path().join(format!("cache-{name}")));
        let display = pipeline.load(&media).unwrap();
        let pixel = display.pixels.get_pixel(32, 24).0;
        assert_eq!(pixel[3], 128, "{name}: {pixel:?}");
        for channel in &pixel[..3] {
            assert!((185..=191).contains(channel), "{name}: {pixel:?}");
        }
        assert_eq!(
            *pipeline.analysis(&media, || false).unwrap().pixels,
            *display.pixels
        );
        assert_eq!(
            *detail(&mut pipeline, &media, display.pixels.dimensions()),
            *display.pixels
        );
        assert_eq!(
            (
                pipeline.stats.decodes,
                pipeline.stats.color_initializations,
                pipeline.stats.color_transforms
            ),
            (1, 1, 1)
        );
    }
}
#[test]
fn premultiplied_heif_has_straight_output_and_invalid_icc_fails_visibly() {
    let root = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(4 * 1024 * 1024, root.path().join("cache"));
    let media = fixture(root.path(), "premultiplied.heic");
    let pixel = pipeline.load(&media).unwrap().pixels.get_pixel(32, 24).0;
    assert_eq!(pixel[3], 128);
    for channel in &pixel[..3] {
        assert!((125..=131).contains(channel), "{pixel:?}");
    }
    let media = fixture(root.path(), "invalid-icc.heic");
    let error = pipeline
        .load(&media)
        .err()
        .expect("Invalid ICC must not silently fall back");
    assert!(
        format!("{error:#}")
            .to_ascii_lowercase()
            .contains("profile")
    );
}
#[test]
fn cancellation_during_heif_preparation_publishes_no_partial_pixels_and_allows_retry() {
    let root = tempfile::tempdir().unwrap();
    let media = fixture(root.path(), "orientation-1.heic");
    let disk = root.path().join("cache");
    let mut pipeline = Pipeline::new(4 * 1024 * 1024, disk.clone());
    assert!(pipeline.load_cancellable(&media, || true).is_err());
    assert_eq!(pipeline.stats.decodes, 0);
    let calls = AtomicUsize::new(0);
    assert!(
        pipeline
            .load_cancellable(&media, || calls.fetch_add(1, Ordering::Relaxed) >= 16)
            .is_err()
    );
    assert_eq!(
        (
            pipeline.stats.decodes,
            pipeline.stats.resident_bytes,
            pipeline.stats.detail_bytes
        ),
        (1, 0, 0)
    );
    assert!(!std::fs::read_dir(disk).unwrap().any(|entry| {
        entry
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "png")
    }));
    assert!(pipeline.load(&media).is_ok());
    assert_eq!(pipeline.stats.decodes, 2);
}

#[test]
fn decoder_applies_clean_aperture_before_container_rotation() {
    let root = tempfile::tempdir().unwrap();
    let normal = fixture(root.path(), "orientation-1.heic");
    let rotated = fixture(root.path(), "orientation-6.heic");
    let library = libheif_rs::LibHeif::new_checked().unwrap();
    let mut pipeline = Pipeline::new(4 * 1024 * 1024, root.path().join("cache"));
    for (media, expected) in [(&normal, (64, 48)), (&rotated, (48, 64))] {
        let context =
            libheif_rs::HeifContext::read_from_file(media.path.to_str().unwrap()).unwrap();
        let handle = context.primary_image_handle().unwrap();
        // The encoder pads to a 64x64 HEVC image and records a clean aperture.
        // Removing the library's transform handling would leak 16 padded rows.
        assert_eq!((handle.ispe_width(), handle.ispe_height()), (64, 64));
        let mut options = libheif_rs::DecodingOptions::new().unwrap();
        options.set_ignore_transformations(true);
        let uncropped = library
            .decode(
                &handle,
                libheif_rs::ColorSpace::Rgb(libheif_rs::RgbChroma::Rgba),
                Some(options),
            )
            .unwrap();
        let plane = uncropped.planes().interleaved.unwrap();
        assert_eq!((plane.width, plane.height), (64, 64));
        assert_eq!(pipeline.load(media).unwrap().pixels.dimensions(), expected);
    }
}
