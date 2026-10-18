use photosort::core::{
    catalog::Catalog,
    manual_groups::{self, Edit},
    model::{Decision, Fingerprint, MediaId},
    operations::{self, Action, Plan},
    organize::{self, Draft},
    session::Session,
};
use std::{
    collections::HashMap,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
fn catalog(root: &Path) -> Catalog {
    Catalog::scan(root, || false, |_| {}).unwrap()
}
fn draft(c: &Catalog) -> Draft {
    let mut labels = c
        .media
        .iter()
        .map(|m| {
            (
                m.id,
                m.path
                    .parent()
                    .unwrap()
                    .strip_prefix(&c.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
        })
        .collect();
    organize::include_inventory(c, &mut labels);
    Draft::from_catalog(c, labels)
}
fn plan(c: &Catalog, d: &Draft) -> Plan {
    operations::organize_plan_with_layout(
        c,
        d.labels(),
        &HashMap::new(),
        &c.root,
        true,
        &d.directory_edits(),
    )
    .unwrap()
}
fn run(p: &Plan) -> operations::Report {
    let journal = tempfile::tempdir().unwrap();
    operations::execute(p, journal.path(), || false, |_, _| {}).unwrap()
}
fn spelling(path: &Path, name: &str) -> bool {
    std::fs::read_dir(path)
        .unwrap()
        .any(|entry| entry.unwrap().file_name() == name)
}
fn fixture() -> (tempfile::TempDir, Catalog, Draft) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("Trip/Day/Empty")).unwrap();
    std::fs::create_dir(dir.path().join("Other")).unwrap();
    for name in ["a.jpg", "a.arw", "a.xmp", "notes"] {
        std::fs::write(dir.path().join("Trip/Day").join(name), name.as_bytes()).unwrap();
    }
    let c = catalog(dir.path());
    let d = draft(&c);
    (dir, c, d)
}
#[test]
fn nested_case_renames_preserve_names_bytes_identities_and_manual_groups() {
    let (_dir, c, mut d) = fixture();
    d.rename_folder("Trip", "TRIP").unwrap();
    d.rename_folder("TRIP/Day", "DAY").unwrap();
    let p = plan(&c, &d);
    assert_eq!(p.actions.len(), 2);
    assert!(
        p.actions
            .iter()
            .all(|a| matches!(a, Action::RenameDirectory(_)))
    );
    let p: Plan = serde_json::from_slice(&serde_json::to_vec(&p).unwrap()).unwrap();
    let mut session = Session::default();
    session.reconcile(&c.media);
    let ids = c.media.iter().map(|m| m.id).collect::<Vec<_>>();
    session.manual_groups = manual_groups::edit(
        1,
        Arc::default(),
        Arc::default(),
        &c.media,
        Edit::Move {
            selected: ids.clone(),
            target: None,
        },
        || false,
    )
    .unwrap()
    .manual;
    session.decisions.insert(
        ids[0],
        Decision {
            rating: Some(5),
            ..Default::default()
        },
    );
    let report = run(&p);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(spelling(&c.root, "TRIP") && !spelling(&c.root, "Trip"));
    assert!(spelling(&c.root.join("TRIP"), "DAY"));
    assert!(c.root.join("TRIP/DAY/Empty").is_dir());
    assert_eq!(
        report.relocated.len(),
        c.inventory.len(),
        "Collapse intermediate identities"
    );
    session.apply_report(&report, &c.media);
    let refreshed = catalog(&c.root);
    session.reconcile(&refreshed.media);
    assert_eq!(session.decisions.values().next().unwrap().rating, Some(5));
    assert_eq!(session.manual_groups.len(), ids.len());
    for original in &c.inventory {
        let target = c.root.join("TRIP/DAY").join(original.file_name().unwrap());
        assert_eq!(
            std::fs::read(&target).unwrap(),
            original.file_name().unwrap().as_encoded_bytes()
        );
        let old = report
            .relocated
            .iter()
            .find(|(old, _, _)| *old == MediaId::for_path(original))
            .unwrap();
        assert_eq!(old.1, MediaId::for_path(&target));
    }
    for m in &c.media {
        assert_eq!(
            m.fingerprint,
            Fingerprint::read(&c.root.join("TRIP/DAY").join(m.path.file_name().unwrap())).unwrap(),
            "No file copy/re-encode"
        );
    }
    assert!(plan(&refreshed, &draft(&refreshed)).actions.is_empty());
}
#[test]
fn case_rename_then_companion_move_retains_final_review_identity() {
    let (_dir, c, mut d) = fixture();
    let media = &c.media[0];
    d.rename_folder("Trip", "TRIP").unwrap();
    d.move_selection(
        &organize::Selection {
            files: [media.id].into(),
            ..Default::default()
        },
        "Other",
        &c.media,
        true,
    )
    .unwrap();
    let p = plan(&c, &d);
    let report = run(&p);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let new_id = MediaId::for_path(&c.root.join("Other").join(media.path.file_name().unwrap()));
    assert!(
        report
            .relocated
            .iter()
            .any(|(old, new, _)| *old == media.id && *new == new_id)
    );
    for name in ["a.jpg", "a.arw", "a.xmp"] {
        assert!(c.root.join("Other").join(name).is_file());
    }
    assert!(c.root.join("TRIP/Day/notes").is_file());
}
#[test]
fn cancellation_between_renames_keeps_an_exact_retry_plan() {
    let (_dir, c, mut d) = fixture();
    d.rename_folder("Trip", "TRIP").unwrap();
    d.rename_folder("TRIP/Day", "DAY").unwrap();
    let p = plan(&c, &d);
    let stop = AtomicBool::new(false);
    let journals = tempfile::tempdir().unwrap();
    let first = operations::execute(
        &p,
        journals.path(),
        || stop.load(Ordering::SeqCst),
        |done, _| {
            if done == 1 {
                stop.store(true, Ordering::SeqCst);
            }
        },
    )
    .unwrap();
    assert!(first.cancelled && first.errors.is_empty());
    assert_eq!(first.completed.len(), 1);
    assert_eq!(first.remaining.actions.len(), 1);
    assert!(spelling(&c.root.join("TRIP"), "Day"));
    let second = run(&first.remaining);
    assert!(second.errors.is_empty(), "{:?}", second.errors);
    assert!(spelling(&c.root.join("TRIP"), "DAY"));
}
#[test]
fn changed_file_or_tree_stops_rename_and_all_dependent_moves() {
    for extra in [false, true] {
        let (_dir, c, mut d) = fixture();
        d.rename_folder("Trip", "TRIP").unwrap();
        d.move_photos(&[c.media[0].id], "Other").unwrap();
        let p = plan(&c, &d);
        if extra {
            std::fs::write(c.root.join("Trip/new.txt"), b"new").unwrap();
        } else {
            std::fs::write(&c.media[0].path, b"changed").unwrap();
        }
        let report = run(&p);
        assert_eq!(report.errors.len(), 1);
        assert!(report.completed.is_empty());
        assert_eq!(report.remaining.actions.len(), p.actions.len());
        assert!(spelling(&c.root, "Trip"));
        assert_eq!(std::fs::read_dir(c.root.join("Other")).unwrap().count(), 0);
    }
}
#[test]
fn empty_folder_spelling_and_reverted_draft_are_handled_without_file_moves() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("empty/nested")).unwrap();
    let c = catalog(temp.path());
    let mut d = draft(&c);
    d.rename_folder("empty", "EMPTY").unwrap();
    assert!(operations::organize_changes_pending(
        &c,
        d.labels(),
        &HashMap::new(),
        &c.root,
        &d.directory_edits()
    ));
    let p = plan(&c, &d);
    assert_eq!(p.actions.len(), 1);
    d.rename_folder("EMPTY", "empty").unwrap();
    assert!(plan(&c, &d).actions.is_empty());
    let report = run(&p);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(spelling(&c.root, "EMPTY"));
    assert!(c.root.join("EMPTY/nested").is_dir());
}
#[cfg(unix)]
#[test]
fn symlink_inside_renamed_tree_is_never_moved() {
    let (_dir, c, mut d) = fixture();
    d.rename_folder("Trip", "TRIP").unwrap();
    let p = plan(&c, &d);
    std::os::unix::fs::symlink(&c.root, c.root.join("Trip/link")).unwrap();
    let report = run(&p);
    assert_eq!(report.errors.len(), 1);
    assert!(report.completed.is_empty());
    assert!(spelling(&c.root, "Trip"));
}
#[test]
fn output_library_keeps_source_folder_spelling() {
    let (_dir, c, mut d) = fixture();
    let output = tempfile::tempdir().unwrap();
    d.rename_folder("Trip", "TRIP").unwrap();
    let p = operations::organize_plan_with_layout(
        &c,
        d.labels(),
        &HashMap::new(),
        output.path(),
        true,
        &d.directory_edits(),
    )
    .unwrap();
    assert!(
        !p.actions
            .iter()
            .any(|a| matches!(a, Action::RenameDirectory(_)))
    );
    let r = run(&p);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(output.path().join("TRIP/Day/a.jpg").is_file());
}
#[test]
#[ignore = "copies all seven original photo samples into a disposable library"]
fn native_seven_samples_keep_content_and_identity_through_case_renames() {
    let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/samples");
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("Samples/Set")).unwrap();
    let reference: serde_json::Value = serde_json::from_slice(
        &std::fs::read(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata/reference.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let mut count = 0;
    for case in reference["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["sample"] == true)
    {
        let name = case["name"].as_str().unwrap();
        std::fs::copy(
            samples.join(name),
            temp.path().join("Samples/Set").join(name),
        )
        .unwrap();
        count += 1;
    }
    assert_eq!(count, 7);
    let c = catalog(temp.path());
    let before: HashMap<_, _> = c
        .media
        .iter()
        .map(|m| {
            (
                m.path.file_name().unwrap().to_owned(),
                (
                    m.fingerprint,
                    operations::content_hash(&m.path, || false).unwrap(),
                ),
            )
        })
        .collect();
    let mut d = draft(&c);
    d.rename_folder("Samples", "SAMPLES").unwrap();
    d.rename_folder("SAMPLES/Set", "SET").unwrap();
    let report = run(&plan(&c, &d));
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.completed.len(), 2);
    let current = catalog(&c.root);
    assert_eq!(current.media.len(), 7);
    for m in &current.media {
        let (fingerprint, hash) = &before[m.path.file_name().unwrap()];
        assert_eq!(*fingerprint, m.fingerprint);
        assert_eq!(*hash, operations::content_hash(&m.path, || false).unwrap());
        assert_eq!(
            *hash,
            operations::content_hash(&samples.join(m.path.file_name().unwrap()), || false).unwrap()
        );
    }
}

#[test]
fn exclusive_rename_refuses_a_distinct_occupied_destination() {
    let (_dir, c, mut d) = fixture();
    d.rename_folder("Trip", "TRIP").unwrap();
    let mut p = plan(&c, &d);
    let Action::RenameDirectory(rename) = &mut p.actions[0] else {
        panic!()
    };
    rename.to = c.root.join("Other");
    let report = run(&p);
    assert_eq!(report.errors.len(), 1);
    assert!(report.completed.is_empty());
    assert!(c.root.join("Other").is_dir());
    assert_eq!(std::fs::read_dir(c.root.join("Other")).unwrap().count(), 0);
    assert!(spelling(&c.root, "Trip"));
}
#[test]
fn planning_and_mutation_preflight_cancel_without_partial_rename() {
    use std::cell::Cell;
    let (_dir, c, mut d) = fixture();
    d.rename_folder("Trip", "TRIP").unwrap();
    let calls = Cell::new(0);
    let cancelled = || {
        calls.set(calls.get() + 1);
        calls.get() >= 4
    };
    assert!(
        operations::organize_plan_with_layout_cancellable(
            &c,
            d.labels(),
            &HashMap::new(),
            &c.root,
            true,
            &d.directory_edits(),
            cancelled
        )
        .is_err()
    );
    assert!(spelling(&c.root, "Trip"));
    let p = plan(&c, &d);
    calls.set(0);
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&p, journal.path(), cancelled, |_, _| {}).unwrap();
    assert!(report.cancelled && report.completed.is_empty() && report.errors.is_empty());
    assert_eq!(report.remaining.actions.len(), p.actions.len());
    assert!(spelling(&c.root, "Trip"));
    let retry = run(&report.remaining);
    assert!(retry.errors.is_empty());
}

#[test]
fn future_folder_spelling_reserves_existing_filenames_before_later_moves() {
    let (_dir, original, _) = fixture();
    std::fs::write(original.root.join("Other/a.jpg"), b"incoming").unwrap();
    let c = catalog(&original.root);
    let mut d = draft(&c);
    let incoming = MediaId::for_path(&c.root.join("Other/a.jpg"));
    d.rename_folder("Trip", "TRIP").unwrap();
    d.move_photos(&[incoming], "TRIP/Day").unwrap();
    let p = plan(&c, &d);
    assert!(p.actions.iter().any(|a| matches!(a, Action::Move { files } if files.iter().any(|(_, to)| to.ends_with("TRIP/Day/a_1.jpg")))));
    let report = run(&p);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(
        std::fs::read(c.root.join("TRIP/Day/a.jpg")).unwrap(),
        b"a.jpg"
    );
    assert_eq!(
        std::fs::read(c.root.join("TRIP/Day/a_1.jpg")).unwrap(),
        b"incoming"
    );
}
