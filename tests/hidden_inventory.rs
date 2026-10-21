use photosort::core::{
    catalog::Catalog,
    model::*,
    operations::{self, Action},
    organize::Draft,
};
use std::{
    collections::{HashMap, HashSet},
    fs,
};

fn labels(catalog: &Catalog) -> HashMap<MediaId, String> {
    catalog
        .organize_files()
        .into_iter()
        .map(|(id, path)| {
            (
                id,
                path.parent()
                    .unwrap()
                    .strip_prefix(&catalog.root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
            )
        })
        .collect()
}

#[test]
fn hidden_media_discovery_is_batched_and_supporting_inventory_is_shared() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("library");
    fs::create_dir_all(root.join(".photos/.metadata")).unwrap();
    fs::create_dir(root.join(".unrelated")).unwrap();
    for n in 0..257 {
        fs::write(
            root.join(".photos").join(format!(".image-{n}.jpg")),
            b"no decode",
        )
        .unwrap();
    }
    fs::write(root.join(".photos/.movie.mov"), b"no video decode").unwrap();
    fs::write(root.join(".photos/.image-0.xmp"), b"companion").unwrap();
    fs::write(root.join(".photos/.metadata/.notes.json"), b"document").unwrap();
    fs::write(root.join(".unrelated/.notes.json"), b"out of scope").unwrap();
    let mut batches = Vec::new();
    let catalog = Catalog::scan(&root, || false, |items| batches.push(items)).unwrap();
    assert_eq!(catalog.media.len(), 258);
    assert_eq!(
        batches.iter().map(Vec::len).collect::<Vec<_>>(),
        vec![128, 128, 2]
    );
    assert_eq!(
        batches
            .into_iter()
            .flatten()
            .map(|m| m.id)
            .collect::<HashSet<_>>()
            .len(),
        258
    );
    assert_eq!(catalog.inventory.len(), 261);
    assert_eq!(
        catalog
            .media
            .iter()
            .filter(|m| m.kind == MediaKind::Video)
            .count(),
        1
    );
    let hidden_pair = catalog.root.join(".photos/.image-0");
    assert_eq!(catalog.companion_index()[&hidden_pair].len(), 2);
    // Move the filesystem out of reach after scanning. Every consumer below
    // must reuse inventory rather than discovering or decoding it again.
    fs::rename(&root, base.path().join("offline")).unwrap();
    let files = catalog.organize_files();
    assert_eq!(files.len(), 259);
    assert!(
        files
            .iter()
            .any(|(_, p)| p.ends_with(".metadata/.notes.json"))
    );
    assert!(
        !files
            .iter()
            .any(|(_, p)| p.ends_with(".unrelated/.notes.json"))
    );
    assert_eq!(catalog.companion_index()[&hidden_pair].len(), 2);
}

#[test]
fn hidden_current_folders_are_a_noop_and_subtree_moves_keep_names_and_companions() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().canonicalize().unwrap();
    fs::create_dir_all(root.join(".before/.meta")).unwrap();
    let contents = [
        (".pair.jpg", "jpeg"),
        (".pair.ARW", "raw"),
        (".pair.xmp", "xmp"),
        (".DS_Store", "hidden document"),
        (".meta/.notes.json", "metadata"),
    ];
    for (name, bytes) in contents {
        fs::write(root.join(".before").join(name), bytes).unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let mut draft = Draft::from_catalog(&catalog, labels(&catalog));
    let before = operations::organize_plan_with_layout(
        &catalog,
        draft.labels(),
        &HashMap::new(),
        &root,
        true,
        &draft.directory_edits(),
    )
    .unwrap();
    assert!(
        before.actions.is_empty(),
        "Discovery must not turn hidden folders into rename requests"
    );
    assert!(!operations::organize_changes_pending(
        &catalog,
        draft.labels(),
        &HashMap::new(),
        &root,
        &draft.directory_edits()
    ));
    draft.rename_folder(".before", ".after").unwrap();
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
            .filter(|a| matches!(a, Action::Move { .. }))
            .count(),
        3,
        "One companion group plus two independent supporting files"
    );
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!root.join(".before").exists());
    assert!(!root.join("after").exists());
    for (name, bytes) in contents {
        assert_eq!(
            fs::read_to_string(root.join(".after").join(name)).unwrap(),
            bytes
        );
    }
}

#[test]
fn hidden_empty_folders_and_new_hidden_names_use_the_same_directory_plan() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().canonicalize().unwrap();
    fs::create_dir_all(root.join(".empty/.nested")).unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let mut draft = Draft::from_catalog(&catalog, HashMap::new());
    assert!(draft.folders().contains(".empty/.nested"));
    draft.rename_folder(".empty", ".renamed").unwrap();
    draft.create_folder("", ".new").unwrap();
    for invalid in [".", "..", "../outside", ".bad/child", ".bad\\child"] {
        assert!(draft.create_folder("", invalid).is_err());
    }
    let plan = operations::organize_plan_with_layout(
        &catalog,
        draft.labels(),
        &HashMap::new(),
        &root,
        true,
        &draft.directory_edits(),
    )
    .unwrap();
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!root.join(".empty").exists());
    assert!(root.join(".renamed/.nested").is_dir());
    assert!(root.join(".new").is_dir());
}

#[cfg(unix)]
#[test]
fn hidden_links_are_not_followed_and_cancellation_stops_the_shared_scan() {
    use std::{cell::Cell, os::unix::fs::symlink};
    let base = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    fs::write(outside.path().join("photo.jpg"), b"outside").unwrap();
    symlink(outside.path(), base.path().join(".linked-folder")).unwrap();
    symlink(
        outside.path().join("photo.jpg"),
        base.path().join(".linked.jpg"),
    )
    .unwrap();
    fs::create_dir(base.path().join(".real")).unwrap();
    fs::write(base.path().join(".real/.photo.jpg"), b"inside").unwrap();
    let catalog = Catalog::scan(base.path(), || false, |_| {}).unwrap();
    assert_eq!(catalog.media.len(), 1);
    assert_eq!(catalog.inventory.len(), 1);
    assert!(catalog.media[0].path.ends_with(".real/.photo.jpg"));
    let checks = Cell::new(0);
    assert!(
        Catalog::scan(
            base.path(),
            || {
                let n = checks.get() + 1;
                checks.set(n);
                n >= 3
            },
            |_| panic!("cancelled short scan must not publish a batch")
        )
        .is_err()
    );
}

#[test]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR; reads originals and moves independent copies"]
fn seven_reference_samples_survive_hidden_subtree_move() -> anyhow::Result<()> {
    let samples = std::path::PathBuf::from(
        std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"),
    );
    let source = Catalog::scan(&samples, || false, |_| {})?;
    assert_eq!(source.media.len(), 7);
    let temp = tempfile::tempdir()?;
    let root = temp.path().canonicalize()?;
    fs::create_dir_all(root.join(".before/.meta"))?;
    fs::create_dir_all(root.join(".empty/nested"))?;
    let mut originals = Vec::new();
    for media in &source.media {
        originals.push((
            media.path.clone(),
            operations::content_hash(&media.path, || false)?,
        ));
        fs::copy(&media.path, root.join(".before").join(media.name()))?;
    }
    fs::write(root.join(".before/.DS_Store"), b"hidden supporting file")?;
    fs::write(root.join(".before/.meta/notes.json"), b"metadata document")?;
    fs::write(root.join(".before/arw_sample.xmp"), b"companion bytes")?;
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    assert_eq!(catalog.media.len(), 7);
    assert_eq!(catalog.organize_files().len(), 9);
    let hashes: Vec<_> = catalog
        .inventory
        .iter()
        .map(|path| {
            Ok((
                path.strip_prefix(root.join(".before"))?.to_path_buf(),
                operations::content_hash(path, || false)?,
            ))
        })
        .collect::<anyhow::Result<_>>()?;
    assert_eq!(hashes.len(), 10);
    let mut draft = Draft::from_catalog(&catalog, labels(&catalog));
    let unchanged = operations::organize_plan_with_layout(
        &catalog,
        draft.labels(),
        &HashMap::new(),
        &root,
        true,
        &draft.directory_edits(),
    )?;
    assert!(unchanged.actions.is_empty());
    draft.rename_folder(".before", ".after")?;
    let plan = operations::organize_plan_with_layout(
        &catalog,
        draft.labels(),
        &HashMap::new(),
        &root,
        true,
        &draft.directory_edits(),
    )?;
    assert_eq!(
        plan.actions
            .iter()
            .filter(|action| matches!(action, Action::Move { .. }))
            .count(),
        9
    );
    let journal = tempfile::tempdir()?;
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {})?;
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.remaining.actions.is_empty());
    assert!(!root.join(".before").exists());
    assert!(!root.join("after").exists());
    assert!(root.join(".empty/nested").is_dir());
    assert_eq!(Catalog::scan(&root, || false, |_| {})?.inventory.len(), 10);
    for (path, hash) in hashes {
        assert_eq!(
            operations::content_hash(&root.join(".after").join(path), || false)?,
            hash
        );
    }
    for (path, hash) in originals {
        assert_eq!(
            operations::content_hash(&path, || false)?,
            hash,
            "Original sample changed"
        );
    }
    Ok(())
}
