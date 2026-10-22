use super::*;
use crate::test_support::worker_patience;

#[derive(Clone, Copy, Debug)]
enum Request {
    Analysis,
    Rotation,
    Metadata,
}
impl Request {
    fn submit(self, runtime: &Runtime) -> Result<u64, String> {
        let media = Arc::new(Vec::new());
        match self {
            Self::Analysis => runtime.analyze(media, AnalysisProfile::Technical),
            Self::Rotation => runtime.orient(media, PathBuf::from("/missing/rotation.onnx")),
            Self::Metadata => runtime.index_metadata(media),
        }
    }
    fn active(self, runtime: &Runtime) -> u64 {
        match self {
            Self::Analysis | Self::Rotation => runtime.analysis.current(),
            Self::Metadata => runtime.metadata_generation.current(),
        }
    }
}

#[test]
fn rejected_requests_preserve_accepted_analysis_rotation_and_metadata() {
    for request in [Request::Analysis, Request::Rotation, Request::Metadata] {
        let cache = tempfile::tempdir().unwrap();
        let mut runtime = Runtime::with_cache(|| {}, cache.path().join("cache"));
        // Freeze admission at a real bounded channel; the image owner stays idle.
        let (queue, rx) = mpsc::sync_channel(1);
        let worker_queue = std::mem::replace(&mut runtime.images, queue);
        let accepted = request.submit(&runtime).unwrap();
        let replacement = match request {
            Request::Analysis => Request::Rotation,
            Request::Rotation => Request::Analysis,
            Request::Metadata => Request::Metadata,
        };
        assert!(
            replacement.submit(&runtime).is_err(),
            "full {request:?} queue"
        );
        assert_eq!(
            request.active(&runtime),
            accepted,
            "full {request:?} queue cancelled accepted work"
        );
        let message = rx.recv().unwrap();
        drop(rx);
        assert!(
            request.submit(&runtime).is_err(),
            "disconnected {request:?} queue"
        );
        assert_eq!(
            request.active(&runtime),
            accepted,
            "disconnected {request:?} queue cancelled accepted work"
        );
        // The previously accepted request still reaches the actual image owner.
        assert!(worker_queue.try_send(message).is_ok());
        let event = runtime
            .events
            .recv_timeout(worker_patience())
            .unwrap()
            .event;
        match (request, event) {
            (Request::Analysis, Event::AnalysisDone { run, stats }) => {
                assert_eq!(run, accepted);
                assert_eq!(stats.decodes, 0);
                assert_eq!(stats.model_initializations, 0);
                assert_eq!(stats.model_inferences, 0);
            }
            (Request::Rotation, Event::ModelsNeeded { run, .. })
            | (Request::Metadata, Event::MetadataBatch { run, .. }) => assert_eq!(run, accepted),
            _ => panic!("accepted {request:?} did not finish normally"),
        }
    }
}

#[test]
fn receiver_waits_for_admission_commit_without_cancelling_running_work_early() {
    let generation = Arc::new(RunGeneration::default());
    let (tx, rx) = mpsc::sync_channel(1);
    let (received_tx, received_rx) = mpsc::sync_channel(0);
    let worker_generation = generation.clone();
    let worker = thread::spawn(move || {
        let run = rx.recv_timeout(worker_patience()).unwrap();
        // Receipt really happened inside submit, before it published the ID.
        assert_eq!(worker_generation.current(), 0);
        assert!(matches!(
            worker_generation.admission.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ));
        received_tx.send(()).unwrap();
        assert!(worker_generation.accepts(run));
    });
    let run = generation
        .submit(|run| {
            tx.send(run).unwrap();
            received_rx.recv_timeout(worker_patience()).unwrap();
            assert_eq!(generation.current(), 0);
            Ok(())
        })
        .unwrap();
    worker.join().unwrap();
    assert_eq!(generation.current(), run);
}

#[test]
fn cancellation_and_supersession_keep_analysis_and_metadata_independent() {
    let cache = tempfile::tempdir().unwrap();
    let mut runtime = Runtime::with_cache(|| {}, cache.path().join("cache"));
    let (queue, _rx) = mpsc::sync_channel(8);
    runtime.images = queue;
    let analysis = Request::Analysis.submit(&runtime).unwrap();
    let metadata = Request::Metadata.submit(&runtime).unwrap();
    let rotation = Request::Rotation.submit(&runtime).unwrap();
    assert!(!runtime.analysis.accepts(analysis));
    assert!(runtime.analysis.accepts(rotation));
    assert!(runtime.metadata_generation.accepts(metadata));
    runtime.cancel_analysis();
    assert!(!runtime.analysis.accepts(rotation));
    assert!(runtime.metadata_generation.accepts(metadata));
    let replacement = Request::Analysis.submit(&runtime).unwrap();
    runtime.cancel_metadata();
    assert!(!runtime.metadata_generation.accepts(metadata));
    assert!(runtime.analysis.accepts(replacement));
    runtime.next_epoch();
    assert!(!runtime.analysis.accepts(replacement));
}

#[test]
fn cancellation_serializes_with_an_admission_in_progress() {
    let generation = Arc::new(RunGeneration::default());
    let (started_tx, started_rx) = mpsc::sync_channel(0);
    let (cancel_tx, cancel_rx) = mpsc::sync_channel(0);
    let cancel_generation = generation.clone();
    let cancel = thread::spawn(move || {
        cancel_rx.recv_timeout(worker_patience()).unwrap();
        assert!(matches!(
            cancel_generation.admission.try_lock(),
            Err(std::sync::TryLockError::WouldBlock)
        ));
        started_tx.send(()).unwrap();
        cancel_generation.cancel();
    });
    let accepted = generation
        .submit(|_| {
            cancel_tx.send(()).unwrap();
            started_rx.recv_timeout(worker_patience()).unwrap();
            Ok(())
        })
        .unwrap();
    cancel.join().unwrap();
    assert!(!generation.accepts(accepted));
    let next = generation.submit(|_| Ok(())).unwrap();
    assert!(generation.accepts(next));
}

#[test]
fn obsolete_requests_cannot_replace_current_work_or_start_image_and_model_setup() {
    for explicit_cancel in [false, true] {
        let cache = tempfile::tempdir().unwrap();
        let path = cache.path().join("photo.png");
        image::RgbaImage::new(4, 4).save(&path).unwrap();
        let media = Arc::new(vec![Media {
            id: MediaId::for_path(&path),
            fingerprint: Fingerprint::read(&path).unwrap(),
            path,
            kind: MediaKind::Photo,
        }]);
        let mut runtime = Runtime::with_cache(|| {}, cache.path().join("cache"));
        let (queue, rx) = mpsc::sync_channel(8);
        let worker_queue = std::mem::replace(&mut runtime.images, queue);
        runtime
            .analyze(media.clone(), AnalysisProfile::Faces)
            .unwrap();
        runtime
            .orient(media.clone(), PathBuf::from("/missing/obsolete-model.onnx"))
            .unwrap();
        runtime.index_metadata(media).unwrap();
        let obsolete: Vec<_> = rx.try_iter().collect();
        assert_eq!(obsolete.len(), 3);
        if explicit_cancel {
            runtime.cancel_analysis();
            runtime.cancel_metadata();
        }
        let analysis = Request::Analysis.submit(&runtime).unwrap();
        let metadata = Request::Metadata.submit(&runtime).unwrap();
        // Hold receipt until the whole sequence is queued. Reordered obsolete
        // requests must not replace pending current work, initialize models, read
        // metadata, decode pixels or report a missing model for an expired run.
        let analysis_gate = runtime.analysis.admission.lock().unwrap();
        let metadata_gate = runtime.metadata_generation.admission.lock().unwrap();
        for message in rx.try_iter().chain(obsolete) {
            assert!(worker_queue.try_send(message).is_ok());
        }
        drop(metadata_gate);
        drop(analysis_gate);
        let mut analysis_done = false;
        let mut metadata_done = false;
        for _ in 0..2 {
            match runtime
                .events
                .recv_timeout(worker_patience())
                .unwrap()
                .event
            {
                Event::AnalysisDone { run, stats } => {
                    assert_eq!(run, analysis);
                    assert!(!analysis_done);
                    analysis_done = true;
                    assert_eq!(stats.decodes, 0);
                    assert_eq!(stats.model_initializations, 0);
                    assert_eq!(stats.model_inferences, 0);
                }
                Event::MetadataBatch { run, stats, .. } => {
                    assert_eq!(run, metadata);
                    assert!(!metadata_done);
                    metadata_done = true;
                    assert_eq!(stats.metadata_reads, 0);
                    assert_eq!(stats.decodes, 0);
                }
                _ => panic!("obsolete request published a result or setup failure"),
            }
        }
        assert!(analysis_done && metadata_done);
        assert!(runtime.events.try_recv().is_err());
    }
}
