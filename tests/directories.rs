use photosort::core::{
    catalog::Catalog,
    model::MediaId,
    operations::{self, Action, Plan},
    organize::Draft,
};
use std::{collections::HashMap, path::Path};
fn scan(root: &Path) -> Catalog {
    Catalog::scan(root, || false, |_| {}).unwrap()
}
fn labels(c: &Catalog) -> HashMap<MediaId, String> {
    c.media
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
                    .into_owned(),
            )
        })
        .collect()
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
fn run(plan: &Plan, journals: &Path) -> operations::Report {
    operations::execute(plan, journals, || false, |_, _| {}).unwrap()
}
#[test]
fn unchanged_empty_hierarchy_is_in_catalog_and_draft_without_operations() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("empty/nested")).unwrap();
    let c = scan(temp.path());
    let d = Draft::from_catalog(&c, labels(&c));
    assert!(d.folders().contains("empty/nested"));
    assert!(d.directory_edits().required.contains("empty/nested"));
    assert!(plan(&c, &d).actions.is_empty());
}
#[test]
fn new_empty_folders_are_reviewed_then_created_by_shared_executor() {
    let temp = tempfile::tempdir().unwrap();
    let c = scan(temp.path());
    let mut d = Draft::from_catalog(&c, labels(&c));
    d.create_folder("", "New").unwrap();
    d.create_folder("New", "Nested").unwrap();
    let p = plan(&c, &d);
    assert!(
        p.actions
            .iter()
            .all(|a| matches!(a, Action::CreateDirectory { .. }))
    );
    assert!(!c.root.join("New").exists());
    let report = run(&p, &temp.path().join("journal"));
    assert!(report.errors.is_empty());
    assert!(c.root.join("New/Nested").is_dir());
}
#[test]
fn renaming_empty_subtree_creates_destination_before_removing_old_tree() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("before/nested")).unwrap();
    let c = scan(temp.path());
    let mut d = Draft::from_catalog(&c, labels(&c));
    d.rename_folder("before", "after").unwrap();
    let p = plan(&c, &d);
    let report = run(&p, &temp.path().join("journal"));
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(c.root.join("after/nested").is_dir());
    assert!(!c.root.join("before").exists());
    assert!(c.root.is_dir());
}
#[test]
fn successful_moves_clean_only_their_empty_ancestors_and_never_the_root() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("before/day")).unwrap();
    std::fs::create_dir(temp.path().join("unrelated_empty")).unwrap();
    std::fs::write(temp.path().join("before/day/a.jpg"), b"fixture").unwrap();
    let c = scan(temp.path());
    let target = HashMap::from([(c.media[0].id, "after".into())]);
    let p = operations::organize_plan(&c, &target, &c.root, true).unwrap();
    assert!(
        !p.actions
            .iter()
            .any(|a| matches!(a, Action::RemoveEmptyDirectory {path, ..} if path == &c.root))
    );
    assert!(run(&p, &temp.path().join("journal")).errors.is_empty());
    assert!(!c.root.join("before").exists());
    assert!(c.root.join("unrelated_empty").exists());
    assert_eq!(
        std::fs::read(c.root.join("after/a.jpg")).unwrap(),
        b"fixture"
    );
}
#[test]
fn cleanup_leaves_unplanned_and_new_files_untouched() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("before")).unwrap();
    std::fs::write(temp.path().join("before/a.jpg"), b"fixture").unwrap();
    let c = scan(temp.path());
    let target = HashMap::from([(c.media[0].id, "after".into())]);
    let p = operations::organize_plan(&c, &target, &c.root, true).unwrap();
    std::fs::write(c.root.join("before/new.txt"), "arrived after preview").unwrap();
    assert!(run(&p, &temp.path().join("journal")).errors.is_empty());
    assert_eq!(
        std::fs::read_to_string(c.root.join("before/new.txt")).unwrap(),
        "arrived after preview"
    );
}
#[test]
fn partial_failure_cleans_successful_paths_but_preserves_failed_empty_source() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["a", "b"] {
        std::fs::create_dir(temp.path().join(name)).unwrap();
        std::fs::write(
            temp.path().join(name).join(format!("{name}.jpg")),
            b"fixture",
        )
        .unwrap();
    }
    let c = scan(temp.path());
    let target = c.media.iter().map(|m| (m.id, "out".into())).collect();
    let p = operations::organize_plan(&c, &target, &c.root, true).unwrap();
    std::fs::remove_file(c.root.join("b/b.jpg")).unwrap();
    let report = run(&p, &temp.path().join("journal"));
    assert_eq!(report.errors.len(), 1);
    assert!(!c.root.join("a").exists());
    assert!(
        c.root.join("b").is_dir(),
        "No successful prerequisite for this empty directory"
    );
}
#[test]
fn failed_creation_and_cancellation_leave_original_empty_folders() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("before")).unwrap();
    let c = scan(temp.path());
    let mut d = Draft::from_catalog(&c, labels(&c));
    d.rename_folder("before", "after").unwrap();
    let p = plan(&c, &d);
    let report = operations::execute(&p, &temp.path().join("journal"), || true, |_, _| {}).unwrap();
    assert!(report.cancelled && report.completed.is_empty());
    assert!(c.root.join("before").is_dir());
    std::fs::write(c.root.join("after"), "new conflicting file").unwrap();
    let report = run(&p, &temp.path().join("journal"));
    assert_eq!(report.errors.len(), 1);
    assert!(c.root.join("before").is_dir());
}
#[cfg(unix)]
#[test]
fn cleanup_rejects_replaced_symlinks_and_paths_outside_the_library() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let outside = tempfile::tempdir().unwrap();
    std::os::unix::fs::symlink(outside.path(), root.join("link")).unwrap();
    for path in [
        root.join("link"),
        outside.path().to_path_buf(),
        root.clone(),
    ] {
        let dependency: std::path::PathBuf = root.join("created");
        let p = Plan {
            actions: vec![
                Action::CreateDirectory {
                    path: dependency.clone(),
                },
                Action::RemoveEmptyDirectory {
                    path,
                    root: root.clone(),
                    after: vec![dependency],
                },
            ],
            description: "test".into(),
            ..Default::default()
        };
        assert_eq!(run(&p, &root.join("journal")).errors.len(), 1);
        assert!(outside.path().is_dir());
        assert!(root.is_dir());
    }
}
