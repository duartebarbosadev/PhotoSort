use photosort::core::{
    catalog::Catalog,
    model::{Decision, MediaId},
    operations::{self, Action, Plan, TrashRequest},
    organize::{Draft, Intent},
    session::Session,
    views::GroupMode,
};
use std::{cell::Cell, collections::HashMap, fs};
fn fixture() -> (tempfile::TempDir, Catalog) {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "A/a.jpg",
        "A/a.ARW",
        "A/a.xmp",
        "A/child/notes.txt",
        "A/.hidden.txt",
        "A-1/b.jpg",
        "other.txt",
    ] {
        let p = dir.path().join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, name).unwrap();
    }
    fs::create_dir_all(dir.path().join("A/empty/deep")).unwrap();
    let c = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    (dir, c)
}
fn request(c: &Catalog) -> TrashRequest {
    TrashRequest {
        folders: vec![c.root.join("A")],
        ..Default::default()
    }
}
fn plan(c: &Catalog) -> Plan {
    operations::trash_plan(c, &request(c), true, || false).unwrap()
}
fn run(p: &Plan) -> operations::Report {
    let log = tempfile::tempdir().unwrap();
    operations::execute(p, log.path(), || false, |_, _| {}).unwrap()
}
fn session(c: &Catalog) -> Session {
    let labels = c
        .organize_files()
        .into_iter()
        .map(|(id, p)| {
            (
                id,
                p.parent()
                    .unwrap()
                    .strip_prefix(&c.root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned(),
            )
        })
        .collect();
    let mut draft = Draft::from_catalog(c, labels);
    draft.create_folder("", "Picks").unwrap();
    let kept = MediaId::for_path(&c.root.join("A-1/b.jpg"));
    draft.move_photos(&[kept], "Picks").unwrap();
    Session {
        organize: Intent::capture(
            c,
            HashMap::from([(GroupMode::Current, draft)]),
            HashMap::from([(kept, "Favourite".into())]),
            c.root.clone(),
        ),
        decisions: HashMap::from([(
            kept,
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        )]),
        fingerprints: c.media.iter().map(|m| (m.id, m.fingerprint)).collect(),
        ..Default::default()
    }
}
#[test]
fn folder_trash_deduplicates_roots_files_and_lists_the_complete_discovery_inventory() {
    let (_dir, c) = fixture();
    let mut r = request(&c);
    r.folders.extend([c.root.join("A/child"), c.root.join("A")]);
    r.files.push(MediaId::for_path(&c.root.join("A/a.jpg")));
    let p = operations::trash_plan(&c, &r, true, || false).unwrap();
    assert_eq!(p.actions.len(), 1);
    let Action::TrashDirectory(folder) = &p.actions[0] else {
        panic!()
    };
    assert_eq!(folder.inventory.files.len(), 5);
    assert_eq!(folder.inventory.directory_count(), 4);
    assert!(
        folder
            .inventory
            .files
            .iter()
            .all(|s| s.path.starts_with(c.root.join("A")))
    );
    assert!(
        folder
            .inventory
            .files
            .iter()
            .any(|s| s.path.ends_with(".hidden.txt"))
    );
    let json = serde_json::to_vec(&p).unwrap();
    let reopened: Plan = serde_json::from_slice(&json).unwrap();
    assert_eq!(reopened.actions[0].label(), p.actions[0].label());
    assert!(c.root.join("A/a.jpg").is_file());
}
#[test]
fn folder_trash_and_staged_decisions_share_companion_expansion_without_ratings() {
    let (_dir, c) = fixture();
    let id = MediaId::for_path(&c.root.join("A/a.jpg"));
    for companions in [false, true] {
        let explicit = operations::trash_plan(
            &c,
            &TrashRequest {
                files: vec![id],
                ..Default::default()
            },
            companions,
            || false,
        )
        .unwrap();
        let staged = operations::decisions_plan(
            &c,
            &HashMap::from([(
                id,
                Decision {
                    rejected: true,
                    rating: Some(5),
                    turns: 1,
                },
            )]),
            companions,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(explicit.actions).unwrap(),
            serde_json::to_value(staged.actions).unwrap()
        );
    }
}
#[test]
fn folder_trash_rejects_root_outside_missing_unknown_and_cancelled_requests() {
    let (_dir, c) = fixture();
    for folder in [
        c.root.clone(),
        c.root.parent().unwrap().to_path_buf(),
        c.root.join("missing"),
    ] {
        assert!(
            operations::trash_plan(
                &c,
                &TrashRequest {
                    folders: vec![folder],
                    ..Default::default()
                },
                true,
                || false
            )
            .is_err()
        );
    }
    assert!(
        operations::trash_plan(
            &c,
            &TrashRequest {
                files: vec![MediaId(u64::MAX)],
                ..Default::default()
            },
            true,
            || false
        )
        .is_err()
    );
    let calls = Cell::new(0);
    assert!(
        operations::trash_plan(&c, &request(&c), true, || {
            calls.set(calls.get() + 1);
            calls.get() > 6
        })
        .is_err()
    );
    assert!(c.root.join("A/a.jpg").exists());
}
#[test]
fn folder_trash_rejects_changed_discovery_and_review_contents_without_mutation() {
    for change in ["new", "removed", "replaced", "empty"] {
        let (_dir, c) = fixture();
        let p = plan(&c);
        match change {
            "new" => fs::write(c.root.join("A/new.txt"), b"unreviewed").unwrap(),
            "removed" => fs::remove_file(c.root.join("A/child/notes.txt")).unwrap(),
            "replaced" => {
                let old = c.root.join("A/a.jpg");
                fs::remove_file(&old).unwrap();
                fs::write(old, b"replacement").unwrap();
            }
            _ => fs::create_dir(c.root.join("A/new-empty")).unwrap(),
        }
        assert!(
            operations::trash_plan(&c, &request(&c), true, || false).is_err(),
            "{change}"
        );
        let report = run(&p);
        assert!(report.completed.is_empty(), "{change}");
        assert_eq!(report.remaining.actions.len(), 1);
        assert_eq!(report.errors.len(), 1);
        assert!(c.root.join("A").is_dir());
        assert!(c.root.join("A-1/b.jpg").is_file());
    }
}
#[cfg(unix)]
#[test]
fn folder_trash_rejects_symlinks_at_discovery_and_after_review() {
    for before in [false, true] {
        let (_dir, c) = fixture();
        let p = plan(&c);
        std::os::unix::fs::symlink(c.root.join("A-1"), c.root.join("A/link")).unwrap();
        let c = if before {
            Catalog::scan(&c.root, || false, |_| {}).unwrap()
        } else {
            c
        };
        assert!(operations::trash_plan(&c, &request(&c), true, || false).is_err());
        assert_eq!(run(&p).errors.len(), 1);
        assert!(c.root.join("A-1/b.jpg").is_file());
    }
}
#[test]
fn folder_trash_session_roundtrip_prunes_deleted_drafts_and_keeps_unrelated_edits() {
    let (_dir, c) = fixture();
    let original = session(&c);
    let context = operations::recovery::Context::capture_catalog(
        false,
        &c,
        &c.media.iter().enumerate().map(|(i, m)| (m.id, i)).collect(),
        original,
        Default::default(),
    );
    let saved = serde_json::to_vec(&context).unwrap();
    let mut reopened: operations::recovery::Context = serde_json::from_slice(&saved).unwrap();
    let p = plan(&c);
    // Model an executor completion without putting fixture files in the OS Trash.
    let report = operations::Report {
        completed: p.actions,
        ..Default::default()
    };
    reopened.session.apply_report(&report, &c.media);
    fs::remove_dir_all(c.root.join("A")).unwrap();
    let after = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    reopened.session.reconcile_catalog(&after);
    let kept = MediaId::for_path(&c.root.join("A-1/b.jpg"));
    let intent = reopened.session.organize.unwrap();
    assert_eq!(intent.names[&kept], "Favourite");
    let draft = &intent.drafts[&GroupMode::Current];
    assert_eq!(draft.labels().len(), 1);
    assert_eq!(draft.labels()[&kept], "Picks");
    assert!(!draft.folders().contains("A"));
    assert!(draft.folders().contains("Picks"));
    assert_eq!(reopened.session.decisions[&kept].rating, Some(5));
}
#[test]
fn folder_trash_draft_relocation_replacement_and_completed_organize_are_reconciled() {
    let (_dir, c) = fixture();
    let mut s = session(&c);
    let old = c.root.join("A-1/b.jpg");
    let to = c.root.join("A-1/moved.jpg");
    fs::rename(&old, &to).unwrap();
    let after = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    let m = after.media.iter().find(|m| m.path == to).unwrap();
    s.apply_verified_identities(&[(MediaId::for_path(&old), m.id, m.fingerprint)]);
    s.reconcile_catalog(&after);
    assert_eq!(s.organize.as_ref().unwrap().names[&m.id], "Favourite");
    fs::remove_file(&to).unwrap();
    fs::write(&to, b"replacement").unwrap();
    s.reconcile_catalog(&Catalog::scan(&c.root, || false, |_| {}).unwrap());
    assert!(s.organize.as_ref().unwrap().names.is_empty());
    s.apply_report(
        &operations::Report {
            remaining: Plan {
                output_root: Some(c.root.clone()),
                ..Default::default()
            },
            ..Default::default()
        },
        &c.media,
    );
    assert!(s.organize.is_none());
    let old_session: Session =
        serde_json::from_str(r#"{"decisions":{},"fingerprints":{}}"#).unwrap();
    assert!(old_session.organize.is_none());
}

#[test]
fn folder_trash_intent_capture_is_offline_and_new_files_stay_in_place() {
    let (_dir, c) = fixture();
    let mut saved = session(&c);
    let hidden = c.root.with_extension("offline");
    fs::rename(&c.root, &hidden).unwrap();
    let offline = session(&c);
    assert!(offline.organize.is_some());
    fs::rename(&hidden, &c.root).unwrap();
    fs::write(c.root.join("A-1/new.jpg"), b"new").unwrap();
    let after = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    saved.reconcile_catalog(&after);
    let intent = saved.organize.unwrap();
    let labels = intent.drafts[&GroupMode::Current].labels();
    assert_eq!(
        labels[&MediaId::for_path(&c.root.join("A-1/new.jpg"))],
        "A-1"
    );
    assert_eq!(
        labels[&MediaId::for_path(&c.root.join("A-1/b.jpg"))],
        "Picks"
    );
}

#[test]
fn folder_trash_replaced_empty_folder_never_inherits_a_persisted_rename() {
    let (_dir, c) = fixture();
    let labels = c
        .organize_files()
        .into_iter()
        .map(|(id, _)| (id, String::new()))
        .collect();
    let mut draft = Draft::from_catalog(&c, labels);
    draft.rename_folder("A/empty/deep", "DEEP").unwrap();
    let mut saved = Session {
        organize: Intent::capture(
            &c,
            HashMap::from([(GroupMode::Current, draft)]),
            HashMap::new(),
            c.root.clone(),
        ),
        ..Default::default()
    };
    let original = c.root.join("A/empty/deep");
    fs::rename(&original, c.root.join("original-empty")).unwrap();
    fs::create_dir(&original).unwrap();
    saved.reconcile_catalog(&Catalog::scan(&c.root, || false, |_| {}).unwrap());
    let draft = &saved.organize.as_ref().unwrap().drafts[&GroupMode::Current];
    assert!(
        !draft
            .directory_edits()
            .renamed
            .iter()
            .any(|(source, _)| source == "A/empty/deep")
    );
    assert!(
        operations::trash_plan(
            &c,
            &TrashRequest {
                folders: vec![original],
                ..Default::default()
            },
            true,
            || false
        )
        .is_err()
    );
}
