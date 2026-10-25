use photosort::core::{
    catalog::Catalog,
    model::*,
    operations,
    organize::{Draft, Selection},
};
use std::{
    collections::{HashMap, HashSet},
    fs,
};

fn files(ids: &[u64]) -> Selection {
    Selection {
        files: ids.iter().map(|id| MediaId(*id)).collect(),
        ..Default::default()
    }
}
fn folders(paths: &[&str]) -> Selection {
    Selection {
        folders: paths.iter().map(|p| (*p).into()).collect(),
        ..Default::default()
    }
}

#[test]
fn multi_folder_collisions_and_stale_selections_are_atomic() {
    let mut draft = Draft::new(
        [(1, "a/day"), (2, "b/day"), (3, "")]
            .into_iter()
            .map(|(id, p)| (MediaId(id), p.into()))
            .collect(),
    );
    draft.create_folder("", "dest").unwrap();
    let before = draft.clone();
    let mut mixed = folders(&["a/day", "b/day"]);
    mixed.files.insert(MediaId(3));
    for (selection, target) in [
        (mixed, "dest"),
        (files(&[1, 99]), "dest"),
        (folders(&["a"]), "a/day"),
        (folders(&[""]), "dest"),
        (folders(&["missing"]), "dest"),
        (files(&[1]), "missing"),
    ] {
        assert!(draft.validate_move(&selection, target).is_err());
        assert!(draft.move_selection(&selection, target, &[], true).is_err());
        assert_eq!(draft, before);
    }
    draft.create_folder("dest", "a").unwrap();
    let before = draft.clone();
    assert!(
        draft
            .move_selection(&folders(&["a", "b"]), "dest", &[], true)
            .is_err()
    );
    assert_eq!(draft, before);
}

#[test]
fn selected_ancestor_owns_descendants_even_with_interleaved_similar_names() {
    let mut draft = Draft::new(
        [(1, "a/child"), (2, "a.b"), (3, "z")]
            .into_iter()
            .map(|(id, p)| (MediaId(id), p.into()))
            .collect(),
    );
    draft.create_folder("a/child", "empty").unwrap();
    draft
        .move_selection(&folders(&["a", "a.b", "a/child"]), "z", &[], true)
        .unwrap();
    assert_eq!(draft.labels()[&MediaId(1)], "z/a/child");
    assert_eq!(draft.labels()[&MediaId(2)], "z/a.b");
    assert!(draft.folders().contains("z/a/child/empty"));
    assert!(!draft.folders().contains("z/child"));
    draft
        .move_selection(&folders(&["z/a", "z/a.b"]), "", &[], true)
        .unwrap();
    assert_eq!(draft.labels()[&MediaId(1)], "a/child");
    assert_eq!(draft.labels()[&MediaId(2)], "a.b");
    let before = draft.clone();
    draft
        .move_selection(&folders(&["a", "a.b"]), "", &[], true)
        .unwrap();
    assert_eq!(draft, before);
}

#[test]
fn shared_selection_expands_photo_companions_but_not_same_stem_video() {
    let media: Vec<_> = [
        (1, "one.jpg", MediaKind::Photo),
        (2, "one.ARW", MediaKind::Raw),
        (3, "one.mov", MediaKind::Video),
    ]
    .into_iter()
    .map(|(id, path, kind)| Media {
        id: MediaId(id),
        path: format!("/missing/{path}").into(),
        kind,
        fingerprint: Default::default(),
    })
    .collect();
    let mut draft = Draft::new((1..=4).map(|id| (MediaId(id), "source".into())).collect());
    draft.create_folder("", "target").unwrap();
    let mut independent = draft.clone();
    independent
        .move_selection(&files(&[1]), "target", &media, false)
        .unwrap();
    assert_eq!(independent.labels()[&MediaId(2)], "source");
    draft
        .move_selection(&files(&[1, 4]), "target", &media, true)
        .unwrap();
    for id in [1, 2, 4] {
        assert_eq!(draft.labels()[&MediaId(id)], "target");
    }
    assert_eq!(draft.labels()[&MediaId(3)], "source");
}

#[test]
fn mixed_selection_moves_explicit_files_then_the_remaining_subtree() {
    let mut draft = Draft::new(
        [(1, "a"), (2, "a/child"), (3, "target")]
            .into_iter()
            .map(|(id, p)| (MediaId(id), p.into()))
            .collect(),
    );
    let mut selection = folders(&["a"]);
    selection.files.insert(MediaId(1));
    draft
        .move_selection(&selection, "target", &[], true)
        .unwrap();
    assert_eq!(draft.labels()[&MediaId(1)], "target");
    assert_eq!(draft.labels()[&MediaId(2)], "target/a/child");
}

#[test]
fn selection_moves_feed_one_companion_safe_plan_with_documents_and_empty_trees() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir_all(root.join("trip/empty")).unwrap();
    fs::create_dir(root.join("target")).unwrap();
    let sources = [
        "trip/one.jpg",
        "trip/one.ARW",
        "trip/one.xmp",
        "trip/notes.json",
    ];
    for path in sources {
        fs::write(root.join(path), path).unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let labels = catalog
        .organize_files()
        .into_iter()
        .map(|(id, _)| (id, "trip".into()))
        .collect();
    let mut draft = Draft::from_catalog(&catalog, labels);
    draft
        .move_selection(&folders(&["trip"]), "target", &catalog.media, true)
        .unwrap();
    let plan = operations::organize_plan_with_layout(
        &catalog,
        draft.labels(),
        &HashMap::new(),
        &root,
        true,
        &draft.directory_edits(),
    )
    .unwrap();
    assert_eq!(
        plan.actions
            .iter()
            .filter(|a| matches!(a, operations::Action::Move { .. }))
            .count(),
        2
    );
    for path in sources {
        assert!(root.join(path).exists());
    }
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!root.join("trip").exists());
    assert!(root.join("target/trip/empty").is_dir());
    for path in sources {
        assert_eq!(
            fs::read_to_string(root.join("target").join(path)).unwrap(),
            path
        );
    }
    let after = Catalog::scan(&root, || false, |_| {}).unwrap();
    assert_eq!(after.inventory.len(), 4);
    assert_eq!(
        after
            .media
            .iter()
            .map(|m| m.id)
            .collect::<HashSet<_>>()
            .len(),
        2
    );
}

#[test]
fn selective_reset_expands_descendants_companions_and_validates_atomically() {
    let media: Vec<_> = [
        (1, "pair.jpg", MediaKind::Photo),
        (2, "pair.ARW", MediaKind::Raw),
        (3, "pair.mov", MediaKind::Video),
    ]
    .into_iter()
    .map(|(id, path, kind)| Media {
        id: MediaId(id),
        path: format!("/missing/{path}").into(),
        kind,
        fingerprint: Default::default(),
    })
    .collect();
    let initial: HashMap<_, _> = [
        (1, "Japan"),
        (2, "Japan"),
        (3, "videos"),
        (4, "notes"),
        (5, "untouched"),
    ]
    .into_iter()
    .map(|(id, p)| (MediaId(id), p.into()))
    .collect();
    let mut draft = Draft::new(
        [
            (1, "edited/sub"),
            (2, "elsewhere"),
            (3, "other"),
            (4, "edited"),
            (5, "edited-sibling"),
        ]
        .into_iter()
        .map(|(id, p)| (MediaId(id), p.into()))
        .collect(),
    );
    draft.create_folder("edited", "empty").unwrap();
    let before = draft.clone();
    let mut incomplete = initial.clone();
    incomplete.remove(&MediaId(2));
    assert!(
        draft
            .reset_destinations(&folders(&["edited"]), &incomplete, &media, true)
            .is_err()
    );
    assert_eq!(draft, before);
    assert!(
        draft
            .reset_destinations(&files(&[1, 99]), &initial, &media, true)
            .is_err()
    );
    assert!(
        draft
            .reset_destinations(&folders(&["missing"]), &initial, &media, true)
            .is_err()
    );
    assert_eq!(draft, before);
    draft
        .reset_destinations(&folders(&["edited", "edited/sub"]), &initial, &media, true)
        .unwrap();
    for (id, expected) in [
        (1, "Japan"),
        (2, "Japan"),
        (3, "other"),
        (4, "notes"),
        (5, "edited-sibling"),
    ] {
        assert_eq!(draft.labels()[&MediaId(id)], expected);
    }
    assert!(draft.folders().contains("edited/empty"));
    let mut independent = before;
    independent
        .reset_destinations(&files(&[1]), &initial, &media, false)
        .unwrap();
    assert_eq!(independent.labels()[&MediaId(2)], "elsewhere");
}
#[test]
fn selective_reset_feeds_shared_plan_and_keeps_unrelated_edits() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    fs::create_dir(root.join("trip")).unwrap();
    for name in ["a.jpg", "a.ARW", "a.xmp", "notes.txt", "b.jpg"] {
        fs::write(root.join("trip").join(name), name).unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let initial: HashMap<_, _> = catalog
        .organize_files()
        .iter()
        .map(|(id, _)| (*id, "trip".into()))
        .collect();
    let mut draft = Draft::from_catalog(&catalog, initial.clone());
    draft.rename_folder("trip", "Edited").unwrap();
    let id = catalog
        .media
        .iter()
        .find(|m| m.name() == "a.ARW")
        .unwrap()
        .id;
    draft
        .reset_destinations(
            &Selection {
                files: HashSet::from([id]),
                ..Default::default()
            },
            &initial,
            &catalog.media,
            true,
        )
        .unwrap();
    let plan = operations::organize_plan_with_layout(
        &catalog,
        draft.labels(),
        &HashMap::new(),
        &root,
        true,
        &draft.directory_edits(),
    )
    .unwrap();
    let result = operations::execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
    assert!(result.errors.is_empty(), "{:?}", result.errors);
    for name in ["a.jpg", "a.ARW", "a.xmp"] {
        assert_eq!(
            fs::read(root.join("trip").join(name)).unwrap(),
            name.as_bytes()
        );
    }
    for name in ["notes.txt", "b.jpg"] {
        assert_eq!(
            fs::read(root.join("Edited").join(name)).unwrap(),
            name.as_bytes()
        );
    }
}

#[test]
fn items_left_in_place_move_with_their_companions_and_can_be_left_again() {
    let media: Vec<_> = [
        (1, "a/one.jpg", MediaKind::Photo),
        (2, "a/one.ARW", MediaKind::Raw),
        (3, "a/one.mov", MediaKind::Video),
        (4, "a/clip.mov", MediaKind::Video),
        (5, "a/two.jpg", MediaKind::Photo),
    ]
    .into_iter()
    .map(|(id, path, kind)| Media {
        id: MediaId(id),
        path: format!("/missing/{path}").into(),
        kind,
        fingerprint: Default::default(),
    })
    .collect();
    // The grouping places only two.jpg; the rest stay where they are.
    let initial: HashMap<_, _> = [(MediaId(5), "Person 001".to_owned())].into();
    let mut draft = Draft::new(initial.clone());
    draft.create_folder("", "Trip").unwrap();
    // Unknown items and missing folders are still refused, atomically.
    let before = draft.clone();
    assert!(
        draft
            .move_selection(&files(&[1, 99]), "Person 001", &media, true)
            .is_err()
    );
    assert!(
        draft
            .move_selection(&files(&[1]), "missing", &media, true)
            .is_err()
    );
    assert_eq!(draft, before);
    // A photo left in place moves with its RAW, not its same-name video.
    draft
        .move_selection(&files(&[1]), "Person 001", &media, true)
        .unwrap();
    for id in [1, 2] {
        assert_eq!(draft.labels()[&MediaId(id)], "Person 001");
    }
    assert!(!draft.labels().contains_key(&MediaId(3)));
    // Without companions it moves alone; a video moves too.
    let mut alone = before.clone();
    alone
        .move_selection(&files(&[1, 4]), "Trip", &media, false)
        .unwrap();
    assert_eq!(alone.labels()[&MediaId(1)], "Trip");
    assert_eq!(alone.labels()[&MediaId(4)], "Trip");
    assert!(!alone.labels().contains_key(&MediaId(2)));
    // Left in place again, they lose their destination, companions too.
    draft.keep_in_place(&[MediaId(1)].into(), &media, true);
    assert!(!draft.labels().contains_key(&MediaId(1)));
    assert!(!draft.labels().contains_key(&MediaId(2)));
    assert_eq!(draft.labels()[&MediaId(5)], "Person 001");
    // Removing a new folder returns what it held: placed items to their
    // proposal, items left in place to no destination.
    draft
        .move_selection(&files(&[1, 5]), "Trip", &media, true)
        .unwrap();
    draft.remove_folder("Trip", &initial).unwrap();
    assert_eq!(draft.labels()[&MediaId(5)], "Person 001");
    assert!(!draft.labels().contains_key(&MediaId(1)));
    assert!(!draft.labels().contains_key(&MediaId(2)));
}
