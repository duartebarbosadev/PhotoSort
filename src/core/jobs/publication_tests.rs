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
