//! Existing folder and file names are used byte-for-byte. Only names
//! PhotoSort creates are cleaned, so opening a library never proposes renames.
// Windows cannot create these names, so the fixtures are Unix-only.
#![cfg(unix)]
use photosort::core::{
    catalog::Catalog,
    model::*,
    names::relative_label,
    operations::{self, Action, Plan},
    organize::{self, Draft},
};
use std::{collections::HashMap, fs, path::Path};

const ODD: &[&str] = &[
    "Archive/toEdit/23:Dec/DCIM/10051129",
    "Archive/toEdit/23:Dec/DCIM/10351210",
    "?*\"<>|",
    "trailing.",
    "trailing ",
    " leading",
    "tab\there",
    r"back\slash",
];

fn current_labels(catalog: &Catalog) -> HashMap<MediaId, String> {
    catalog
        .organize_files()
        .into_iter()
        .filter_map(|(id, path)| {
            let parent = path.parent()?.strip_prefix(&catalog.root).ok()?;
            Some((id, relative_label(parent)?))
        })
        .collect()
}

fn library() -> (tempfile::TempDir, Catalog) {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().canonicalize().unwrap();
    let long = "x".repeat(120);
    for folder in ODD.iter().copied().chain([long.as_str()]) {
        let dir = root.join(folder);
        fs::create_dir_all(&dir).unwrap();
        for name in [
            "GAB00432.ARW",
            "GAB00432.JPG",
            "GAB00432.xmp",
            "shot:1?.jpg",
        ] {
            fs::write(dir.join(name), name).unwrap();
        }
    }
    fs::create_dir_all(root.join("Archive/toEdit/23:Dec/Empty:Too")).unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    assert_eq!(catalog.media.len(), (ODD.len() + 1) * 3);
    (base, catalog)
}

fn layout_plan(catalog: &Catalog, draft: &Draft) -> Plan {
    operations::organize_plan_with_layout(
        catalog,
        draft.labels(),
        &HashMap::new(),
        &catalog.root,
        true,
        &draft.directory_edits(),
    )
    .unwrap()
}

#[test]
fn unchanged_current_folders_plan_nothing_for_odd_existing_names() {
    let (_base, catalog) = library();
    let labels = current_labels(&catalog);
    assert!(
        labels
            .values()
            .any(|l| l == "Archive/toEdit/23:Dec/DCIM/10051129")
    );
    assert!(labels.values().any(|l| l == r"back\slash"));
    // Without a draft: the check that decides whether leaving Organize asks.
    let none = Default::default();
    assert!(!operations::organize_changes_pending(
        &catalog,
        &labels,
        &HashMap::new(),
        &catalog.root,
        &none
    ));
    let plan = operations::organize_plan(&catalog, &labels, &catalog.root, true).unwrap();
    assert!(plan.actions.is_empty(), "{:?}", plan.actions);
    // With the editable board's draft, which also carries existing folders.
    let draft = Draft::from_catalog(&catalog, labels);
    assert!(draft.folders().contains("Archive/toEdit/23:Dec/Empty:Too"));
    assert!(!operations::organize_changes_pending(
        &catalog,
        draft.labels(),
        &HashMap::new(),
        &catalog.root,
        &draft.directory_edits()
    ));
    let plan = layout_plan(&catalog, &draft);
    assert!(plan.actions.is_empty(), "{:?}", plan.actions);
}

#[test]
fn keeping_an_existing_name_is_not_a_rename() {
    let (_base, catalog) = library();
    let mut draft = Draft::from_catalog(&catalog, current_labels(&catalog));
    assert_eq!(
        draft
            .rename_folder("Archive/toEdit/23:Dec", "23:Dec")
            .unwrap(),
        "Archive/toEdit/23:Dec"
    );
    assert!(layout_plan(&catalog, &draft).actions.is_empty());
    let (id, path) = catalog
        .organize_files()
        .into_iter()
        .find(|(_, p)| p.ends_with("23:Dec/DCIM/10051129/shot:1?.jpg"))
        .unwrap();
    let stem = path.file_stem().unwrap().to_str().unwrap().to_owned();
    let mut names = HashMap::new();
    organize::set_file_name(&catalog, &mut names, id, &stem, true).unwrap();
    assert!(names.is_empty());
    // An older session may still hold the unchanged name.
    names.insert(id, stem);
    assert!(!operations::organize_changes_pending(
        &catalog,
        draft.labels(),
        &names,
        &catalog.root,
        &draft.directory_edits()
    ));
    let plan = operations::organize_plan_with_layout(
        &catalog,
        draft.labels(),
        &names,
        &catalog.root,
        true,
        &draft.directory_edits(),
    )
    .unwrap();
    assert!(plan.actions.is_empty(), "{:?}", plan.actions);
}

#[test]
fn generated_and_edited_layouts_keep_existing_parents_verbatim() {
    let (_base, catalog) = library();
    let root = catalog.root.clone();
    // A generated grouping moves photos out, but existing folders stay as named.
    let dated: HashMap<_, _> = catalog
        .organize_files()
        .into_iter()
        .map(|(id, _)| (id, "2023-12-01".to_owned()))
        .collect();
    let draft = Draft::from_catalog(&catalog, dated);
    let generated = layout_plan(&catalog, &draft);
    for action in &generated.actions {
        let paths: Vec<&Path> = match action {
            Action::Move { files } => files.iter().map(|(_, to)| to.as_path()).collect(),
            Action::CreateDirectory { path } | Action::RemoveEmptyDirectory { path, .. } => {
                vec![path]
            }
            other => panic!("unexpected {other:?}"),
        };
        for path in paths {
            let text = path.to_string_lossy();
            assert!(
                !text.contains("23_Dec") && !text.contains("Unsorted"),
                "{text}"
            );
        }
    }
    assert!(
        !generated
            .actions
            .iter()
            .any(|a| matches!(a, Action::CreateDirectory { .. }))
    );
    // A photo moved into a new folder under `23:Dec` lands under that folder.
    let mut draft = Draft::from_catalog(&catalog, current_labels(&catalog));
    let new = draft
        .create_folder("Archive/toEdit/23:Dec/DCIM", "Picks")
        .unwrap();
    let pair: Vec<_> = catalog
        .organize_files()
        .into_iter()
        .filter(|(_, p)| p.starts_with(catalog.root.join("?*\"<>|")))
        .filter(|(_, p)| p.file_stem().is_some_and(|s| s == "GAB00432"))
        .map(|(id, _)| id)
        .collect();
    assert_eq!(
        pair.len(),
        2,
        "RAW and JPEG; the XMP follows as a companion"
    );
    draft.move_photos(&pair, &new).unwrap();
    let edited = layout_plan(&catalog, &draft);
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&edited, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let picks = root.join("Archive/toEdit/23:Dec/DCIM/Picks");
    for name in ["GAB00432.ARW", "GAB00432.JPG", "GAB00432.xmp"] {
        assert_eq!(fs::read_to_string(picks.join(name)).unwrap(), name);
    }
    assert!(!root.join("Archive/toEdit/23_Dec").exists());
    for folder in ODD {
        assert!(root.join(folder).is_dir(), "{folder}");
    }
    assert!(root.join("?*\"<>|/shot:1?.jpg").is_file());
}

#[test]
fn folders_with_names_that_are_not_unicode_stay_in_place() {
    use std::os::unix::ffi::OsStrExt;
    let base = tempfile::tempdir().unwrap();
    let root = base.path().canonicalize().unwrap();
    let odd = root.join(std::ffi::OsStr::from_bytes(b"bad-\xff"));
    // APFS refuses names that are not UTF-8; Linux CI covers this case.
    if fs::create_dir(&odd).is_err() {
        return;
    }
    fs::write(odd.join("a.jpg"), b"a").unwrap();
    fs::create_dir(odd.join("Empty")).unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    assert_eq!(catalog.media.len(), 1);
    let labels = current_labels(&catalog);
    assert!(labels.is_empty());
    let draft = Draft::from_catalog(&catalog, labels);
    assert!(draft.folders().iter().all(|f| !f.contains('\u{fffd}')));
    assert!(!operations::organize_changes_pending(
        &catalog,
        draft.labels(),
        &HashMap::new(),
        &root,
        &draft.directory_edits()
    ));
    assert!(layout_plan(&catalog, &draft).actions.is_empty());
}
