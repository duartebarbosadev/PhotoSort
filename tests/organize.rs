use photosort::core::{model::MediaId, organize::Draft};
use std::collections::HashMap;
fn draft() -> Draft {
    Draft::new(
        [(1, "trip/day1"), (2, "trip/day2"), (3, "trips"), (4, "")]
            .into_iter()
            .map(|(id, p)| (MediaId(id), p.to_owned()))
            .collect(),
    )
}
#[test]
fn rename_directory_moves_descendants_not_similarly_named_siblings() {
    let mut d = draft();
    d.create_folder("trip/day1", "empty").unwrap();
    d.rename_folder("trip", "holiday").unwrap();
    assert_eq!(d.labels()[&MediaId(1)], "holiday/day1");
    assert_eq!(d.labels()[&MediaId(2)], "holiday/day2");
    assert_eq!(d.labels()[&MediaId(3)], "trips");
    assert!(d.folders().contains("holiday/day1/empty"));
    assert!(!d.folders().contains("trip"));
}
#[test]
fn creating_parent_nesting_and_unnesting_preserve_every_photo_once() {
    let mut d = draft();
    d.create_parent("trip", "2026").unwrap();
    d.nest_folder("2026", "trips").unwrap();
    assert_eq!(d.labels()[&MediaId(1)], "trips/2026/trip/day1");
    d.unnest_folder("trips/2026/trip").unwrap();
    assert_eq!(d.labels()[&MediaId(1)], "trip/day1");
    assert_eq!(d.labels().len(), 4);
}
#[test]
fn cycles_collisions_root_and_invalid_names_leave_draft_unchanged() {
    let mut d = draft();
    let before = d.clone();
    assert!(d.nest_folder("trip", "trip/day1").is_err());
    assert!(d.rename_folder("trip", "trips").is_err());
    assert!(d.rename_folder("", "root").is_err());
    assert!(d.create_parent("trip", "trips").is_err());
    for name in [
        "",
        ".",
        "..",
        "../outside",
        "/absolute",
        "a\\b",
        "a:b",
        " spaced ",
    ] {
        assert!(d.create_folder("", name).is_err(), "{name}");
    }
    assert_eq!(before, d);
}
#[test]
fn moving_selection_is_atomic_and_supports_root_and_empty_folders() {
    let mut d = draft();
    let dest = d.create_folder("trip", "keepers").unwrap();
    let before = d.clone();
    assert!(d.move_photos(&[MediaId(1), MediaId(99)], &dest).is_err());
    assert_eq!(d, before);
    d.move_photos(&[MediaId(1), MediaId(2), MediaId(1)], &dest)
        .unwrap();
    assert_eq!(d.labels()[&MediaId(1)], dest);
    assert_eq!(d.labels()[&MediaId(2)], dest);
    d.move_photos(&[MediaId(1)], "").unwrap();
    assert_eq!(d.labels()[&MediaId(1)], "");
    assert_eq!(d.labels().len(), 4);
}
#[test]
fn folder_edits_feed_existing_planner_and_companion_executor_without_decoding() {
    use photosort::core::{catalog, operations};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("trip/day1")).unwrap();
    // Deliberately undecodable media: folder operations must never decode photos.
    for name in ["one.jpg", "one.ARW", "one.xmp"] {
        std::fs::write(root.join("trip/day1").join(name), name).unwrap();
    }
    let catalog = catalog::Catalog::scan(&root, || false, |_| {}).unwrap();
    let mut d = Draft::new(
        catalog
            .media
            .iter()
            .map(|m| (m.id, "trip/day1".into()))
            .collect::<HashMap<_, _>>(),
    );
    d.rename_folder("trip", "holiday").unwrap();
    let plan = operations::organize_plan(&catalog, d.labels(), &root, true).unwrap();
    assert_eq!(
        plan.actions
            .iter()
            .filter(|a| matches!(a, operations::Action::Move { .. }))
            .count(),
        1,
        "One shared companion group, no duplicate move"
    );
    assert!(
        root.join("trip/day1/one.jpg").exists(),
        "Preview never mutates files"
    );
    let report = operations::execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for name in ["one.jpg", "one.ARW", "one.xmp"] {
        assert_eq!(
            std::fs::read_to_string(root.join("holiday/day1").join(name)).unwrap(),
            name
        );
        assert!(!root.join("trip/day1").join(name).exists());
    }
}

#[test]
fn extras_reuse_inventory_and_follow_reference_directory_scope() {
    use photosort::core::{catalog::Catalog, model::*};
    let photo = Media {
        id: MediaId(1),
        path: "/nonexistent/photos/a.jpg".into(),
        kind: MediaKind::Photo,
        fingerprint: Default::default(),
    };
    let catalog = Catalog {
        root: "/nonexistent".into(),
        media: vec![photo],
        inventory: [
            "photos/a.jpg",
            "photos/a.xmp",
            "photos/a.json",
            "photos/sub/README",
            "elsewhere/notes.txt",
        ]
        .map(|p| std::path::Path::new("/nonexistent").join(p))
        .to_vec(),
        ..Default::default()
    };
    let paths: Vec<_> = catalog
        .organize_files()
        .into_iter()
        .map(|(_, p)| p)
        .collect();
    // Compared as paths: Windows joins with `\`, and paths treat both
    // separators alike there.
    assert_eq!(
        paths,
        [
            "/nonexistent/photos/a.jpg",
            "/nonexistent/photos/a.json",
            "/nonexistent/photos/sub/README"
        ]
        .map(std::path::PathBuf::from)
    );
    let mut labels = HashMap::from([(MediaId(1), "generated".into())]);
    photosort::core::organize::include_inventory(&catalog, &mut labels);
    assert_eq!(labels[&MediaId(1)], "generated");
    assert_eq!(
        labels[&MediaId::for_path(&std::path::Path::new("/nonexistent").join("photos/a.json"))],
        "photos"
    );
    // All paths are nonexistent: inventory augmentation cannot walk or decode.
    assert_eq!(labels.len(), 3);
}
#[test]
fn subtree_edits_move_documents_and_extensionless_files_with_media() {
    use photosort::core::{catalog::Catalog, operations, organize};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("trip/sub")).unwrap();
    for name in ["a.jpg", "a.xmp", "a.json", "README", "sub/notes.txt"] {
        std::fs::write(root.join("trip").join(name), name).unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let mut labels = catalog
        .media
        .iter()
        .map(|m| (m.id, "trip".into()))
        .collect();
    organize::include_inventory(&catalog, &mut labels);
    let unchanged = operations::organize_plan(&catalog, &labels, &root, true).unwrap();
    assert!(unchanged.actions.is_empty());
    let mut draft = Draft::new(labels);
    draft.rename_folder("trip", "holiday").unwrap();
    let plan = operations::organize_plan(&catalog, draft.labels(), &root, true).unwrap();
    assert_eq!(
        plan.actions
            .iter()
            .filter(|a| matches!(a, operations::Action::Move { .. }))
            .count(),
        4
    );
    let report = operations::execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for name in ["a.jpg", "a.xmp", "a.json", "README", "sub/notes.txt"] {
        assert_eq!(
            std::fs::read_to_string(root.join("holiday").join(name)).unwrap(),
            name
        );
        assert!(!root.join("trip").join(name).exists());
    }
    assert!(!root.join("holiday/README.").exists());
}
#[test]
fn independent_document_move_does_not_claim_a_photos_xmp() {
    use photosort::core::{catalog::Catalog, model::MediaId, operations};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for name in ["a.jpg", "a.xmp", "a.json"] {
        std::fs::write(root.join(name), name).unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let labels = HashMap::from([(MediaId::for_path(&root.join("a.json")), "notes".into())]);
    for companions in [false, true] {
        let plan = operations::organize_plan(&catalog, &labels, &root, companions).unwrap();
        assert_eq!(plan.actions.len(), 1);
        assert_eq!(
            plan.actions[0].sources(),
            vec![root.join("a.json").as_path()]
        );
    }
}
#[test]
fn document_collisions_and_stale_sources_use_shared_mutation_guards() {
    use photosort::core::{catalog::Catalog, model::MediaId, operations};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("output")).unwrap();
    for (name, data) in [
        ("a.jpg", "photo"),
        ("README", "original"),
        ("output/README", "existing"),
    ] {
        std::fs::write(root.join(name), data).unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let labels = HashMap::from([(MediaId::for_path(&root.join("README")), "output".into())]);
    let plan = operations::organize_plan(&catalog, &labels, &root, true).unwrap();
    let operations::Action::Move { files } = &plan.actions[0] else {
        panic!()
    };
    assert_eq!(files[0].1, root.join("output/README_1"));
    std::fs::write(root.join("README"), "changed after review").unwrap();
    let report = operations::execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
    assert_eq!(report.errors.len(), 1);
    assert!(report.completed.is_empty());
    assert_eq!(
        std::fs::read_to_string(root.join("output/README")).unwrap(),
        "existing"
    );
    assert!(!root.join("output/README_1").exists());
}

#[test]
fn departure_prediction_reuses_inventory_without_filesystem_access() {
    use photosort::core::{catalog::Catalog, model::*, operations, organize::DirectoryEdits};
    let root = std::path::PathBuf::from("/nonexistent-photosort-departure");
    let path = root.join("trip/photo.jpg");
    let id = MediaId::for_path(&path);
    let catalog = Catalog {
        root: root.clone(),
        media: vec![Media {
            id,
            path,
            kind: MediaKind::Photo,
            fingerprint: Default::default(),
        }],
        directories: vec![root.join("trip")],
        ..Default::default()
    };
    let mut labels = HashMap::from([(id, "trip".into())]);
    let names = HashMap::from([(id, "photo".into())]);
    let mut dirs = DirectoryEdits::default();
    let pending = |labels: &HashMap<_, _>, dirs: &DirectoryEdits| {
        operations::organize_changes_pending(&catalog, labels, &names, &root, dirs)
    };
    assert!(!pending(&labels, &dirs));
    dirs.required.insert("trip".into());
    assert!(!pending(&labels, &dirs));
    dirs.required.insert("new-empty".into());
    assert!(pending(&labels, &dirs));
    dirs.required.clear();
    labels.insert(id, "elsewhere".into());
    assert!(pending(&labels, &dirs));
}

#[test]
fn entering_unchanged_filename_does_not_create_collision_rename() {
    use photosort::core::{catalog::Catalog, operations};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::write(root.join("photo.jpg"), b"no decoding needed").unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let id = catalog.media[0].id;
    let labels = HashMap::from([(id, String::new())]);
    let names = HashMap::from([(id, "photo".into())]);
    let plan =
        operations::organize_plan_with_names(&catalog, &labels, &names, &root, true).unwrap();
    assert!(plan.actions.is_empty());
    assert!(!operations::organize_changes_pending(
        &catalog,
        &labels,
        &names,
        &root,
        &Default::default()
    ));
}
