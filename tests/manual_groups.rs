use crate::test_support;
use photosort::core::{
    catalog::Catalog,
    jobs::{Event, IoWork, Runtime},
    manual_groups::{self, Edit, ManualGroups},
    model::*,
    operations,
    session::Session,
};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
fn media(n: usize) -> Vec<Media> {
    (0..n)
        .map(|i| Media {
            id: MediaId(i as u64),
            path: format!("/missing/{i}.jpg").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint {
                bytes: 1,
                identity: [1, i as u64, 0, 0],
                ..Default::default()
            },
        })
        .collect()
}
fn grouped(photos: &[Media], selected: Vec<MediaId>) -> Arc<ManualGroups> {
    manual_groups::edit(
        1,
        Arc::default(),
        Arc::default(),
        photos,
        Edit::Move {
            selected,
            target: None,
        },
        || false,
    )
    .unwrap()
    .manual
}
#[test]
fn corrections_pin_membership_across_regenerated_numbers_and_split_reset_and_undo() {
    let m = media(6);
    let auto = Arc::new(HashMap::from([
        (MediaId(0), 1),
        (MediaId(1), 1),
        (MediaId(2), 2),
        (MediaId(3), 2),
        (MediaId(4), 3),
        (MediaId(5), 3),
    ]));
    let moved = manual_groups::edit(
        1,
        auto.clone(),
        Arc::default(),
        &m,
        Edit::Move {
            selected: vec![MediaId(2)],
            target: Some(1),
        },
        || false,
    )
    .unwrap();
    let pinned = moved.groups[&MediaId(0)];
    assert_eq!(moved.groups[&MediaId(1)], pinned);
    assert_eq!(moved.groups[&MediaId(2)], pinned);
    assert_ne!(moved.groups[&MediaId(3)], pinned);
    assert_eq!(moved.manual.len(), 3);
    let regenerated = Arc::new(HashMap::from([
        (MediaId(0), 99),
        (MediaId(1), 3),
        (MediaId(2), 1),
        (MediaId(3), 99),
        (MediaId(4), 99),
        (MediaId(5), 1),
    ]));
    let result = moved.manual.resolve(&regenerated, || false).unwrap();
    assert_eq!(result[&MediaId(0)], result[&MediaId(2)]);
    assert_ne!(
        result[&MediaId(0)],
        result[&MediaId(3)],
        "New AI members must not silently join a pinned group"
    );
    let split = manual_groups::edit(
        2,
        regenerated.clone(),
        moved.manual.clone(),
        &m,
        Edit::Move {
            selected: vec![MediaId(0)],
            target: None,
        },
        || false,
    )
    .unwrap();
    assert_ne!(split.groups[&MediaId(0)], split.groups[&MediaId(1)]);
    let restored = manual_groups::edit(
        3,
        regenerated.clone(),
        split.manual,
        &m,
        Edit::Restore(vec![MediaId(2)]),
        || false,
    )
    .unwrap();
    assert_eq!(restored.groups[&MediaId(2)], 1);
    let undo = manual_groups::edit(
        4,
        regenerated.clone(),
        restored.manual,
        &m,
        Edit::Replace(moved.manual),
        || false,
    )
    .unwrap();
    assert_eq!(undo.groups, result);
    let all = manual_groups::edit(
        5,
        regenerated.clone(),
        undo.manual,
        &m,
        Edit::RestoreAll,
        || false,
    )
    .unwrap();
    assert_eq!(all.groups, *regenerated);
}
#[test]
fn session_roundtrip_strict_identity_and_verified_relocation_keep_only_valid_intent() {
    let m = media(4);
    let session = Session {
        organize: None,
        manual_groups: grouped(&m, vec![MediaId(0), MediaId(1)]),
        fingerprints: m.iter().map(|m| (m.id, m.fingerprint)).collect(),
        ..Default::default()
    };
    let mut loaded: Session =
        serde_json::from_slice(&serde_json::to_vec(&session).unwrap()).unwrap();
    assert_eq!(loaded.manual_groups, session.manual_groups);
    let mut changed = m.clone();
    changed[0].fingerprint.identity[3] += 1;
    loaded.reconcile(&changed);
    assert!(!loaded.manual_groups.contains(MediaId(0)));
    assert!(loaded.manual_groups.contains(MediaId(1)));
    let mut relocated = changed[1].clone();
    relocated.id = MediaId(19);
    relocated.fingerprint.identity[2] += 1;
    loaded.apply_verified_identities(&[(MediaId(1), relocated.id, relocated.fingerprint)]);
    loaded.reconcile(&[relocated]);
    assert!(loaded.manual_groups.contains(MediaId(19)));
    assert!(!loaded.manual_groups.contains(MediaId(1)));
    let legacy: Session = serde_json::from_str(r#"{"decisions":{},"fingerprints":{}}"#).unwrap();
    assert!(legacy.manual_groups.is_empty());
}
#[test]
fn destination_merge_keeps_unrelated_groups_separate_and_swaps_are_simultaneous() {
    let m = media(4);
    let mut a = Session {
        organize: None,
        manual_groups: grouped(&m, vec![MediaId(0), MediaId(1)]),
        ..Default::default()
    };
    let b = Session {
        organize: None,
        manual_groups: grouped(&m, vec![MediaId(2), MediaId(3)]),
        ..Default::default()
    };
    a.merge(b);
    let groups = a.manual_groups.resolve(&HashMap::new(), || false).unwrap();
    assert_eq!(groups[&MediaId(0)], groups[&MediaId(1)]);
    assert_ne!(groups[&MediaId(0)], groups[&MediaId(2)]);
    a.apply_verified_identities(&[
        (MediaId(0), MediaId(2), m[2].fingerprint),
        (MediaId(2), MediaId(0), m[0].fingerprint),
    ]);
    let groups = a.manual_groups.resolve(&HashMap::new(), || false).unwrap();
    assert_eq!(groups[&MediaId(2)], groups[&MediaId(1)]);
    assert_eq!(groups[&MediaId(0)], groups[&MediaId(3)]);
}
#[test]
fn video_unknown_targets_and_cancellation_do_not_change_the_previous_snapshot() {
    let mut m = media(4);
    m[3].kind = MediaKind::Video;
    let previous = grouped(&m, vec![MediaId(0), MediaId(1)]);
    for selected in [vec![MediaId(3)], vec![MediaId(99)]] {
        assert!(
            manual_groups::edit(
                1,
                Arc::default(),
                previous.clone(),
                &m,
                Edit::Move {
                    selected,
                    target: None
                },
                || false
            )
            .is_err()
        );
    }
    assert!(
        manual_groups::edit(
            1,
            Arc::default(),
            previous.clone(),
            &m,
            Edit::Move {
                selected: vec![MediaId(2)],
                target: Some(99)
            },
            || false
        )
        .is_err()
    );
    assert!(
        manual_groups::edit(
            1,
            Arc::default(),
            previous.clone(),
            &m,
            Edit::RestoreAll,
            || true
        )
        .is_err()
    );
    assert_eq!(previous.len(), 2);
}
fn result(runtime: &mut Runtime) -> manual_groups::Resolved {
    let deadline = Instant::now() + test_support::worker_patience();
    loop {
        match runtime.events.recv_timeout(Duration::from_millis(20)) {
            Ok(e) => match e.event {
                Event::GroupsEdited(r) => return r,
                Event::Error(e) => panic!("{e}"),
                _ => {}
            },
            Err(_) => assert!(Instant::now() < deadline),
        }
    }
}
#[test]
fn hundred_thousand_photos_use_background_membership_without_any_image_work() {
    let dir = tempfile::tempdir().unwrap();
    let m = media(100_000);
    let selected = m.iter().map(|m| m.id).collect();
    let catalog = Arc::new(Catalog {
        media: m,
        ..Default::default()
    });
    let mut runtime = Runtime::with_cache(|| {}, dir.path().join("cache"));
    let start = Instant::now();
    runtime
        .io(IoWork::EditGroups {
            scene: false,
            cancelled: Arc::default(),
            run: 19,
            catalog: catalog.clone(),
            automatic: Arc::default(),
            manual: Arc::default(),
            edit: Edit::Move {
                selected,
                target: None,
            },
        })
        .unwrap();
    let grouped = result(&mut runtime);
    assert_eq!(grouped.run, 19);
    assert_eq!(grouped.manual.len(), 100_000);
    assert!(grouped.groups.values().all(|g| *g == 1));
    println!("100k-photo background edit: {:?}", start.elapsed());
    let checks = std::cell::Cell::new(0);
    assert!(
        grouped
            .manual
            .resolve(&grouped.groups, || {
                checks.set(checks.get() + 1);
                checks.get() > 4
            })
            .is_err()
    );
    assert!(
        checks.get() <= 6,
        "Cancellation is checked during large group work"
    );
    let cancelled = Arc::new(std::sync::atomic::AtomicBool::new(true));
    runtime
        .io(IoWork::EditGroups {
            scene: false,
            cancelled,
            run: 20,
            catalog: catalog.clone(),
            automatic: Arc::default(),
            manual: grouped.manual.clone(),
            edit: Edit::RestoreAll,
        })
        .unwrap();
    runtime
        .io(IoWork::EditGroups {
            scene: false,
            cancelled: Arc::default(),
            run: 21,
            catalog,
            automatic: Arc::default(),
            manual: grouped.manual.clone(),
            edit: Edit::Resolve,
        })
        .unwrap();
    let next = result(&mut runtime);
    assert_eq!(
        next.run, 21,
        "Canceled jobs must neither publish nor poison a later edit"
    );
    assert_eq!(next.manual.len(), 100_000);
    // Nonexistent paths prove this service performs no scan, decoding, metadata or model IO.
    runtime.shutdown();
}
fn move_and_restore(catalog: Catalog) {
    let selected = catalog.media.iter().map(|m| m.id).collect();
    let mut session = Session {
        organize: None,
        manual_groups: grouped(&catalog.media, selected),
        scene_groups: grouped(
            &catalog.media,
            catalog.media.iter().take(2).map(|m| m.id).collect(),
        ),
        fingerprints: catalog
            .media
            .iter()
            .map(|m| (m.id, m.fingerprint))
            .collect(),
        ..Default::default()
    };
    let hashes: HashMap<_, _> = catalog
        .media
        .iter()
        .map(|m| {
            (
                m.name().to_string(),
                operations::content_hash(&m.path, || false).unwrap(),
            )
        })
        .collect();
    let labels = catalog
        .media
        .iter()
        .map(|m| (m.id, "Grouped".to_string()))
        .collect();
    let plan = operations::organize_plan(&catalog, &labels, &catalog.root, true).unwrap();
    let report =
        operations::execute(&plan, &catalog.root.join("journal"), || false, |_, _| {}).unwrap();
    assert!(!report.cancelled && report.errors.is_empty());
    session.apply_report(&report, &catalog.media);
    let after = Catalog::scan(&catalog.root, || false, |_| {}).unwrap();
    session.reconcile(&after.media);
    session.save(&catalog.root).unwrap();
    let loaded = Session::load(&catalog.root).unwrap();
    assert_eq!(loaded.manual_groups.len(), catalog.media.len());
    assert_eq!(loaded.scene_groups.len(), 2);
    let scene_names: Vec<_> = catalog.media.iter().take(2).map(|m| m.name()).collect();
    for m in after.media {
        assert!(loaded.manual_groups.contains(m.id));
        assert_eq!(
            loaded.scene_groups.contains(m.id),
            scene_names.contains(&m.name())
        );
        assert_eq!(
            hashes[&m.name()],
            operations::content_hash(&m.path, || false).unwrap()
        );
    }
}
#[test]
fn actual_file_moves_preserve_manual_membership_through_report_and_session_reload() {
    let dir = tempfile::tempdir().unwrap();
    for n in 0..3 {
        std::fs::write(dir.path().join(format!("{n}.jpg")), format!("photo {n}")).unwrap();
    }
    move_and_restore(Catalog::scan(dir.path(), || false, |_| {}).unwrap());
}
#[test]
#[ignore = "Uses the seven local original samples, copying them into a disposable library"]
fn seven_sample_copies_keep_groups_and_bytes_after_file_moves_and_restart() {
    let originals =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../PhotoSort/tests/samples");
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/metadata/reference.json")).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let mut hashes = HashMap::new();
    for case in fixture["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["sample"] == true)
    {
        let name = case["name"].as_str().unwrap();
        let source = originals.join(name);
        hashes.insert(
            source.clone(),
            operations::content_hash(&source, || false).unwrap(),
        );
        std::fs::copy(source, dir.path().join(name)).unwrap();
    }
    assert_eq!(hashes.len(), 7);
    move_and_restore(Catalog::scan(dir.path(), || false, |_| {}).unwrap());
    for (p, h) in hashes {
        assert_eq!(h, operations::content_hash(&p, || false).unwrap());
    }
}

#[test]
fn scene_and_subject_namespaces_roundtrip_independently_and_reject_changed_members() {
    let m = media(4);
    let session = Session {
        organize: None,
        manual_groups: grouped(&m, vec![MediaId(0), MediaId(1)]),
        scene_groups: grouped(&m, vec![MediaId(0), MediaId(2)]),
        fingerprints: m.iter().map(|m| (m.id, m.fingerprint)).collect(),
        ..Default::default()
    };
    let json = serde_json::to_value(&session).unwrap();
    let mut loaded: Session = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(loaded.manual_groups, session.manual_groups);
    assert_eq!(loaded.scene_groups, session.scene_groups);
    let mut old_format = json;
    old_format.as_object_mut().unwrap().remove("scene_groups");
    let old: Session = serde_json::from_value(old_format).unwrap();
    assert_eq!(old.manual_groups, session.manual_groups);
    assert!(old.scene_groups.is_empty());
    let mut changed = m;
    changed[0].fingerprint.identity[3] += 1;
    changed.pop();
    loaded.reconcile(&changed);
    assert_eq!(loaded.manual_groups.len(), 1);
    assert!(loaded.manual_groups.contains(MediaId(1)));
    assert_eq!(loaded.scene_groups.len(), 1);
    assert!(loaded.scene_groups.contains(MediaId(2)));
}

#[test]
fn both_namespaces_merge_and_relocate_without_joining_unrelated_cohorts() {
    let m = media(6);
    let mut destination = Session {
        organize: None,
        manual_groups: grouped(&m, vec![MediaId(0), MediaId(1)]),
        scene_groups: grouped(&m, vec![MediaId(0), MediaId(2)]),
        decisions: HashMap::from([(
            MediaId(0),
            Decision {
                rating: Some(1),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    destination.merge(Session {
        organize: None,
        manual_groups: grouped(&m, vec![MediaId(3), MediaId(4)]),
        scene_groups: grouped(&m, vec![MediaId(3), MediaId(5)]),
        decisions: HashMap::from([(
            MediaId(3),
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        )]),
        ..Default::default()
    });
    for groups in [&destination.manual_groups, &destination.scene_groups] {
        let resolved = groups.resolve(&HashMap::new(), || false).unwrap();
        assert_ne!(resolved[&MediaId(0)], resolved[&MediaId(3)]);
    }
    destination.apply_verified_identities(&[
        (MediaId(0), MediaId(3), m[3].fingerprint),
        (MediaId(3), MediaId(0), m[0].fingerprint),
    ]);
    assert_eq!(
        destination
            .decisions
            .get(&MediaId(3))
            .and_then(|d| d.rating),
        Some(1)
    );
    assert_eq!(
        destination
            .decisions
            .get(&MediaId(0))
            .and_then(|d| d.rating),
        Some(5)
    );
    assert_eq!(
        destination.fingerprints.get(&MediaId(3)),
        Some(&m[3].fingerprint)
    );
    assert_eq!(
        destination.fingerprints.get(&MediaId(0)),
        Some(&m[0].fingerprint)
    );
    let subjects = destination
        .manual_groups
        .resolve(&HashMap::new(), || false)
        .unwrap();
    let scenes = destination
        .scene_groups
        .resolve(&HashMap::new(), || false)
        .unwrap();
    assert_eq!(subjects[&MediaId(3)], subjects[&MediaId(1)]);
    assert_eq!(subjects[&MediaId(0)], subjects[&MediaId(4)]);
    assert_eq!(scenes[&MediaId(3)], scenes[&MediaId(2)]);
    assert_eq!(scenes[&MediaId(0)], scenes[&MediaId(5)]);
    assert!(!scenes.contains_key(&MediaId(1)));
    assert!(!subjects.contains_key(&MediaId(2)));
}

#[test]
fn identity_chains_and_in_place_updates_preserve_all_intent_and_verified_fingerprints() {
    let m = media(5);
    let mut session = Session {
        organize: None,
        suggested: Default::default(),
        pick_suggested: Default::default(),
        rotations_staged: Default::default(),
        manual_groups: grouped(&m, vec![MediaId(0), MediaId(1)]),
        scene_groups: grouped(&m, vec![MediaId(1), MediaId(2)]),
        decisions: m
            .iter()
            .enumerate()
            .map(|(i, m)| {
                (
                    m.id,
                    Decision {
                        rating: Some(i as u8 + 1),
                        ..Default::default()
                    },
                )
            })
            .collect(),
        fingerprints: m.iter().map(|m| (m.id, m.fingerprint)).collect(),
    };
    let changes: Vec<_> = [(0, 1), (1, 2), (2, 9), (3, 3)]
        .into_iter()
        .map(|(old, new)| {
            (
                MediaId(old),
                MediaId(new),
                Fingerprint {
                    identity: [2, new, 0, 0],
                    bytes: 99,
                    ..Default::default()
                },
            )
        })
        .collect();
    session.apply_verified_identities(&changes);
    let mut after = m.clone();
    for (i, (_, new, fingerprint)) in changes.iter().enumerate() {
        after[i].id = *new;
        after[i].fingerprint = *fingerprint;
    }
    session.reconcile(&after);
    assert_eq!(session.decisions.len(), 5);
    for (i, photo) in after.iter().enumerate() {
        assert_eq!(session.decisions[&photo.id].rating, Some(i as u8 + 1));
        assert_eq!(session.fingerprints[&photo.id], photo.fingerprint);
    }
    assert_eq!(session.manual_groups.len(), 2);
    assert!(
        session.manual_groups.contains(MediaId(1)) && session.manual_groups.contains(MediaId(2))
    );
    assert_eq!(session.scene_groups.len(), 2);
    assert!(session.scene_groups.contains(MediaId(2)) && session.scene_groups.contains(MediaId(9)));
}

#[test]
fn scene_resolution_excludes_missing_evidence_and_cancels_before_publication() {
    use photosort::core::manual_groups::{group_choices, resolve_scenes};
    let media: Vec<_> = (0..3)
        .map(|i| Media {
            id: MediaId(i),
            path: format!("{i}.jpg").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        })
        .collect();
    let saved = manual_groups::edit(
        1,
        Arc::default(),
        Arc::default(),
        &media,
        Edit::Move {
            selected: media.iter().map(|m| m.id).collect(),
            target: None,
        },
        || false,
    )
    .unwrap()
    .manual;
    let automatic = HashMap::from([(MediaId(0), 4), (MediaId(1), 8)]);
    let groups = resolve_scenes(&automatic, &saved, || false).unwrap();
    assert_eq!(groups.len(), 2);
    assert_eq!(groups[&MediaId(0)], groups[&MediaId(1)]);
    assert!(!groups.contains_key(&MediaId(2)));
    assert_eq!(
        group_choices(&groups, || false).unwrap(),
        vec![(9, 2, MediaId(0))]
    );
    assert!(resolve_scenes(&automatic, &saved, || true).is_err());
    assert!(group_choices(&groups, || true).is_err());
}
