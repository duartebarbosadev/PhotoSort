use super::{
    Artifact, Judged, Links, Pair, Strictness,
    candidates::candidates_with,
    dot,
    store::{Store, Verdicts},
    verify_geometry, verify_pair,
};
use crate::core::{model::MediaId, parallel};
use anyhow::{Context, Result};
use std::{
    collections::{HashMap, hash_map::Entry},
    sync::Arc,
};

/// The largest group whose never-compared pairs are judged so that it can
/// form: a burst of up to this many frames becomes one group. Every pair of
/// a group has to be accepted, so the work grows with the square of its
/// size: half a million verdicts here, some ten seconds the first time and
/// 50 MB of cache. Two groups that would pass it together merge only when
/// their photos were all compared already, as every group did before: a
/// longer burst comes out as a few groups of up to this many.
pub const JUDGED_GROUP: usize = 1000;
/// Pairs judged between two looks at whether one was rejected. Few at
/// first: they start with the pairs least alike.
const FIRST_ROUND: usize = 8;
const ROUND: usize = 2048;
/// Pairs one thread takes at a time.
const BLOCK: usize = 16;
/// Verdicts of earlier runs kept at hand for the merges, about 40 bytes
/// each. Past this the records of further photos are not read, and their
/// pairs are judged again.
const HELD: usize = 2_000_000;
/// Verdicts judged for merges that wait to be written to their records.
const UNWRITTEN: usize = 100_000;

#[derive(Clone)]
pub struct Input {
    pub id: MediaId,
    pub path: std::path::PathBuf,
    pub key: String,
    pub global: Vec<f32>,
    pub timestamp: Option<i64>,
}
/// Uses small global vectors for neighbor search, and a bounded regional cache
/// for verification. No decoder or model is reachable from this policy worker.
pub fn group(
    inputs: Vec<Input>,
    store: &Store,
    strictness: Strictness,
    cancelled: impl Fn() -> bool,
) -> Result<Option<HashMap<MediaId, usize>>> {
    group_with(inputs, store, strictness, 1, cancelled, |_, _| {})
}

/// `group` with the neighbor search, and the pairs judged for a merge, on up
/// to `threads` threads. `progress` hears how far the search, then the pair
/// checks, then the merging have come.
pub fn group_with(
    mut inputs: Vec<Input>,
    store: &Store,
    strictness: Strictness,
    threads: usize,
    cancelled: impl Fn() -> bool,
    mut progress: impl FnMut(usize, usize),
) -> Result<Option<HashMap<MediaId, usize>>> {
    inputs.sort_by(|a, b| a.path.cmp(&b.path));
    let globals: Vec<_> = inputs
        .iter_mut()
        .map(|i| Artifact {
            global: std::mem::take(&mut i.global),
            subjects: Vec::new(),
            faces: Vec::new(),
        })
        .collect();
    let times: Vec<_> = inputs.iter().map(|i| i.timestamp).collect();
    // A third of the bar each for the search, the checks and the merging,
    // counted in photos.
    let count = inputs.len();
    let third = count / 3;
    let Some(candidates) = candidates_with(&globals, &times, threads, &cancelled, |done, _| {
        progress(done / 3, count)
    }) else {
        return Ok(None);
    };
    drop(globals);
    let mut evidence = Evidence {
        inputs: &inputs,
        store,
        kept: HashMap::new(),
        tick: 0,
        room: 256,
    };
    let mut links = Links::new(count);
    // Candidates come sorted, so each first photo's verdicts are read and
    // written as one record.
    let mut verdicts: Option<(usize, Verdicts<Pair>)> = None;
    for (a, b) in candidates {
        if cancelled() {
            return Ok(None);
        }
        if verdicts.as_ref().is_none_or(|(first, _)| *first != a) {
            if let Some((_, finished)) = verdicts.take() {
                store.put_pairs(finished)?;
            }
            verdicts = Some((a, store.pairs(&inputs[a].key)));
            progress(third + a / 3, count);
        }
        let (_, known) = verdicts.as_mut().unwrap();
        let id = Store::pair_id(&inputs[a].key, &inputs[b].key, times[a], times[b]);
        let pair = match known.get(&id).map(|pair| pair.with_strictness(strictness)) {
            Some(pair) if !pair.needs_geometry(strictness) => pair,
            previous => {
                let (left, right) = (evidence.load(a)?, evidence.load(b)?);
                let Some(pair) = verdict(
                    previous,
                    &left,
                    &right,
                    (times[a], times[b]),
                    strictness,
                    &cancelled,
                ) else {
                    return Ok(None);
                };
                known.insert(id, pair.clone());
                pair
            }
        };
        if cancelled() {
            return Ok(None);
        }
        links.add((a, b), pair.accepted, pair.confidence);
    }
    if let Some((_, finished)) = verdicts.take() {
        store.put_pairs(finished)?;
    }
    // A group as large as the limit has all its photos at hand.
    evidence.room = evidence.room.max(JUDGED_GROUP);
    let mut merging = Merging {
        evidence,
        times: &times,
        strictness,
        threads,
        cancelled: &cancelled,
        known: HashMap::new(),
        held: 0,
        judged: HashMap::new(),
        unwritten: 0,
    };
    let groups = links.clusters(
        JUDGED_GROUP,
        &cancelled,
        |tried, pairs| {
            let merged = tried * (count - 2 * third) / pairs.max(1);
            progress(2 * third + merged, count)
        },
        |missing| merging.judge(missing),
    );
    // Also when cancelled: the next run goes on from what was judged.
    let saved = merging.write();
    drop(merging);
    let groups = groups?;
    saved?;
    progress(count, count);
    Ok(groups.map(|groups| {
        inputs
            .into_iter()
            .zip(groups)
            .map(|(i, g)| (i.id, g))
            .collect()
    }))
}

/// One pair's verdict from its photos' evidence. `previous` is what the
/// cache held, read under this strictness. None when cancelled.
fn verdict(
    previous: Option<Pair>,
    left: &Artifact,
    right: &Artifact,
    times: (Option<i64>, Option<i64>),
    strictness: Strictness,
    cancelled: impl Fn() -> bool,
) -> Option<Pair> {
    let judge = |geometry| verify_pair(left, right, geometry, times.0, times.1, strictness);
    let mut pair = previous.unwrap_or_else(|| judge(Default::default()));
    if pair.needs_geometry(strictness) {
        pair = judge(verify_geometry(left, right, cancelled)?);
    }
    Some(pair)
}

/// The photos' regional evidence, read when a pair needs it and kept while
/// recently used.
struct Evidence<'a> {
    inputs: &'a [Input],
    store: &'a Store,
    kept: HashMap<usize, (Arc<Artifact>, u64)>,
    tick: u64,
    room: usize,
}
impl Evidence<'_> {
    fn load(&mut self, index: usize) -> Result<Arc<Artifact>> {
        self.tick += 1;
        if let Some((artifact, used)) = self.kept.get_mut(&index) {
            *used = self.tick;
            return Ok(artifact.clone());
        }
        let features = self
            .store
            .get(&self.inputs[index].key)
            .context("Subject evidence is no longer available; retry analysis")?;
        let faces = features
            .identities
            .context("Subject evidence has no verified face identities")?;
        let artifact = Arc::new(features.dense.into_artifact(faces));
        if self.kept.len() >= self.room
            && let Some(old) = self
                .kept
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| *key)
        {
            self.kept.remove(&old);
        }
        self.kept.insert(index, (artifact.clone(), self.tick));
        Ok(artifact)
    }
}

/// A pair two groups need before they can merge, never judged so far.
struct Open {
    /// How alike the two photos are as a whole.
    scene: f32,
    pair: (usize, usize),
    id: [u8; 32],
}

/// Judges the pairs that were never compared when two groups could merge.
/// A pair is asked for once in a run: its groups merge, or stay apart for
/// good.
struct Merging<'a, C> {
    evidence: Evidence<'a>,
    times: &'a [Option<i64>],
    strictness: Strictness,
    threads: usize,
    cancelled: &'a C,
    /// What earlier runs judged, by first photo: whether each pair is
    /// accepted under this strictness. And how many verdicts that is.
    known: HashMap<usize, HashMap<[u8; 32], bool>>,
    held: usize,
    /// This run's verdicts by first photo, not yet in their records.
    judged: HashMap<usize, Vec<([u8; 32], Pair)>>,
    unwritten: usize,
}
impl<C: Fn() -> bool> Merging<'_, C> {
    /// Whether every one of these pairs is accepted, by the rule and in the
    /// order of photos every other pair is judged with. It stops at the first
    /// round that holds a rejection: the groups stay apart whatever the rest
    /// would say. What it judges is kept for the next run.
    fn judge(&mut self, missing: Vec<(usize, usize)>) -> Result<Judged> {
        let (inputs, store) = (self.evidence.inputs, self.evidence.store);
        let (times, strictness) = (self.times, self.strictness);
        let mut open = Vec::new();
        for (n, &(a, b)) in missing.iter().enumerate() {
            if n % 256 == 0 && (self.cancelled)() {
                return Ok(Judged::Cancelled);
            }
            if !self.known.contains_key(&a) {
                let mut known = HashMap::new();
                if self.held < HELD {
                    for (id, pair) in store.merge_pairs(&inputs[a].key).iter() {
                        let pair = pair.with_strictness(strictness);
                        if !pair.needs_geometry(strictness) {
                            known.insert(*id, pair.accepted);
                        }
                    }
                    self.held += known.len();
                }
                self.known.insert(a, known);
            }
            let id = Store::pair_id(&inputs[a].key, &inputs[b].key, times[a], times[b]);
            match self.known[&a].get(&id) {
                Some(true) => {}
                Some(false) => return Ok(Judged::Rejected(a, b)),
                None => open.push(Open {
                    scene: 0.0,
                    pair: (a, b),
                    id,
                }),
            }
        }
        let mut photos: HashMap<usize, Arc<Artifact>> = HashMap::new();
        for (n, open) in open.iter_mut().enumerate() {
            if n % 256 == 0 && (self.cancelled)() {
                return Ok(Judged::Cancelled);
            }
            for index in [open.pair.0, open.pair.1] {
                if let Entry::Vacant(photo) = photos.entry(index) {
                    photo.insert(self.evidence.load(index)?);
                }
            }
            open.scene = dot(&photos[&open.pair.0].global, &photos[&open.pair.1].global);
        }
        // The least alike first: where a rejection is most likely.
        open.sort_by(|a, b| a.scene.total_cmp(&b.scene).then(a.pair.cmp(&b.pair)));
        let mut rejected = None;
        let mut rest = open.as_slice();
        let mut size = FIRST_ROUND;
        while rejected.is_none() && !rest.is_empty() {
            let (round, later) = rest.split_at(size.min(rest.len()));
            // A thread is worth starting for a few milliseconds of work.
            let threads = self.threads.min(round.len() / (4 * BLOCK) + 1);
            let judged = parallel::blocks(
                threads,
                round.len().div_ceil(BLOCK),
                self.cancelled,
                |_| {},
                |block, stop| {
                    let pairs = &round[block * BLOCK..((block + 1) * BLOCK).min(round.len())];
                    let mut judged = Vec::with_capacity(pairs.len());
                    for open in pairs {
                        let (a, b) = open.pair;
                        judged.push(verdict(
                            None,
                            &photos[&a],
                            &photos[&b],
                            (times[a], times[b]),
                            strictness,
                            stop,
                        )?);
                    }
                    Some(judged)
                },
            );
            let Some(judged) = judged else {
                return Ok(Judged::Cancelled);
            };
            for (open, pair) in round.iter().zip(judged.into_iter().flatten()) {
                if !pair.accepted && rejected.is_none() {
                    rejected = Some(open.pair);
                }
                self.judged
                    .entry(open.pair.0)
                    .or_default()
                    .push((open.id, pair));
            }
            self.unwritten += round.len();
            (rest, size) = (later, ROUND);
        }
        if self.unwritten >= UNWRITTEN {
            self.write()?;
        }
        Ok(match rejected {
            Some((a, b)) => Judged::Rejected(a, b),
            None => Judged::Accepted,
        })
    }
    /// Adds this run's verdicts to their photos' merge records.
    fn write(&mut self) -> Result<()> {
        let (inputs, store) = (self.evidence.inputs, self.evidence.store);
        self.unwritten = 0;
        for (first, judged) in self.judged.drain() {
            let mut record = store.merge_pairs(&inputs[first].key);
            for (id, pair) in judged {
                record.insert(id, pair);
            }
            store.put_pairs(record)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[test]
    fn missing_pairs_are_judged_up_to_the_limit_and_a_rejection_is_asked_once() {
        // Two triples, each fully accepted, with two accepted pairs between
        // them: the seven other pairs across were never compared.
        let links = || {
            let mut links = Links::new(7);
            for pair in [(0, 1), (0, 2), (1, 2), (3, 4), (3, 5), (4, 5)] {
                links.add(pair, true, 0.9);
            }
            links.add((2, 3), true, 0.8);
            links.add((1, 4), true, 0.75);
            // A photo rejected by one of the first triple stays out of it.
            links.add((2, 6), true, 0.7);
            links.add((0, 6), false, 0.1);
            links
        };
        let asked = RefCell::new(Vec::new());
        let group = |limit, rejected: Option<(usize, usize)>| {
            asked.borrow_mut().clear();
            links()
                .clusters(
                    limit,
                    || false,
                    |_, _| {},
                    |missing| {
                        asked.borrow_mut().push(missing.clone());
                        Ok(match rejected.filter(|pair| missing.contains(pair)) {
                            Some((a, b)) => Judged::Rejected(a, b),
                            None => Judged::Accepted,
                        })
                    },
                )
                .unwrap()
                .unwrap()
        };
        // Judged and accepted: one group, and the stranger is never asked about.
        assert_eq!(group(6, None), [1, 1, 1, 1, 1, 1, 2]);
        let pairs = asked.borrow().concat();
        assert_eq!(pairs.len(), 7);
        assert!(
            pairs
                .iter()
                .all(|(a, b)| a < b && *a < 3 && (3..6).contains(b))
        );
        // One of them rejected: the triples stay apart, and the second pair
        // between them does not ask again.
        assert_eq!(group(6, Some((1, 5))), [1, 1, 1, 2, 2, 2, 3]);
        assert_eq!(asked.borrow().len(), 1);
        // A merged group over the limit is not judged: as without judging.
        assert_eq!(group(5, None), [1, 1, 1, 2, 2, 2, 3]);
        assert!(asked.borrow().is_empty());
        // Cancelled while judging: no groups.
        let cancelled = links()
            .clusters(6, || false, |_, _| {}, |_| Ok(Judged::Cancelled))
            .unwrap();
        assert!(cancelled.is_none());
    }
}
