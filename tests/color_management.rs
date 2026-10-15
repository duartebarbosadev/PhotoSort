use image::{ImageEncoder, Rgba, RgbaImage};
use photosort::core::{
    inspection::{Region, Request},
    model::*,
    pipeline::Pipeline,
};
use std::{fs::File, path::Path};
fn linear() -> Vec<u8> {
    static PROFILE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    PROFILE
        .get_or_init(|| {
            let mut p = moxcms::ColorProfile::new_srgb();
            p.cicp = None; // The fixture changes the transfer curve to linear.
            let curve = moxcms::ToneReprCurve::Parametric(vec![1.]);
            p.red_trc = Some(curve.clone());
            p.green_trc = Some(curve.clone());
            p.blue_trc = Some(curve);
            p.encode().unwrap()
        })
        .clone()
}
fn photo(path: &Path, tagged: bool) -> Media {
    let mut encoder = image::codecs::png::PngEncoder::new(File::create(path).unwrap());
    if tagged {
        encoder.set_icc_profile(linear()).unwrap();
    }
    let pixels = RgbaImage::from_pixel(20, 20, Rgba([128, 128, 128, 127]));
    encoder
        .write_image(pixels.as_raw(), 20, 20, image::ExtendedColorType::Rgba8)
        .unwrap();
    Media {
        id: MediaId::for_path(path),
        path: path.into(),
        fingerprint: Fingerprint::read(path).unwrap(),
        kind: MediaKind::Photo,
    }
}
#[test]
fn display_analysis_and_detail_share_one_color_conversion_and_restart_reuses_corrected_proxies() {
    let temp = tempfile::tempdir().unwrap();
    let media = photo(&temp.path().join("linear.png"), true);
    let cache = temp.path().join("cache");
    let original = std::fs::read(&media.path).unwrap();
    let mut pipeline = Pipeline::new(8 * 1024 * 1024, cache.clone());
    let display = pipeline.load(&media).unwrap();
    let analysis = pipeline.analysis(&media, || false).unwrap();
    let detail = pipeline
        .inspect(&Request {
            slot: 0,
            media: media.clone(),
            region: Region {
                x: 0,
                y: 0,
                width: 20,
                height: 20,
            },
            output_edge: 20,
        })
        .unwrap();
    for pixels in [
        &display.pixels,
        &display.thumbnail,
        &analysis.pixels,
        &detail,
    ] {
        assert_eq!(pixels.get_pixel(0, 0).0, [188, 188, 188, 127]);
    }
    assert_eq!(pipeline.stats.decodes, 1);
    #[cfg(not(feature = "native-imaging"))]
    assert_eq!(pipeline.stats.color_initializations, 1);
    #[cfg(not(feature = "native-imaging"))]
    assert_eq!(pipeline.stats.color_transforms, 1);
    #[cfg(feature = "native-imaging")]
    assert_eq!(pipeline.stats.native_renders, 1);
    let second = photo(&temp.path().join("same-profile.png"), true);
    pipeline.load(&second).unwrap();
    #[cfg(not(feature = "native-imaging"))]
    assert_eq!(pipeline.stats.color_initializations, 1);
    #[cfg(not(feature = "native-imaging"))]
    assert_eq!(pipeline.stats.color_transforms, 2);
    #[cfg(feature = "native-imaging")]
    assert_eq!(pipeline.stats.native_renders, 2);
    drop(pipeline);
    let mut pipeline = Pipeline::new(8 * 1024 * 1024, cache);
    assert_eq!(
        pipeline.load(&media).unwrap().pixels.get_pixel(0, 0).0,
        [188, 188, 188, 127]
    );
    assert_eq!(
        pipeline
            .analysis(&media, || false)
            .unwrap()
            .pixels
            .get_pixel(0, 0)
            .0,
        [188, 188, 188, 127]
    );
    assert_eq!(pipeline.stats.decodes, 0);
    assert_eq!(pipeline.stats.color_initializations, 0);
    assert_eq!(pipeline.stats.color_transforms, 0);
    assert_eq!(std::fs::read(&media.path).unwrap(), original);
}
#[test]
fn untagged_images_remain_byte_exact_and_cancellation_cannot_cache_a_conversion() {
    let temp = tempfile::tempdir().unwrap();
    let media = photo(&temp.path().join("untagged.png"), false);
    let mut pipeline = Pipeline::new(8 * 1024 * 1024, temp.path().join("cache"));
    assert_eq!(
        pipeline.load(&media).unwrap().pixels.get_pixel(0, 0).0,
        [128, 128, 128, 127]
    );
    assert_eq!(pipeline.stats.color_transforms, 0);
    let tagged = photo(&temp.path().join("tagged.png"), true);
    assert!(pipeline.load_cancellable(&tagged, || true).is_err());
    assert_eq!(pipeline.stats.color_transforms, 0);
    assert_eq!(
        pipeline.load(&tagged).unwrap().pixels.get_pixel(0, 0).0,
        [188, 188, 188, 127]
    );
    #[cfg(not(feature = "native-imaging"))]
    assert_eq!(pipeline.stats.color_transforms, 1);
}

#[cfg(all(target_os = "macos", feature = "heif"))]
#[test]
fn tagged_opaque_pixels_agree_with_independent_apple_colorsync_conversion() {
    let temp = tempfile::tempdir().unwrap();
    for (name, profile) in [
        ("linear", linear()),
        (
            "p3",
            std::fs::read("/System/Library/ColorSync/Profiles/Display P3.icc").unwrap(),
        ),
        (
            "adobe",
            std::fs::read("/System/Library/ColorSync/Profiles/AdobeRGB1998.icc").unwrap(),
        ),
    ] {
        let path = temp.path().join(format!("{name}.png"));
        let mut encoder = image::codecs::png::PngEncoder::new(File::create(&path).unwrap());
        encoder.set_icc_profile(profile).unwrap();
        let pixels = image::RgbImage::from_fn(64, 32, |x, y| {
            image::Rgb([(x * 4) as u8, (y * 8) as u8, ((x + y) * 2) as u8])
        });
        encoder
            .write_image(pixels.as_raw(), 64, 32, image::ExtendedColorType::Rgb8)
            .unwrap();
        let media = Media {
            id: MediaId::for_path(&path),
            fingerprint: Fingerprint::read(&path).unwrap(),
            kind: MediaKind::Photo,
            path: path.clone(),
        };
        let mut pipeline = Pipeline::new(8 * 1024 * 1024, temp.path().join("cache"));
        let result = pipeline.load(&media).unwrap();
        let rendered = photosort::core::apple_image::decode(&path, None).unwrap();
        let (w, h) = rendered.pixels.dimensions();
        let native = rendered.pixels.into_raw();
        assert_eq!((w, h), (64, 32));
        let max = result
            .pixels
            .as_raw()
            .iter()
            .zip(&native)
            .map(|(a, b)| i16::from(*a).abs_diff(i16::from(*b)))
            .max()
            .unwrap();
        if max > 4 {
            for i in [0usize, 31, 63, 1024, 2047] {
                eprintln!(
                    "{name} pixel {i}: Rust {:?}, Apple {:?}",
                    &result.pixels.as_raw()[i * 4..i * 4 + 4],
                    &native[i * 4..i * 4 + 4]
                );
            }
        }
        assert!(
            max <= 4,
            "{name}: ColorSync maximum channel difference {max}"
        );
    }
}
#[cfg(feature = "raw")]
#[test]
#[ignore = "Uses the seven original PhotoSort samples through the shared native pipeline"]
fn original_samples_keep_shared_display_analysis_and_restart_usable() {
    use photosort::core::catalog::Catalog;
    let source = std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("PHOTOSORT_SAMPLE_DIR");
    let catalog = Catalog::scan(Path::new(&source), || false, |_| {}).unwrap();
    assert_eq!(catalog.media.len(), 7);
    let cache = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    for media in &catalog.media {
        let before = blake3::hash(&std::fs::read(&media.path).unwrap());
        let display = pipeline.load(media).unwrap();
        let analysis = pipeline.analysis(media, || false).unwrap();
        assert!(display.pixels.width() > 0 && analysis.pixels.width() > 0);
        assert_eq!(before, blake3::hash(&std::fs::read(&media.path).unwrap()));
    }
    let mut restarted = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    for media in &catalog.media {
        restarted.load(media).unwrap();
        restarted.analysis(media, || false).unwrap();
    }
    assert_eq!(restarted.stats.decodes, 0);
    assert_eq!(restarted.stats.color_transforms, 0);
    eprintln!(
        "native sample color initializations={} conversions={} restart_decodes={}",
        pipeline.stats.color_initializations,
        pipeline.stats.color_transforms,
        restarted.stats.decodes
    );
}

#[cfg(all(target_os = "macos", feature = "heif"))]
#[test]
fn cmyk_and_ycck_profiles_preserve_ink_channels_and_share_corrected_results() {
    let temp = tempfile::tempdir().unwrap();
    let profile =
        std::fs::read("/System/Library/ColorSync/Profiles/Generic CMYK Profile.icc").unwrap();
    let mut pipeline = Pipeline::new(8 * 1024 * 1024, temp.path().join("cache"));
    for (name, color) in [
        ("cmyk", jpeg_encoder::ColorType::Cmyk),
        ("ycck", jpeg_encoder::ColorType::CmykAsYcck),
    ] {
        let path = temp.path().join(format!("{name}.jpg"));
        let mut encoder = jpeg_encoder::Encoder::new(File::create(&path).unwrap(), 100);
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::F_1_1);
        encoder.add_icc_profile(&profile).unwrap();
        let mut ink = Vec::new();
        for y in 0..32u8 {
            for x in 0..64u8 {
                ink.extend_from_slice(&[x * 4, y * 8, (x + y) * 2, (x / 8) * 24]);
            }
        }
        encoder.encode(&ink, 64, 32, color).unwrap();
        let original = std::fs::read(&path).unwrap();
        let media = Media {
            id: MediaId::for_path(&path),
            fingerprint: Fingerprint::read(&path).unwrap(),
            kind: MediaKind::Photo,
            path: path.clone(),
        };
        let before = pipeline.stats.decodes;
        let display = pipeline.load(&media).unwrap();
        let analysis = pipeline.analysis(&media, || false).unwrap();
        let detail = pipeline
            .inspect(&Request {
                slot: 0,
                media: media.clone(),
                region: Region {
                    x: 0,
                    y: 0,
                    width: 64,
                    height: 32,
                },
                output_edge: 64,
            })
            .unwrap();
        assert_eq!(pipeline.stats.decodes, before + 1);
        #[cfg(feature = "native-imaging")]
        assert_eq!(pipeline.stats.native_renders, pipeline.stats.decodes);
        #[cfg(not(feature = "native-imaging"))]
        assert_eq!(pipeline.stats.color_initializations, 1);
        assert_eq!(display.pixels.as_raw(), analysis.pixels.as_raw());
        assert_eq!(display.pixels.as_raw(), detail.as_raw());
        let rendered = photosort::core::apple_image::decode(&path, None).unwrap();
        let (w, h) = rendered.pixels.dimensions();
        let native = rendered.pixels.into_raw();
        assert_eq!((w, h), (64, 32));
        let max = display
            .pixels
            .as_raw()
            .iter()
            .zip(&native)
            .map(|(a, b)| i16::from(*a).abs_diff(i16::from(*b)))
            .max()
            .unwrap();
        // Independent CMMs are not pixel-identical. Preserve the existing
        // moxcms bound; track the measured libvips/LCMS fixture envelope
        // separately.
        let bound = if cfg!(feature = "native-imaging") {
            9
        } else {
            8
        };
        assert!(
            max <= bound,
            "{name}: maximum difference from ColorSync {max}"
        );
        #[cfg(feature = "native-imaging")]
        {
            let mean = display
                .pixels
                .as_raw()
                .as_chunks::<4>()
                .0
                .iter()
                .zip(native.as_chunks::<4>().0)
                .flat_map(|(a, b)| {
                    (0..3).map(move |channel| a[channel].abs_diff(b[channel]) as f64)
                })
                .sum::<f64>()
                / (64.0 * 32.0 * 3.0);
            assert!(
                mean <= 1.0,
                "{name}: mean RGB difference from ColorSync {mean}"
            );
        }
        let mut restarted = Pipeline::new(8 * 1024 * 1024, temp.path().join("cache"));
        assert_eq!(
            restarted.load(&media).unwrap().pixels.as_raw(),
            display.pixels.as_raw()
        );
        assert_eq!(restarted.stats.decodes, 0);
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
    #[cfg(not(feature = "native-imaging"))]
    assert_eq!(pipeline.stats.color_transforms, 2);
}

#[test]
fn a_cmyk_profile_on_rgb_jpeg_is_rejected_before_color_conversion() {
    // The signature routes the decoder; the mismatched channel count must be
    // rejected even before parsing this deliberately incomplete ICC profile.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("mismatched.jpg");
    let mut profile = vec![0; 128];
    profile[16..20].copy_from_slice(b"CMYK");
    let mut encoder = jpeg_encoder::Encoder::new(File::create(&path).unwrap(), 90);
    encoder.add_icc_profile(&profile).unwrap();
    encoder
        .encode(&[128; 3 * 64], 8, 8, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    let media = Media {
        id: MediaId::for_path(&path),
        fingerprint: Fingerprint::read(&path).unwrap(),
        kind: MediaKind::Photo,
        path,
    };
    let mut pipeline = Pipeline::new(8 * 1024 * 1024, temp.path().join("cache"));
    let error = pipeline
        .load(&media)
        .err()
        .expect("mismatched profile must fail");
    #[cfg(not(feature = "native-imaging"))]
    assert!(format!("{error:#}").contains("CMYK profile does not match JPEG color channels"));
    #[cfg(feature = "native-imaging")]
    {
        assert!(
            format!("{error:#}").contains("profile is invalid or does not match image channels")
        );
        assert_eq!(pipeline.stats.native_renders, 1);
    }
    assert_eq!(pipeline.stats.resident_bytes, 0);
    assert_eq!(pipeline.stats.color_transforms, 0);
}
