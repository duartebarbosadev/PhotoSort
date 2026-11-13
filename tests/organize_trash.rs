use photosort::core::{
    catalog::Catalog,
    decisions::{self, Change},
    metadata,
    model::*,
    operations::{self, Action, recovery},
    organize::{Draft, Selection},
    session::Session,
    views,
};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    fs,
    path::PathBuf,
};

fn fixture() -> (tempfile::TempDir, Catalog) {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "A/photo.jpg",
        "A/photo.ARW",
        "A/photo.xmp",
        "A/photo.txt",
        "A/child/notes.txt",
        "A-1/other.jpg",
        "A/clip.mov",
    ] {
        let p = dir.path().join(name);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(p, name.as_bytes()).unwrap();
    }
    let catalog = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    (dir, catalog)
}
fn id(c: &Catalog, name: &str) -> MediaId {
    MediaId::for_path(&c.root.join(name))
}
fn index(c: &Catalog) -> HashMap<MediaId, usize> {
    c.media.iter().enumerate().map(|(i, m)| (m.id, i)).collect()
}
fn paths(plan: &operations::Plan) -> HashSet<PathBuf> {
    plan.actions
        .iter()
        .flat_map(|a| match a {
            Action::Trash { files } => files.iter().map(|s| s.path.clone()).collect::<Vec<_>>(),
            _ => panic!("unexpected non-Trash action"),
        })
        .collect()
}
#[test]
fn organize_trash_staging_and_folder_scope_use_cached_inventory_without_io() {
    let (_dir, c) = fixture();
    let labels = c
        .organize_files()
        .into_iter()
        .map(|(id, p)| {
            (
                id,
                // Folder labels always use `/`, as the app writes them.
                p.parent()
                    .unwrap()
                    .strip_prefix(&c.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
        })
        .collect();
    let draft = Draft::from_catalog(&c, labels);
    // Making the entire source unavailable proves selection/staging needs no scan or metadata.
    let unavailable = c.root.with_extension("offline");
    fs::rename(&c.root, &unavailable).unwrap();
    let files = draft
        .selection_files(&Selection {
            files: HashSet::from([id(&c, "A/photo.txt")]),
            folders: BTreeSet::from(["A".into(), "A/child".into()]),
        })
        .unwrap();
    assert_eq!(files.len(), 5);
    assert!(!files.contains(&id(&c, "A-1/other.jpg")));
    let mut decisions = HashMap::new();
    let mut repeated: Vec<_> = files.iter().copied().collect();
    repeated.extend(files.iter().copied());
    repeated.push(MediaId(u64::MAX));
    let changed = decisions::stage_files(
        &c.media,
        &index(&c),
        &c.supporting_fingerprints,
        &mut decisions,
        repeated,
        Change::ToggleTrash,
    );
    assert_eq!(changed.len(), 5);
    assert!(decisions.values().all(|d| d.rejected));
    for change in [Change::Rating(5), Change::Rotate(1)] {
        decisions::stage_files(
            &c.media,
            &index(&c),
            &c.supporting_fingerprints,
            &mut decisions,
            files.iter().copied(),
            change,
        );
    }
    for name in ["A/photo.txt", "A/child/notes.txt", "A/clip.mov"] {
        assert_eq!(
            decisions[&id(&c, name)],
            Decision {
                rejected: true,
                ..Default::default()
            }
        );
    }
    fs::rename(unavailable, &c.root).unwrap();
}
#[test]
fn organize_trash_documents_do_not_expand_photo_companions_or_load_metadata() {
    let (_dir, c) = fixture();
    let doc = id(&c, "A/photo.txt");
    let mut decisions = HashMap::from([(
        doc,
        Decision {
            rejected: true,
            rating: Some(5),
            turns: 1,
        },
    )]);
    for companions in [false, true] {
        let plan = operations::decisions_plan_with_metadata(
            &c,
            &decisions,
            companions,
            metadata::WritePolicy::SidecarOnly,
            |_| panic!("document is not an image"),
            || false,
        )
        .unwrap();
        assert_eq!(paths(&plan), HashSet::from([c.root.join("A/photo.txt")]));
    }
    decisions.insert(
        id(&c, "A/photo.jpg"),
        Decision {
            rejected: true,
            ..Default::default()
        },
    );
    let plan = operations::decisions_plan_with_metadata(
        &c,
        &decisions,
        true,
        metadata::WritePolicy::SidecarOnly,
        |_| panic!("Trash planning must not decode metadata"),
        || false,
    )
    .unwrap();
    assert_eq!(
        paths(&plan),
        ["A/photo.txt", "A/photo.jpg", "A/photo.ARW", "A/photo.xmp"]
            .map(|p| c.root.join(p))
            .into()
    );
    assert!(
        operations::decisions_plan_with_metadata(
            &c,
            &decisions,
            true,
            metadata::WritePolicy::SidecarOnly,
            |_| panic!(),
            || true
        )
        .is_err()
    );
    fs::write(c.root.join("A/photo.txt"), b"changed since discovery").unwrap();
    assert!(
        operations::decisions_plan(&c, &decisions, true)
            .unwrap_err()
            .to_string()
            .contains("changed since discovery")
    );
}
#[test]
fn organize_trash_review_rechecks_documents_before_mutating_and_retains_retry() {
    let (_dir, c) = fixture();
    let doc = id(&c, "A/photo.txt");
    let decisions = HashMap::from([(
        doc,
        Decision {
            rejected: true,
            ..Default::default()
        },
    )]);
    let plan = operations::decisions_plan(&c, &decisions, true).unwrap();
    fs::write(c.root.join("A/photo.txt"), b"changed after review").unwrap();
    let report = operations::execute(&plan, &c.root.join("journals"), || false, |_, _| {}).unwrap();
    assert!(report.completed.is_empty());
    assert_eq!(report.errors.len(), 1);
    assert_eq!(paths(&report.remaining), paths(&plan));
    assert_eq!(
        fs::read(c.root.join("A/photo.txt")).unwrap(),
        b"changed after review"
    );
    assert_eq!(
        fs::read(c.root.join("A/photo.jpg")).unwrap(),
        b"A/photo.jpg"
    );
}
#[test]
fn organize_trash_supporting_intent_survives_serialization_moves_and_recovery_context() {
    let (_dir, c) = fixture();
    let doc = id(&c, "A/photo.txt");
    let mut session = Session {
        decisions: HashMap::from([(
            doc,
            Decision {
                rejected: true,
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    let context = recovery::Context::capture_catalog(
        false,
        &c,
        &index(&c),
        session.clone(),
        views::Saved::default(),
    );
    assert!(context.media.is_empty());
    assert_eq!(context.session.fingerprints.len(), 1);
    let context: recovery::Context =
        serde_json::from_slice(&serde_json::to_vec(&context).unwrap()).unwrap();
    session = context.session;
    session.reconcile_catalog(&c);
    assert!(session.decisions[&doc].rejected);
    let labels = c
        .organize_files()
        .into_iter()
        .map(|(id, _)| (id, "Sorted".into()))
        .collect();
    let plan = operations::organize_plan(&c, &labels, &c.root, true).unwrap();
    let report = operations::execute(&plan, &c.root.join("journals"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    session.apply_report(&report, &c.media);
    let after = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    session.reconcile_catalog(&after);
    let new_id = id(&after, "Sorted/photo.txt");
    assert!(!session.decisions.contains_key(&doc));
    assert!(session.decisions[&new_id].rejected);
    let trash = operations::decisions_plan(&after, &session.decisions, true).unwrap();
    assert_eq!(
        paths(&trash),
        HashSet::from([c.root.join("Sorted/photo.txt")])
    );
    fs::write(c.root.join("Sorted/photo.txt"), b"replacement").unwrap();
    let replaced = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    session.reconcile_catalog(&replaced);
    assert!(!session.decisions.contains_key(&new_id));
}

#[test]
fn trash_takes_only_raw_and_xmp_with_a_photo_never_same_name_photos() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "shot.jpeg",
        "shot.heic",
        "shot.ARW",
        "shot.xmp",
        "pair.jpg",
        "pair.ARW",
        "pair.xmp",
    ] {
        fs::write(dir.path().join(name), name.as_bytes()).unwrap();
    }
    let c = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    let trashed = |names: &[&str], companions: bool| {
        let decisions = names
            .iter()
            .map(|n| {
                (
                    id(&c, n),
                    Decision {
                        rejected: true,
                        ..Default::default()
                    },
                )
            })
            .collect();
        let plan = operations::decisions_plan(&c, &decisions, companions).unwrap();
        let mut got: Vec<_> = paths(&plan)
            .into_iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        got.sort();
        got
    };
    // A JPEG and a HEIC with the same name are two photos: the kept one keeps its RAW and XMP.
    assert_eq!(trashed(&["shot.jpeg"], true), ["shot.jpeg"]);
    assert_eq!(trashed(&["shot.ARW"], true), ["shot.ARW"]);
    assert_eq!(
        trashed(&["shot.jpeg", "shot.heic"], true),
        ["shot.ARW", "shot.heic", "shot.jpeg", "shot.xmp"]
    );
    // Without companions the RAW is its own photo, so the shared XMP stays with it.
    assert_eq!(
        trashed(&["shot.jpeg", "shot.heic"], false),
        ["shot.heic", "shot.jpeg"]
    );
    // A plain RAW + JPEG pair still goes together.
    assert_eq!(
        trashed(&["pair.jpg"], true),
        ["pair.ARW", "pair.jpg", "pair.xmp"]
    );
    assert_eq!(trashed(&["pair.jpg"], false), ["pair.jpg"]);
    // A review that listed only the XMP never trashes the RAW beside it.
    let decisions = HashMap::from([(
        id(&c, "pair.jpg"),
        Decision {
            rejected: true,
            ..Default::default()
        },
    )]);
    let plan = operations::decisions_plan_with_followers(
        &c,
        &decisions,
        true,
        Some(&HashSet::from([id(&c, "pair.xmp")])),
        metadata::WritePolicy::SidecarOnly,
        |m| Ok(metadata::read(&m.path)),
        || false,
    )
    .unwrap();
    assert_eq!(
        paths(&plan),
        HashSet::from([c.root.join("pair.jpg"), c.root.join("pair.xmp")])
    );
    // A rated RAW the review kept out of the listed files stays, and its
    // rating is written; listed, it goes and the rating is dropped.
    let mut decisions = decisions;
    decisions.insert(
        id(&c, "pair.ARW"),
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let actions = |followers: &[&str]| {
        operations::decisions_plan_with_followers(
            &c,
            &decisions,
            true,
            Some(&followers.iter().map(|name| id(&c, name)).collect()),
            metadata::WritePolicy::SidecarOnly,
            |m| Ok(metadata::read(&m.path)),
            || false,
        )
        .unwrap()
        .actions
    };
    let kept = actions(&[]);
    assert!(
        matches!(&kept[..], [
            Action::Trash { files },
            Action::Metadata { source, rating: Some(4), .. },
        ] if files.len() == 1
            && files[0].path == c.root.join("pair.jpg")
            && source.path == c.root.join("pair.ARW")),
        "{kept:?}"
    );
    let trashed = actions(&["pair.ARW", "pair.xmp"]);
    assert!(
        matches!(&trashed[..], [Action::Trash { files }] if files.len() == 3),
        "{trashed:?}"
    );
}

/// darktable and digiKam name a sidecar after the whole file ("IMG.CR2.xmp"),
/// and a video can have one too. Each goes where its file goes; one that
/// names nothing follows its folder.
#[test]
fn sidecars_named_after_the_whole_file_or_beside_a_video_follow_it() {
    let dir = tempfile::tempdir().unwrap();
    let rated = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="3"/></rdf:RDF></x:xmpmeta>"#;
    fs::create_dir(dir.path().join("Old")).unwrap();
    for name in [
        "IMG_1.CR2",
        "IMG_1.CR2.xmp",
        "IMG_1.xmp",
        "solo.NEF",
        "solo.NEF.xmp",
        "clip.MOV",
        "clip.MOV.xmp",
        "clip.xmp",
        "gone.NEF.xmp",
    ] {
        fs::write(dir.path().join("Old").join(name), rated).unwrap();
    }
    let c = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    let names = |paths: Vec<PathBuf>| -> BTreeSet<String> {
        paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    };
    let set =
        |names: &[&str]| -> BTreeSet<String> { names.iter().map(|n| n.to_string()).collect() };
    // Only the sidecar that names nothing is a file of its own.
    assert_eq!(
        names(
            c.organize_files()
                .iter()
                .map(|(_, p)| p.to_path_buf())
                .collect()
        ),
        set(&["IMG_1.CR2", "solo.NEF", "clip.MOV", "gone.NEF.xmp"])
    );
    // The rating is read from the sidecar that is there.
    let solo = c.root.join("Old/solo.NEF");
    assert_eq!(metadata::sidecar(&solo), c.root.join("Old/solo.NEF.xmp"));
    assert_eq!(metadata::read(&solo).rating, 3);

    // Trash: both spellings go with the RAW, and a video's with the video.
    let rejected: HashMap<MediaId, Decision> = ["Old/IMG_1.CR2", "Old/clip.MOV"]
        .iter()
        .map(|name| {
            (
                id(&c, name),
                Decision {
                    rejected: true,
                    ..Default::default()
                },
            )
        })
        .collect();
    let plan = operations::decisions_plan(&c, &rejected, false).unwrap();
    assert_eq!(
        names(paths(&plan).into_iter().collect()),
        set(&[
            "IMG_1.CR2",
            "IMG_1.CR2.xmp",
            "IMG_1.xmp",
            "clip.MOV",
            "clip.MOV.xmp",
            "clip.xmp"
        ])
    );

    // Move and rename, companions off: "solo.NEF.xmp" becomes "Lisbon.NEF.xmp".
    let labels: HashMap<MediaId, String> = c
        .organize_files()
        .into_iter()
        .map(|(id, _)| (id, "New".to_string()))
        .collect();
    let renamed = HashMap::from([(id(&c, "Old/solo.NEF"), "Lisbon".to_string())]);
    let plan = operations::organize_plan_with_names(&c, &labels, &renamed, &c.root, false).unwrap();
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let moved = fs::read_dir(c.root.join("New"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(
        names(moved),
        set(&[
            "IMG_1.CR2",
            "IMG_1.CR2.xmp",
            "IMG_1.xmp",
            "Lisbon.NEF",
            "Lisbon.NEF.xmp",
            "clip.MOV",
            "clip.MOV.xmp",
            "clip.xmp",
            "gone.NEF.xmp"
        ])
    );
    assert!(!c.root.join("Old").exists());
}

/// The second half of an Apply whose moves came first: a folder those moves
/// made is removed once the Trash marks leave nothing in it. A folder that
/// keeps a photo stays. One that was there before goes only when the review
/// listed it, and only after every photo in it reached the Trash.
#[test]
fn a_folder_the_moves_made_is_removed_once_the_trash_empties_it() {
    let (_dir, c) = fixture();
    let trash = tempfile::tempdir().unwrap();
    operations::trash_stand_in(&c.root, trash.path());
    // A folder of its own that the moves leave alone.
    fs::create_dir(c.root.join("Old")).unwrap();
    fs::write(c.root.join("Old/solo.jpg"), b"solo").unwrap();
    let c = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    let labels = HashMap::from([
        (id(&c, "A/photo.jpg"), "New/Deep".to_string()),
        (id(&c, "A/photo.ARW"), "New/Deep".to_string()),
        (id(&c, "A/clip.mov"), "Mixed".to_string()),
        (id(&c, "A-1/other.jpg"), "Mixed".to_string()),
    ]);
    let plan = operations::organize_plan(&c, &labels, &c.root, true).unwrap();
    let journal = tempfile::tempdir().unwrap();
    let moved = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert!(moved.errors.is_empty(), "{:?}", moved.errors);
    // The run says which folders it made, parents included.
    assert_eq!(
        moved
            .created_folders
            .iter()
            .cloned()
            .collect::<HashSet<_>>(),
        HashSet::from([
            c.root.join("New"),
            c.root.join("New/Deep"),
            c.root.join("Mixed")
        ])
    );
    let c = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    let rejected = |name: &str| {
        (
            id(&c, name),
            Decision {
                rejected: true,
                ..Default::default()
            },
        )
    };
    let decisions = HashMap::from([
        rejected("New/Deep/photo.jpg"),
        rejected("Mixed/clip.mov"),
        rejected("Old/solo.jpg"),
    ]);
    let mut plan = operations::decisions_plan(&c, &decisions, true).unwrap();
    operations::remove_folders_emptied_by_trash(&mut plan, &c, &moved.created_folders, &[], || {
        false
    })
    .unwrap();
    // Only the folders left with nothing, the deepest first.
    let removed: Vec<_> = plan
        .actions
        .iter()
        .filter_map(|action| match action {
            Action::RemoveEmptyDirectory {
                path, leftovers, ..
            } => Some((path.clone(), leftovers.is_empty())),
            _ => None,
        })
        .collect();
    assert_eq!(
        removed,
        [(c.root.join("New/Deep"), true), (c.root.join("New"), true)]
    );
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.remaining.actions.is_empty());
    // The photo went to the Trash with its RAW and XMP, from its new folder.
    for name in [
        "photo.jpg",
        "photo.ARW",
        "photo.xmp",
        "clip.mov",
        "solo.jpg",
    ] {
        assert!(trash.path().join(name).is_file(), "{name}");
    }
    assert!(!c.root.join("New").exists());
    assert!(c.root.join("Mixed/other.jpg").is_file());
    // The review did not list "Old", so it stays, empty: it was there before.
    assert!(c.root.join("Old").is_dir(), "not listed, so not removed");

    // A folder that was there before goes too once the review listed it:
    // both its photos are marked. A stopped Apply leaves it, with the photo
    // not yet in the Trash, and the retry finishes both.
    for name in ["Old/x.jpg", "Old/y.jpg"] {
        fs::write(c.root.join(name), name).unwrap();
    }
    let c = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    let marked = [c.root.join("Old/x.jpg"), c.root.join("Old/y.jpg")];
    let listing =
        operations::folders_emptied_by_trash(&c, &operations::Plan::default(), &marked, || false)
            .unwrap();
    let listed: Vec<(PathBuf, Vec<String>)> = listing
        .actions
        .iter()
        .filter_map(|action| match action {
            Action::RemoveEmptyDirectory {
                path, leftovers, ..
            } => Some((path.clone(), leftovers.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(listed, [(c.root.join("Old"), Vec::new())]);
    let decisions = HashMap::from([rejected("Old/x.jpg"), rejected("Old/y.jpg")]);
    let mut plan = operations::decisions_plan(&c, &decisions, true).unwrap();
    operations::remove_folders_emptied_by_trash(&mut plan, &c, &[], &listed, || false).unwrap();
    assert_eq!(plan.actions.len(), 3, "two photos, then the folder");
    let done = std::cell::Cell::new(0);
    let stopped = operations::execute(
        &plan,
        journal.path(),
        || done.get() >= 1,
        |finished, _| done.set(finished),
    )
    .unwrap();
    assert!(stopped.cancelled);
    assert_eq!(stopped.remaining.actions.len(), 2);
    assert!(c.root.join("Old").is_dir(), "a photo is still in it");
    let retried =
        operations::execute(&stopped.remaining, journal.path(), || false, |_, _| {}).unwrap();
    assert!(retried.errors.is_empty(), "{:?}", retried.errors);
    assert!(retried.remaining.actions.is_empty());
    assert!(!c.root.join("Old").exists());
    assert!(c.root.is_dir());
}
