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
            memory: None,
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
            memory: None,
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
                region: None,
                memory: None,
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
                }),
                memory: None,
            },
            || false
        )
        .is_err()
    );
}
/// A flat gray baseline JPEG of any size in a few bytes per 64 pixels: one-code
/// Huffman tables make every 8x8 block two zero bits (DC unchanged, end of block).
fn flat_jpeg(path: &Path, width: u16, height: u16) {
    let mut jpeg = vec![0xFF, 0xD8];
    let segment = |jpeg: &mut Vec<u8>, marker: u8, body: &[u8]| {
        jpeg.extend([0xFF, marker]);
        jpeg.extend((body.len() as u16 + 2).to_be_bytes());
        jpeg.extend(body);
    };
    segment(&mut jpeg, 0xDB, &[[0].as_slice(), &[1; 64]].concat());
    let [w0, w1] = width.to_be_bytes();
    let [h0, h1] = height.to_be_bytes();
    segment(&mut jpeg, 0xC0, &[8, h0, h1, w0, w1, 1, 1, 0x11, 0]);
    for class in [0x00, 0x10] {
        let mut table = vec![class, 1];
        table.extend([0; 15]);
        table.push(0);
        segment(&mut jpeg, 0xC4, &table);
    }
    segment(&mut jpeg, 0xDA, &[1, 1, 0, 0, 63, 0]);
    let blocks = usize::from(width / 8) * usize::from(height / 8);
    jpeg.resize(jpeg.len() + blocks * 2 / 8, 0);
    jpeg.extend([0xFF, 0xD9]);
    std::fs::write(path, jpeg).unwrap();
}
#[test]
fn previews_and_regions_of_huge_sources_decode_and_full_size_follows_memory() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.jpg");
    // 552 million samples, past the 512 MiB budget for a full-size decode.
    flat_jpeg(&path, 24000, 23000);
    let preview = render(
        &path,
        Request {
            edge: Some(256),
            region: None,
            memory: None,
        },
        || false,
    )
    .unwrap();
    assert_eq!((preview.width, preview.height), (256, 245));
    assert_eq!(
        (preview.source_width, preview.source_height),
        (24000, 23000)
    );
    assert!(rgba(preview).chunks(4).all(|p| p == [128, 128, 128, 255]));
    // A 1:1 region decodes only its crop, whatever the source size.
    let region = render_region(
        &path,
        Request {
            edge: None,
            region: Some(Region {
                x: 12000,
                y: 20000,
                width: 300,
                height: 200,
            }),
            memory: None,
        },
        1,
        0,
        || false,
    )
    .unwrap();
    assert_eq!((region.width, region.height), (300, 200));
    // A full-size render holds the source (one grey byte a pixel, plus RGBA
    // out): refused over its memory, saying what it needs.
    let full = render(&path, Request::default(), || false).unwrap_err();
    assert_eq!(full.needed(), Some(24000 * 23000 * 5), "{full}");
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
        let output = render(
            &path,
            Request {
                edge,
                region: None,
                memory: None,
            },
            || false,
        )
        .unwrap();
        assert!(output.profiled);
        for pixel in rgba(output).as_chunks::<4>().0 {
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
            let error = render(
                &path,
                Request {
                    edge,
                    region: None,
                    memory: None,
                },
                || false,
            )
            .unwrap_err();
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
                region: None,
                memory: None,
            },
            || false
        )
        .unwrap()
        .width,
        32
    );
}
/// A gradient JPEG, and the same with a scan header libjpeg warns about.
fn odd_scan_jpeg(width: u16, height: u16) -> (Vec<u8>, Vec<u8>) {
    let source: Vec<u8> = (0..u32::from(height))
        .flat_map(|y| (0..u32::from(width)).flat_map(move |x| [(x / 4) as u8, (y / 2) as u8, 128]))
        .collect();
    let mut jpeg = Vec::new();
    jpeg_encoder::Encoder::new(&mut jpeg, 90)
        .encode(&source, width, height, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    let scan = jpeg.windows(2).position(|w| w == [0xFF, 0xDA]).unwrap();
    // After the component table: spectral start, spectral end (63), approximation.
    let spectral_end = scan + 5 + usize::from(jpeg[scan + 4]) * 2 + 1;
    assert_eq!(jpeg[spectral_end], 63);
    let mut odd = jpeg.clone();
    odd[spectral_end] = 0;
    (jpeg, odd)
}
#[test]
fn renders_on_many_threads_keep_each_files_pixels_error_and_warning() {
    let temp = tempfile::tempdir().unwrap();
    let file = |name: &str| temp.path().join(name);
    let mut files = Vec::new();
    // Photos that open, each with its own size and colour.
    for n in 0..4u8 {
        let path = file(&format!("good-{n}.png"));
        let pixels = RgbaImage::from_pixel(40 + u32::from(n), 30, Rgba([n * 60, 200, 17, 255]));
        png(&path, &pixels, None);
        files.push(path);
    }
    for n in 0..3u16 {
        let (clean, odd) = odd_scan_jpeg(160 + n * 16, 120);
        // Opens, with a warning.
        std::fs::write(file(&format!("warns-{n}.jpg")), &odd).unwrap();
        // Cut short: fails inside the JPEG decoder.
        std::fs::write(file(&format!("cut-{n}.jpg")), &clean[..clean.len() / 2]).unwrap();
        files.extend([
            file(&format!("warns-{n}.jpg")),
            file(&format!("cut-{n}.jpg")),
        ]);
    }
    // Fails inside the PNG decoder, is no image at all, does not exist.
    png(
        &file("cut.png"),
        &RgbaImage::from_pixel(300, 200, Rgba([1, 2, 3, 255])),
        None,
    );
    let whole = std::fs::read(file("cut.png")).unwrap();
    std::fs::write(file("cut.png"), &whole[..whole.len() / 2]).unwrap();
    std::fs::write(file("notes.jpg"), b"not an image at all").unwrap();
    files.extend(["cut.png", "notes.jpg", "missing-a.jpg", "missing-b.png"].map(file));

    type Outcome = Result<(u32, u32, Vec<u8>, Option<String>), String>;
    let outcome = |path: &Path| -> Outcome {
        render(path, Request::default(), || false)
            .map(|image| {
                let warning = image.decoder_warning.clone();
                (image.width, image.height, rgba(image), warning)
            })
            .map_err(|error| error.to_string())
    };
    // One at a time first: what each file gives when nothing else runs.
    let alone: Vec<Outcome> = files.iter().map(|path| outcome(path)).collect();
    let error = |name: &str| {
        let index = files.iter().position(|path| path.ends_with(name)).unwrap();
        alone[index].clone().unwrap_err()
    };
    // The failures differ, so a message that went to the wrong file shows.
    let kinds = [
        "cut-0.jpg",
        "cut.png",
        "notes.jpg",
        "missing-a.jpg",
        "missing-b.png",
    ]
    .map(error);
    for (index, kind) in kinds.iter().enumerate() {
        assert!(
            !kind.is_empty() && !kinds[..index].contains(kind),
            "{kinds:?}"
        );
    }
    assert!(
        error("missing-a.jpg").contains("missing-a.jpg"),
        "{kinds:?}"
    );
    for (path, outcome) in files.iter().zip(&alone) {
        let name = path.file_name().unwrap().to_str().unwrap();
        match outcome {
            Ok((.., warning)) if name.starts_with("warns") => {
                assert!(
                    warning
                        .as_deref()
                        .is_some_and(|w| w.contains("Invalid SOS"))
                );
            }
            Ok((.., warning)) => assert!(name.starts_with("good") && warning.is_none()),
            Err(_) => assert!(!name.starts_with("good") && !name.starts_with("warns")),
        }
    }
    // Then on eight threads at once, each starting at another file.
    std::thread::scope(|scope| {
        for thread in 0..8 {
            let (files, alone, outcome) = (&files, &alone, &outcome);
            scope.spawn(move || {
                for round in 0..30 {
                    for step in 0..files.len() {
                        let index = (thread * 3 + round + step) % files.len();
                        assert_eq!(
                            outcome(&files[index]),
                            alone[index],
                            "{:?} on thread {thread}",
                            files[index]
                        );
                    }
                }
            });
        }
    });
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
    // A JPEG the decoder only warns about opens, and says what it warned
    // about: some phones write a scan header libjpeg calls invalid.
    let path = temp.path().join("odd-scan-header.jpg");
    let (jpeg, odd) = odd_scan_jpeg(640, 480);
    std::fs::write(&path, odd).unwrap();
    // Full size, a preview decoded at a fraction, and one only reduced.
    for (edge, size) in [
        (None, (640, 480)),
        (Some(100), (100, 75)),
        (Some(500), (500, 375)),
    ] {
        let request = Request {
            edge,
            region: None,
            memory: None,
        };
        let image = render(&path, request, || false).unwrap();
        assert_eq!((image.width, image.height), size);
        assert!(
            image
                .decoder_warning
                .as_deref()
                .is_some_and(|s| s.contains("Invalid SOS parameters")),
            "{:?}",
            image.decoder_warning
        );
    }
    // One cut short still fails.
    std::fs::write(&path, &jpeg[..jpeg.len() / 2]).unwrap();
    assert!(render(&path, Request::default(), || false).is_err());
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
                memory: None,
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
    let unique: std::collections::BTreeSet<_> = samples
        .as_chunks::<4>()
        .0
        .iter()
        .take(64)
        .map(|p| p[0])
        .collect();
    assert!(
        unique.len() > 40,
        "color was quantized to eight bits: {} values",
        unique.len()
    );
    for pixel in samples.as_chunks::<4>().0 {
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
        assert!(pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
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
                memory: None,
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
        let output = render(
            &path,
            Request {
                edge,
                region: None,
                memory: None,
            },
            || false,
        )
        .unwrap();
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
        let output = render(
            &path,
            Request {
                edge,
                region: None,
                memory: None,
            },
            || false,
        )
        .unwrap();
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
#[test]
fn jpeg_previews_shrink_while_decoding_to_the_requested_edge() {
    // 1600x1200 → 400 uses a 1/4 scaled decode with nothing left to reduce;
    // → 500 uses 1/2 (800x600) and one reduce. Both keep the aspect ratio and
    // the colors of a smooth gradient.
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("gradient.jpg");
    let (width, height) = (1600u16, 1200u16);
    let source: Vec<u8> = (0..u32::from(height))
        .flat_map(|y| (0..u32::from(width)).flat_map(move |x| [(x / 8) as u8, (y / 6) as u8, 128]))
        .collect();
    jpeg_encoder::Encoder::new(File::create(&path).unwrap(), 95)
        .encode(&source, width, height, jpeg_encoder::ColorType::Rgb)
        .unwrap();
    for (edge, expected) in [(400, (400, 300)), (500, (500, 375))] {
        let image = render(
            &path,
            Request {
                edge: Some(edge),
                ..Default::default()
            },
            || false,
        )
        .unwrap();
        assert_eq!((image.width, image.height), expected);
        assert_eq!((image.source_width, image.source_height), (1600, 1200));
        let (w, h) = expected;
        let pixels = rgba(image);
        for (x, y) in [(w / 4, h / 4), (w / 2, h / 2), (w * 3 / 4, h * 3 / 4)] {
            let at = ((y * w + x) * 4) as usize;
            let want = [(x * 1600 / w / 8) as i16, (y * 1200 / h / 6) as i16, 128];
            for c in 0..3 {
                let got = i16::from(pixels[at + c]);
                assert!(
                    (got - want[c]).abs() <= 4,
                    "edge {edge} at {x},{y} channel {c}: {got} vs {}",
                    want[c]
                );
            }
            assert_eq!(pixels[at + 3], 255);
        }
    }
}
