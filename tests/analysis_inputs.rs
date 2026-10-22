//! Shared neutral inputs must not inherit display resizing or RAW enhancements.
#[path = "../src/test_support.rs"]
mod test_support;
use photosort::core::{catalog::Catalog, metadata, pipeline::Pipeline};
use std::{
    sync::Arc,
    sync::atomic::{AtomicUsize, Ordering},
};

#[test]
fn analysis_and_display_share_decoding_but_keep_independent_cached_pixels() {
    let root = tempfile::tempdir().unwrap();
    let source = image::RgbImage::from_fn(2500, 1300, |x, y| {
        image::Rgb([(x % 256) as u8, (y % 256) as u8, ((x + y) % 256) as u8])
    });
    source.save(root.path().join("photo.png")).unwrap();
    let catalog = Catalog::scan(root.path(), || false, |_| {}).unwrap();
    let media = &catalog.media[0];
    for display_first in [false, true] {
        let cache = tempfile::tempdir().unwrap();
        let mut pipeline = Pipeline::new(32 * 1024 * 1024, cache.path().into());
        if display_first {
            pipeline.load(media).unwrap();
        }
        let analysis = pipeline.analysis(media, || false).unwrap();
        assert_eq!(analysis.pixels.dimensions(), (1024, 532));
        let display = pipeline.load(media).unwrap();
        assert_eq!(display.pixels.dimensions(), (2048, 1065));
        assert!(!Arc::ptr_eq(&analysis.pixels, &display.pixels));
        for _ in photosort::core::model::Step::ALL {
            assert!(Arc::ptr_eq(
                &analysis.pixels,
                &pipeline.analysis(media, || false).unwrap().pixels
            ));
            assert!(Arc::ptr_eq(
                &display.pixels,
                &pipeline.load(media).unwrap().pixels
            ));
        }
        assert_eq!(
            (pipeline.stats.decodes, pipeline.stats.metadata_reads),
            (1, 1)
        );
        assert!(pipeline.stats.resident_bytes <= 32 * 1024 * 1024);
        let mut reopened = Pipeline::new(32 * 1024 * 1024, cache.path().into());
        assert_eq!(
            *reopened.analysis(media, || false).unwrap().pixels,
            *analysis.pixels
        );
        assert_eq!(*reopened.load(media).unwrap().pixels, *display.pixels);
        assert_eq!((reopened.stats.decodes, reopened.stats.disk_hits), (0, 2));
        reopened.clear_memory();
        assert_eq!(reopened.stats.resident_bytes, 0);
        assert_eq!(reopened.maintain_cache(true, || false).unwrap().files, 0);
    }
}

#[test]
fn analysis_respects_orientation_and_invalidates_with_source_and_sidecar_changes() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("photo.png");
    image::RgbImage::from_fn(9, 5, |x, y| image::Rgb([x as u8, y as u8, 0]))
        .save(&path)
        .unwrap();
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let cache = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
    let first = pipeline.analysis(&media, || false).unwrap();
    assert_eq!(first.pixels.dimensions(), (9, 5));
    assert_eq!(first.pixels.get_pixel(8, 4).0, [8, 4, 0, 255]);
    metadata::write_sidecar(&path, Some(4), Some(6)).unwrap();
    let second = pipeline.analysis(&media, || false).unwrap();
    assert_eq!(second.pixels.dimensions(), (5, 9));
    assert_eq!(second.pixels.get_pixel(0, 8).0, [8, 4, 0, 255]);
    assert_eq!(second.metadata.rating, 4);
    image::RgbImage::from_pixel(9, 5, image::Rgb([20, 30, 40]))
        .save(&path)
        .unwrap();
    let third = pipeline.analysis(&media, || false).unwrap();
    assert_eq!(third.pixels.get_pixel(0, 8).0, [20, 30, 40, 255]);
    assert_eq!(
        (pipeline.stats.decodes, pipeline.stats.metadata_reads),
        (3, 3)
    );
}

#[test]
fn cancelled_or_changed_preparation_never_publishes_analysis_pixels() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("photo.png");
    image::RgbImage::new(40, 20).save(&path).unwrap();
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let cache = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().canonicalize().unwrap());
    assert!(pipeline.analysis(&media, || true).is_err());
    assert_eq!(pipeline.stats.decodes, 0);
    let calls = AtomicUsize::new(0);
    assert!(
        pipeline
            .analysis(&media, || {
                calls.fetch_add(1, Ordering::Relaxed);
                calls.load(Ordering::Relaxed) >= 2
            })
            .is_err()
    );
    assert_eq!(std::fs::read_dir(cache.path()).unwrap().count(), 0);
    assert_eq!(pipeline.stats.resident_bytes, 0);
    // A completed canonical decode may be reused after cancellation.
    assert!(pipeline.analysis(&media, || false).is_ok());
    assert_eq!(pipeline.stats.decodes, 1);
    pipeline.clear_memory();
    pipeline.maintain_cache(true, || false).unwrap();
    let calls = AtomicUsize::new(0);
    assert!(
        pipeline
            .analysis(&media, || {
                calls.fetch_add(1, Ordering::Relaxed);
                if calls.load(Ordering::Relaxed) == 2 {
                    image::RgbImage::from_pixel(40, 20, image::Rgb([70, 80, 90]))
                        .save(&path)
                        .unwrap();
                }
                false
            })
            .is_err()
    );
    // Maintenance retains its namespace lock. It is coordination state, not
    // decoded pixels or analysis evidence; every other file is still a failure.
    for entry in std::fs::read_dir(cache.path()).unwrap() {
        let entry = entry.unwrap();
        assert_eq!(entry.file_name(), ".photosort-compiled-cache.lock");
        assert!(entry.file_type().unwrap().is_file());
        assert_eq!(entry.metadata().unwrap().len(), 0);
    }
    assert_eq!(pipeline.stats.resident_bytes, 0);
}

#[test]
fn palette_analysis_filters_colors_instead_of_aliasing_palette_indices() {
    // Python/Pillow's P-mode thumbnail silently uses nearest-neighbor even
    // when Lanczos is requested: this alternating pattern becomes pure white.
    // Filter decoded colors instead, so subpixel detail averages near middle gray.
    let root = tempfile::tempdir().unwrap();
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/analysis/palette.png"),
        root.path().join("palette.png"),
    )
    .unwrap();
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let cache = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
    let asset = pipeline.analysis(&media, || false).unwrap();
    assert_eq!(asset.pixels.dimensions(), (1024, 1));
    let interior = &asset.pixels.as_raw()[16..asset.pixels.len() - 16];
    for pixel in interior.as_chunks::<4>().0 {
        assert!(pixel[..3].iter().all(|v| (127..=128).contains(v)));
    }
}

#[cfg(feature = "raw")]
#[test]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR and PHOTOSORT_ANALYSIS_REFERENCE captured by scripts/capture_analysis_reference.py"]
fn seven_samples_use_neutral_inputs_and_raw_embedded_or_fallback_paths() -> anyhow::Result<()> {
    use photosort::core::{analysis, model::MediaKind, operations};
    use std::{path::PathBuf, time::Instant};
    let root = PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap());
    let reference = PathBuf::from(std::env::var_os("PHOTOSORT_ANALYSIS_REFERENCE").unwrap());
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    assert_eq!(catalog.media.len(), 7);
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    for media in &catalog.media {
        let hash = operations::content_hash(&media.path, || false)?;
        let t = Instant::now();
        let input = pipeline.analysis(media, || false)?;
        let cold = t.elapsed();
        let expected = image::open(reference.join(format!("{}.png", media.name())))?.to_rgba8();
        assert_eq!(
            input.pixels.dimensions(),
            expected.dimensions(),
            "{}",
            media.name()
        );
        let difference = input
            .pixels
            .iter()
            .zip(expected.iter())
            .map(|(a, b)| a.abs_diff(*b) as f64)
            .sum::<f64>()
            / expected.len() as f64;
        if media
            .path
            .extension()
            .is_none_or(|ext| !ext.eq_ignore_ascii_case("png"))
        {
            assert!(
                difference < 1.0,
                "{}: neutral pixels drifted from reference by {difference}",
                media.name()
            );
        }
        let score = analysis::analyze(&input, Default::default());
        let expected_asset = photosort::core::pipeline::Asset {
            decoder_warning: None,
            pixels: Arc::new(expected.clone()),
            thumbnail: Arc::new(expected.clone()),
            metadata: input.metadata.clone(),
        };
        let reference_score = analysis::analyze(&expected_asset, Default::default());
        assert_eq!(
            score.issues,
            reference_score.issues,
            "{}: technical category differs",
            media.name()
        );
        let decodes = pipeline.stats.decodes;
        for _ in photosort::core::model::Step::ALL {
            assert!(Arc::ptr_eq(
                &input.pixels,
                &pipeline.analysis(media, || false)?.pixels
            ));
        }
        assert_eq!(pipeline.stats.decodes, decodes);
        if media.kind == MediaKind::Raw {
            assert_eq!(
                (pipeline.stats.raw_embedded, pipeline.stats.raw_fallback),
                (1, 0)
            );
        }
        assert_eq!(operations::content_hash(&media.path, || false)?, hash);
        eprintln!(
            "{}: Python-input sharpness {:.2}, neutral-input difference {:.2}%",
            media.name(),
            reference_score.sharpness,
            (score.sharpness / reference_score.sharpness - 1.) * 100.
        );
        eprintln!(
            "{}: neutral {:?}, cold {:?}, reference mean absolute channel difference {difference:.4}, sharpness {:.2}",
            media.name(),
            input.pixels.dimensions(),
            cold,
            score.sharpness
        );
    }
    let fallback = Catalog::scan(&reference, || false, |_| {})?
        .media
        .into_iter()
        .find(|m| m.name() == "no-preview.ARW")
        .unwrap();
    let cancelled_cache = tempfile::tempdir()?;
    let mut cancelled_pipeline = Pipeline::new(32 * 1024 * 1024, cancelled_cache.path().into());
    let polls = AtomicUsize::new(0);
    let cancelled = cancelled_pipeline.analysis(&fallback, || {
        polls.fetch_add(1, Ordering::Relaxed);
        polls.load(Ordering::Relaxed) >= 3
    });
    let error = cancelled
        .err()
        .expect("Native RAW cancellation must fail preparation");
    assert!(
        error.to_string().contains("LibRaw decode failed"),
        "Cancellation must interrupt LibRaw itself: {error}"
    );
    assert!(
        polls.load(Ordering::Relaxed) >= 3,
        "LibRaw did not poll cancellation"
    );
    assert_eq!(std::fs::read_dir(cancelled_cache.path())?.count(), 0);
    assert_eq!(cancelled_pipeline.stats.resident_bytes, 0);
    let t = Instant::now();
    let input = pipeline.analysis(&fallback, || false)?;
    assert_eq!(
        (pipeline.stats.raw_embedded, pipeline.stats.raw_fallback),
        (1, 1)
    );
    let expected = image::open(reference.join("no-preview.ARW.png"))?.to_rgba8();
    assert_eq!(input.pixels.dimensions(), expected.dimensions());
    let difference = input
        .pixels
        .iter()
        .zip(expected.iter())
        .map(|(a, b)| a.abs_diff(*b) as f64)
        .sum::<f64>()
        / expected.len() as f64;
    assert!(
        difference < 1.0,
        "Neutral fallback drifted from reference by {difference}"
    );
    eprintln!(
        "RAW half-size fallback: {:?}, reference mean absolute channel difference {difference:.4}",
        t.elapsed()
    );
    Ok(())
}

#[test]
fn cancelled_analysis_retires_without_errors_and_serves_display_from_shared_decode() {
    use photosort::core::jobs::{AnalysisProfile, Event, ImageWork, Runtime};
    use std::time::Instant;
    let root = tempfile::tempdir().unwrap();
    image::RgbImage::from_pixel(200, 100, image::Rgb([40, 80, 120]))
        .save(root.path().join("photo.png"))
        .unwrap();
    let media = Catalog::scan(root.path(), || false, |_| {})
        .unwrap()
        .media
        .remove(0);
    let cache = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::with_cache(|| {}, cache.path().into());
    let run = runtime
        .analyze(
            Arc::new(vec![media.clone(); 400]),
            AnalysisProfile::Technical,
        )
        .unwrap();
    let deadline = Instant::now() + test_support::worker_patience();
    loop {
        assert!(Instant::now() < deadline);
        match runtime
            .events
            .recv_timeout(test_support::worker_patience())
            .unwrap()
            .event
        {
            Event::Analyzed { run: found, .. } if found == run => break,
            Event::AnalysisError { error, .. }
            | Event::RunError { error, .. }
            | Event::Error(error) => panic!("{error}"),
            _ => {}
        }
    }
    runtime.cancel_analysis();
    runtime.image(ImageWork::Asset(media, true)).unwrap();
    loop {
        assert!(Instant::now() < deadline);
        match runtime
            .events
            .recv_timeout(test_support::worker_patience())
            .unwrap()
            .event
        {
            Event::Asset { stats, .. } => {
                assert_eq!(stats.decodes, 1);
                break;
            }
            Event::AnalysisDone { run: found, .. } if found == run => {
                panic!("Cancelled run completed")
            }
            Event::AnalysisError { error, .. }
            | Event::AssetError { error, .. }
            | Event::Error(error) => panic!("{error}"),
            _ => {}
        }
    }
    runtime.shutdown();
    while let Ok(event) = runtime.events.try_recv() {
        match event.event {
            Event::AnalysisError { error, .. } | Event::Error(error) => panic!("{error}"),
            Event::AnalysisDone { run: found, .. } if found == run => {
                panic!("Cancelled run completed")
            }
            _ => {}
        }
    }
}

#[test]
fn opaque_proxies_drop_redundant_alpha_without_changing_pixels_or_repeating_work() {
    for transparent in [false, true] {
        let source = tempfile::tempdir().unwrap();
        let path = source.path().join("rgba.png");
        image::RgbaImage::from_fn(256, 128, |x, y| {
            image::Rgba([
                x as u8,
                y as u8,
                91,
                if transparent { x as u8 } else { 255 },
            ])
        })
        .save(&path)
        .unwrap();
        let catalog = Catalog::scan(source.path(), || false, |_| {}).unwrap();
        let media = &catalog.media[0];
        let cache = tempfile::tempdir().unwrap();
        let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
        let display = pipeline.load(media).unwrap();
        let analysis = pipeline.analysis(media, || false).unwrap();
        if transparent {
            assert_eq!(display.pixels.get_pixel(0, 0)[3], 0);
            assert_eq!(display.pixels.get_pixel(127, 0)[3], 127);
        }
        for _ in photosort::core::model::Step::ALL {
            assert!(Arc::ptr_eq(
                &display.pixels,
                &pipeline.load(media).unwrap().pixels
            ));
            assert!(Arc::ptr_eq(
                &analysis.pixels,
                &pipeline.analysis(media, || false).unwrap().pixels
            ));
        }
        assert_eq!(
            (pipeline.stats.decodes, pipeline.stats.metadata_reads),
            (1, 1)
        );
        let proxies: Vec<_> = std::fs::read_dir(cache.path())
            .unwrap()
            .map(|p| p.unwrap().path())
            .filter(|p| p.extension().is_some_and(|ext| ext == "png"))
            .collect();
        // Display proxy, analysis proxy and the grid thumbnail.
        assert_eq!(proxies.len(), 3);
        for path in proxies {
            assert_eq!(
                image::open(path).unwrap().color(),
                if transparent {
                    image::ColorType::Rgba8
                } else {
                    image::ColorType::Rgb8
                }
            );
        }
        let mut reopened = Pipeline::new(1024 * 1024, cache.path().into());
        assert_eq!(*reopened.load(media).unwrap().pixels, *display.pixels);
        assert_eq!(
            *reopened.analysis(media, || false).unwrap().pixels,
            *analysis.pixels
        );
        assert_eq!(
            (
                reopened.stats.decodes,
                reopened.stats.metadata_reads,
                reopened.stats.disk_hits
            ),
            (0, 0, 2)
        );
    }
}
