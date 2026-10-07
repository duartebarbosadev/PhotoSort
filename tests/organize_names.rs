use photosort::core::{catalog::Catalog, model::MediaId, operations, organize};
use std::{collections::HashMap, fs, path::Path};
fn fixture(root: &Path) -> Catalog {
    for name in [
        "pair.ARW",
        "pair.JPG",
        "pair.xmp",
        "pair.MOV",
        "pair",
        "notes.txt",
        ".hidden",
    ] {
        fs::write(root.join(name), name.as_bytes()).unwrap();
    }
    Catalog::scan(root, || false, |_| {}).unwrap()
}
fn labels(catalog: &Catalog) -> HashMap<MediaId, String> {
    catalog
        .organize_files()
        .into_iter()
        .map(|(id, _)| (id, String::new()))
        .collect()
}
#[test]
fn late_companion_name_is_order_independent_and_moves_the_entire_cohort_once() {
    for reverse in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let mut catalog = fixture(&root);
        if reverse {
            catalog.media.reverse();
        }
        let id = MediaId::for_path(&root.join("pair.JPG"));
        let names = HashMap::from([(id, "chosen".into())]);
        let plan =
            operations::organize_plan_with_names(&catalog, &labels(&catalog), &names, &root, true)
                .unwrap();
        assert_eq!(plan.actions.len(), 1);
        assert!(
            root.join("pair.JPG").exists(),
            "Planning must not move files"
        );
        let report =
            operations::execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        for ext in ["ARW", "JPG", "xmp"] {
            assert_eq!(
                fs::read(root.join(format!("chosen.{ext}"))).unwrap(),
                format!("pair.{ext}").as_bytes()
            );
            assert!(!root.join(format!("pair.{ext}")).exists());
        }
        assert_eq!(fs::read(root.join("pair.MOV")).unwrap(), b"pair.MOV");
        assert_eq!(fs::read(root.join("pair")).unwrap(), b"pair");
    }
}
#[test]
fn conflicting_companion_names_fail_before_mutation_and_cancellation_is_honored() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = fixture(dir.path());
    let names = HashMap::from([
        (
            MediaId::for_path(&catalog.root.join("pair.ARW")),
            "raw".into(),
        ),
        (
            MediaId::for_path(&catalog.root.join("pair.JPG")),
            "jpeg".into(),
        ),
    ]);
    let error = operations::organize_plan_with_names(
        &catalog,
        &labels(&catalog),
        &names,
        &catalog.root,
        true,
    )
    .unwrap_err();
    assert!(error.to_string().contains("conflicting names"));
    assert!(dir.path().join("pair.ARW").exists() && dir.path().join("pair.JPG").exists());
    assert!(
        organize::resolve_file_names(&catalog.organize_files(), &names, false, || true).is_err()
    );
    let resolved =
        organize::resolve_file_names(&catalog.organize_files(), &names, false, || false).unwrap();
    assert_eq!(resolved.len(), 2);
}
#[test]
fn draft_names_handle_supporting_files_extensions_restoration_and_atomic_validation() {
    let dir = tempfile::tempdir().unwrap();
    let long = "a".repeat(120);
    let long_path = dir.path().join(format!("{long}.jpg"));
    fs::write(&long_path, b"long existing name").unwrap();
    let catalog = fixture(dir.path());
    let id = MediaId::for_path(&catalog.root.join("pair.JPG"));
    let mut names = HashMap::new();
    organize::set_file_name(&catalog, &mut names, id, "keepers", true).unwrap();
    assert_eq!(names.len(), 2);
    organize::set_file_name(
        &catalog,
        &mut names,
        MediaId::for_path(&long_path.canonicalize().unwrap()),
        &long,
        true,
    )
    .unwrap();
    assert_eq!(
        names.len(),
        2,
        "An unchanged existing long name is not a rename"
    );
    let before = names.clone();
    for invalid in ["../escape", "a/b", "a\\b", "name.", " ", "bad:name"] {
        assert!(organize::set_file_name(&catalog, &mut names, id, invalid, true).is_err());
        assert_eq!(names, before);
    }
    assert!(organize::set_file_name(&catalog, &mut names, MediaId(0), "missing", true).is_err());
    assert_eq!(names, before);
    organize::set_file_name(&catalog, &mut names, id, "", true).unwrap();
    assert!(names.is_empty());
    for (old, new) in [
        ("notes.txt", "Trip notes"),
        ("pair", "README"),
        (".hidden", ".private"),
    ] {
        organize::set_file_name(
            &catalog,
            &mut names,
            MediaId::for_path(&catalog.root.join(old)),
            new,
            true,
        )
        .unwrap();
    }
    let plan = operations::organize_plan_with_names(
        &catalog,
        &labels(&catalog),
        &names,
        &catalog.root,
        true,
    )
    .unwrap();
    assert_eq!(plan.actions.len(), 3);
    let report =
        operations::execute(&plan, &dir.path().join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty());
    for (old, new) in [
        ("notes.txt", "Trip notes.txt"),
        ("pair", "README"),
        (".hidden", ".private"),
    ] {
        assert_eq!(fs::read(dir.path().join(new)).unwrap(), old.as_bytes());
    }
    assert!(dir.path().join("pair.ARW").exists());
}
#[test]
fn one_collision_suffix_covers_the_entire_renamed_companion_cohort() {
    let dir = tempfile::tempdir().unwrap();
    let catalog = fixture(dir.path());
    fs::write(dir.path().join("chosen.JPG"), b"occupied").unwrap();
    let names = HashMap::from([(
        MediaId::for_path(&catalog.root.join("pair.JPG")),
        "chosen".into(),
    )]);
    let plan = operations::organize_plan_with_names(
        &catalog,
        &labels(&catalog),
        &names,
        &catalog.root,
        true,
    )
    .unwrap();
    let report =
        operations::execute(&plan, &dir.path().join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty());
    assert_eq!(
        fs::read(dir.path().join("chosen.JPG")).unwrap(),
        b"occupied"
    );
    for ext in ["ARW", "JPG", "xmp"] {
        assert!(dir.path().join(format!("chosen_1.{ext}")).exists());
    }
}
#[test]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR; copies and renames all seven reference samples"]
fn seven_sample_renames_preserve_original_and_destination_bytes() {
    let source = std::path::PathBuf::from(
        std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("sample directory"),
    );
    let dir = tempfile::tempdir().unwrap();
    let references: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/metadata/reference.json")).unwrap();
    let mut hashes = HashMap::new();
    for case in references["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|v| v["sample"] == true)
    {
        let name = case["name"].as_str().unwrap();
        hashes.insert(
            name.to_owned(),
            blake3::hash(&fs::read(source.join(name)).unwrap()),
        );
        fs::copy(source.join(name), dir.path().join(name)).unwrap();
    }
    assert_eq!(hashes.len(), 7);
    let catalog = Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    let mut names = HashMap::new();
    for (i, m) in catalog.media.iter().enumerate() {
        organize::set_file_name(&catalog, &mut names, m.id, &format!("renamed-{i}"), true).unwrap();
    }
    let plan = operations::organize_plan_with_names(
        &catalog,
        &labels(&catalog),
        &names,
        &catalog.root,
        true,
    )
    .unwrap();
    let report =
        operations::execute(&plan, &dir.path().join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for m in &catalog.media {
        let name = m.name();
        let target = dir.path().join(organize::renamed_file_name(
            &m.path,
            std::ffi::OsStr::new(&names[&m.id]),
        ));
        assert_eq!(blake3::hash(&fs::read(target).unwrap()), hashes[&name]);
        assert_eq!(
            blake3::hash(&fs::read(source.join(name)).unwrap()),
            hashes[&m.name()]
        );
    }
}
