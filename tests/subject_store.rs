use photosort::core::{
    cache,
    faces::Faces,
    model::MediaId,
    subjects::{
        DenseEmbedding, Evidence, Strictness,
        grouping::{Input, group},
        store::{Features, Store},
    },
};
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

#[test]
fn legacy_subject_face_and_pair_evidence_migrates_through_one_store() {
    let root = tempfile::tempdir().unwrap();
    let value = features();
    let key = Store::key("legacy", "model");
    let face_key = Store::face_key("legacy", "face-model");
    let pair_key = Store::pair_key("a", "b", Some(2), Some(3));
    let pair = photosort::core::subjects::Pair {
        subject_similarity: 0.12345678912345678,
        face_similarity: Some(0.4567891234567891),
        scene_similarity: 0.9876543219876543,
        complete: true,
        geometry: Default::default(),
        time_delta_seconds: Some(1.0),
        confidence: 0.8765432187654321,
        accepted: false,
        reason: "cached evidence".into(),
    };
    for (key, bytes) in [
        (&key, serde_json::to_vec(&value).unwrap()),
        (&face_key, serde_json::to_vec(&Faces::default()).unwrap()),
        (&pair_key, serde_json::to_vec(&pair).unwrap()),
    ] {
        std::fs::write(root.path().join(format!("{key}.json")), bytes).unwrap();
    }
    let store = Store::new(root.path().into(), true).unwrap();
    assert_eq!(store.get(&key).unwrap().dense.global, value.dense.global);
    assert!(store.faces(&face_key).unwrap().complete());
    assert_eq!(
        store.pair(&pair_key).unwrap().subject_similarity.to_bits(),
        pair.subject_similarity.to_bits()
    );
    for key in [&key, &face_key, &pair_key] {
        assert!(root.path().join(format!("{key}.bin")).is_file());
    }
    cache::maintain(root.path(), Some(0), || false).unwrap();
    assert_eq!(store.get(&key).unwrap().dense.global, value.dense.global);
    assert!(store.faces(&face_key).unwrap().complete());
    assert_eq!(
        store.pair(&pair_key).unwrap().confidence.to_bits(),
        pair.confidence.to_bits()
    );
}
