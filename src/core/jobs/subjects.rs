//! Same-subject grouping actor. It consumes evidence prepared by the image worker.
use super::{AnalysisStats, Event, Publisher, RunGeneration};
use crate::core::{cache, subjects};
use std::{
    sync::{Arc, atomic::Ordering, mpsc::Receiver},
    thread,
    time::Duration,
};

pub(super) struct SubjectJob {
    pub(super) epoch: u64,
    pub(super) run: u64,
    pub(super) inputs: Vec<subjects::grouping::Input>,
    pub(super) store: Arc<subjects::store::Store>,
    pub(super) strictness: subjects::Strictness,
    pub(super) stats: AnalysisStats,
}

pub(super) fn spawn(
    publisher: Publisher,
    jobs: Receiver<SubjectJob>,
    generation: Arc<RunGeneration>,
    cache: cache::Control,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("photosort-subjects".into())
        .spawn(move || {
            while !publisher.stop.load(Ordering::Relaxed) {
                let Ok(job) = jobs.recv_timeout(Duration::from_millis(100)) else {
                    continue;
                };
                let _ = publisher.guard(super::Worker::Subjects, || {
                    group(&publisher, &generation, &cache, job)
                });
            }
        })
        .expect("Start subject policy worker")
}

fn group(
    publisher: &Publisher,
    generation: &RunGeneration,
    cache: &cache::Control,
    job: SubjectJob,
) {
    let cancelled = || publisher.cancelled(job.epoch) || generation.current() != job.run;
    if cancelled() {
        return;
    }
    match subjects::grouping::group(job.inputs, &job.store, job.strictness, cancelled) {
        Ok(Some(groups)) if !cancelled() => {
            publisher.send_checked(
                job.epoch,
                Event::SubjectGroups {
                    run: job.run,
                    groups,
                },
                cancelled,
            );
            cache.request(false);
            publisher.send_checked(
                job.epoch,
                Event::AnalysisDone {
                    run: job.run,
                    stats: job.stats,
                },
                cancelled,
            );
        }
        Err(error) if !cancelled() => publisher.send_checked(
            job.epoch,
            Event::RunError {
                run: job.run,
                error: format!("Same-subject grouping failed: {error:#}"),
            },
            cancelled,
        ),
        _ => {}
    }
}
