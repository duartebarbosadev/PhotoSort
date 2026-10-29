#[path = "../src/test_support.rs"]
mod test_support;
use photosort::core::{
    catalog::Catalog,
    jobs::{Event, IoWork, Runtime},
    model::*,
    session::Session,
    views::{Bookmark, Saved},
};
use std::{
    collections::HashMap,
    fs,
    time::{Duration, Instant},
};
fn fixture() -> (tempfile::TempDir, Catalog, Saved) {
    let dir = tempfile::tempdir().unwrap();
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        fs::write(dir.path().join(name), name).unwrap();
    }
    let catalog = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    let ids: Vec<_> = catalog.media.iter().map(|m| m.id).collect();
    let view = Bookmark {
        active: Some(ids[1]),
        selected: ids.clone(),
        comparison: ids[..2].to_vec(),
        query: "jpg".into(),
        scroll: 42.0,
        ..Default::default()
    };
    let mut saved = Saved {
        step: Some(Step::Cull),
        bookmarks: [(Step::Cull, view)].into_iter().collect(),
        ..Default::default()
    };
    saved.capture_fingerprints(
        &catalog.media,
        &catalog
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect(),
    );
    (dir, catalog, saved)
}
#[test]
fn views_round_trip_separately_from_decisions_and_drain_across_navigation() {
    let (dir, catalog, saved) = fixture();
    let session = Session {
        suggested: Default::default(),
        pick_suggested: Default::default(),
        organize: None,
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: [(
            catalog.media[0].id,
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        fingerprints: catalog
            .media
            .iter()
            .map(|m| (m.id, m.fingerprint))
            .collect(),
    };
    session.save(dir.path()).unwrap();
    let mut runtime = Runtime::with_cache(|| {}, dir.path().join("cache"));
    runtime
        .io(IoWork::SaveViews {
            root: dir.path().into(),
            views: saved.clone(),
        })
        .unwrap();
    runtime.next_epoch();
    runtime.shutdown();
    assert_eq!(Saved::load(&dir.path().join(".")).unwrap(), saved);
    assert_eq!(
        Session::load(dir.path()).unwrap().decisions,
        session.decisions
    );
    let other = tempfile::tempdir().unwrap();
    assert_eq!(Saved::load(other.path()).unwrap(), Saved::default());
    assert!(
        !fs::read_to_string(Saved::path(dir.path()))
            .unwrap()
            .contains("decisions")
    );
}
#[test]
fn replaced_and_missing_files_do_not_inherit_focus_or_comparison() {
    let (dir, catalog, mut saved) = fixture();
    let photo = &catalog.media[1];
    let modified = fs::metadata(&photo.path).unwrap().modified().unwrap();
    let replacement = dir.path().join("replacement");
    fs::write(&replacement, b"other").unwrap(); // b.jpg is also five bytes.
    fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(modified))
        .unwrap();
    fs::rename(replacement, &photo.path).unwrap();
    fs::remove_file(&catalog.media[2].path).unwrap();
    let current = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    let fresh = current.media.iter().find(|m| m.id == photo.id).unwrap();
    assert_eq!(fresh.fingerprint.bytes, photo.fingerprint.bytes);
    assert_eq!(fresh.fingerprint.modified_ns, photo.fingerprint.modified_ns);
    #[cfg(unix)]
    assert_ne!(fresh.fingerprint.identity, photo.fingerprint.identity);
    saved.reconcile(&current.media);
    let view = &saved.bookmarks[&Step::Cull];
    // A file that disappeared drops out everywhere.
    assert!(!view.selected.contains(&catalog.media[2].id));
    // Telling the same-size, same-time replacement apart needs a file identity
    // that changes with the file. Windows has none from metadata alone: NTFS
    // hands a file renamed onto a just-vacated name the old creation time, and
    // its timestamps tick every ~15 ms.
    if cfg!(unix) {
        assert_eq!(view.active, None);
        assert!(view.missing_focus);
        assert_eq!(view.selected, vec![catalog.media[0].id]);
        assert!(view.comparison.is_empty());
    }
}
#[test]
fn relocation_uses_verified_report_fingerprints_without_reading_images() {
    let (_dir, catalog, mut saved) = fixture();
    let old = catalog.media[1].id;
    let new = MediaId(999);
    let fingerprint = Fingerprint {
        bytes: 9,
        ..Default::default()
    };
    saved.relocate(&[(old, new, fingerprint)]);
    assert_eq!(saved.bookmarks[&Step::Cull].active, Some(new));
    assert_eq!(saved.fingerprints[&new], fingerprint);
    assert!(!saved.fingerprints.contains_key(&old));
    let text = serde_json::to_string(&(1, &saved)).unwrap();
    assert!(!text.contains("keep") && !text.contains("confirmed"));
}
#[test]
fn corrupt_view_file_cannot_prevent_scanning_or_session_loading() {
    let (dir, catalog, _) = fixture();
    let path = Saved::path(&catalog.root);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"broken view").unwrap();
    let mut runtime = Runtime::with_cache(|| {}, dir.path().join("cache"));
    runtime.io(IoWork::Scan(catalog.root.clone())).unwrap();
    let until = Instant::now() + test_support::worker_patience();
    let mut warning = false;
    let mut opened = false;
    while Instant::now() < until && !opened {
        if let Ok(envelope) = runtime.events.recv_timeout(Duration::from_millis(100)) {
            match envelope.event {
                Event::ViewError(_) => warning = true,
                Event::Opened(c, session, views) => {
                    assert_eq!(c.media.len(), 3);
                    assert!(session.decisions.is_empty());
                    assert_eq!(views, Saved::default());
                    opened = true;
                }
                Event::Error(e) => panic!("unexpected scan failure: {e}"),
                _ => {}
            }
        }
    }
    runtime.shutdown();
    assert!(warning && opened);
}
#[test]
fn all_workflow_bookmarks_reconcile_from_one_catalog_snapshot() {
    let (_dir, catalog, mut saved) = fixture();
    for step in Step::ALL {
        saved
            .bookmarks
            .insert(step, saved.bookmarks[&Step::Cull].clone());
    }
    saved.capture_fingerprints(
        &catalog.media,
        &catalog
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect::<HashMap<_, _>>(),
    );
    // No filesystem access during reconciliation: the catalog snapshot remains authoritative.
    for media in &catalog.media {
        fs::remove_file(&media.path).unwrap();
    }
    saved.reconcile(&catalog.media);
    for view in saved.bookmarks.values() {
        assert_eq!(view.selected.len(), 3);
        assert_eq!(view.comparison.len(), 2);
    }
}

#[test]
fn older_decision_sessions_migrate_without_losing_user_choices() {
    let (_dir, catalog, _) = fixture();
    let m = &catalog.media[0];
    let mut old = m.fingerprint;
    old.identity = [0; 4];
    let mut session = Session {
        suggested: Default::default(),
        pick_suggested: Default::default(),
        organize: None,
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: [(
            m.id,
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        fingerprints: [(m.id, old)].into_iter().collect(),
    };
    session.reconcile(&catalog.media);
    assert_eq!(session.decisions[&m.id].rating, Some(5));
    assert_eq!(session.fingerprints[&m.id], m.fingerprint);
}

#[test]
fn group_order_preferences_round_trip_and_legacy_views_default_to_flat() {
    use photosort::core::browsing::{GroupSort, Options};
    let old =
        r#"{"rating":"All","group":null,"sort":"Oldest","folder":{"path":null,"recursive":true}}"#;
    assert_eq!(
        serde_json::from_str::<Options>(old).unwrap().group_sort,
        GroupSort::Flat
    );
    let view = Bookmark {
        browse: Options {
            group_sort: GroupSort::Similarity,
            ..Default::default()
        },
        ..Default::default()
    };
    let restored: Bookmark = serde_json::from_str(&serde_json::to_string(&view).unwrap()).unwrap();
    assert_eq!(restored.browse.group_sort, GroupSort::Similarity);
}

#[test]
fn destination_import_preserves_all_workflows_and_clears_source_inventory_state() {
    use photosort::core::views::EditorBookmark;
    let (dir, catalog, mut source) = fixture();
    let original = source.bookmarks[&Step::Cull].clone();
    for step in Step::ALL {
        let mut view = original.clone();
        view.browse.folder.path = Some(catalog.root.clone());
        view.browse.group = Some(900);
        view.group_anchor = view.active;
        view.editor = Some(EditorBookmark {
            selected: view.selected.clone(),
            folders: vec!["old".into()],
            folder: "old".into(),
            photo_scroll: 99.0,
            folder_scroll: 12.0,
            folder_anchor: Some("old".into()),
            compact_folders: true,
            collapsed: [photosort::core::browsing::tree::Key::Folder("old".into())].into(),
            ..Default::default()
        });
        source.bookmarks.insert(step, view);
    }
    let moves: Vec<_> = catalog
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, MediaId(i as u64 + 100), m.fingerprint))
        .collect();
    // Import uses receipts only: even absent files cannot trigger a rescan/decode.
    for m in &catalog.media {
        fs::remove_file(&m.path).unwrap();
    }
    let mut destination = Saved::default();
    assert!(destination.import_relocated(&source, &moves));
    assert_eq!(destination.step, Some(Step::Cull));
    for step in Step::ALL {
        let view = &destination.bookmarks[&step];
        assert_eq!(view.active, Some(MediaId(101)));
        assert_eq!(
            view.selected,
            vec![MediaId(100), MediaId(101), MediaId(102)]
        );
        assert_eq!(view.comparison, vec![MediaId(100), MediaId(101)]);
        assert_eq!(view.group_anchor, Some(MediaId(101)));
        assert!(view.browse.folder.path.is_none() && view.browse.group.is_none());
        assert_eq!(view.scroll, 0.0);
        let editor = view.editor.as_ref().unwrap();
        assert_eq!(editor.selected, view.selected);
        assert!(
            editor.folder.is_empty()
                && editor.folders.is_empty()
                && editor.collapsed.is_empty()
                && editor.folder_anchor.is_none()
        );
        assert!(editor.compact_folders);
    }
    destination.save(dir.path()).unwrap();
    assert_eq!(Saved::load(dir.path()).unwrap(), destination);
}

#[test]
fn partial_import_preserves_unrelated_destination_workflows_and_rejects_unverified_ids() {
    let (_dir, catalog, source) = fixture();
    let mut destination = Saved {
        step: Some(Step::Organize),
        bookmarks: [(
            Step::Organize,
            Bookmark {
                active: Some(MediaId(444)),
                ..Default::default()
            },
        )]
        .into_iter()
        .collect(),
        fingerprints: [(MediaId(444), Fingerprint::default())]
            .into_iter()
            .collect(),
    };
    let unaffected = destination.bookmarks[&Step::Organize].clone();
    let old = catalog.media[1].id;
    let fp = catalog.media[1].fingerprint;
    assert!(destination.import_relocated(&source, &[(old, MediaId(888), fp)]));
    assert_eq!(destination.bookmarks[&Step::Organize], unaffected);
    assert_eq!(
        destination.bookmarks[&Step::Cull].active,
        Some(MediaId(888))
    );
    assert_eq!(
        destination.bookmarks[&Step::Cull].selected,
        vec![MediaId(888)]
    );
    assert!(destination.bookmarks[&Step::Cull].comparison.is_empty());
    assert_eq!(destination.fingerprints.len(), 2);
    let previous = destination.clone();
    assert!(!destination.import_relocated(&source, &[(MediaId(999), MediaId(1000), fp)]));
    assert!(!destination.import_relocated(&source, &[(old, old, fp)]));
    assert_eq!(destination, previous);
    // A replaced arrival cannot inherit focus on the following catalog scan.
    destination.reconcile(&catalog.media);
    assert!(destination.bookmarks[&Step::Cull].active.is_none());
}

#[test]
fn report_import_scope_requires_completed_move_and_verified_destination() {
    use photosort::core::operations::{Action, Plan, Report, Source};
    let dir = tempfile::tempdir().unwrap();
    let output = dir.path().join("out");
    let inside = output.join("a.jpg");
    let sibling = dir.path().join("outside/b.jpg");
    let source = Source {
        path: dir.path().join("source.jpg"),
        fingerprint: Fingerprint::default(),
    };
    let mut report = Report {
        completed: vec![Action::Move {
            files: vec![
                (source.clone(), inside.clone()),
                (source.clone(), sibling.clone()),
            ],
        }],
        remaining: Plan {
            actions: vec![Action::Move {
                files: vec![(source, output.join("failed.jpg"))],
            }],
            ..Default::default()
        },
        relocated: vec![
            (
                MediaId(1),
                MediaId::for_path(&inside),
                Fingerprint::default(),
            ),
            (
                MediaId(2),
                MediaId::for_path(&sibling),
                Fingerprint::default(),
            ),
            (
                MediaId(3),
                MediaId::for_path(&output.join("failed.jpg")),
                Fingerprint::default(),
            ),
        ],
        ..Default::default()
    };
    assert_eq!(report.relocations_into(&output), vec![report.relocated[0]]);
    report.relocated.clear();
    assert!(report.relocations_into(&output).is_empty());
}

#[test]
#[ignore = "copies the seven PHOTOSORT_SAMPLE_DIR photos into disposable libraries"]
fn seven_samples_carry_workflow_bookmarks_into_destination_without_changing_bytes() {
    use photosort::core::operations::{
        Action, Plan, Source,
        recovery::{Context, Store},
    };
    let samples = std::path::PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap());
    let originals = Catalog::scan(&samples, || false, |_| {}).unwrap();
    assert_eq!(originals.media.len(), 7);
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let root = base.join("source");
    let output = base.join("destination");
    fs::create_dir(&root).unwrap();
    fs::create_dir(&output).unwrap();
    let mut hashes = HashMap::new();
    for m in &originals.media {
        hashes.insert(m.name(), blake3::hash(&fs::read(&m.path).unwrap()));
        fs::copy(&m.path, root.join(m.name())).unwrap();
    }
    fs::write(root.join("README"), b"supporting document").unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let extra = MediaId::for_path(&root.join("README"));
    let mut views = Saved {
        step: Some(Step::Organize),
        ..Default::default()
    };
    for step in Step::ALL {
        views.bookmarks.insert(
            step,
            Bookmark {
                active: Some(catalog.media[3].id),
                selected: catalog.media.iter().map(|m| m.id).collect(),
                comparison: catalog.media[..2].iter().map(|m| m.id).collect(),
                editor: Some(photosort::core::views::EditorBookmark {
                    selected: vec![extra, catalog.media[3].id],
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
    }
    views.capture_catalog_fingerprints(
        &catalog,
        &catalog
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect(),
    );
    let mut plan = Plan {
        actions: catalog
            .media
            .iter()
            .map(|m| Action::Move {
                files: vec![(
                    Source::capture(m.path.clone()).unwrap(),
                    output.join(m.name()),
                )],
            })
            .collect(),
        output_root: Some(output.clone()),
        ..Default::default()
    };
    plan.actions.push(Action::Move {
        files: vec![(
            Source::capture(root.join("README")).unwrap(),
            output.join("README"),
        )],
    });
    let expected = MediaId::for_path(&output.join(catalog.media[3].name()));
    let context = Context {
        root,
        rotation_only: false,
        media: catalog.media,
        session: Session::default(),
        views,
    };
    let report = Store::new(base.join("journals"))
        .execute(&context, &plan, None, || false, |_, _| {})
        .unwrap();
    assert_eq!(report.completed.len(), 8);
    assert!(report.errors.is_empty() && report.remaining.actions.is_empty());
    let mut destination = Saved::load(&output).unwrap();
    destination.reconcile_catalog(&Catalog::scan(&output, || false, |_| {}).unwrap());
    assert_eq!(destination.step, Some(Step::Organize));
    for step in Step::ALL {
        let view = &destination.bookmarks[&step];
        assert_eq!(view.active, Some(expected));
        assert_eq!(view.selected.len(), 7);
        assert_eq!(view.comparison.len(), 2);
        assert_eq!(
            view.editor.as_ref().unwrap().selected,
            vec![MediaId::for_path(&output.join("README")), expected]
        );
    }
    assert_eq!(
        fs::read(output.join("README")).unwrap(),
        b"supporting document"
    );
    for m in originals.media {
        assert_eq!(blake3::hash(&fs::read(&m.path).unwrap()), hashes[&m.name()]);
        assert_eq!(
            blake3::hash(&fs::read(output.join(m.name())).unwrap()),
            hashes[&m.name()]
        );
    }
}

#[test]
fn supporting_file_bookmarks_reuse_catalog_identity_without_becoming_review_media() {
    use photosort::core::views::EditorBookmark;
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("photos/sub")).unwrap();
    fs::create_dir(dir.path().join("unrelated")).unwrap();
    for name in [
        "photos/a.jpg",
        "photos/readme.txt",
        "photos/sub/LICENSE",
        "photos/a.xmp",
        "unrelated/note.txt",
    ] {
        fs::write(dir.path().join(name), b"data").unwrap();
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(
        dir.path().join("photos/readme.txt"),
        dir.path().join("photos/link.txt"),
    )
    .unwrap();
    let catalog = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    assert_eq!(catalog.media.len(), 1);
    assert_eq!(catalog.supporting_fingerprints.len(), 2);
    let extra = MediaId::for_path(&catalog.root.join("photos/readme.txt"));
    let nested = MediaId::for_path(&catalog.root.join("photos/sub/LICENSE"));
    let photo = catalog.media[0].id;
    let mut saved = Saved::default();
    for step in Step::ALL {
        saved.bookmarks.insert(
            step,
            Bookmark {
                active: Some(extra),
                selected: vec![photo, extra],
                comparison: vec![photo, extra],
                editor: Some(EditorBookmark {
                    selected: vec![photo, extra, nested],
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
    }
    // After the scan, all checkpoint and reconciliation work must use its snapshot.
    for path in &catalog.inventory {
        fs::remove_file(path).unwrap();
    }
    let index = catalog
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    saved.capture_catalog_fingerprints(&catalog, &index);
    for _ in 0..3 {
        saved.reconcile_catalog(&catalog);
        saved.capture_catalog_fingerprints(&catalog, &index);
    }
    assert_eq!(saved.fingerprints.len(), 3);
    for step in Step::ALL {
        let view = &saved.bookmarks[&step];
        assert_eq!(
            view.editor.as_ref().unwrap().selected,
            vec![photo, extra, nested]
        );
        assert!(view.active.is_none() && view.comparison.is_empty());
        assert_eq!(view.selected, vec![photo]);
    }
    saved.save(dir.path()).unwrap();
    assert_eq!(
        Saved::load(dir.path()).unwrap().fingerprints,
        saved.fingerprints
    );
}

#[test]
#[cfg_attr(
    windows,
    ignore = "NTFS tunnelling reuses the creation time of a file replaced under the same name \
              within 15 s; telling files apart there would need a handle per file on every \
              scan"
)]
fn replaced_supporting_file_does_not_inherit_selection_at_same_size_and_mtime() {
    use photosort::core::views::EditorBookmark;
    let (dir, _, mut saved) = fixture();
    let path = dir.path().join("notes.txt");
    fs::write(&path, b"first").unwrap();
    let catalog = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    let id = MediaId::for_path(&catalog.root.join("notes.txt"));
    saved.bookmarks.get_mut(&Step::Cull).unwrap().editor = Some(EditorBookmark {
        selected: vec![id],
        focused: Some(id),
        anchor: Some(id),
        ..Default::default()
    });
    saved.capture_catalog_fingerprints(
        &catalog,
        &catalog
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect(),
    );
    let before = fs::metadata(&path).unwrap().modified().unwrap();
    let replacement = dir.path().join("new.txt");
    fs::write(&replacement, b"other").unwrap();
    fs::File::options()
        .write(true)
        .open(&replacement)
        .unwrap()
        .set_times(fs::FileTimes::new().set_modified(before))
        .unwrap();
    fs::rename(&replacement, &path).unwrap();
    let fresh = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    assert_eq!(
        fresh.supporting_fingerprints[&id].bytes,
        catalog.supporting_fingerprints[&id].bytes
    );
    assert_eq!(
        fresh.supporting_fingerprints[&id].modified_ns,
        catalog.supporting_fingerprints[&id].modified_ns
    );
    #[cfg(unix)]
    assert_ne!(
        fresh.supporting_fingerprints[&id].identity,
        catalog.supporting_fingerprints[&id].identity
    );
    saved.reconcile_catalog(&fresh);
    let editor = saved.bookmarks[&Step::Cull].editor.as_ref().unwrap();
    assert!(editor.focused.is_none() && editor.anchor.is_none());
    assert!(
        saved.bookmarks[&Step::Cull]
            .editor
            .as_ref()
            .unwrap()
            .selected
            .is_empty()
    );
}

#[test]
fn supporting_identity_phase_honors_scan_cancellation() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.jpg"), b"photo").unwrap();
    fs::write(dir.path().join("README"), b"supporting file").unwrap();
    let stop = std::cell::Cell::new(false);
    let result = Catalog::scan(dir.path(), || stop.get(), |_| stop.set(true));
    assert!(result.is_err());
    assert!(fs::read(dir.path().join("README")).is_ok());
}

#[test]
fn restored_shared_and_independent_views_use_the_live_zoom_limits() {
    use photosort::core::inspection::View;
    let (_, catalog, mut saved) = fixture();
    let view = saved.bookmarks.get_mut(&Step::Cull).unwrap();
    view.view.zoom = 1e6;
    view.independent_views.insert(
        0,
        View {
            zoom: 1e-6,
            ..Default::default()
        },
    );
    saved.reconcile(&catalog.media);
    let view = &saved.bookmarks[&Step::Cull];
    assert_eq!(view.view.zoom, View::MAX_ZOOM);
    assert_eq!(view.independent_views[&0].zoom, View::MIN_ZOOM);
}

#[test]
fn file_keyboard_unselected_document_focus_and_anchor_follow_verified_identity() {
    use photosort::core::views::EditorBookmark;
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("photo.jpg"), b"photo").unwrap();
    fs::write(dir.path().join("notes.txt"), b"notes").unwrap();
    let catalog = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    let id = MediaId::for_path(&catalog.root.join("notes.txt"));
    let mut saved = Saved::default();
    saved.bookmarks.insert(
        Step::Organize,
        Bookmark {
            editor: Some(EditorBookmark {
                focused: Some(id),
                anchor: Some(id),
                ..Default::default()
            }),
            ..Default::default()
        },
    );
    saved.capture_catalog_fingerprints(
        &catalog,
        &catalog
            .media
            .iter()
            .enumerate()
            .map(|(i, m)| (m.id, i))
            .collect(),
    );
    saved.reconcile_catalog(&catalog);
    assert_eq!(
        saved.fingerprints.get(&id),
        catalog.supporting_fingerprints.get(&id)
    );
    saved.save(dir.path()).unwrap();
    let mut loaded = Saved::load(dir.path()).unwrap();
    let new = MediaId(123456789);
    loaded.relocate(&[(id, new, catalog.supporting_fingerprints[&id])]);
    let editor = loaded.bookmarks[&Step::Organize].editor.as_ref().unwrap();
    assert!(editor.selected.is_empty());
    assert_eq!((editor.focused, editor.anchor), (Some(new), Some(new)));
    fs::remove_file(catalog.root.join("notes.txt")).unwrap();
    saved.reconcile_catalog(&Catalog::scan(dir.path(), || false, |_| {}).unwrap());
    let editor = saved.bookmarks[&Step::Organize].editor.as_ref().unwrap();
    assert!(editor.focused.is_none() && editor.anchor.is_none());
}
