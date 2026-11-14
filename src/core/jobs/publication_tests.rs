use super::*;
use std::sync::Mutex;

fn publisher(tx: SyncSender<Envelope>, wake: impl Fn() + Send + Sync + 'static) -> Publisher {
    Publisher {
        tx,
        generation: Arc::new(AtomicU64::new(1)),
        stop: Arc::new(AtomicBool::new(false)),
        wake: Arc::new(wake),
    }
}

#[test]
fn immediate_ui_drain_observes_the_result_that_requested_its_wake() {
    let (tx, rx) = mpsc::sync_channel(1);
    let rx = Mutex::new(rx);
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let observed = delivered.clone();
    let publisher = publisher(tx, move || {
        // Deterministically exercise a UI that runs before the publisher returns.
        // No timer/poll loop should be needed to rescue the final result.
        while let Ok(envelope) = rx.lock().unwrap().try_recv() {
            if let Event::Error(message) = envelope.event {
                observed.lock().unwrap().push((envelope.epoch, message));
            } else {
                panic!("unexpected test event");
            }
        }
    });
    publisher.send(1, Event::Error("first".into()));
    publisher.send(1, Event::Error("last".into()));
    assert_eq!(
        *delivered.lock().unwrap(),
        vec![(1, "first".into()), (1, "last".into())]
    );
}

#[test]
fn full_queue_wakes_to_drain_then_wakes_again_after_publication() {
    let (tx, rx) = mpsc::sync_channel(1);
    tx.send(Envelope {
        epoch: 1,
        event: Event::Error("queued".into()),
    })
    .unwrap();
    let rx = Mutex::new(rx);
    let delivered = Arc::new(Mutex::new(Vec::new()));
    let observed = delivered.clone();
    let publisher = publisher(tx, move || {
        while let Ok(envelope) = rx.lock().unwrap().try_recv() {
            if let Event::Error(message) = envelope.event {
                observed.lock().unwrap().push(message);
            }
        }
    });
    publisher.send(1, Event::Error("pending".into()));
    assert_eq!(*delivered.lock().unwrap(), vec!["queued", "pending"]);
}

#[test]
fn cancellation_during_backpressure_does_not_publish_a_stale_result() {
    // Exercise both folder navigation and shutdown while a full queue is drained.
    for shutdown in [false, true] {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(Envelope {
            epoch: 1,
            event: Event::Error("queued".into()),
        })
        .unwrap();
        let generation = Arc::new(AtomicU64::new(1));
        let stop = Arc::new(AtomicBool::new(false));
        let next_generation = generation.clone();
        let next_stop = stop.clone();
        let wakes = Arc::new(AtomicU64::new(0));
        let count = wakes.clone();
        let publisher = Publisher {
            tx,
            generation,
            stop,
            wake: Arc::new(move || {
                count.fetch_add(1, Ordering::Relaxed);
                if shutdown {
                    next_stop.store(true, Ordering::Release);
                } else {
                    next_generation.store(2, Ordering::Release);
                }
            }),
        };
        publisher.send(1, Event::Error("cancelled".into()));
        assert_eq!(wakes.load(Ordering::Relaxed), 1);
        assert!(matches!(rx.try_recv().unwrap().event, Event::Error(s) if s == "queued"));
        assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    }
}

#[test]
fn storage_notices_follow_a_folder_change_while_waiting_for_queue_space() {
    for shutdown in [false, true] {
        let (tx, rx) = mpsc::sync_channel(1);
        tx.send(Envelope {
            epoch: 1,
            event: Event::Error("queued".into()),
        })
        .unwrap();
        let rx = Mutex::new(rx);
        let generation = Arc::new(AtomicU64::new(1));
        let stop = Arc::new(AtomicBool::new(false));
        let (next_generation, next_stop) = (generation.clone(), stop.clone());
        let delivered = Arc::new(Mutex::new(Vec::new()));
        let observed = delivered.clone();
        let publisher = Publisher {
            tx,
            generation,
            stop,
            wake: Arc::new(move || {
                // The UI opens another folder (or quits) while draining the queue.
                if shutdown {
                    next_stop.store(true, Ordering::Release);
                } else {
                    next_generation.store(2, Ordering::Release);
                }
                while let Ok(envelope) = rx.lock().unwrap().try_recv() {
                    if let Event::Error(message) | Event::ViewError(message) = envelope.event {
                        observed.lock().unwrap().push((envelope.epoch, message));
                    }
                }
            }),
        };
        publisher.send_current(Event::ViewError("could not be saved".into()));
        let mut expected = vec![(1, "queued".to_string())];
        if !shutdown {
            expected.push((2, "could not be saved".into()));
        }
        assert_eq!(*delivered.lock().unwrap(), expected, "shutdown: {shutdown}");
    }
}

#[test]
fn rejected_publication_does_not_request_an_empty_repaint() {
    let (tx, rx) = mpsc::sync_channel(1);
    let wakes = Arc::new(AtomicU64::new(0));
    let count = wakes.clone();
    let publisher = publisher(tx, move || {
        count.fetch_add(1, Ordering::Relaxed);
    });
    publisher.send(0, Event::Error("old folder".into()));
    publisher.send_checked(1, Event::Error("cancelled run".into()), || true);
    assert!(matches!(rx.try_recv(), Err(mpsc::TryRecvError::Empty)));
    drop(rx);
    publisher.send(1, Event::Error("disconnected".into()));
    assert_eq!(wakes.load(Ordering::Relaxed), 0);
}

#[test]
fn a_panicking_task_publishes_a_readable_error_and_the_worker_continues() {
    let (tx, rx) = mpsc::sync_channel(4);
    let publisher = publisher(tx, || {});
    assert!(!publisher.guard(Worker::Images, || panic!("decoder state went bad")));
    let envelope = rx.try_recv().unwrap();
    assert!(matches!(
        envelope.event,
        Event::WorkerStopped(Worker::Images)
    ));
    let mut ran = false;
    assert!(publisher.guard(Worker::Files, || ran = true));
    assert!(ran && rx.try_recv().is_err());
}

#[test]
fn policy_workers_stop_once_the_image_worker_is_gone() {
    let (tx, _events) = mpsc::sync_channel(1);
    let publisher = publisher(tx, || {});
    // The image worker owns both senders; after it stops they are dropped.
    let (_, duplicate_rx) = mpsc::sync_channel(1);
    let (evidence_tx, _evidence_rx) = mpsc::sync_channel(1);
    let (_, subject_rx) = mpsc::sync_channel(1);
    let workers = [
        similarity::spawn(
            publisher.clone(),
            duplicate_rx,
            evidence_tx,
            Default::default(),
            Default::default(),
        ),
        subjects::spawn(
            publisher.clone(),
            subject_rx,
            Default::default(),
            Default::default(),
        ),
    ];
    let (done, finished) = mpsc::channel();
    thread::spawn(move || {
        for worker in workers {
            worker.join().unwrap();
        }
        let _ = done.send(());
    });
    // Without the disconnect check both threads would spin until shutdown.
    let stopped = finished.recv_timeout(Duration::from_secs(5)).is_ok();
    publisher.stop.store(true, Ordering::Release);
    assert!(stopped);
}

/// During duplicate review the similarity worker asks the image worker for
/// evidence photo after photo. The idle worker must take each request up at
/// once, and answer eyes it judged before from the cache, without pixels.
#[test]
fn the_idle_image_worker_answers_evidence_at_once_and_cached_eyes_without_pixels() {
    use crate::core::{
        eyes,
        faces::Faces,
        model::*,
        models,
        near_duplicates::batch::Input,
        pipeline::Pipeline,
        subjects::{DenseEmbedding, store::Features, store::Store},
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("photo.png");
    std::fs::copy(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/near_duplicate/00.png"),
        &path,
    )
    .unwrap();
    let media = Media {
        id: MediaId(1),
        fingerprint: Fingerprint::read(&path).unwrap(),
        path,
        kind: MediaKind::Photo,
    };
    let source = Pipeline::key(&media).unwrap();
    let cache = root.path().join("cache");
    let store = Arc::new(Store::new(cache.clone(), true).unwrap());
    let key = Store::key(&source, "models");
    let features = Features {
        dense: DenseEmbedding::from_tokens(&vec![1.0; 257 * 384], 384).unwrap(),
        faces: Some(Faces::default()),
        identities: Some(vec![]),
    };
    store.put(&key, &features).unwrap();
    let eyes_key = Store::eyes_key(
        &source,
        &format!("{}:{}", eyes::RECIPE, models::face_signature()),
    );
    store.put_eyes(&eyes_key, &[]).unwrap();
    let input = Input {
        media,
        source,
        key,
        hash: String::new(),
        group: 1,
        embedding: Vec::new(),
        sharpness: None,
        metadata_fields: 0,
        shot: None,
        camera: String::new(),
        captured_ms: None,
        faces: 1,
    };

    let (tx, _events) = mpsc::sync_channel(64);
    let publisher = Publisher {
        generation: Arc::new(AtomicU64::new(0)),
        ..publisher(tx, || {})
    };
    let stop = publisher.stop.clone();
    let (_images, image_rx) = mpsc::sync_channel(1);
    let (_priority, priority_rx) = mpsc::sync_channel(1);
    let (evidence, evidence_rx) = mpsc::sync_channel(1);
    let (similarity_tx, _similarity_rx) = mpsc::sync_channel(1);
    let (subject_tx, _subject_rx) = mpsc::sync_channel(1);
    let wake = Wake::default();
    let worker = images::spawn(images::Wiring {
        publisher,
        previews: previews::Builder::new(cache.clone(), 0, Arc::default()).throttle(),
        cache: cache.clone(),
        cache_control: crate::core::cache::Control::with_directory(cache),
        processing: Default::default(),
        raw_look: Default::default(),
        inspection: Default::default(),
        analysis: Default::default(),
        metadata: Default::default(),
        images: image_rx,
        priority: priority_rx,
        evidence: evidence_rx,
        wake: wake.clone(),
        similarity: similarity_tx,
        subjects: subject_tx,
    });
    let ask = |people: bool| {
        let (reply, answer) = mpsc::sync_channel(1);
        evidence
            .send(similarity::EvidenceRequest {
                epoch: 0,
                run: 0,
                input: input.clone(),
                store: store.clone(),
                faces: true,
                people,
                reply,
            })
            .unwrap();
        wake.notify();
        answer
            .recv_timeout(crate::test_support::worker_patience())
            .unwrap()
            .unwrap()
    };
    const REQUESTS: u32 = 60;
    let started = std::time::Instant::now();
    for _ in 0..REQUESTS {
        let (asset, _, people, stats) = ask(true);
        assert!(asset.is_none() && people == Some(vec![]));
        assert_eq!(stats.decodes, 0, "cached eyes need no pixels");
    }
    // Without the wake each request waited for the worker's next idle poll.
    assert!(started.elapsed() < Duration::from_millis(15) * REQUESTS / 2);
    // Faces alone still come with the photo.
    let (asset, faces, people, _) = ask(false);
    assert!(asset.is_some() && faces.is_some_and(|f| f.is_empty()) && people.is_none());
    stop.store(true, Ordering::Release);
    worker.join().unwrap();
}
