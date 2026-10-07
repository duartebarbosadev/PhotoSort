use photosort::core::{
    analysis::{Analysis, Thresholds},
    catalog::Catalog,
    jobs::{AnalysisProfile, AnalysisStats, Event, Runtime},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
#[test]
fn cached_measurements_reclassify_with_dark_precedence_without_touching_quality_fields() {
    let mut value = Analysis {
        sharpness: 50.0,
        brightness: 10.0,
        black_fraction: 0.97,
        quality_sharpness: Some(111.0),
        aesthetic: Some(0.8),
        score: 80.0,
        ..Default::default()
    };
    value.reclassify(Thresholds::default());
    assert!(
        value.issues.is_empty(),
        "Night photo must remain recoverable"
    );
    value.black_fraction = 0.99;
    value.reclassify(Thresholds::default());
    assert_eq!(value.issues, vec!["Very dark"]);
    value.brightness = 200.0;
    value.reclassify(Thresholds::default());
    assert_eq!(value.issues, vec!["Possibly soft"]);
    value.reclassify(Thresholds {
        blur: 10.0,
        bright: 190.0,
        ..Default::default()
    });
    assert_eq!(value.issues, vec!["Overexposed"]);
    assert_eq!(value.quality_sharpness, Some(111.0));
    assert_eq!(value.aesthetic, Some(0.8));
    assert_eq!(value.score, 80.0);
}
#[test]
fn threshold_bounds_match_reference_and_reject_non_finite_values() {
    let mut t = Thresholds {
        blur: f64::NAN,
        dark: -1.0,
        bright: 1000.0,
    };
    t.normalize();
    assert_eq!(
        t,
        Thresholds {
            blur: 100.0,
            dark: 0.0,
            bright: 255.0
        }
    );
    t.blur = 1.0;
    t.dark = 200.0;
    t.bright = f64::INFINITY;
    t.normalize();
    assert_eq!(
        t,
        Thresholds {
            blur: 10.0,
            dark: 128.0,
            bright: 248.0
        }
    );
}
#[test]
fn repeated_worker_analysis_uses_new_thresholds_without_repeating_measurement_decode_or_models() {
    let dir = tempfile::tempdir().unwrap();
    image::RgbaImage::from_fn(640, 480, |x, y| {
        let v = if (x + y) % 2 == 0 { 127 } else { 129 };
        image::Rgba([v, v, v, 255])
    })
    .save(dir.path().join("pattern.png"))
    .unwrap();
    let catalog = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    let mut runtime = Runtime::with_cache(|| {}, dir.path().join("cache"));
    let mut stats = Vec::<AnalysisStats>::new();
    for (threshold, soft) in [(100.0, true), (10.0, false), (100.0, true)] {
        let mut settings = Thresholds {
            blur: threshold,
            ..Default::default()
        };
        let run = runtime
            .analyze_with_thresholds(
                Arc::new(catalog.media.clone()),
                AnalysisProfile::Technical,
                settings,
            )
            .unwrap();
        settings.blur = 1000.0; // Changing a future settings value cannot mutate the accepted run.
        assert_eq!(settings.blur, 1000.0);
        let deadline = Instant::now() + Duration::from_secs(15);
        let mut received = false;
        loop {
            assert!(Instant::now() < deadline, "Analysis did not complete");
            match runtime.events.recv_timeout(Duration::from_secs(1)) {
                Ok(envelope) => match envelope.event {
                    Event::Analyzed {
                        run: found,
                        analysis,
                        ..
                    } if found == run => {
                        assert!(
                            analysis.quality_sharpness.is_some(),
                            "Whole-image evidence was not cached alongside local evidence"
                        );
                        assert_eq!(
                            analysis.issues.iter().any(|s| s == "Possibly soft"),
                            soft,
                            "sharpness {}",
                            analysis.sharpness
                        );
                        received = true;
                    }
                    Event::AnalysisDone {
                        run: found,
                        stats: current,
                    } if found == run => {
                        assert!(received);
                        stats.push(current);
                        break;
                    }
                    Event::AnalysisError { error, .. } | Event::Error(error) => panic!("{error}"),
                    _ => {}
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => panic!("{e}"),
            }
        }
    }
    assert_eq!(stats[0].decodes, 1);
    assert_eq!(stats[0].technical_measurements, 1);
    assert_eq!(stats[0].quality_measurements, 1);
    assert_eq!(stats[0].model_initializations, 0);
    assert_eq!(stats[0].model_inferences, 0);
    assert_eq!(stats[0], stats[1]);
    assert_eq!(stats[1], stats[2]);
    runtime.shutdown();
}
