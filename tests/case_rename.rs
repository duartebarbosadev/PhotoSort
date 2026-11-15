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
    // A NAS thumbnail folder is not part of the review, and goes where its
    // folder goes.
    std::fs::create_dir_all(c.root.join("Trip/Day/@eaDir/a.jpg")).unwrap();
    std::fs::write(c.root.join("Trip/Day/@eaDir/a.jpg/thumb.jpg"), b"thumb").unwrap();
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
    assert!(c.root.join("TRIP/DAY/@eaDir/a.jpg/thumb.jpg").is_file());
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
/// The previous version scanned NAS thumbnail folders like any other, so a
/// folder plan it saved lists what they hold and has no `system_folders`. It
/// must still apply after the upgrade, while a plan reviewed with the list
/// accepts only the system folders that review named.
#[cfg(unix)]
#[test]
fn folder_plan_saved_before_system_folders_were_listed_still_applies() {
    use std::os::unix::fs::MetadataExt;
    for saved in [
        "before the list",
        "with a system folder added since",
        "with an empty list",
    ] {
        let (_dir, c, mut d) = fixture();
        let thumbnails = c.root.join("Trip/Day/@eaDir");
        std::fs::create_dir_all(thumbnails.join("a.jpg")).unwrap();
        std::fs::write(thumbnails.join("a.jpg/thumb.jpg"), b"thumb").unwrap();
        d.rename_folder("Trip", "TRIP").unwrap();
        let mut plan = serde_json::to_value(plan(&c, &d)).unwrap();
        let rename = &mut plan["actions"][0]["RenameDirectory"];
        assert_eq!(rename["system_folders"].as_array().unwrap().len(), 1);
        if saved == "with an empty list" {
            rename["system_folders"] = serde_json::json!([]);
        } else {
            // As the previous version wrote it: no list, and the thumbnail
            // folder's own content among the files and folders.
            rename.as_object_mut().unwrap().remove("system_folders");
            let file = operations::Source::capture(thumbnails.join("a.jpg/thumb.jpg")).unwrap();
            rename["files"]
                .as_array_mut()
                .unwrap()
                .push(serde_json::to_value(file).unwrap());
            for folder in [thumbnails.clone(), thumbnails.join("a.jpg")] {
                let meta = std::fs::metadata(&folder).unwrap();
                let nanoseconds = |time: std::time::SystemTime| {
                    time.duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos() as u64
                };
                rename["directories"]
                    .as_array_mut()
                    .unwrap()
                    .push(serde_json::json!([
                        folder,
                        {
                            "modified_ns": nanoseconds(meta.modified().unwrap()),
                            "created_ns": meta.created().ok().map(nanoseconds),
                            "identity": [meta.dev(), meta.ino()],
                        }
                    ]));
            }
        }
        if saved == "with a system folder added since" {
            std::fs::create_dir(c.root.join("Trip/Day/Empty/@eaDir")).unwrap();
        }
        let plan: Plan = serde_json::from_str(&plan.to_string()).unwrap();
        let report = run(&plan);
        if saved == "before the list" {
            assert!(report.errors.is_empty(), "{:?}", report.errors);
            assert!(spelling(&c.root, "TRIP") && !spelling(&c.root, "Trip"));
            assert!(c.root.join("TRIP/Day/@eaDir/a.jpg/thumb.jpg").is_file());
        } else {
            assert_eq!(report.errors.len(), 1, "{saved}");
            assert!(
                report.errors[0].contains("contents changed since review"),
                "{saved}: {:?}",
                report.errors
            );
            assert!(
                spelling(&c.root, "Trip") && report.completed.is_empty(),
                "{saved}"
            );
        }
    }
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

/// A People, Location or Camera label can spell an existing folder with other
/// capital letters. Where the volume treats them as one folder, the photos
/// already there stay as they are instead of becoming "x_1.jpg".
#[test]
fn a_label_in_other_capital_letters_leaves_the_photos_of_that_folder_in_place() {
    a_label_spelled_another_way("anna", "Anna", |c| c.names_ignore_case);
}

/// The same for accents: a typed or generated label has "é" as one character,
/// a folder copied from an older Mac volume has "e" and a combining accent.
/// A folder renamed on the board to the other form is respelled, as it is
/// for capital letters, and its photos keep their names.
#[test]
fn a_label_with_composed_accents_leaves_the_photos_of_a_decomposed_folder_in_place() {
    let (decomposed, composed) = ("Fe\u{301}rias", "F\u{e9}rias");
    a_label_spelled_another_way(decomposed, composed, |c| c.names_ignore_form);

    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(decomposed)).unwrap();
    std::fs::write(dir.path().join(decomposed).join("x.jpg"), b"x").unwrap();
    let c = catalog(dir.path());
    let mut d = draft(&c);
    d.rename_folder(decomposed, composed).unwrap();
    let p = plan(&c, &d);
    assert!(
        matches!(p.actions.as_slice(), [Action::RenameDirectory(_)]),
        "{:?}",
        p.actions
    );
    let r = run(&p);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(spelling(&c.root, composed) && !spelling(&c.root, decomposed));
    assert!(spelling(&c.root.join(composed), "x.jpg"));
}

/// `folder` holds a photo and `label` names it in another spelling. Where
/// the volume takes both for one folder (`one_folder`), nothing there moves.
fn a_label_spelled_another_way(folder: &str, label: &str, one_folder: fn(&Catalog) -> bool) {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        format!("{folder}/x.jpg"),
        format!("{folder}/x.xmp"),
        "inbox/y.jpg".to_owned(),
    ] {
        let path = dir.path().join(&name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, name.as_bytes()).unwrap();
    }
    let c = catalog(dir.path());
    let id = |name: &str| MediaId::for_path(&c.root.join(name));
    let here = HashMap::from([(id(&format!("{folder}/x.jpg")), label.to_string())]);
    let pending = operations::organize_changes_pending(
        &c,
        &here,
        &HashMap::new(),
        &c.root,
        &Default::default(),
    );
    let mut labels = here.clone();
    labels.insert(id("inbox/y.jpg"), label.to_string());
    let p = operations::organize_plan(&c, &labels, &c.root, true).unwrap();
    let moves: Vec<_> = p
        .actions
        .iter()
        .filter_map(|a| match a {
            Action::Move { files } => Some(files.clone()),
            _ => None,
        })
        .flatten()
        .map(|(source, to)| (source.path, to))
        .collect();
    if !one_folder(&c) {
        // Two folders on this volume: the photo does move to the new one.
        assert!(pending);
        assert!(moves.contains(&(
            c.root.join(folder).join("x.jpg"),
            c.root.join(label).join("x.jpg")
        )));
        return;
    }
    assert!(!pending);
    // Only the photo from elsewhere moves, into the folder as it is spelled.
    assert_eq!(
        moves,
        [(
            c.root.join("inbox/y.jpg"),
            c.root.join(folder).join("y.jpg")
        )]
    );
    let r = run(&p);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(spelling(&c.root, folder) && !spelling(&c.root, label));
    let mut names: Vec<_> = std::fs::read_dir(c.root.join(folder))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    assert_eq!(names, ["x.jpg", "x.xmp", "y.jpg"]);
    // Nothing is left to do, however often it is planned again.
    let again = catalog(dir.path());
    let labels: HashMap<_, _> = again
        .media
        .iter()
        .map(|m| (m.id, label.to_string()))
        .collect();
    let p = operations::organize_plan(&again, &labels, &again.root, true).unwrap();
    assert!(p.actions.is_empty(), "{:?}", p.actions);
}
