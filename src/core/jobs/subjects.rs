//! Same-subject grouping actor. It consumes evidence prepared by the image worker.
use super::{AnalysisStats, Event, Publisher, RunGeneration};
use crate::core::{cache, subjects};
use std::{
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver},
    },
    thread,
    time::Duration,
};

pub(super) struct SubjectJob {
    pub(super) epoch: u64,
    pub(super) run: u64,
    pub(super) inputs: Vec<subjects::grouping::Input>,
    pub(super) store: Arc<subjects::store::Store>,
    pub(super) strictness: subjects::Strictness,
    /// Pick Best: its Same subject groups too, from the same evidence.
    pub(super) same_subject: bool,
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
                // The image worker owns the sender; once it has stopped
                // there is no more work, and waiting would spin.
                let job = match jobs.recv_timeout(Duration::from_millis(100)) {
                    Ok(job) => job,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
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
    // The grouping reports its own progress: on a large library the neighbor
    // search, the pair checks and the merging of long bursts each take a while.
    // Pick Best groups twice, each pass half of the bar.
    let passes = if job.same_subject { 2 } else { 1 };
    let mut steps = super::ProgressSteps::default();
    let mut report = |pass: usize, done: usize, total: usize| {
        let (done, total) = (pass * total + done, passes * total);
        if steps.due(done, total) {
            publisher.send_checked(
                job.epoch,
                Event::Progress {
                    run: job.run,
                    done,
                    total,
                },
                cancelled,
            )
        }
    };
    let inputs = job.same_subject.then(|| job.inputs.clone());
    let grouped = subjects::grouping::group_with(
        job.inputs,
        &job.store,
        job.strictness,
        job.stats.model_threads,
        cancelled,
        |done, total| report(0, done, total),
    );
    // Verdicts the first pass judged are in the store: this pass reads
    // them again under its own rule, and judges only what it newly asks.
    let same_subject = match (&grouped, inputs) {
        (Ok(Some(_)), Some(inputs)) if !cancelled() => Some(
            subjects::grouping::group_with(
                inputs,
                &job.store,
                subjects::Rule::SameSubject(job.strictness),
                job.stats.model_threads,
                cancelled,
                |done, total| report(1, done, total),
            )
            .map_err(|error| format!("Same subject grouping failed: {error:#}")),
        ),
        _ => None,
    };
    match grouped {
        Ok(Some(groups)) if !cancelled() => {
            publisher.send_checked(
                job.epoch,
                Event::SubjectGroups {
                    run: job.run,
                    groups,
                },
                cancelled,
            );
            match same_subject {
                Some(Ok(Some(groups))) => publisher.send_checked(
                    job.epoch,
                    Event::SameSubjectGroups {
                        run: job.run,
                        groups: Ok(groups),
                    },
                    cancelled,
                ),
                Some(Err(error)) => publisher.send_checked(
                    job.epoch,
                    Event::SameSubjectGroups {
                        run: job.run,
                        groups: Err(error),
                    },
                    cancelled,
                ),
                // Cancelled, or not Pick Best.
                _ => {}
            }
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
