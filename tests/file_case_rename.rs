use photosort::core::{
    catalog::Catalog,
    model::{Decision, Fingerprint, MediaId},
    operations::{self, Action, Plan, Source, recovery},
    organize::{self, Draft},
    session::Session,
    views,
};
use std::{cell::Cell, collections::HashMap, fs, path::Path};
fn fixture(root: &Path) -> Catalog {
    for name in ["pair.ARW", "pair.JPG", "pair.xmp", "notes.txt", ".hidden"] {
        fs::write(root.join(name), name.as_bytes()).unwrap();
    }
    Catalog::scan(root, || false, |_| {}).unwrap()
}
fn plan(c: &Catalog, names: HashMap<MediaId, String>) -> Plan {
    let labels = c
        .organize_files()
        .into_iter()
        .map(|(id, _)| (id, String::new()))
        .collect();
    operations::organize_plan_with_names(c, &labels, &names, &c.root, true).unwrap()
}
fn spelling(root: &Path, name: &str) -> bool {
    fs::read_dir(root)
        .unwrap()
        .any(|entry| entry.unwrap().file_name() == name)
}
fn context(c: &Catalog, session: Session) -> recovery::Context {
    recovery::Context {
        root: c.root.clone(),
        rotation_only: false,
        media: c.media.clone(),
        session,
        views: views::Saved::load(&c.root).unwrap_or_default(),
    }
}
#[test]
fn case_only_companions_and_documents_keep_exact_names_bytes_and_identities() {
    let temp = tempfile::tempdir().unwrap();
    let c = fixture(temp.path());
    let mut names = HashMap::new();
    for (from, to) in [
        ("pair.JPG", "Pair"),
        ("notes.txt", "NOTES"),
        (".hidden", ".HIDDEN"),
    ] {
        organize::set_file_name(
            &c,
            &mut names,
            MediaId::for_path(&c.root.join(from)),
            to,
            true,
        )
        .unwrap();
    }
    let plan = plan(&c, names);
    assert_eq!(plan.actions.len(), 5);
    assert!(
        plan.actions
            .iter()
            .all(|a| matches!(a, Action::RenameFile { .. }))
    );
    let plan: Plan = serde_json::from_slice(&serde_json::to_vec(&plan).unwrap()).unwrap();
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.relocated.len(), 5);
    assert_eq!(report.relocations_into(&c.root).len(), 5);
    assert_eq!(report.affected_media(&c.media).len(), 5);
    for action in &plan.actions {
        let Action::RenameFile { source, to } = action else {
            unreachable!()
        };
        assert!(!spelling(
            &c.root,
            source.path.file_name().unwrap().to_str().unwrap()
        ));
        assert!(spelling(&c.root, to.file_name().unwrap().to_str().unwrap()));
        assert_eq!(
            fs::read(to).unwrap(),
            source.path.file_name().unwrap().as_encoded_bytes()
        );
        assert_eq!(Fingerprint::read(to).unwrap(), source.fingerprint);
    }
}
#[test]
fn source_changes_symlinks_and_already_changed_spelling_are_refused() {
    for changed_spelling in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let c = fixture(temp.path());
        let mut p = plan(
            &c,
            HashMap::from([(MediaId::for_path(&c.root.join("notes.txt")), "Notes".into())]),
        );
        assert_eq!(p.actions.len(), 1);
        if changed_spelling {
            fs::rename(c.root.join("notes.txt"), c.root.join("Notes.txt")).unwrap();
        } else {
            fs::write(c.root.join("notes.txt"), b"new content after review").unwrap();
        }
        let journal = tempfile::tempdir().unwrap();
        let report = operations::execute(&p, journal.path(), || false, |_, _| {}).unwrap();
        assert_eq!(report.errors.len(), 1);
        assert!(report.completed.is_empty() && report.relocated.is_empty());
        assert_eq!(report.remaining.actions.len(), 1);
        let file = if changed_spelling {
            "Notes.txt"
        } else {
            "notes.txt"
        };
        assert_eq!(
            fs::read(c.root.join(file)).unwrap(),
            if changed_spelling {
                b"notes.txt".as_slice()
            } else {
                b"new content after review".as_slice()
            }
        );
        // The dedicated operation cannot be repurposed as an arbitrary move.
        p.actions = vec![Action::RenameFile {
            source: Source::capture(c.root.join(file)).unwrap(),
            to: c.root.join("pair.JPG"),
        }];
        let report = operations::execute(&p, journal.path(), || false, |_, _| {}).unwrap();
        assert_eq!(report.errors.len(), 1);
        assert_eq!(fs::read(c.root.join("pair.JPG")).unwrap(), b"pair.JPG");
    }
    #[cfg(unix)]
    {
        let temp = tempfile::tempdir().unwrap();
        let c = fixture(temp.path());
        let p = plan(
            &c,
            HashMap::from([(MediaId::for_path(&c.root.join("notes.txt")), "Notes".into())]),
        );
        fs::remove_file(c.root.join("notes.txt")).unwrap();
        std::os::unix::fs::symlink(c.root.join("pair.JPG"), c.root.join("notes.txt")).unwrap();
        let journal = tempfile::tempdir().unwrap();
        let report = operations::execute(&p, journal.path(), || false, |_, _| {}).unwrap();
        assert_eq!(report.errors.len(), 1);
        assert_eq!(fs::read(c.root.join("pair.JPG")).unwrap(), b"pair.JPG");
    }
}
#[test]
fn cancelled_and_interrupted_companion_renames_restore_only_unfinished_files() {
    for interrupted in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let c = fixture(temp.path());
        let id = MediaId::for_path(&c.root.join("pair.JPG"));
        let p = plan(&c, HashMap::from([(id, "Pair".into())]));
        let journal = tempfile::tempdir().unwrap();
        let store = recovery::Store::new(journal.path().canonicalize().unwrap());
        let mut session = Session::default();
        session.reconcile(&c.media);
        session.decisions.insert(
            id,
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        let ctx = context(&c, session);
        let stop = Cell::new(false);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            store.execute(
                &ctx,
                &p,
                None,
                || stop.get(),
                |_, _| {
                    if interrupted {
                        panic!("simulated interruption after durable receipt");
                    }
                    stop.set(true);
                },
            )
        }));
        if interrupted {
            assert!(result.is_err());
        } else {
            let report = result.unwrap().unwrap();
            assert!(report.cancelled);
            assert_eq!(report.completed.len(), 1);
        }
        let restarted = recovery::Store::new(journal.path().canonicalize().unwrap());
        let restored = restarted.restore(&c.root, &|| false).unwrap().unwrap();
        assert_eq!(restored.uncertain, 0);
        assert_eq!(restored.plan.actions.len(), 2);
        assert_eq!(
            serde_json::to_value(&restored.plan.actions).unwrap(),
            serde_json::to_value(&p.actions[1..]).unwrap()
        );
        let refreshed = Catalog::scan(&c.root, || false, |_| {}).unwrap();
        let ctx = context(&refreshed, Session::load(&c.root).unwrap());
        let report = restarted
            .execute(
                &ctx,
                &restored.plan,
                Some(&restored.ticket),
                || false,
                |_, _| {},
            )
            .unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert_eq!(report.completed.len(), 2);
        assert!(restarted.restore(&c.root, &|| false).unwrap().is_none());
        let final_catalog = Catalog::scan(&c.root, || false, |_| {}).unwrap();
        let mut saved = Session::load(&c.root).unwrap();
        saved.reconcile(&final_catalog.media);
        assert_eq!(
            saved.decisions[&MediaId::for_path(&c.root.join("Pair.JPG"))].rating,
            Some(4)
        );
        for ext in ["ARW", "JPG", "xmp"] {
            assert!(spelling(&c.root, &format!("Pair.{ext}")));
            assert!(!spelling(&c.root, &format!("pair.{ext}")));
            assert_eq!(
                fs::read(c.root.join(format!("Pair.{ext}"))).unwrap(),
                format!("pair.{ext}").as_bytes()
            );
        }
    }
}
#[test]
fn folder_and_file_case_changes_share_rebased_sources_and_collapsed_identities() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("Trip")).unwrap();
    for ext in ["ARW", "JPG", "xmp"] {
        fs::write(temp.path().join(format!("Trip/pair.{ext}")), ext).unwrap();
    }
    let c = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let labels = c
        .organize_files()
        .into_iter()
        .map(|(id, _)| (id, "Trip".into()))
        .collect();
    let mut draft = Draft::from_catalog(&c, labels);
    draft.rename_folder("Trip", "TRIP").unwrap();
    let names = HashMap::from([(
        MediaId::for_path(&c.root.join("Trip/pair.JPG")),
        "Pair".into(),
    )]);
    let p = operations::organize_plan_with_layout(
        &c,
        draft.labels(),
        &names,
        &c.root,
        true,
        &draft.directory_edits(),
    )
    .unwrap();
    assert_eq!(p.actions.len(), 4);
    assert!(matches!(p.actions[0], Action::RenameDirectory(_)));
    assert!(
        p.actions[1..]
            .iter()
            .all(|a| matches!(a, Action::RenameFile { .. }))
    );
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&p, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert_eq!(report.relocated.len(), 3);
    for ext in ["ARW", "JPG", "xmp"] {
        let from = c.root.join(format!("Trip/pair.{ext}"));
        let to = c.root.join(format!("TRIP/Pair.{ext}"));
        assert!(spelling(&c.root.join("TRIP"), &format!("Pair.{ext}")));
        assert_eq!(fs::read(&to).unwrap(), ext.as_bytes());
        assert!(report.relocated.iter().any(
            |(old, new, _)| *old == MediaId::for_path(&from) && *new == MediaId::for_path(&to)
        ));
    }
}
#[test]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR; case-renames seven disposable copies"]
fn seven_samples_keep_bytes_and_file_identity_through_case_only_renames() {
    let source =
        std::path::PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("samples"));
    let temp = tempfile::tempdir().unwrap();
    let refs: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/metadata/reference.json")).unwrap();
    let mut hashes = HashMap::new();
    for case in refs["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["sample"] == true)
    {
        let name = case["name"].as_str().unwrap();
        let bytes = fs::read(source.join(name)).unwrap();
        hashes.insert(name.to_owned(), blake3::hash(&bytes));
        fs::write(temp.path().join(name), bytes).unwrap();
    }
    assert_eq!(hashes.len(), 7);
    let c = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let mut names = HashMap::new();
    for m in &c.media {
        let stem = m.path.file_stem().unwrap().to_str().unwrap();
        let next = if stem == stem.to_uppercase() {
            stem.to_lowercase()
        } else {
            stem.to_uppercase()
        };
        organize::set_file_name(&c, &mut names, m.id, &next, true).unwrap();
    }
    let p = plan(&c, names);
    assert_eq!(p.actions.len(), 7);
    assert!(
        p.actions
            .iter()
            .all(|a| matches!(a, Action::RenameFile { .. }))
    );
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&p, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for action in p.actions {
        let Action::RenameFile { source: before, to } = action else {
            unreachable!()
        };
        let name = before.path.file_name().unwrap().to_str().unwrap();
        assert_eq!(blake3::hash(&fs::read(&to).unwrap()), hashes[name]);
        assert_eq!(Fingerprint::read(&to).unwrap(), before.fingerprint);
        assert_eq!(
            blake3::hash(&fs::read(source.join(name)).unwrap()),
            hashes[name]
        );
    }
}
