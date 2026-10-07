use photosort::core::{cache, regional::Embedding, subjects::store::Store};
#[test]
fn regional_artifacts_share_cache_pinning_budget_and_recipe_invalidation() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    let key = Store::regional_key(&Store::key("photo-and-sidecar", "models"));
    assert_ne!(
        key,
        Store::regional_key(&Store::key("changed-sidecar", "models"))
    );
    assert_ne!(
        key,
        Store::regional_key(&Store::key("photo-and-sidecar", "changed-model"))
    );
    let mut vector = vec![0.; 384];
    vector[0] = 1.;
    let artifact = Embedding {
        vectors: vec![vector; 6],
    };
    store.put_regional(&key, &artifact).unwrap();
    let reopened = Store::new(root.path().into(), true).unwrap();
    assert_eq!(reopened.regional(&key).unwrap().vectors, artifact.vectors);
    cache::maintain(root.path(), Some(0), || false).unwrap();
    assert_eq!(reopened.regional(&key).unwrap().vectors, artifact.vectors);
    assert!(
        Store::new(root.path().into(), true)
            .unwrap()
            .regional(&key)
            .is_none()
    );
    let scratch = Store::new(root.path().into(), false).unwrap();
    scratch.put_regional(&key, &artifact).unwrap();
    assert!(!root.path().join(format!("{key}.bin")).exists());
    assert!(scratch.regional(&key).is_some());
    assert!(scratch.put_regional("../foreign", &artifact).is_err());
    for vectors in [
        vec![],
        vec![vec![1.; 384]; 2],
        vec![vec![1.; 383]; 6],
        vec![vec![f32::NAN; 384]; 6],
    ] {
        assert!(scratch.put_regional(&key, &Embedding { vectors }).is_err());
    }
}

#[test]
fn similarity_settings_persist_with_reference_bounds_and_reclassify_cached_regions() {
    use photosort::core::{
        model::MediaId,
        preferences::Preferences,
        regional::{
            self,
            grouping::{Record, group},
        },
    };
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("prefs.json");
    std::fs::write(&path, b"[1,{}]").unwrap();
    let mut prefs = Preferences::load(&path).unwrap();
    assert_eq!(prefs.similarity_distance, 0.055);
    for (raw, expected) in [
        (0., 0.02),
        (1., 0.20),
        (f32::NAN, 0.055),
        (f32::INFINITY, 0.055),
        (0.123, 0.123),
    ] {
        prefs.similarity_distance = raw;
        prefs.save(&path).unwrap();
        assert_eq!(
            Preferences::load(&path).unwrap().similarity_distance,
            expected
        );
    }
    let store = Store::new(root.path().into(), false).unwrap();
    let mut records = Vec::new();
    for (i, angle) in [0f32, 12.].into_iter().enumerate() {
        let key = Store::regional_key(&i.to_string());
        let mut vector = vec![0.; 384];
        vector[0] = angle.to_radians().cos();
        vector[1] = angle.to_radians().sin();
        store
            .put_regional(
                &key,
                &Embedding {
                    vectors: vec![vector; 6],
                },
            )
            .unwrap();
        records.push(Record {
            id: MediaId(i as u64),
            key: Some(key),
            portrait: false,
            date: Some("2025-01-01".into()),
        });
    }
    let tight = group(&records, &store, false, 0.02, || false)
        .unwrap()
        .unwrap();
    let standard = group(&records, &store, false, regional::DEFAULT_EPS, || false)
        .unwrap()
        .unwrap();
    assert_ne!(tight.groups[&MediaId(0)], tight.groups[&MediaId(1)]);
    assert_eq!(standard.groups[&MediaId(0)], standard.groups[&MediaId(1)]);
}

#[test]
fn legacy_regional_cache_migrates_once_and_pins_exact_values_without_persistence() {
    let root = tempfile::tempdir().unwrap();
    let key = Store::regional_key("legacy-photo");
    let value = Embedding {
        vectors: vec![(0..384).map(|i| (i as f32 - 191.5) / 384.0).collect(); 6],
    };
    let legacy = root.path().join(format!("{key}.json"));
    let binary = root.path().join(format!("{key}.bin"));
    std::fs::write(&legacy, serde_json::to_vec(&value).unwrap()).unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    assert_eq!(store.regional(&key).unwrap().vectors, value.vectors);
    assert!(binary.is_file());
    assert!(std::fs::metadata(&binary).unwrap().len() < std::fs::metadata(&legacy).unwrap().len());
    cache::maintain(root.path(), Some(0), || false).unwrap();
    store.persistent(false);
    assert_eq!(store.regional(&key).unwrap().vectors, value.vectors);
    assert!(!binary.exists() && !legacy.exists());
    assert!(
        Store::new(root.path().into(), true)
            .unwrap()
            .regional(&key)
            .is_none()
    );
}
#[test]
fn damaged_binary_is_a_miss_even_when_an_old_json_record_exists() {
    let root = tempfile::tempdir().unwrap();
    let key = Store::regional_key("damaged");
    let value = Embedding {
        vectors: vec![vec![0.12345679; 384]; 6],
    };
    let store = Store::new(root.path().into(), true).unwrap();
    store.put_regional(&key, &value).unwrap();
    let binary = root.path().join(format!("{key}.bin"));
    let original = std::fs::read(&binary).unwrap();
    std::fs::write(
        root.path().join(format!("{key}.json")),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    for index in [0, 7, 8, 9, original.len() - 1] {
        let mut damaged = original.clone();
        damaged[index] ^= 1;
        std::fs::write(&binary, damaged).unwrap();
        assert!(
            Store::new(root.path().into(), true)
                .unwrap()
                .regional(&key)
                .is_none()
        );
    }
    assert_eq!(
        store.regional(&key).unwrap().vectors,
        value.vectors,
        "Pinned evidence is isolated from disk replacement"
    );
}
#[test]
fn concurrent_legacy_readers_share_valid_pinned_evidence_after_disk_clear() {
    let root = tempfile::tempdir().unwrap();
    let key = Store::regional_key("shared");
    let value = Embedding {
        vectors: vec![vec![0.125; 384]; 6],
    };
    std::fs::write(
        root.path().join(format!("{key}.json")),
        serde_json::to_vec(&value).unwrap(),
    )
    .unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| assert_eq!(store.regional(&key).unwrap().vectors, value.vectors));
        }
    });
    cache::maintain(root.path(), Some(0), || false).unwrap();
    assert_eq!(store.regional(&key).unwrap().vectors, value.vectors);
}
