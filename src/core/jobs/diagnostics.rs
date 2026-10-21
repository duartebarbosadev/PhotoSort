//! Observe shared worker outcomes without copying image data or doing more work.
use super::Event;

impl Event {
    pub(super) fn record_diagnostic(&self, epoch: u64) {
        match self {
            Self::Opened(catalog, ..) => {
                tracing::info!(
                    epoch,
                    photos = catalog.media.len(),
                    files = catalog.inventory.len(),
                    warnings = catalog.warnings.len(),
                    "folder discovery finished"
                );
                if !catalog.warnings.is_empty() {
                    tracing::debug!(epoch, warnings = ?catalog.warnings, "folder discovery warnings");
                }
            }
            Self::AnalysisDone { run, stats } => {
                tracing::info!(epoch, run, ?stats, "analysis finished");
            }
            Self::ModelsNeeded { run, issue } => {
                tracing::warn!(epoch, run, ?issue, "analysis needs model setup");
            }
            Self::Planned(plan) => {
                tracing::info!(epoch, actions = plan.actions.len(), "file plan prepared");
            }
            Self::PlanningCancelled => tracing::info!(epoch, "file planning cancelled"),
            Self::Applied(report) => {
                tracing::info!(epoch, completed = report.completed.len(), remaining = report.remaining.actions.len(),
                    cancelled = report.cancelled, errors = ?report.errors, "file plan finished");
            }
            Self::Error(error)
            | Self::ApplyFailed(error)
            | Self::ViewError(error)
            | Self::PreferencesError(error)
            | Self::Checkpointed(Err(error))
            | Self::OperationsAmended(Err(error)) => {
                tracing::error!(epoch, %error, "worker failed");
            }
            Self::AnalysisError { run, id, error } => {
                tracing::warn!(epoch, run, ?id, %error, "photo analysis failed");
            }
            Self::RunError { run, error } => {
                tracing::error!(epoch, run, %error, "analysis run failed");
            }
            Self::AssetError { id, preview, error } => {
                tracing::warn!(epoch, ?id, preview, %error, "image request failed");
            }
            // No per-frame, per-pixel, per-photo success, or retry events.
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use std::io::Write;

    #[derive(Clone, Default)]
    struct Capture(Arc<Mutex<Vec<u8>>>);
    impl Write for Capture {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    #[test]
    fn shared_outcomes_log_once_even_when_publication_retries() {
        // Parallel tests emit shared callsites outside this scoped subscriber.
        // Register that second context too: tracing's single-dispatch fast path
        // otherwise caches its disabled interest for our capture. Production
        // installs one global subscriber before starting any workers.
        let _outside = tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default());
        let capture = Capture::default();
        let writer = capture.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_writer(move || writer.clone())
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let (tx, rx) = mpsc::sync_channel(1);
            tx.send(Envelope {
                epoch: 1,
                event: Event::Saved,
            })
            .unwrap();
            let rx = Mutex::new(rx);
            let publisher = Publisher {
                tx,
                generation: Arc::new(AtomicU64::new(1)),
                stop: Arc::new(AtomicBool::new(false)),
                wake: Arc::new(move || {
                    let _ = rx.lock().unwrap().try_recv();
                }),
            };
            publisher.send(
                1,
                Event::AnalysisDone {
                    run: 42,
                    stats: AnalysisStats::default(),
                },
            );
            publisher.send(1, Event::Planned(Plan::default()));
            publisher.send(1, Event::Applied(Report::default()));
            publisher.send(1, Event::Error("fixture failure".into()));
            publisher.send(
                1,
                Event::Progress {
                    run: 42,
                    done: 1,
                    total: 1,
                },
            );
        });
        let bytes = capture.0.lock().unwrap();
        let text = std::str::from_utf8(&bytes).unwrap();
        for message in [
            "analysis finished",
            "file plan prepared",
            "file plan finished",
            "worker failed",
        ] {
            assert_eq!(text.matches(message).count(), 1, "{text}");
        }
        assert_eq!(text.lines().count(), 4);
        assert!(text.contains("run=42") && text.contains("fixture failure"));
    }
}
