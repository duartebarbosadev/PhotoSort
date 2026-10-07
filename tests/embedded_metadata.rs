use photosort::core::{
    catalog::Catalog,
    metadata::{self, WritePolicy, WriteTarget},
    model::*,
    operations::{self, Action, Plan},
    pipeline::Pipeline,
};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};
fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata/mixed.jpg")
}
fn plan(catalog: &Catalog, decisions: &HashMap<MediaId, Decision>, policy: WritePolicy) -> Plan {
    let mut pipeline = Pipeline::new(16 * 1024 * 1024, catalog.root.join("cache"));
    operations::decisions_plan_with_metadata(
        catalog,
        decisions,
        false,
        policy,
        |m| pipeline.metadata(m),
        || false,
    )
    .unwrap()
}
#[test]
fn legacy_plans_remain_sidecar_only_and_policy_round_trips() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("photo.jpg");
    std::fs::copy(fixture(), &path).unwrap();
    let c = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let d = HashMap::from([(
        c.media[0].id,
        Decision {
            rating: Some(2),
            ..Default::default()
        },
    )]);
    let p = operations::decisions_plan(&c, &d, false).unwrap();
    let mut value = serde_json::to_value(&p).unwrap();
    value["actions"][0]["Metadata"]
        .as_object_mut()
        .unwrap()
        .remove("target");
    let restored: Plan = serde_json::from_value(value).unwrap();
    assert!(matches!(
        restored.actions[0],
        Action::Metadata {
            target: WriteTarget::Sidecar,
            ..
        }
    ));
    let prefs = photosort::core::preferences::Preferences {
        metadata_writes: WritePolicy::SidecarOnly,
        ..Default::default()
    };
    let file = temp.path().join("prefs.json");
    prefs.save(&file).unwrap();
    assert_eq!(
        photosort::core::preferences::Preferences::load(&file)
            .unwrap()
            .metadata_writes,
        WritePolicy::SidecarOnly
    );
}
#[test]
fn shared_sidecars_are_resolved_before_any_companion_edit_is_accepted() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["pair.jpg", "pair.png"] {
        std::fs::write(temp.path().join(name), b"fixture").unwrap();
    }
    let c = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let decisions = HashMap::from([
        (
            c.media[0].id,
            Decision {
                rating: Some(2),
                ..Default::default()
            },
        ),
        (
            c.media[1].id,
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        ),
    ]);
    let load = |_: &Media| {
        Ok(Metadata {
            embedded_metadata_writable: true,
            ..Default::default()
        })
    };
    let independent = operations::decisions_plan_with_metadata(
        &c,
        &decisions,
        false,
        WritePolicy::EmbedWhenSupported,
        load,
        || false,
    )
    .unwrap();
    assert_eq!(independent.actions.len(), 2);
    assert!(independent.actions.iter().all(|a| matches!(
        a,
        Action::Metadata {
            target: WriteTarget::Embedded,
            ..
        }
    )));
    let report = operations::Report {
        completed: vec![independent.actions[0].clone()],
        ..Default::default()
    };
    assert_eq!(report.affected_media(&c.media).len(), 1);
    metadata::write_sidecar(&c.media[0].path, Some(1), None).unwrap();
    assert!(
        operations::decisions_plan_with_metadata(
            &c,
            &decisions,
            false,
            WritePolicy::EmbedWhenSupported,
            load,
            || false
        )
        .is_err()
    );
    let single = HashMap::from([(c.media[0].id, decisions[&c.media[0].id].clone())]);
    let shared = operations::decisions_plan_with_metadata(
        &c,
        &single,
        false,
        WritePolicy::EmbedWhenSupported,
        load,
        || false,
    )
    .unwrap();
    assert!(matches!(
        shared.actions[0],
        Action::Metadata {
            target: WriteTarget::Sidecar,
            ..
        }
    ));
    let report = operations::Report {
        completed: shared.actions,
        ..Default::default()
    };
    assert_eq!(report.affected_media(&c.media).len(), 2);
    assert!(
        operations::decisions_plan_with_metadata(
            &c,
            &single,
            false,
            WritePolicy::EmbedWhenSupported,
            load,
            || true
        )
        .is_err()
    );
}
#[test]
fn unsupported_formats_and_minimal_builds_retain_reviewed_sidecar_fallback() {
    let temp = tempfile::tempdir().unwrap();
    image::RgbImage::new(12, 8)
        .save(temp.path().join("photo.bmp"))
        .unwrap();
    let c = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let source = &c.media[0];
    let before = std::fs::read(&source.path).unwrap();
    let p = plan(
        &c,
        &HashMap::from([(
            source.id,
            Decision {
                rating: Some(3),
                turns: 1,
                ..Default::default()
            },
        )]),
        WritePolicy::EmbedWhenSupported,
    );
    assert!(matches!(
        p.actions[0],
        Action::Metadata {
            target: WriteTarget::Sidecar,
            ..
        }
    ));
    let report =
        operations::execute(&p, &temp.path().join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(std::fs::read(&source.path).unwrap(), before);
    let updated = metadata::read(&source.path);
    assert_eq!((updated.rating, updated.orientation), (3, 6));
}
#[cfg(feature = "metadata-native")]
fn jpeg_scan(bytes: &[u8]) -> &[u8] {
    assert_eq!(&bytes[..2], &[0xff, 0xd8]);
    let mut p = 2;
    loop {
        assert_eq!(bytes[p], 0xff);
        let marker = bytes[p + 1];
        if marker == 0xda {
            return &bytes[p..];
        }
        let n = u16::from_be_bytes([bytes[p + 2], bytes[p + 3]]) as usize;
        p += 2 + n;
    }
}
#[cfg(feature = "metadata-native")]
#[test]
fn embedded_edits_preserve_jpeg_pixels_unrelated_metadata_and_hardlinked_sources() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("métadonnées.jpg");
    std::fs::copy(fixture(), &path).unwrap();
    let alias = temp.path().join("independent-copy.jpg");
    std::fs::hard_link(&path, &alias).unwrap();
    let before = std::fs::read(&path).unwrap();
    let meta = metadata::read(&path);
    assert!(meta.embedded_metadata_writable);
    let expected = Fingerprint::read(&path).unwrap();
    metadata::write_embedded(&path, expected, Some(5), Some(3)).unwrap();
    let after = std::fs::read(&path).unwrap();
    assert_ne!(before, after);
    assert_eq!(jpeg_scan(&before), jpeg_scan(&after));
    assert_eq!(std::fs::read(&alias).unwrap(), before);
    let updated = metadata::read(&path);
    assert_eq!((updated.rating, updated.orientation), (5, 3));
    assert_eq!(updated.camera, meta.camera);
    assert_eq!(updated.lens, meta.lens);
    assert_eq!(updated.captured, meta.captured);
    assert!(!metadata::sidecar(&path).exists());
    assert!(!temp.path().read_dir().unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".photosort-")
    }));
    metadata::write_embedded(&path, Fingerprint::read(&path).unwrap(), Some(0), None).unwrap();
    assert_eq!(metadata::read(&path).rating, 0);
}
#[cfg(feature = "metadata-native")]
#[test]
fn failed_or_stale_embedded_writes_preserve_sources_and_retry_plan() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("photo.jpg");
    std::fs::copy(fixture(), &path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let c = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let p = plan(
        &c,
        &HashMap::from([(
            c.media[0].id,
            Decision {
                rating: Some(2),
                ..Default::default()
            },
        )]),
        WritePolicy::EmbedWhenSupported,
    );
    assert!(matches!(
        p.actions[0],
        Action::Metadata {
            target: WriteTarget::Embedded,
            ..
        }
    ));
    metadata::write_sidecar(&path, Some(1), None).unwrap();
    let report =
        operations::execute(&p, &temp.path().join("journal"), || false, |_, _| {}).unwrap();
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.remaining.actions.len(), 1);
    assert!(report.completed.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(metadata::write_embedded(&path, c.media[0].fingerprint, Some(6), None).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let unsupported = temp.path().join("bad.bmp");
    image::RgbImage::new(8, 8).save(&unsupported).unwrap();
    let old = std::fs::read(&unsupported).unwrap();
    assert!(
        metadata::write_embedded(
            &unsupported,
            Fingerprint::read(&unsupported).unwrap(),
            Some(3),
            None
        )
        .is_err()
    );
    assert_eq!(std::fs::read(&unsupported).unwrap(), old);
    std::fs::write(&path, b"externally replaced").unwrap();
    assert!(metadata::write_embedded(&path, c.media[0].fingerprint, Some(3), None).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), b"externally replaced");
    assert!(!temp.path().read_dir().unwrap().any(|e| {
        e.unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with(".photosort-metadata-")
    }));
}
#[cfg(feature = "metadata-native")]
#[test]
fn image_actor_reuses_indexed_metadata_while_planning_embedded_edits() {
    use photosort::core::jobs::{Event, ImageWork, Runtime};
    use std::{
        sync::Arc,
        time::{Duration, Instant},
    };
    let temp = tempfile::tempdir().unwrap();
    std::fs::copy(fixture(), temp.path().join("photo.jpg")).unwrap();
    let c = Arc::new(Catalog::scan(temp.path(), || false, |_| {}).unwrap());
    let media = c.media[0].clone();
    let mut rt = Runtime::with_cache(|| {}, temp.path().join("cache"));
    rt.index_metadata(Arc::new(c.media.clone())).unwrap();
    let indexed_deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(Instant::now() < indexed_deadline);
        match rt.events.recv_timeout(Duration::from_millis(200)) {
            Ok(e) => match e.event {
                Event::MetadataBatch {
                    done, total, stats, ..
                } if done == total => {
                    assert_eq!((stats.metadata_reads, stats.decodes), (1, 0));
                    break;
                }
                Event::Error(e) => panic!("{e}"),
                _ => {}
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => panic!("{e}"),
        }
    }
    rt.image(ImageWork::PlanDecisions {
        catalog: c,
        decisions: HashMap::from([(
            media.id,
            Decision {
                rating: Some(2),
                ..Default::default()
            },
        )]),
        companions: false,
        policy: WritePolicy::EmbedWhenSupported,
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    let mut planned = false;
    loop {
        assert!(Instant::now() < deadline);
        match rt.events.recv_timeout(Duration::from_millis(200)) {
            Ok(e) => match e.event {
                Event::Planned(p) => {
                    assert!(matches!(
                        p.actions[0],
                        Action::Metadata {
                            target: WriteTarget::Embedded,
                            ..
                        }
                    ));
                    planned = true;
                    rt.image(ImageWork::Asset(media.clone(), true)).unwrap();
                }
                Event::Asset { stats, .. } => {
                    assert!(planned);
                    assert_eq!((stats.metadata_reads, stats.decodes), (1, 1));
                    break;
                }
                Event::Error(e) => panic!("{e}"),
                _ => {}
            },
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(e) => panic!("{e}"),
        }
    }
    rt.shutdown();
}
#[cfg(feature = "metadata-native")]
#[test]
#[ignore = "copies the seven reference samples and verifies native metadata edits and pixel preservation"]
fn seven_sample_copies_embed_ratings_and_rotations_without_changing_pixels() {
    let samples =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let temp = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, temp.path().join("cache"));
    for original in Catalog::scan(&samples, || false, |_| {}).unwrap().media {
        let original_bytes = std::fs::read(&original.path).unwrap();
        let path = temp.path().join(original.name());
        std::fs::copy(&original.path, &path).unwrap();
        let media = Media {
            id: MediaId::for_path(&path),
            fingerprint: Fingerprint::read(&path).unwrap(),
            path,
            kind: original.kind,
        };
        let before = pipeline.load(&media).unwrap();
        assert!(
            before.metadata.embedded_metadata_writable,
            "{}",
            media.name()
        );
        metadata::write_embedded(&media.path, media.fingerprint, Some(5), None).unwrap();
        let after = pipeline.load(&media).unwrap();
        assert_eq!(after.metadata.rating, 5);
        assert_eq!(
            after.pixels,
            before.pixels,
            "{} pixels changed",
            media.name()
        );
        assert_eq!(after.metadata.camera, before.metadata.camera);
        assert_eq!(after.metadata.orientation, before.metadata.orientation);
        let turned = metadata::rotate_orientation(before.metadata.orientation, 1);
        metadata::write_embedded(
            &media.path,
            Fingerprint::read(&media.path).unwrap(),
            None,
            Some(turned),
        )
        .unwrap();
        assert_eq!(metadata::read(&media.path).orientation, turned);
        metadata::write_embedded(
            &media.path,
            Fingerprint::read(&media.path).unwrap(),
            None,
            Some(before.metadata.orientation),
        )
        .unwrap();
        let restored = pipeline.load(&media).unwrap();
        assert_eq!(
            restored.pixels,
            before.pixels,
            "{} pixels changed after rotation round-trip",
            media.name()
        );
        assert_eq!(std::fs::read(&original.path).unwrap(), original_bytes);
        assert!(!metadata::sidecar(&media.path).exists());
        eprintln!(
            "{}: embedded rating and rotation round-trip verified, canonical pixels unchanged",
            media.name()
        );
    }
}

#[cfg(feature = "metadata-native")]
#[test]
fn embedded_rotation_preserves_unapplied_ratings_and_saved_focus_across_reload() {
    use photosort::core::{
        session::Session,
        views::{Bookmark, Saved},
    };
    let temp = tempfile::tempdir().unwrap();
    std::fs::copy(fixture(), temp.path().join("photo.jpg")).unwrap();
    let c = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let m = &c.media[0];
    let p = plan(
        &c,
        &HashMap::from([(
            m.id,
            Decision {
                turns: 1,
                ..Default::default()
            },
        )]),
        WritePolicy::EmbedWhenSupported,
    );
    let mut session = Session {
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: HashMap::from([(
            m.id,
            Decision {
                rating: Some(2),
                ..Default::default()
            },
        )]),
        fingerprints: HashMap::from([(m.id, m.fingerprint)]),
    };
    let mut views = Saved::default();
    views.bookmarks.insert(
        Step::Cull,
        Bookmark {
            active: Some(m.id),
            selected: vec![m.id],
            ..Default::default()
        },
    );
    views.fingerprints.insert(m.id, m.fingerprint);
    views
        .bookmarks
        .get_mut(&Step::Cull)
        .unwrap()
        .browse
        .folder
        .path = Some(temp.path().to_path_buf());
    let report =
        operations::execute(&p, &temp.path().join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.relocated.len(), 1);
    assert_eq!(report.relocated[0].0, report.relocated[0].1);
    session.apply_verified_identities(&report.relocated);
    views.relocate(&report.relocated);
    let refreshed = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    session.reconcile(&refreshed.media);
    views.reconcile(&refreshed.media);
    assert_eq!(session.decisions[&m.id].rating, Some(2));
    assert_eq!(views.bookmarks[&Step::Cull].active, Some(m.id));
    assert_eq!(views.bookmarks[&Step::Cull].selected, vec![m.id]);
    assert_eq!(
        views.bookmarks[&Step::Cull].browse.folder.path.as_deref(),
        Some(temp.path())
    );
    assert_eq!(metadata::read(&m.path).orientation, 3);
}

#[cfg(all(feature = "metadata-native", unix))]
#[test]
fn embedded_updates_preserve_permissions_and_refuse_symlink_sources() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("photo.jpg");
    std::fs::copy(fixture(), &path).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    metadata::write_embedded(&path, Fingerprint::read(&path).unwrap(), Some(3), None).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o640
    );
    let link = temp.path().join("link.jpg");
    symlink(&path, &link).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(
        metadata::write_embedded(&link, Fingerprint::read(&path).unwrap(), Some(1), None).is_err()
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
}
