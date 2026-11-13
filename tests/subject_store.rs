use photosort::core::{
    cache,
    faces::{Face, Faces},
    model::MediaId,
    subjects::{
        DenseEmbedding, Evidence, Strictness,
        grouping::{Input, group, group_with},
        store::{Features, Store},
    },
};
use std::collections::HashSet;
fn features() -> Features {
    let mut tokens = vec![0.0; 257 * 384];
    for row in tokens.chunks_mut(384) {
        row[0] = 1.0;
    }
    Features {
        dense: DenseEmbedding::from_tokens(&tokens, 384).unwrap(),
        faces: Some(Faces::default()),
        identities: Some(vec![]),
    }
}
#[test]
fn subject_cache_fingerprints_pins_and_clears_only_owned_persistent_data() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    let key = Store::key("source1", "models1");
    assert_ne!(key, Store::key("source2", "models1"));
    assert_ne!(key, Store::key("source1", "models2"));
    let value = features();
    store.put(&key, &value).unwrap();
    let reopened = Store::new(root.path().into(), true).unwrap();
    assert_eq!(reopened.get(&key).unwrap().dense.global, value.dense.global);
    std::fs::write(root.path().join("foreign.txt"), "keep").unwrap();
    cache::maintain(root.path(), Some(0), || false).unwrap();
    assert!(root.path().join("foreign.txt").is_file());
    assert!(store.get(&key).is_some());
    assert!(
        reopened.get(&key).is_some(),
        "Active snapshots must survive cache clearing"
    );
    assert!(
        Store::new(root.path().into(), true)
            .unwrap()
            .get(&key)
            .is_none()
    );
    let private = Store::new(root.path().into(), false).unwrap();
    private.put(&key, &value).unwrap();
    assert!(!root.path().join(format!("{key}.bin")).exists());
    assert!(private.get(&key).is_some());
    assert!(private.put("../invalid", &value).is_err());
    let mut invalid = value.clone();
    invalid.identities = Some(vec![Evidence {
        descriptor: vec![1.0; 384],
        bbox: [0., 0., 1., 1.],
        area: 1.0,
        kind: "face".into(),
    }]);
    assert!(
        store.put(&key, &invalid).is_err(),
        "A missing face cannot silently become no-face evidence"
    );
}
#[test]
fn grouping_consumes_shared_evidence_and_stays_independent_of_input_order() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let mut inputs = Vec::new();
    for i in 0..4 {
        let key = Store::key(&i.to_string(), "models");
        let mut f = features();
        if i == 3 {
            f.dense.global.fill(0.0);
            f.dense.global[1] = 1.0;
        }
        store.put(&key, &f).unwrap();
        inputs.push(Input {
            id: MediaId(i),
            path: format!("/library/{i}").into(),
            key,
            global: f.dense.global,
            timestamp: Some(0),
        });
    }
    let expected = group(inputs.clone(), &store, Strictness::Standard, || false)
        .unwrap()
        .unwrap();
    assert_eq!(expected[&MediaId(0)], expected[&MediaId(2)]);
    assert_ne!(expected[&MediaId(0)], expected[&MediaId(3)]);
    inputs.reverse();
    assert_eq!(
        group(inputs.clone(), &store, Strictness::Standard, || false)
            .unwrap()
            .unwrap(),
        expected
    );
    assert!(
        group(inputs, &store, Strictness::Broad, || true)
            .unwrap()
            .is_none()
    );
}

#[test]
fn policy_changes_reuse_cached_pairs_and_timestamp_changes_invalidate_them() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    let mut inputs = Vec::new();
    for i in 0..3 {
        let key = Store::key(&i.to_string(), "models");
        let f = features();
        store.put(&key, &f).unwrap();
        inputs.push(Input {
            id: MediaId(i),
            path: format!("/{i}").into(),
            key,
            global: f.dense.global,
            timestamp: Some(0),
        });
    }
    let expected = group(inputs.clone(), &store, Strictness::Standard, || false)
        .unwrap()
        .unwrap();
    // Only raw pair records remain in the persistent cache. A fresh store has no
    // pinned artifacts; a successful reclassification therefore proves no reload.
    for i in &inputs {
        std::fs::remove_file(root.path().join(format!("{}.bin", i.key))).unwrap();
    }
    drop(store);
    let store = Store::new(root.path().into(), true).unwrap();
    assert_eq!(
        group(inputs.clone(), &store, Strictness::Conservative, || false)
            .unwrap()
            .unwrap(),
        expected
    );
    inputs[0].timestamp = Some(3000);
    assert!(
        group(inputs, &store, Strictness::Broad, || false).is_err(),
        "Changed capture time must not reuse stale pair confidence"
    );
}

struct Noise(u64);
impl Noise {
    /// Between -1 and 1.
    fn next(&mut self) -> f32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        (self.0 % 2001) as f32 / 1000.0 - 1.0
    }
    /// A scene: what the whole image and each of its sixteen regions show.
    fn scene(&mut self) -> Vec<Vec<f32>> {
        (0..17)
            .map(|_| (0..384).map(|_| self.next()).collect())
            .collect()
    }
    /// One frame of a burst of the scene: everything a little different.
    fn frame(&mut self, scene: &[Vec<f32>]) -> Features {
        let mut vectors = scene
            .iter()
            .map(|v| {
                v.iter()
                    .map(|v| v + 0.1 * self.next())
                    .collect::<Vec<f32>>()
            })
            .collect::<Vec<_>>()
            .into_iter();
        let global = vectors.next().unwrap();
        let regions = vectors
            .enumerate()
            .map(|(n, descriptor)| {
                let (x, y) = ((n % 4) as f32 / 4.0, (n / 4) as f32 / 4.0);
                Evidence {
                    descriptor,
                    bbox: [x, y, x + 0.25, y + 0.25],
                    area: 1.0 / 16.0,
                    kind: "region".into(),
                }
            })
            .collect();
        Features {
            dense: DenseEmbedding { global, regions },
            faces: Some(Faces::default()),
            identities: Some(vec![]),
        }
    }
}
fn stored(store: &Store, inputs: &mut Vec<Input>, path: String, time: i64, features: Features) {
    let key = Store::key(&path, "models");
    store.put(&key, &features).unwrap();
    inputs.push(Input {
        id: MediaId(inputs.len() as u64),
        path: path.into(),
        key,
        global: features.dense.global,
        timestamp: Some(time),
    });
}

#[test]
fn long_bursts_are_one_group_each_and_a_second_run_reuses_their_verdicts() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    let mut noise = Noise(7);
    let mut inputs = Vec::new();
    // Longer than the 32 nearest photos each frame is compared with.
    let bursts = [41u64, 60, 120];
    for (burst, frames) in bursts.into_iter().enumerate() {
        let scene = noise.scene();
        for n in 0..frames {
            let time = burst as i64 * 3_600_000 + n as i64 * 100;
            let path = format!("/library/{burst}/{n:03}.jpg");
            stored(&store, &mut inputs, path, time, noise.frame(&scene));
        }
    }
    // Another scene, taken right after the last burst.
    let other = noise.scene();
    let time = 2 * 3_600_000 + 12_000;
    stored(
        &store,
        &mut inputs,
        "/library/2/other.jpg".into(),
        time,
        noise.frame(&other),
    );
    let groups = group_with(
        inputs.clone(),
        &store,
        Strictness::Standard,
        3,
        || false,
        |_, _| {},
    )
    .unwrap()
    .unwrap();
    let mut first = 0;
    let mut seen = HashSet::new();
    for frames in bursts {
        let burst: HashSet<usize> = (first..first + frames)
            .map(|id| groups[&MediaId(id)])
            .collect();
        assert_eq!(burst.len(), 1, "a burst of {frames} is one group");
        seen.extend(burst);
        first += frames;
    }
    assert!(
        seen.insert(groups[&MediaId(first)]),
        "the other scene joined"
    );
    assert_eq!(seen.len(), 4);
    // The evidence is gone: every verdict, also those judged for the merges,
    // comes from the cache. Neither the order nor the threads change groups.
    for input in &inputs {
        std::fs::remove_file(root.path().join(format!("{}.bin", input.key))).unwrap();
    }
    drop(store);
    let store = Store::new(root.path().into(), true).unwrap();
    inputs.reverse();
    let again = group(inputs, &store, Strictness::Standard, || false);
    assert_eq!(again.unwrap().unwrap(), groups);
}

#[test]
fn a_rejected_pair_keeps_a_long_burst_in_two_groups() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let mut noise = Noise(11);
    let scene = noise.scene();
    let drift = noise.scene().remove(0);
    let mut inputs = Vec::new();
    let frames = 60;
    for n in 0..frames {
        let mut features = noise.frame(&scene);
        // The view moves a little with every frame, so the first and the
        // last are never among each other's nearest.
        for (value, drift) in features.dense.global.iter_mut().zip(&drift) {
            *value += drift * 0.3 * n as f32 / (frames - 1) as f32;
        }
        // One person in every frame, looking different enough in the first
        // and the last to be someone else there, and close enough to both
        // in all the frames between.
        let turn = match n {
            0 => -25f32,
            n if n == frames - 1 => 25.0,
            _ => 0.0,
        }
        .to_radians();
        let mut descriptor = vec![0.0; 384];
        (descriptor[0], descriptor[1]) = (turn.cos(), turn.sin());
        features.faces = Some(Faces {
            count: 1,
            geometry: vec![Face {
                bbox: [0.4, 0.4, 0.6, 0.6],
                eyes: [[0.45, 0.45], [0.55, 0.45]],
                landmarks: vec![[0.5, 0.5]],
                blink: [0.0, 0.0],
            }],
            ..Default::default()
        });
        features.identities = Some(vec![Evidence {
            descriptor,
            bbox: [0.4, 0.4, 0.6, 0.6],
            area: 0.04,
            kind: "face".into(),
        }]);
        let path = format!("/library/{n:03}.jpg");
        stored(&store, &mut inputs, path, n as i64 * 100, features);
    }
    let groups = group(inputs, &store, Strictness::Standard, || false)
        .unwrap()
        .unwrap();
    // Every other pair is accepted: the frames gather around the two that
    // cannot share a group, and nothing is left over.
    let (first, last) = (groups[&MediaId(0)], groups[&MediaId(frames as u64 - 1)]);
    assert_ne!(first, last);
    assert!(groups.values().all(|group| [first, last].contains(group)));
}
