//! Similarity actor: groups prepared evidence and verifies near-duplicate candidates.
//! Pixels and faces for duplicate review come from the image worker on request.
use super::{AnalysisProfile, AnalysisStats, Event, Publisher, RunGeneration};
use crate::core::{
    cache, eyes, face_groups, faces, manual_groups, model::*, near_duplicates, pipeline::Pipeline,
    regional, subjects,
};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::Ordering,
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread,
    time::Duration,
};

pub(super) struct SimilarityJob {
    pub(super) scene_groups: Arc<manual_groups::ManualGroups>,
    pub(super) similarity_distance: f32,
    pub(super) regional: Vec<regional::grouping::Record>,
    pub(super) sources: Vec<(Media, String)>,
    pub(super) profile: AnalysisProfile,
    pub(super) epoch: u64,
    pub(super) run: u64,
    pub(super) inputs: Vec<near_duplicates::batch::Input>,
    pub(super) store: Arc<subjects::store::Store>,
    pub(super) stats: AnalysisStats,
}
pub(super) type EvidenceReply = (
    crate::core::pipeline::Asset,
    Option<Vec<faces::Face>>,
    Option<Vec<eyes::Person>>,
    AnalysisStats,
);
/// One photo's pixels, faces and people, as a request asked for them.
type Evidence = (
    crate::core::pipeline::Asset,
    Option<Vec<faces::Face>>,
    Option<Vec<eyes::Person>>,
);
pub(super) struct EvidenceRequest {
    pub(super) epoch: u64,
    pub(super) run: u64,
    pub(super) input: near_duplicates::batch::Input,
    pub(super) store: Arc<subjects::store::Store>,
    pub(super) faces: bool,
    /// Each person's eyes too, judged for Easy Delete.
    pub(super) people: bool,
    pub(super) reply: SyncSender<anyhow::Result<EvidenceReply>>,
}
struct EvidenceClient<'a> {
    job: &'a SimilarityJob,
    requests: &'a SyncSender<EvidenceRequest>,
    cancelled: &'a dyn Fn() -> bool,
    stats: AnalysisStats,
}
impl EvidenceClient<'_> {
    fn request(
        &mut self,
        input: &near_duplicates::batch::Input,
        faces: bool,
        people: bool,
    ) -> anyhow::Result<Evidence> {
        let (reply, rx) = mpsc::sync_channel(1);
        let mut request = EvidenceRequest {
            epoch: self.job.epoch,
            run: self.job.run,
            input: input.clone(),
            store: self.job.store.clone(),
            faces,
            people,
            reply,
        };
        loop {
            anyhow::ensure!(!(self.cancelled)(), "Duplicate review cancelled");
            match self.requests.try_send(request) {
                Ok(()) => break,
                Err(TrySendError::Full(returned)) => {
                    request = returned;
                    thread::sleep(Duration::from_millis(2));
                }
                Err(TrySendError::Disconnected(_)) => anyhow::bail!("Image worker stopped"),
            }
        }
        loop {
            anyhow::ensure!(!(self.cancelled)(), "Duplicate review cancelled");
            match rx.recv_timeout(Duration::from_millis(15)) {
                Ok(result) => {
                    let (asset, faces, people, stats) = result?;
                    self.stats = stats;
                    return Ok((asset, faces, people));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => anyhow::bail!("Image worker stopped"),
            }
        }
    }
}
impl near_duplicates::batch::Evidence for EvidenceClient<'_> {
    fn image(
        &mut self,
        input: &near_duplicates::batch::Input,
    ) -> anyhow::Result<crate::core::pipeline::Asset> {
        self.request(input, false, false).map(|(asset, ..)| asset)
    }
    fn faces(&mut self, input: &near_duplicates::batch::Input) -> Option<Vec<faces::Face>> {
        self.request(input, true, false)
            .ok()
            .and_then(|(_, faces, _)| faces)
    }
}

type Choices = Vec<(usize, usize, MediaId)>;

pub(super) fn spawn(
    publisher: Publisher,
    jobs: Receiver<SimilarityJob>,
    evidence: SyncSender<EvidenceRequest>,
    generation: Arc<RunGeneration>,
    cache: cache::Control,
) -> thread::JoinHandle<()> {
    thread::Builder::new()
        .name("photosort-similarity".into())
        .spawn(move || {
            let worker = Similarity {
                publisher: &publisher,
                generation: &generation,
                cache: &cache,
                evidence: &evidence,
            };
            while !publisher.stop.load(Ordering::Relaxed) {
                let Ok(job) = jobs.recv_timeout(Duration::from_millis(100)) else {
                    continue;
                };
                let _ = publisher.guard(super::Worker::Similarity, || worker.handle(job));
            }
        })
        .expect("Start general similarity policy worker")
}

struct Similarity<'a> {
    publisher: &'a Publisher,
    generation: &'a RunGeneration,
    cache: &'a cache::Control,
    evidence: &'a SyncSender<EvidenceRequest>,
}

impl Similarity<'_> {
    fn cancelled(&self, job: &SimilarityJob) -> bool {
        self.publisher.cancelled(job.epoch) || self.generation.current() != job.run
    }
    fn send(&self, job: &SimilarityJob, event: Event) {
        self.publisher
            .send_checked(job.epoch, event, || self.cancelled(job));
    }

    fn handle(&self, mut job: SimilarityJob) {
        if self.cancelled(&job) {
            return;
        }
        let Some(mut grouped) = self.group(&job) else {
            return;
        };
        let mut automatic = Arc::default();
        let mut choices = Vec::new();
        if matches!(
            job.profile,
            AnalysisProfile::Visual | AnalysisProfile::Duplicates
        ) {
            automatic = Arc::new(std::mem::take(&mut grouped.groups));
            let Some((groups, index)) = self.correct_scenes(&job, &automatic) else {
                return;
            };
            grouped.groups = groups;
            choices = index;
        }
        for input in &mut job.inputs {
            input.group = grouped.groups.get(&input.media.id).copied().unwrap_or(0);
        }
        self.publish_groups(&job, grouped, automatic, choices);
        if job.profile != AnalysisProfile::Duplicates {
            self.cache.request(false);
            self.send(
                &job,
                Event::AnalysisDone {
                    run: job.run,
                    stats: job.stats,
                },
            );
            return;
        }
        self.review_duplicates(&job);
    }

    /// Cluster the prepared records; `None` when cancelled, stale or failed.
    fn group(&self, job: &SimilarityJob) -> Option<regional::grouping::Output> {
        let cancelled = || self.cancelled(job);
        let grouping = if job.profile == AnalysisProfile::Faces {
            face_groups::group(&job.regional, &job.store, cancelled)
        } else {
            regional::grouping::group(
                &job.regional,
                &job.store,
                job.profile == AnalysisProfile::Mixed,
                job.similarity_distance,
                cancelled,
            )
        };
        let grouped = match grouping {
            Ok(Some(grouped)) => grouped,
            Ok(None) => return None,
            Err(error) => {
                self.send(
                    job,
                    Event::RunError {
                        run: job.run,
                        error: format!("Similarity grouping failed: {error:#}"),
                    },
                );
                return None;
            }
        };
        let valid = job.sources.iter().all(|(media, source)| {
            !cancelled() && Pipeline::key(media).is_ok_and(|key| key == *source)
        });
        if cancelled() {
            return None;
        }
        if !valid {
            self.send(
                job,
                Event::RunError {
                    run: job.run,
                    error: "Photos changed during similarity grouping; analyze again".into(),
                },
            );
            return None;
        }
        Some(grouped)
    }

    /// Apply manual scene corrections; `None` after reporting a failure.
    fn correct_scenes(
        &self,
        job: &SimilarityJob,
        automatic: &HashMap<MediaId, usize>,
    ) -> Option<(HashMap<MediaId, usize>, Choices)> {
        let cancelled = || self.cancelled(job);
        let resolved = manual_groups::resolve_scenes(automatic, &job.scene_groups, cancelled)
            .and_then(|groups| {
                let choices = manual_groups::group_choices(&groups, cancelled)?;
                Ok((groups, choices))
            });
        match resolved {
            Ok(resolved) => Some(resolved),
            Err(error) => {
                self.send(
                    job,
                    Event::RunError {
                        run: job.run,
                        error: format!("Scene corrections failed: {error:#}"),
                    },
                );
                None
            }
        }
    }

    fn publish_groups(
        &self,
        job: &SimilarityJob,
        grouped: regional::grouping::Output,
        automatic: Arc<HashMap<MediaId, usize>>,
        choices: Choices,
    ) {
        let event = if job.profile == AnalysisProfile::Faces {
            Event::FaceGroups {
                run: job.run,
                groups: grouped,
            }
        } else if job.profile == AnalysisProfile::Mixed {
            Event::MixedGroups {
                run: job.run,
                labels: grouped.folders,
            }
        } else {
            Event::GeneralGroups {
                run: job.run,
                automatic,
                choices,
                groups: grouped.groups,
            }
        };
        self.send(job, event);
    }

    fn review_duplicates(&self, job: &SimilarityJob) {
        let cancelled = || self.cancelled(job);
        self.send(job, Event::DuplicateStarted { run: job.run });
        let mut evidence = EvidenceClient {
            job,
            requests: self.evidence,
            cancelled: &cancelled,
            stats: job.stats,
        };
        let result = near_duplicates::batch::review(
            &job.inputs,
            &job.store,
            &mut evidence,
            cancelled,
            |done, total| {
                self.send(
                    job,
                    Event::Progress {
                        run: job.run,
                        done,
                        total,
                    },
                )
            },
        );
        // Both file and sidecar identity must still match, including exact
        // copies that deliberately bypass all pixel and face requests.
        let valid = job
            .inputs
            .iter()
            .all(|i| !cancelled() && Pipeline::key(&i.media).is_ok_and(|k| k == i.source));
        if cancelled() {
            return;
        }
        // Photos with closed eyes and the other shots of their moment.
        let blinks = match result {
            Ok(Some(_)) if valid => {
                let candidates: Vec<_> = job
                    .inputs
                    .iter()
                    .map(|input| eyes::Candidate {
                        id: input.media.id,
                        group: input.group,
                        camera: &input.camera,
                        at_ms: input.captured_ms,
                        faces: input.faces,
                    })
                    .collect();
                eyes::moments(
                    &candidates,
                    |i| {
                        let (_, _, people) = evidence.request(&job.inputs[i], true, true).ok()?;
                        people
                    },
                    cancelled,
                )
            }
            _ => None,
        };
        if cancelled() {
            return;
        }
        match result {
            Ok(Some(pairs)) if valid => {
                self.send(
                    job,
                    Event::DuplicatePairs {
                        run: job.run,
                        pairs,
                        blinks: blinks.unwrap_or_default(),
                    },
                );
                self.cache.request(false);
                self.send(
                    job,
                    Event::AnalysisDone {
                        run: job.run,
                        stats: evidence.stats,
                    },
                );
            }
            Err(error) => self.send(
                job,
                Event::RunError {
                    run: job.run,
                    error: format!("Duplicate review failed: {error:#}"),
                },
            ),
            Ok(_) if !valid => self.send(
                job,
                Event::RunError {
                    run: job.run,
                    error: "Photos changed during duplicate review; analyze again".into(),
                },
            ),
            _ => {}
        }
    }
}
