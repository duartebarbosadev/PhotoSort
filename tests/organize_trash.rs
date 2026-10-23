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
}
