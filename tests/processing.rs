use photosort::core::{
    models::Models,
    processing::{self, Control, Mode, Preferences},
};
#[test]
fn allocation_modes_follow_available_processors_and_reference_rounding() {
    for available in [0, 1, 2, 4, 6, 8, 12, 16, 64, 128] {
        let available = available.max(1);
        assert_eq!(
            Preferences::default().threads(available),
            ((available as f64 * 0.85) as usize).max(1)
        );
        assert_eq!(
            Preferences {
                mode: Mode::Maximum,
                ..Default::default()
            }
            .threads(available),
            available
        );
        for requested in [0, 1, 3, 128, usize::MAX] {
            let mut p = Preferences {
                mode: Mode::Custom,
                custom_threads: requested,
                ..Default::default()
            };
            p.normalize(available);
            assert_eq!(p.threads(available), requested.clamp(1, available));
        }
    }
}
#[test]
fn existing_preferences_gain_balanced_mode_and_custom_choice_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preferences.json");
    std::fs::write(&path, b"[1,{}]").unwrap();
    let mut p = photosort::core::preferences::Preferences::load(&path).unwrap();
    assert_eq!(p.processing.mode, Mode::Balanced);
    p.processing = Preferences {
        mode: Mode::Custom,
        custom_threads: processing::available(),
        ..Default::default()
    };
    p.save(&path).unwrap();
    assert_eq!(
        photosort::core::preferences::Preferences::load(&path)
            .unwrap()
            .processing,
        p.processing
    );
}
#[test]
fn configuration_coalesces_without_workers_or_eager_model_loading() {
    let control = Control::default();
    let clone = control.clone();
    let before = control.snapshot();
    for threads in 0..10000 {
        control.configure(Preferences {
            mode: Mode::Custom,
            custom_threads: threads,
            ..Default::default()
        });
    }
    assert_eq!(clone.snapshot(), 9999usize.clamp(1, control.available()));
    assert!(before >= 1);
    let mut models = Models::default();
    models.configure_threads(1);
    assert_eq!(models.cpu_threads(), 1);
    assert!(!models.configure_threads(1));
    assert_eq!((models.initializations, models.inferences), (0, 0));
}
#[test]
fn accepted_jobs_keep_their_thread_snapshot_and_reuse_measurements_after_a_change() {
    use photosort::core::{
        catalog::Catalog,
        jobs::{AnalysisProfile, Event, Runtime},
    };
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let root = tempfile::tempdir().unwrap();
    image::RgbaImage::from_pixel(80, 64, image::Rgba([100, 130, 160, 255]))
        .save(root.path().join("image.png"))
        .unwrap();
    let media = Arc::new(Catalog::scan(root.path(), || false, |_| {}).unwrap().media);
    let mut rt = Runtime::with_cache(|| {}, root.path().join("cache"));
    let available = rt.processing.available();
    rt.processing.configure(Preferences {
        mode: Mode::Custom,
        custom_threads: 1,
        ..Default::default()
    });
    let mut stats = Vec::new();
    for expected in [1, available] {
        let run = rt
            .analyze(media.clone(), AnalysisProfile::Technical)
            .unwrap();
        // This happens after enqueue, before the worker is required to have read it.
        rt.processing.configure(Preferences {
            mode: Mode::Maximum,
            ..Default::default()
        });
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline, "Worker timed out");
            match rt.events.recv_timeout(Duration::from_millis(200)) {
                Ok(envelope) => match envelope.event {
                    Event::AnalysisDone {
                        run: found,
                        stats: result,
                    } if found == run => {
                        assert_eq!(result.model_threads, expected);
                        stats.push(result);
                        break;
                    }
                    Event::Error(e) | Event::AnalysisError { error: e, .. } => panic!("{e}"),
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
    stats[0].model_threads = stats[1].model_threads;
    assert_eq!(
        stats[0], stats[1],
        "Budget changes must not discard pixels or measurements"
    );
    rt.shutdown();
}

#[test]
fn backend_and_thread_snapshots_are_coherent_and_changes_remain_lazy() {
    use processing::Backend;
    let control = Control::default();
    assert_eq!(control.snapshot_run().backend, Backend::Cpu);
    control.configure(Preferences {
        backend: Backend::CoreMl,
        mode: Mode::Custom,
        custom_threads: 1,
    });
    let accepted = control.snapshot_run();
    control.configure(Preferences {
        backend: Backend::Cuda,
        mode: Mode::Maximum,
        ..Default::default()
    });
    assert_eq!(
        accepted,
        processing::RunConfig {
            threads: 1,
            backend: Backend::CoreMl
        }
    );
    assert_eq!(control.snapshot_run().backend, Backend::Cuda);
    let mut models = Models::with_processing(control.clone());
    assert!(models.configure(accepted));
    assert!(!models.configure(accepted));
    assert!(control.status().is_empty());
    assert_eq!((models.initializations, models.inferences), (0, 0));
}
#[test]
fn backend_choice_persists_and_old_preferences_preserve_cpu_execution() {
    use processing::Backend;
    let old: Preferences = serde_json::from_str(r#"{"mode":"Custom","custom_threads":1}"#).unwrap();
    assert_eq!(old.backend, Backend::Cpu);
    for backend in Backend::ALL {
        let preferences = Preferences {
            backend,
            ..Default::default()
        };
        let saved = serde_json::to_string(&preferences).unwrap();
        assert_eq!(
            serde_json::from_str::<Preferences>(&saved).unwrap(),
            preferences
        );
    }
}
