use super::*;
use image::{ImageEncoder, Rgba, RgbaImage};
use std::{fs::File, sync::atomic::AtomicUsize};
#[path = "tiff_fixture.rs"]
mod tiff_fixture;
fn rgba(image: Image) -> Vec<u8> {
    match image.pixels {
        Pixels::U8(pixels) => pixels,
        _ => panic!("expected 8-bit RGBA"),
    }
}
fn png(path: &Path, pixels: &RgbaImage, profile: Option<Vec<u8>>) {
    let mut encoder = image::codecs::png::PngEncoder::new(File::create(path).unwrap());
    if let Some(profile) = profile {
        encoder.set_icc_profile(profile).unwrap();
    }
    encoder
        .write_image(
            pixels.as_raw(),
            pixels.width(),
            pixels.height(),
            image::ExtendedColorType::Rgba8,
        )
        .unwrap();
}
fn linear_profile() -> Vec<u8> {
    let mut p = moxcms::ColorProfile::new_srgb();
    p.cicp = None;
    let curve = moxcms::ToneReprCurve::Parametric(vec![1.]);
    p.red_trc = Some(curve.clone());
    p.green_trc = Some(curve.clone());
    p.blue_trc = Some(curve);
    p.encode().unwrap()
}
#[test]
fn literal_filenames_and_straight_alpha_survive_without_source_mutation() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("é [shrink=8].png");
    let pixels = RgbaImage::from_fn(32, 24, |x, y| {
        Rgba([x as u8 * 7, y as u8 * 9, 128, (x * 8) as u8])
    });
    png(&path, &pixels, None);
    let before = std::fs::read(&path).unwrap();
    let output = render(&path, Request::default(), || false).unwrap();
    assert_eq!(
        (
            output.width,
            output.height,
            output.source_width,
            output.source_height
        ),
        (32, 24, 32, 24)
    );
    assert!(!output.profiled);
    assert_eq!(rgba(output), *pixels.as_raw());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
#[test]
fn thumbnails_are_bounded_and_regions_use_unrotated_source_coordinates() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("photo.png");
    let pixels = RgbaImage::from_fn(400, 200, |x, y| Rgba([(x % 255) as u8, y as u8, 70, 255]));
    png(&path, &pixels, None);
    let output = render(
        &path,
        Request {
            edge: Some(100),
            region: None,
        },
        || false,
    )
    .unwrap();
    assert_eq!(
        (
            output.width,
            output.height,
            output.source_width,
            output.source_height
        ),
        (100, 50, 400, 200)
    );
    let output = render(
        &path,
        Request {
            edge: None,
            region: Some(Region {
                x: 123,
                y: 17,
                width: 32,
                height: 21,
            }),
        },
        || false,
    )
    .unwrap();
    assert_eq!(
        rgba(output),
        image::imageops::crop_imm(&pixels, 123, 17, 32, 21)
            .to_image()
            .into_raw()
    );
    assert!(
        render(
            &path,
            Request {
                edge: Some(0),
                region: None
            },
            || false
        )
        .is_err()
    );
    assert!(
        render(
            &path,
            Request {
                edge: None,
                region: Some(Region {
                    x: u32::MAX,
                    y: 0,
                    width: 2,
                    height: 2
                })
            },
            || false
        )
        .is_err()
    );
}
#[test]
fn embedded_linear_profile_normalizes_rgb_but_preserves_alpha() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("linear.png");
    png(
        &path,
        &RgbaImage::from_pixel(40, 20, Rgba([128, 128, 128, 127])),
        Some(linear_profile()),
    );
    for edge in [None, Some(10)] {
        let output = render(&path, Request { edge, region: None }, || false).unwrap();
        assert!(output.profiled);
        for pixel in rgba(output).chunks_exact(4) {
            for sample in &pixel[..3] {
                assert!(sample.abs_diff(188) <= 1, "{pixel:?}");
            }
            assert_eq!(pixel[3], 127);
        }
    }
}
#[test]
fn rgb16_full_resolution_retains_low_bits() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("precise.png");
    let image = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(32, 16, |x, y| {
        image::Rgba([30000 + x as u16, 20000 + y as u16, 7777, 32767])
    });
    image.save(&path).unwrap();
    let output = render(&path, Request::default(), || false).unwrap();
    let Pixels::U16(pixels) = output.pixels else {
        panic!("16-bit source was reduced")
    };
    assert_eq!(pixels, *image.as_raw());
}
#[test]
fn invalid_profiles_and_mismatched_cmyk_profiles_are_visible_errors() {
    let temp = tempfile::tempdir().unwrap();
    for (name, profile) in [("invalid", vec![0; 128]), ("rgb-on-cmyk", linear_profile())] {
        let path = temp.path().join(format!("{name}.tiff"));
        tiff_fixture::write(&path, Some(&profile), 8, false, false);
        for edge in [None, Some(16)] {
            let error = render(&path, Request { edge, region: None }, || false).unwrap_err();
            assert!(error.to_string().contains("profile"), "{error}");
        }
    }
}
#[test]
fn cancellation_and_callback_panics_never_publish_pixels_or_poison_next_job() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.png");
    png(
        &path,
        &RgbaImage::from_pixel(1600, 1200, Rgba([12, 45, 78, 255])),
        None,
    );
    assert!(render(&path, Request::default(), || true).is_err());
    for _ in 0..12 {
        let calls = AtomicUsize::new(0);
        let result = render(&path, Request::default(), || {
            calls.fetch_add(1, Ordering::Relaxed) >= 3
        });
        assert!(result.is_err());
        assert!(calls.load(Ordering::Relaxed) >= 4);
    }
    let calls = AtomicUsize::new(0);
    let result = render(&path, Request::default(), || {
        assert!(
            calls.fetch_add(1, Ordering::Relaxed) < 3,
            "test panic inside native callback"
        );
        false
    });
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("callback panicked")
    );
    assert_eq!(
        render(
            &path,
            Request {
                edge: Some(32),
                region: None
            },
            || false
        )
        .unwrap()
        .width,
        32
    );
}
#[test]
fn truncated_input_returns_error_and_next_job_still_works() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("broken.png");
    png(
        &path,
        &RgbaImage::from_pixel(300, 200, Rgba([12, 34, 56, 255])),
        None,
    );
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(render(&path, Request::default(), || false).is_err());
    std::fs::write(&path, bytes).unwrap();
    assert!(render(&path, Request::default(), || false).is_ok());
}
#[cfg(target_os = "macos")]
#[test]
fn cmyk_tiff_packed_planar_lzw_and_depth_variants_agree() {
    let temp = tempfile::tempdir().unwrap();
    let profile =
        std::fs::read("/System/Library/ColorSync/Profiles/Generic CMYK Profile.icc").unwrap();
    let mut baseline: Option<Vec<u8>> = None;
    for bits in [8, 16] {
        for (planar, compressed) in [(false, false), (true, false), (false, true)] {
            let path = temp
                .path()
                .join(format!("{bits}-{planar}-{compressed}.tiff"));
            tiff_fixture::write(&path, Some(&profile), bits, planar, compressed);
            let output = render(&path, Request::default(), || false).unwrap();
            assert!(output.profiled);
            let samples = match output.pixels {
                Pixels::U8(v) => v,
                Pixels::U16(v) => v
                    .into_iter()
                    .map(|s| ((u32::from(s) + 128) / 257) as u8)
                    .collect(),
            };
            if let Some(reference) = baseline.as_ref() {
                let max = samples
                    .iter()
                    .zip(reference)
                    .map(|(a, b)| u8::abs_diff(*a, *b))
                    .max()
                    .unwrap();
                assert!(max <= 1, "{bits}/{planar}/{compressed}: {max}");
            } else {
                baseline = Some(samples);
            }
        }
    }
}
#[test]
fn concurrent_callers_share_process_initialization_without_global_shutdown() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("photo.png");
    png(
        &path,
        &RgbaImage::from_pixel(30, 20, Rgba([10, 20, 30, 255])),
        None,
    );
    std::thread::scope(|scope| {
        let handles: Vec<_> = (0..4)
            .map(|_| scope.spawn(|| render(&path, Request::default(), || false).unwrap()))
            .collect();
        for handle in handles {
            assert_eq!(rgba(handle.join().unwrap()), [10, 20, 30, 255].repeat(600));
        }
    });
}
#[test]
#[ignore = "Uses original PhotoSort samples; set PHOTOSORT_SAMPLE_DIR"]
fn original_jpeg_png_samples_are_read_only_and_bounded() {
    let root = std::path::PathBuf::from(
        std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("sample directory"),
    );
    let mut count = 0;
    for entry in std::fs::read_dir(root).unwrap() {
        let path = entry.unwrap().path();
        let ext = path
            .extension()
            .unwrap_or_default()
            .to_string_lossy()
            .to_lowercase();
        if !matches!(ext.as_str(), "jpg" | "jpeg" | "png") {
            continue;
        }
        let before = std::fs::read(&path).unwrap();
        let output = render(
            &path,
            Request {
                edge: Some(2048),
                region: None,
            },
            || false,
        )
        .unwrap();
        assert!(output.width <= 2048 && output.height <= 2048);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        count += 1;
    }
    assert_eq!(count, 6);
}

#[test]
fn tagged_rgb16_preserves_alpha_and_more_than_eight_bits_of_color() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("linear16.png");
    let pixels = image::ImageBuffer::<image::Rgba<u16>, Vec<u16>>::from_fn(64, 32, |x, y| {
        image::Rgba([30000 + x as u16, 20000 + y as u16, 7777, 32767])
    });
    let mut encoder = image::codecs::png::PngEncoder::new(File::create(&path).unwrap());
    encoder.set_icc_profile(linear_profile()).unwrap();
    let bytes: Vec<u8> = pixels
        .as_raw()
        .iter()
        .flat_map(|value| value.to_ne_bytes())
        .collect();
    encoder
        .write_image(&bytes, 64, 32, image::ExtendedColorType::Rgba16)
        .unwrap();
    let output = render(&path, Request::default(), || false).unwrap();
    let Pixels::U16(samples) = output.pixels else {
        panic!("lost native precision")
    };
    let unique: std::collections::BTreeSet<_> =
        samples.chunks_exact(4).take(64).map(|p| p[0]).collect();
    assert!(
        unique.len() > 40,
        "color was quantized to eight bits: {} values",
        unique.len()
    );
    for pixel in samples.chunks_exact(4) {
        assert_eq!(pixel[3], 32767);
    }
}

#[cfg(target_os = "macos")]
#[test]
fn cmyk_and_ycck_jpeg_are_converted_before_exposing_rgb() {
    let temp = tempfile::tempdir().unwrap();
    let profile =
        std::fs::read("/System/Library/ColorSync/Profiles/Generic CMYK Profile.icc").unwrap();
    let mut reference: Option<Vec<u8>> = None;
    for (name, color) in [
        ("cmyk", jpeg_encoder::ColorType::Cmyk),
        ("ycck", jpeg_encoder::ColorType::CmykAsYcck),
    ] {
        let path = temp.path().join(format!("{name}.jpg"));
        let mut encoder = jpeg_encoder::Encoder::new(File::create(&path).unwrap(), 100);
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
        encoder.add_icc_profile(&profile).unwrap();
        // A constant field avoids comparing unrelated chroma subsampling.
        encoder
            .encode(&[40, 80, 100, 20].repeat(64 * 32), 64, 32, color)
            .unwrap();
        let full = render(&path, Request::default(), || false).unwrap();
        assert!(full.profiled);
        let pixels = rgba(full);
        assert!(pixels.chunks_exact(4).all(|p| p[3] == 255));
        if let Some(reference) = &reference {
            let max = pixels
                .iter()
                .zip(reference)
                .map(|(a, b)| a.abs_diff(*b))
                .max()
                .unwrap();
            assert!(max <= 2, "CMYK/YCCK mismatch: {max}");
        } else {
            reference = Some(pixels);
        }
        let thumbnail = render(
            &path,
            Request {
                edge: Some(16),
                region: None,
            },
            || false,
        )
        .unwrap();
        assert!(thumbnail.profiled);
        assert_eq!((thumbnail.width, thumbnail.height), (16, 8));
    }
}

#[test]
fn exif_orientation_is_left_for_the_shared_pipeline() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("orientation.jpg");
    let mut encoder = jpeg_encoder::Encoder::new(File::create(&path).unwrap(), 100);
    // Little-endian TIFF IFD: one SHORT orientation field with value 6.
    encoder
        .add_exif_metadata(&[
            b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 1, 3, 0, 1, 0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ])
        .unwrap();
    encoder
        .encode(
            &[50, 100, 150].repeat(64 * 32),
            64,
            32,
            jpeg_encoder::ColorType::Rgb,
        )
        .unwrap();
    for (edge, expected) in [(None, (64, 32)), (Some(16), (16, 8))] {
        let output = render(&path, Request { edge, region: None }, || false).unwrap();
        assert_eq!((output.width, output.height), expected);
        assert_eq!((output.source_width, output.source_height), (64, 32));
    }
}

#[test]
fn profiles_discarded_by_the_png_loader_are_reported_instead_of_silent_fallback() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("invalid-png-profile.png");
    png(
        &path,
        &RgbaImage::from_pixel(40, 20, Rgba([128, 128, 128, 255])),
        Some(vec![0; 128]),
    );
    for edge in [None, Some(10)] {
        let output = render(&path, Request { edge, region: None }, || false).unwrap();
        assert!(!output.profiled);
        assert!(
            output
                .decoder_warning
                .as_deref()
                .is_some_and(|s| s.contains("iCCP"))
        );
    }
    // Warning context belongs to one render and must not leak to another image.
    png(
        &path,
        &RgbaImage::from_pixel(40, 20, Rgba([128, 128, 128, 255])),
        None,
    );
    assert!(
        render(&path, Request::default(), || false)
            .unwrap()
            .decoder_warning
            .is_none()
    );
}
