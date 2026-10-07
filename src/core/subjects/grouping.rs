use super::{
    Artifact, Pair, Strictness, candidates, clusters, store::Store, verify_geometry, verify_pair,
};
use crate::core::model::MediaId;
use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

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
    mut inputs: Vec<Input>,
    store: &Store,
    strictness: Strictness,
    cancelled: impl Fn() -> bool,
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
    let Some(candidates) = candidates(&globals, &times, &cancelled) else {
        return Ok(None);
    };
    drop(globals);
    let mut cache: HashMap<usize, (Arc<Artifact>, u64)> = HashMap::new();
    let mut tick = 0;
    let mut pairs: BTreeMap<(usize, usize), Pair> = BTreeMap::new();
    for (a, b) in candidates {
        if cancelled() {
            return Ok(None);
        }
        let mut load = |index: usize| -> Result<Arc<Artifact>> {
            tick += 1;
            if let Some((artifact, used)) = cache.get_mut(&index) {
                *used = tick;
                return Ok(artifact.clone());
            }
            let features = store
                .get(&inputs[index].key)
                .context("Subject evidence is no longer available; retry analysis")?;
            let faces = features
                .identities
                .context("Subject evidence has no verified face identities")?;
            let artifact = Arc::new(features.dense.into_artifact(faces));
            if cache.len() >= 256
                && let Some(old) = cache
                    .iter()
                    .min_by_key(|(_, (_, used))| *used)
                    .map(|(key, _)| *key)
            {
                cache.remove(&old);
            }
            cache.insert(index, (artifact.clone(), tick));
            Ok(artifact)
        };
        let key = Store::pair_key(&inputs[a].key, &inputs[b].key, times[a], times[b]);
        let previous = store.pair(&key);
        let mut changed = previous.is_none();
        let mut pair = if let Some(pair) = previous {
            pair.with_strictness(strictness)
        } else {
            verify_pair(
                load(a)?.as_ref(),
                load(b)?.as_ref(),
                Default::default(),
                times[a],
                times[b],
                strictness,
            )
        };
        if pair.needs_geometry(strictness) {
            let (left, right) = (load(a)?, load(b)?);
            let Some(geometry) = verify_geometry(&left, &right, &cancelled) else {
                return Ok(None);
            };
            pair = verify_pair(&left, &right, geometry, times[a], times[b], strictness);
            changed = true;
        }
        if cancelled() {
            return Ok(None);
        }
        if changed {
            store.put_pair(&key, &pair)?;
        }
        pairs.insert((a, b), pair);
    }
    Ok(clusters(inputs.len(), &pairs, &cancelled).map(|groups| {
        inputs
            .into_iter()
            .zip(groups)
            .map(|(i, g)| (i.id, g))
            .collect()
    }))
}
