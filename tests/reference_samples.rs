//! Opt-in real-camera regression tests. Originals are read-only; edits use copies.
//! PHOTOSORT_SAMPLE_DIR=tests/samples cargo test --release --test reference_samples -- --ignored --nocapture
#[path = "../src/test_support.rs"]
mod test_support;
use photosort::core::{
    catalog::Catalog, metadata, model::*, models::Models, operations, pipeline::Pipeline,
};
use std::{collections::HashMap, fs, path::PathBuf, sync::Arc, time::Instant};

#[test]
#[ignore = "requires the reference sample directory and installed native models"]
fn reference_samples_decode_metadata_models_and_mutations() -> anyhow::Result<()> {
    let root =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    anyhow::ensure!(
        catalog.media.len() >= 7,
        "Expected the seven reference samples"
    );
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().to_owned());
    let mut models = Models::default();
    let orientation =
        photosort::core::models::orientation_model().expect("Install orientation model");
    let mut found_faces = 0;
    let mut original_hashes = Vec::new();
    for media in &catalog.media {
        let hash = operations::content_hash(&media.path, || false)?;
        original_hashes.push((media.path.clone(), hash));
        let start = Instant::now();
        let asset = pipeline.load(media)?;
        let cold = start.elapsed();
        assert!(asset.pixels.width() > 0 && asset.pixels.height() > 0);
        assert!(asset.pixels.width().max(asset.pixels.height()) <= 2048);
        assert!(asset.thumbnail.width().max(asset.thumbnail.height()) <= 320);
        let decodes = pipeline.stats.decodes;
        for _ in Step::ALL {
            let reused = pipeline.load(media)?;
            assert!(Arc::ptr_eq(&asset.pixels, &reused.pixels));
        }
        assert_eq!(
            pipeline.stats.decodes, decodes,
            "workflow repeated decoding"
        );
        // A fresh process must recover canonical dimensions from the disk cache,
        // even when EXIF reports a different embedded-preview/active-area size.
        let mut reopened = Pipeline::new(128 * 1024 * 1024, cache.path().to_owned());
        let cached = reopened.load(media)?;
        assert_eq!(
            (cached.metadata.width, cached.metadata.height),
            (asset.metadata.width, asset.metadata.height),
            "{}: cached geometry differs from canonical source",
            media.name()
        );
        assert_eq!(
            reopened.stats.decodes, 0,
            "Disk proxy unnecessarily decoded source"
        );
        assert_eq!(reopened.stats.disk_hits, 1);
        let meta = &asset.metadata;
        let (width, height) = if (5..=8).contains(&meta.orientation) {
            (meta.height, meta.width)
        } else {
            (meta.width, meta.height)
        };
        let region = photosort::core::inspection::Region {
            x: width / 4,
            y: height / 4,
            width: 200.min(width / 2),
            height: 200.min(height / 2),
        };
        let request = photosort::core::inspection::Request {
            slot: 0,
            media: media.clone(),
            region,
            output_edge: 512,
        };
        let detail = pipeline.inspect(&request)?;
        assert_eq!(detail.dimensions(), (region.width, region.height));
        // Backend-specific decode counts are covered by the shared pipeline tests.
        // A first detail request may decode once; repeating it must reuse exact pixels.
        assert!((decodes..=decodes + 1).contains(&pipeline.stats.decodes));
        let after_detail = pipeline.stats.decodes;
        assert_eq!(pipeline.inspect(&request)?, detail);
        assert_eq!(pipeline.stats.decodes, after_detail);
        let decodes = pipeline.stats.decodes;
        let faces = models.faces(&asset)?;
        let before_inferences = models.inferences;
        let dense = models.dense_embedding(&asset)?;
        assert_eq!(
            models.inferences,
            before_inferences + 1,
            "CLS and dense regions repeated DINO inference"
        );
        assert_eq!(
            pipeline.stats.decodes, decodes,
            "Dense extraction repeated decoding"
        );
        let vector = &dense.global;
        assert_eq!(dense.regions.len(), 16);
        assert!(
            dense
                .regions
                .iter()
                .all(|r| r.descriptor.len() == 384 && r.kind == "region")
        );
        // Policy consumes already-extracted evidence, with no image/model access.
        let artifact = dense.clone().into_artifact(Vec::new());
        let pair = photosort::core::subjects::verify_pair(
            &artifact,
            &artifact,
            Default::default(),
            None,
            None,
            Default::default(),
        );
        assert!(pair.accepted, "identical region evidence was rejected");
        assert_eq!(models.inferences, before_inferences + 1);
        let score = models.aesthetic(&asset)?;
        let rotation = models.rotation(&orientation, &asset)?;
        assert_eq!(vector.len(), 384);
        assert!((vector.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-4);
        assert!((0.0..=1.0).contains(&score));
        found_faces += faces.count;
        println!(
            "{}: {}x{}, decode {:?}, camera {}, orientation {}, faces {}, eyes closed {}, aesthetic {:.4}, rotation {:?}",
            media.name(),
            asset.pixels.width(),
            asset.pixels.height(),
            cold,
            asset.metadata.camera,
            asset.metadata.orientation,
            faces.count,
            faces.closed_eyes,
            score,
            rotation
        );
        if matches!(media.name().as_str(), "arw_sample.ARW" | "jpg_sample.jpg") {
            assert_eq!(asset.metadata.camera, "ILCE-7M3");
            assert_eq!(asset.metadata.lens, "FE 28-70mm F3.5-5.6 OSS");
            assert_eq!(
                asset.metadata.iso,
                if media.kind == MediaKind::Raw {
                    "64"
                } else {
                    "100"
                }
            );
        }
    }
    // Landscapes may produce detector candidates that Face Mesh rejects.
    // Both no-candidate (four sessions) and rejected-candidate (five) are valid.
    assert!((4..=5).contains(&models.initializations));
    let initialized = models.initializations;
    let asset = pipeline.load(&catalog.media[0])?;
    models.faces(&asset)?;
    models.embedding(&asset)?;
    models.aesthetic(&asset)?;
    models.rotation(&orientation, &asset)?;
    assert_eq!(
        models.initializations, initialized,
        "Model sessions were initialized again"
    );
    println!("Verified {initialized} shared model sessions; {found_faces} accepted faces");
    // Metadata writes and moves use independent copies, never the originals.
    let copies = tempfile::tempdir()?;
    let output = tempfile::tempdir()?;
    let sample = catalog
        .media
        .iter()
        .find(|m| m.name() == "jpg_sample.jpg")
        .unwrap();
    let copy = copies.path().join("copy.jpg");
    fs::copy(&sample.path, &copy)?;
    let original = operations::content_hash(&copy, || false)?;
    metadata::write_sidecar(&copy, Some(5), Some(6))?;
    assert_eq!(metadata::read(&copy).rating, 5);
    assert_eq!(
        operations::content_hash(&copy, || false)?,
        original,
        "sidecar edit rewrote pixels"
    );
    let catalog = Catalog::scan(copies.path(), || false, |_| {})?;
    let media = &catalog.media[0];
    let plan = operations::organize_plan_with_names(
        &catalog,
        &HashMap::from([(media.id, "Verified/Keepers".into())]),
        &HashMap::from([(media.id, "Favorite".into())]),
        output.path(),
        true,
    )?;
    let journals = tempfile::tempdir()?;
    let report = operations::execute(&plan, journals.path(), || false, |_, _| {})?;
    assert!(report.errors.is_empty());
    assert_eq!(report.completed.len(), 1);
    assert!(
        output
            .path()
            .join("Verified/Keepers/Favorite.jpg")
            .is_file()
    );
    assert!(
        output
            .path()
            .join("Verified/Keepers/Favorite.xmp")
            .is_file()
    );
    for (path, hash) in original_hashes {
        assert_eq!(
            operations::content_hash(&path, || false)?,
            hash,
            "Original sample changed"
        );
    }
    println!(
        "All sample originals unchanged; initialized models reused; copied metadata/move operations verified."
    );
    Ok(())
}

#[test]
#[ignore = "desktop-only: places a disposable verification file in OS Trash"]
fn native_trash_moves_only_the_disposable_test_file() -> anyhow::Result<()> {
    let temp = tempfile::tempdir()?;
    let path = temp
        .path()
        .join("PhotoSort-disposable-trash-verification.txt");
    fs::write(
        &path,
        b"Disposable PhotoSort test artifact; safe to discard.",
    )?;
    let plan = operations::Plan {
        actions: vec![operations::Action::Trash {
            files: vec![operations::Source::capture(path.clone())?],
        }],
        description: "Disposable Trash smoke test".into(),
        ..Default::default()
    };
    let journal = tempfile::tempdir()?;
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {})?;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.completed.len(), 1);
    assert!(!path.exists());
    Ok(())
}

#[test]
#[ignore = "requires installed native orientation model"]
fn rotation_worker_cancels_live_inference_and_serves_review_without_redecode() -> anyhow::Result<()>
{
    use photosort::core::jobs::{Event, ImageWork, Runtime};
    use std::time::Duration;
    let root = tempfile::tempdir()?;
    let path = root.path().join("cancel-test.png");
    image::RgbImage::from_fn(96, 64, |x, y| {
        image::Rgb([(x * 2) as u8, (y * 3) as u8, 100])
    })
    .save(&path)?;
    let catalog = Catalog::scan(root.path(), || false, |_| {})?;
    let media = catalog.media[0].clone();
    let mut runtime = Runtime::new(|| {});
    let model = photosort::core::models::orientation_model().expect("Install orientation model");
    let run = runtime
        .orient(Arc::new(vec![media.clone(); 400]), model)
        .map_err(anyhow::Error::msg)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        anyhow::ensure!(
            Instant::now() < deadline,
            "Orientation worker never started"
        );
        match runtime.events.recv_timeout(Duration::from_secs(1)) {
            Ok(e) => match e.event {
                Event::Rotation { run: found, .. } if found == run => break,
                Event::AnalysisError { error, .. } | Event::Error(error) => anyhow::bail!(error),
                _ => {}
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => return Err(e.into()),
        }
    }
    runtime.cancel_analysis();
    let cancelled = Instant::now();
    runtime
        .image(ImageWork::Asset(media, true))
        .map_err(anyhow::Error::msg)?;
    let mut late = 0;
    let preview_elapsed = loop {
        anyhow::ensure!(
            cancelled.elapsed() < Duration::from_secs(5),
            "Preview blocked after cancelling inference"
        );
        let event = runtime.events.recv_timeout(Duration::from_secs(5))?;
        match event.event {
            Event::Asset { stats, .. } => {
                assert_eq!(
                    stats.decodes, 1,
                    "Cancelled analysis and review must reuse the canonical source"
                );
                break cancelled.elapsed();
            }
            Event::Rotation { run: found, .. } if found == run => late += 1,
            Event::AnalysisDone { run: found, .. } if found == run => {
                panic!("Analysis completed all 400 items instead of cancelling")
            }
            Event::AnalysisError { error, .. }
            | Event::AssetError { error, .. }
            | Event::Error(error) => anyhow::bail!(error),
            _ => {}
        }
    };
    while let Ok(event) = runtime.events.recv_timeout(Duration::from_millis(200)) {
        match event.event {
            Event::Rotation { run: found, .. } if found == run => late += 1,
            Event::AnalysisDone { run: found, .. } if found == run => {
                panic!("Cancelled run completed")
            }
            _ => {}
        }
    }
    assert!(
        late <= 9,
        "Cancelled inference continued beyond bounded in-flight results"
    );
    println!(
        "Live inference cancelled; review delivered in {:?}; {late} bounded late results; source decoded once",
        preview_elapsed
    );
    runtime.shutdown();
    Ok(())
}

#[test]
#[ignore = "requires reference samples and installed native quality models"]
fn quality_worker_reuses_models_measurements_and_decode_across_workflows() -> anyhow::Result<()> {
    use photosort::core::jobs::{AnalysisProfile, AnalysisStats, Event, Runtime};
    use std::time::Duration;
    let samples =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let root = tempfile::tempdir()?;
    for name in ["a.jpg", "b.jpg"] {
        fs::copy(samples.join("jpg_sample.jpg"), root.path().join(name))?;
    }
    let catalog = Catalog::scan(root.path(), || false, |_| {})?;
    let cache = tempfile::tempdir()?;
    let mut runtime = Runtime::with_cache(|| {}, cache.path().into());
    let mut snapshots: Vec<AnalysisStats> = Vec::new();
    for profile in [
        AnalysisProfile::Technical,
        AnalysisProfile::Quality,
        AnalysisProfile::Quality,
        AnalysisProfile::Subjects,
        AnalysisProfile::Mixed,
        AnalysisProfile::Visual,
        AnalysisProfile::Duplicates,
        AnalysisProfile::Mixed,
    ] {
        let run = runtime
            .analyze(Arc::new(catalog.media.clone()), profile)
            .map_err(anyhow::Error::msg)?;
        let deadline = Instant::now() + Duration::from_secs(60);
        let mut results = 0;
        let mut mixed_ready = false;
        let mut subjects_ready = false;
        loop {
            anyhow::ensure!(Instant::now() < deadline, "Analysis did not complete");
            match runtime.events.recv_timeout(Duration::from_secs(1)) {
                Ok(envelope) => match envelope.event {
                    Event::Analyzed {
                        run: found,
                        analysis,
                        embedding,
                        ..
                    } if found == run => {
                        assert!(analysis.quality_sharpness.is_some());
                        if profile != AnalysisProfile::Technical {
                            assert!(analysis.aesthetic.is_some());
                            assert!(embedding.is_some());
                        }
                        results += 1;
                    }
                    Event::SubjectGroups { run: found, groups } if found == run => {
                        assert_eq!(groups.len(), 2);
                        assert_eq!(
                            groups[&catalog.media[0].id], groups[&catalog.media[1].id],
                            "Identical copied photos must share a verified group"
                        );
                        subjects_ready = true;
                    }
                    Event::MixedGroups { run: found, labels } if found == run => {
                        assert_eq!(labels.len(), 2);
                        assert!(labels.values().all(|label| label == "2024-04-25/group-1"));
                        mixed_ready = true;
                    }
                    Event::AnalysisDone { run: found, stats } if found == run => {
                        assert_eq!(results, 2);
                        assert_eq!(mixed_ready, profile == AnalysisProfile::Mixed);
                        assert_eq!(
                            subjects_ready,
                            matches!(
                                profile,
                                AnalysisProfile::Quality | AnalysisProfile::Subjects
                            )
                        );
                        snapshots.push(stats);
                        break;
                    }
                    Event::AnalysisError { error, .. } | Event::Error(error) => {
                        anyhow::bail!(error)
                    }
                    _ => {}
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    let technical = snapshots.remove(0);
    assert_eq!(technical.decodes, 2);
    assert_eq!(technical.technical_measurements, 2);
    assert_eq!(technical.quality_measurements, 2);
    assert_eq!(
        (technical.model_initializations, technical.model_inferences),
        (0, 0)
    );
    assert_eq!(
        snapshots[0].technical_measurements,
        technical.technical_measurements
    );
    assert_eq!(
        snapshots[0].quality_measurements,
        technical.quality_measurements
    );
    assert_eq!(snapshots[0].decodes, 2);
    assert_eq!(snapshots[0].quality_measurements, 2);
    assert!(snapshots[0].model_initializations >= 3);
    assert!(snapshots[0].model_inferences > 0);
    assert_eq!(
        snapshots[0], snapshots[1],
        "Repeated Pick Best repeated expensive work"
    );
    assert_eq!(
        snapshots[0], snapshots[2],
        "Similarity workflow repeated existing expensive work"
    );
    let mut regional = snapshots[2];
    regional.model_inferences += 2; // One bounded five-crop batch per photo, once.
    for snapshot in &snapshots[3..] {
        assert_eq!(
            *snapshot, regional,
            "General workflows repeated regional, global, face or quality work"
        );
    }
    println!(
        "Shared workflow reuse: subject {:?}, general {:?}",
        snapshots[0], regional
    );
    runtime.shutdown();
    let mut reopened = Runtime::with_cache(|| {}, cache.path().into());
    let run = reopened
        .analyze(Arc::new(catalog.media.clone()), AnalysisProfile::Subjects)
        .map_err(anyhow::Error::msg)?;
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut groups_ready = false;
    loop {
        anyhow::ensure!(
            Instant::now() < deadline,
            "Reopened subject analysis did not finish"
        );
        match reopened.events.recv_timeout(Duration::from_secs(1)) {
            Ok(envelope) => match envelope.event {
                Event::SubjectGroups { run: found, groups } if found == run => {
                    assert_eq!(groups.len(), 2);
                    groups_ready = true;
                }
                Event::AnalysisDone { run: found, stats } if found == run => {
                    assert!(groups_ready);
                    assert_eq!(stats.decodes, 0, "Restart repeated source decoding");
                    assert_eq!(
                        stats.model_initializations, 0,
                        "Restart loaded models despite cached complete evidence"
                    );
                    assert_eq!(stats.model_inferences, 0, "Restart repeated extraction");
                    println!(
                        "Restart reused complete subject evidence with zero decodes/model loads/inferences"
                    );
                    break;
                }
                Event::AnalysisError { error, .. } | Event::Error(error) => anyhow::bail!(error),
                _ => {}
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(error) => return Err(error.into()),
        }
    }
    reopened.shutdown();
    Ok(())
}

#[test]
#[ignore = "requires reference samples and native face models; run optimized"]
fn near_duplicate_comparator_reuses_shared_image_and_face_evidence() -> anyhow::Result<()> {
    use photosort::core::near_duplicates::{Comparator, Decision as PairDecision, Gray};
    let root =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    assert!(catalog.media.len() >= 7);
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let mut models = Models::default();
    let mut comparator = Comparator::default();
    for media in &catalog.media {
        let hash = operations::content_hash(&media.path, || false)?;
        let asset = pipeline.load(media)?;
        let decodes = pipeline.stats.decodes;
        // This verifies identity on actual decoded camera samples. It does not
        // claim RGB-to-gray or changed-photo equivalence to the Python pipeline.
        let gray: Vec<_> = asset
            .pixels
            .pixels()
            .map(|p| {
                (0.299 * f32::from(p[0]) + 0.587 * f32::from(p[1]) + 0.114 * f32::from(p[2]))
                    / 255.0
            })
            .collect();
        let gray = Gray::new(
            &gray,
            asset.pixels.width() as usize,
            asset.pixels.height() as usize,
        )
        .unwrap();
        let mut evidence = None;
        let mut loads = 0;
        let result = comparator.assess(
            Some(gray),
            Some(gray),
            false,
            |_| {
                if evidence.is_none() {
                    evidence = Some(models.faces(&asset).ok()?);
                    loads += 1;
                }
                Some(evidence.as_ref().unwrap().geometry.clone())
            },
            || false,
        );
        assert_eq!(
            result.decision,
            PairDecision::SafeNearDuplicate,
            "{}: {result:?}",
            media.name()
        );
        assert_eq!(loads, 1, "Same-source face evidence was extracted twice");
        let reused = pipeline.load(media)?;
        assert!(Arc::ptr_eq(&asset.pixels, &reused.pixels));
        assert_eq!(pipeline.stats.decodes, decodes);
        assert_eq!(operations::content_hash(&media.path, || false)?, hash);
        println!(
            "{}: stable image, one shared face extraction, no repeated decode",
            media.name()
        );
    }
    assert!(
        models.initializations <= 2,
        "Comparator initialized a competing model registry"
    );
    Ok(())
}

#[test]
#[ignore = "requires the seven reference samples and installed native models"]
fn duplicate_worker_shares_models_cache_and_previews_on_disposable_samples() -> anyhow::Result<()> {
    use photosort::core::{
        jobs::{AnalysisProfile, Event, Runtime},
        near_duplicates::Decision as PairDecision,
    };
    use std::{io::Write, time::Duration};
    let samples =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let originals = Catalog::scan(&samples, || false, |_| {})?.media;
    assert_eq!(originals.len(), 7);
    let original_hashes: Vec<_> = originals
        .iter()
        .map(|m| operations::content_hash(&m.path, || false))
        .collect::<anyhow::Result<_>>()?;
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    for (i, m) in originals.iter().enumerate() {
        for copy in 0..2 {
            fs::copy(
                &m.path,
                root.path().join(format!("{i}-{copy}-{}", m.name())),
            )?;
        }
    }
    // Different file bytes, identical decoded frame: forces the full near-pair
    // policy and lazy face requests instead of the verified hash shortcut.
    for suffix in [1u8, 2] {
        let path = root.path().join(format!("near-{suffix}.jpg"));
        fs::copy(samples.join("jpg_sample.jpg"), &path)?;
        fs::OpenOptions::new()
            .append(true)
            .open(path)?
            .write_all(&[suffix])?;
    }
    let media = Arc::new(Catalog::scan(root.path(), || false, |_| {})?.media);
    let mut runtime = Runtime::with_cache(|| {}, cache.path().into());
    let mut snapshots = Vec::new();
    for pass in 0..2 {
        let run = runtime
            .analyze(media.clone(), AnalysisProfile::Duplicates)
            .map_err(anyhow::Error::msg)?;
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut pairs = None;
        let mut preview = false;
        loop {
            anyhow::ensure!(Instant::now() < deadline, "Duplicate worker timed out");
            match runtime.events.recv_timeout(Duration::from_millis(500)) {
                Ok(envelope) => match envelope.event {
                    Event::DuplicateStarted { run: found } if found == run => runtime
                        .image(photosort::core::jobs::ImageWork::Asset(
                            media[0].clone(),
                            true,
                        ))
                        .map_err(anyhow::Error::msg)?,
                    Event::Asset { pixels, stats, .. } => {
                        assert_eq!(
                            pixels.width().max(pixels.height()),
                            2048,
                            "Review must receive a full display proxy, not the 1024 analysis input"
                        );
                        assert_eq!(
                            stats.decodes,
                            media.len() as u64 + 1,
                            "Only the requested display photo should need another decode after the bounded original cache was evicted"
                        );
                        preview = true;
                    }
                    Event::DuplicatePairs {
                        run: found,
                        pairs: found_pairs,
                        ..
                    } if found == run => pairs = Some(found_pairs),
                    Event::AnalysisDone { run: found, stats } if found == run => {
                        let pairs = pairs.expect("Completion preceded final pair publication");
                        assert_eq!(
                            pairs
                                .iter()
                                .filter(|p| p.assessment.decision == PairDecision::ExactDuplicate)
                                .count(),
                            7
                        );
                        assert_eq!(pairs.iter().filter(|p|p.assessment.decision==PairDecision::SafeNearDuplicate).count(),1);
                        if pass == 0 {
                            assert!(preview, "Preview did not complete during duplicate work");
                        }
                        eprintln!("Duplicate pass {pass}: {stats:?}");
                        snapshots.push(stats);
                        break;
                    }
                    Event::Error(error) | Event::AnalysisError { error, .. } => {
                        anyhow::bail!(error)
                    }
                    _ => {}
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => anyhow::bail!(error),
            }
        }
    }
    assert_eq!(
        snapshots[0], snapshots[1],
        "Repeated review redecoded or reinferred evidence"
    );
    // Analysis no longer generates display proxies for the whole library.
    // This late full-resolution preview follows eviction from the bounded
    // original cache. Its one demand-driven decode is reused on the warm pass.
    assert_eq!(snapshots[0].decodes, media.len() as u64 + 1);
    assert!(snapshots[0].model_initializations <= 3);
    // Cancel as the policy phase begins, then queue technical work. The image
    // actor must remain available; a waiting evidence reply must not deadlock it.
    let run = runtime
        .analyze(media.clone(), AnalysisProfile::Duplicates)
        .map_err(anyhow::Error::msg)?;
    loop {
        if matches!(runtime.events.recv_timeout(test_support::worker_patience())?.event,Event::DuplicateStarted{run:found} if found==run)
        {
            break;
        }
    }
    runtime.cancel_analysis();
    let run = runtime
        .analyze(media, AnalysisProfile::Technical)
        .map_err(anyhow::Error::msg)?;
    loop {
        if matches!(runtime.events.recv_timeout(test_support::worker_patience())?.event,Event::AnalysisDone{run:found,..} if found==run)
        {
            break;
        }
    }
    runtime.shutdown();
    for (m, hash) in originals.iter().zip(original_hashes) {
        assert_eq!(operations::content_hash(&m.path, || false)?, hash);
    }
    Ok(())
}

#[test]
#[ignore = "requires seven reference samples and the installed DINO model"]
fn general_regional_embeddings_reuse_whole_image_model_and_persistent_evidence()
-> anyhow::Result<()> {
    use photosort::core::{
        regional,
        subjects::store::{Features, Store},
    };
    let root =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let media = Catalog::scan(&root, || false, |_| {})?.media;
    assert_eq!(media.len(), 7);
    let before: Vec<_> = media
        .iter()
        .map(|m| operations::content_hash(&m.path, || false))
        .collect::<anyhow::Result<_>>()?;
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let mut models = Models::default();
    let model_signature = photosort::core::models::analysis_signature();
    let mut snapshots = Vec::new();
    for pass in 0..3 {
        // A reopened Store proves persistence rather than only an in-memory hit.
        let store = Store::new(cache.path().into(), true)?;
        let mut inputs = Vec::new();
        for item in &media {
            let asset = pipeline.load(item)?;
            let key = Store::key(&Pipeline::key(item)?, &model_signature);
            let dense = match store.get(&key) {
                Some(features) => features.dense,
                None => {
                    let dense = models.dense_embedding(&asset)?;
                    store.put(
                        &key,
                        &Features {
                            dense: dense.clone(),
                            faces: None,
                            identities: None,
                        },
                    )?;
                    dense
                }
            };
            let region_key = Store::regional_key(&key);
            let regions = match store.regional(&region_key) {
                Some(regions) => regions,
                None => {
                    let before = models.inferences;
                    let regions = models.similarity_regions(&asset, &dense.global, || false)?;
                    assert_eq!(
                        models.inferences - before,
                        1,
                        "Five crops must share one native batch; the whole image is reused"
                    );
                    store.put_regional(&region_key, &regions)?;
                    regions
                }
            };
            assert_eq!(regions.vectors[0], dense.global);
            assert_eq!(regions.vectors.len(), 6);
            inputs.push(regional::Input {
                key: region_key,
                portrait: regional::portrait(asset.pixels.width(), asset.pixels.height()),
            });
        }
        let result = regional::cluster(
            &inputs,
            regional::DEFAULT_EPS,
            |key| regional::Features::new(store.regional(key).unwrap().vectors),
            || false,
        )?
        .unwrap();
        assert_eq!(result.labels.len(), 7);
        snapshots.push((
            pipeline.stats.decodes,
            models.initializations,
            models.inferences,
        ));
        eprintln!(
            "Regional pass {pass}: {:?}, labels {:?}, clustering {:?}",
            snapshots.last().unwrap(),
            result.labels,
            result.stats
        );
    }
    assert_eq!(snapshots, vec![(7, 1, 14); 3]);
    for (item, hash) in media.iter().zip(before) {
        assert_eq!(operations::content_hash(&item.path, || false)?, hash);
    }
    Ok(())
}

/// A harness can print "test result: ok" before native C++ destructors abort.
/// Check real process termination as well as the worker assertions themselves.
#[test]
#[ignore = "requires seven reference samples and native models; runs three fresh processes"]
fn native_model_workers_exit_cleanly_in_fresh_processes() -> anyhow::Result<()> {
    use std::process::Command;
    let executable = std::env::current_exe()?;
    for pass in 0..3 {
        let output = Command::new(&executable)
            .args([
                "--exact",
                "duplicate_worker_shares_models_cache_and_previews_on_disposable_samples",
                "--ignored",
                "--nocapture",
            ])
            .env("ORT_DISABLE_TELEMETRY", "1")
            .output()?;
        anyhow::ensure!(
            output.status.success(),
            "Native shutdown pass {pass} failed ({}):\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
        anyhow::ensure!(
            String::from_utf8_lossy(&output.stdout).contains("1 passed; 0 failed"),
            "Native shutdown child did not execute the integration test",
        );
        eprintln!("Native model shutdown pass {pass}: clean process exit");
    }
    Ok(())
}

#[test]
#[ignore = "requires reference JPEG and installed DINO/orientation models"]
fn cpu_budget_changes_keep_evidence_and_reconfigure_new_model_work() -> anyhow::Result<()> {
    use photosort::core::{
        jobs::{AnalysisProfile, AnalysisStats, Event, Runtime},
        processing::{Mode, Preferences},
    };
    use std::time::Duration;
    fn done(rt: &Runtime, run: u64) -> anyhow::Result<(AnalysisStats, usize)> {
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut rotations = 0;
        loop {
            anyhow::ensure!(Instant::now() < deadline, "CPU budget analysis timed out");
            match rt.events.recv_timeout(Duration::from_millis(500)) {
                Ok(envelope) => match envelope.event {
                    Event::AnalysisDone { run: found, stats } if found == run => {
                        return Ok((stats, rotations));
                    }
                    Event::Rotation { run: found, .. } if found == run => rotations += 1,
                    Event::Error(e) | Event::AnalysisError { error: e, .. } => anyhow::bail!(e),
                    _ => {}
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => anyhow::bail!(e),
            }
        }
    }
    let sample =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"))
            .join("jpg_sample.jpg");
    let original = operations::content_hash(&sample, || false)?;
    let root = tempfile::tempdir()?;
    let cache = tempfile::tempdir()?;
    for name in ["a.jpg", "b.jpg"] {
        fs::copy(&sample, root.path().join(name))?;
    }
    let media = Arc::new(Catalog::scan(root.path(), || false, |_| {})?.media);
    let mut rt = Runtime::with_cache(|| {}, cache.path().into());
    rt.processing.configure(Preferences {
        mode: Mode::Custom,
        custom_threads: 1,
        ..Default::default()
    });
    let run = rt
        .analyze(media.clone(), AnalysisProfile::Visual)
        .map_err(anyhow::Error::msg)?;
    let (first, _) = done(&rt, run)?;
    assert_eq!(
        (
            first.model_threads,
            first.model_initializations,
            first.model_inferences
        ),
        (1, 1, 4)
    );
    let second_threads = rt.processing.available().min(2);
    rt.processing.configure(Preferences {
        mode: Mode::Custom,
        custom_threads: second_threads,
        ..Default::default()
    });
    let run = rt
        .analyze(media, AnalysisProfile::Visual)
        .map_err(anyhow::Error::msg)?;
    let (warm, _) = done(&rt, run)?;
    let mut expected = first;
    expected.model_threads = second_threads;
    assert_eq!(
        warm, expected,
        "Changing CPU allocation repeated cached analysis"
    );
    fs::copy(&sample, root.path().join("new.jpg"))?;
    let media = Arc::new(Catalog::scan(root.path(), || false, |_| {})?.media);
    let run = rt
        .analyze(media.clone(), AnalysisProfile::Visual)
        .map_err(anyhow::Error::msg)?;
    let (new, _) = done(&rt, run)?;
    assert_eq!(new.decodes, 3);
    assert_eq!(new.technical_measurements, 3);
    assert_eq!(new.quality_measurements, 3);
    assert_eq!(new.model_threads, second_threads);
    assert_eq!(
        new.model_initializations,
        if second_threads == 1 { 1 } else { 2 }
    );
    assert_eq!(new.model_inferences, 6);
    rt.processing.configure(Preferences {
        mode: Mode::Custom,
        custom_threads: 1,
        ..Default::default()
    });
    let model = photosort::core::models::orientation_model().expect("Install orientation model");
    let run = rt.orient(media, model).map_err(anyhow::Error::msg)?;
    let (rotation, count) = done(&rt, run)?;
    assert_eq!(count, 3);
    assert_eq!(rotation.model_threads, 1);
    assert_eq!(rotation.decodes, new.decodes);
    assert_eq!(rotation.technical_measurements, new.technical_measurements);
    assert_eq!(rotation.quality_measurements, new.quality_measurements);
    assert_eq!(
        rotation.model_initializations,
        new.model_initializations + 1
    );
    assert_eq!(rotation.model_inferences, new.model_inferences + 3);
    eprintln!(
        "CPU allocation snapshots: first {first:?}; warm {warm:?}; new {new:?}; rotation {rotation:?}"
    );
    rt.shutdown();
    assert_eq!(operations::content_hash(&sample, || false)?, original);
    Ok(())
}

#[test]
#[ignore = "requires seven reference photographs and installed native orientation model"]
fn orientation_consistency_on_sample_quarter_turns_reuses_decodes_and_session() -> anyhow::Result<()>
{
    use photosort::core::{models::orientation_model, pipeline::Asset};
    let root =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    assert_eq!(catalog.media.len(), 7);
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let mut models = Models::default();
    let model = orientation_model().expect("Install orientation model");
    let detector = photosort::core::rotation::Detector::new(model.clone())?;
    let store = photosort::core::subjects::store::Store::new(cache.path().join("evidence"), true)?;
    let mut originals_inferences = 0;
    for media in &catalog.media {
        let hash = operations::content_hash(&media.path, || false)?;
        // Browsing and rotation share the bounded neutral working image. Exercise
        // the actual detector first so this cannot pass with a page-only recipe.
        let prepared = pipeline.load(media)?;
        let decodes = pipeline.stats.decodes;
        let reads = pipeline.stats.metadata_reads;
        let inferred = models.inferences;
        let detected = detector.detect(media, &mut pipeline, &mut models, &store, &|| false)?;
        assert_eq!(detected.turns, 0, "{}: {detected:?}", media.name());
        assert!(!detected.uncertain);
        assert_eq!(models.inferences - inferred, u64::from(detected.views));
        assert_eq!(
            pipeline.stats.decodes, decodes,
            "Rotation decoded a browsed source again"
        );
        assert_eq!(
            pipeline.stats.metadata_reads, reads,
            "Rotation reloaded browsed metadata"
        );
        for clockwise in 0..4 {
            let pixels = match clockwise {
                0 => prepared.pixels.clone(),
                1 => Arc::new(image::imageops::rotate90(&*prepared.pixels)),
                2 => Arc::new(image::imageops::rotate180(&*prepared.pixels)),
                _ => Arc::new(image::imageops::rotate270(&*prepared.pixels)),
            };
            let asset = Asset {
                decoder_warning: None,
                thumbnail: pixels.clone(),
                pixels,
                metadata: prepared.metadata.clone(),
            };
            let before = models.inferences;
            let result = if clockwise == 0 {
                detected
            } else {
                models.rotation(&model, &asset)?
            };
            assert_eq!(
                models.inferences - before,
                if clockwise == 0 {
                    0
                } else {
                    u64::from(result.views)
                }
            );
            assert_eq!(
                (clockwise + result.turns) % 4,
                0,
                "{} / {clockwise}: {result:?}",
                media.name()
            );
            assert!(!result.uncertain, "{} / {clockwise}", media.name());
            if clockwise == 0 {
                originals_inferences += result.views;
            }
            assert_eq!(
                pipeline.stats.decodes, decodes,
                "Cross-check repeated decoding"
            );
        }
        assert_eq!(operations::content_hash(&media.path, || false)?, hash);
    }
    assert_eq!(models.initializations, 1);
    assert_eq!(pipeline.stats.decodes, 7);
    eprintln!(
        "28 sample variants restored, {} inferences; original seven: {originals_inferences}; one session, seven decodes",
        models.inferences
    );
    Ok(())
}
