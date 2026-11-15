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
    // A folder that gets a new empty folder is not left empty, even when
    // everything that is in it now leaves: it is not listed as removed,
    // and nothing is reported as kept.
    let temp = tempfile::tempdir().unwrap();
    for name in ["Album/a.jpg", "Album/Seen/.DS_Store", "Other/x.jpg"] {
        let path = temp.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, name).unwrap();
    }
    let c = scan(temp.path());
    let mut d = Draft::from_catalog(&c, labels(&c));
    let a = MediaId::for_path(&c.root.join("Album/a.jpg"));
    d.move_photos(&[a], "Other").unwrap();
    d.rename_folder("Album/Seen", "Viewed").unwrap();
    let p = plan(&c, &d);
    let album = c.root.join("Album");
    assert!(
        !p.actions
            .iter()
            .any(|a| matches!(a, Action::RemoveEmptyDirectory { path, .. } if *path == album))
    );
    let report = run(&p, &temp.path().join("journal"));
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.kept_folders.is_empty(), "{:?}", report.kept_folders);
    assert!(album.join("Viewed").is_dir() && !album.join("Seen").exists());
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
                    leftovers: Vec::new(),
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

#[test]
fn folders_left_with_only_system_files_are_listed_with_them_and_kept_when_that_changes() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for name in [
        "Trip/a.jpg",
        "Trip/.DS_Store",
        "Trip/Thumbs.db",
        // Windows writes the name with either capital.
        "Trip/Desktop.ini",
        "Trip/@eaDir/a.jpg/thumb.jpg",
        "Notes/b.jpg",
        "Notes/notes.txt",
        "Bare/c.jpg",
    ] {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, name).unwrap();
    }
    let c = scan(&root);
    let mut all = labels(&c);
    photosort::core::organize::include_inventory(&c, &mut all);
    let mut d = Draft::from_catalog(&c, all);
    d.create_folder("", "Picks").unwrap();
    let photos: Vec<_> = c.media.iter().map(|m| m.id).collect();
    d.move_photos(&photos, "Picks").unwrap();
    let p = plan(&c, &d);
    // The review is told, per folder, what goes to the Trash with it. A
    // folder that keeps a document stays, so it is not planned at all: the
    // review names only folders that go.
    let listed: HashMap<_, _> = p
        .actions
        .iter()
        .filter_map(|action| match action {
            Action::RemoveEmptyDirectory {
                path, leftovers, ..
            } => Some((
                path.file_name().unwrap().to_str().unwrap(),
                leftovers.clone(),
            )),
            _ => None,
        })
        .collect();
    assert_eq!(
        listed["Trip"],
        [".DS_Store", "@eaDir", "Desktop.ini", "Thumbs.db"]
    );
    assert!(!listed.contains_key("Notes") && listed["Bare"].is_empty());
    // The scan alone tells the same, for the count shown before any plan
    // is made: of the folders the photos leave, only Trip goes to the Trash
    // with leftovers. Notes keeps its document and Bare is only removed.
    let leaving: HashMap<&Path, u32> = c
        .media
        .iter()
        .map(|m| (m.path.parent().unwrap(), 1))
        .collect();
    let picks = root.join("Picks");
    assert_eq!(
        c.census()
            .left_with_leftovers(&c.root, &leaving, &[], |path| path == picks),
        1
    );
    // Only what was listed may go: a file that appears after the review
    // keeps the folder, and nothing reaches the Trash.
    std::fs::write(root.join("Trip/new.txt"), b"new").unwrap();
    let report = run(&p, &temp.path().join("journal"));
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    // The report says the folder was kept and what it now holds: its step
    // is over, not left to retry, and it is not one of the folders that went.
    assert!(report.remaining.actions.is_empty());
    assert_eq!(
        report.kept_folders,
        [operations::KeptFolder {
            path: root.join("Trip"),
            for_trash: true,
            why: operations::Kept::Holds(Some("new.txt".into())),
        }]
    );
    // The journal says so too, where a real removal says only "completed".
    let journal = std::fs::read_dir(temp.path().join("journal"))
        .unwrap()
        .flatten()
        .find_map(|entry| std::fs::read_to_string(entry.path()).ok())
        .unwrap();
    assert!(
        journal.contains(r#""kept":{"holds":"new.txt"}"#),
        "{journal}"
    );
    for kept in [
        "Trip/.DS_Store",
        "Trip/Thumbs.db",
        "Trip/Desktop.ini",
        "Trip/new.txt",
        "Trip/@eaDir/a.jpg/thumb.jpg",
        "Notes/notes.txt",
    ] {
        assert!(root.join(kept).is_file(), "{kept}");
    }
    assert!(!root.join("Bare").exists());
    assert_eq!(std::fs::read_dir(root.join("Picks")).unwrap().count(), 3);
}

/// Organizing into another folder takes the photos and the documents with
/// them. What a file manager keeps in a folder for itself stays behind: it
/// is not moved, so it makes no folder at the destination. A folder the
/// moves leave with nothing else is listed with it and goes to the Trash.
/// A subfolder that holds only such files is an empty folder to the user
/// and is treated as one: shown in the layout, made empty at the
/// destination, and left alone where it is, so the folder around it stays
/// too. The library folder itself stays.
#[test]
fn organizing_into_another_folder_leaves_folder_view_files_behind() {
    let temp = tempfile::tempdir().unwrap();
    let base = temp.path().canonicalize().unwrap();
    let (root, output, trash) = (
        base.join("library"),
        base.join("sorted"),
        base.join("trash"),
    );
    operations::trash_stand_in(&root, &trash);
    for name in [
        "top.jpg",
        ".DS_Store",
        "Trip/a.jpg",
        "Trip/Thumbs.db",
        // What macOS writes beside it on a drive Windows formatted.
        "Trip/._Thumbs.db",
        "Trip/desktop.ini",
        "Album/b.jpg",
        "Album/notes.txt",
        "Album/Seen/.DS_Store",
    ] {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, name).unwrap();
    }
    let c = scan(&root);
    let mut all = labels(&c);
    photosort::core::organize::include_inventory(&c, &mut all);
    let d = Draft::from_catalog(&c, all);
    assert!(d.folders().contains("Album/Seen"));
    let p = operations::organize_plan_with_layout(
        &c,
        d.labels(),
        &HashMap::new(),
        &output,
        true,
        &d.directory_edits(),
    )
    .unwrap();
    let listed: Vec<_> = p
        .actions
        .iter()
        .filter_map(|action| match action {
            Action::RemoveEmptyDirectory {
                path, leftovers, ..
            } => Some((path.strip_prefix(&root).unwrap(), leftovers.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        listed,
        [(
            Path::new("Trip"),
            ["._Thumbs.db", "Thumbs.db", "desktop.ini"]
                .map(String::from)
                .to_vec()
        )]
    );
    let report = run(&p, &base.join("journal"));
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    let files = |folder: &Path| {
        let mut files: Vec<_> = walkdir::WalkDir::new(folder)
            .min_depth(1)
            .into_iter()
            .map(|entry| entry.unwrap())
            .map(|entry| {
                let path = entry.path().strip_prefix(folder).unwrap();
                path.to_string_lossy().replace('\\', "/")
            })
            .collect();
        files.sort();
        files
    };
    assert_eq!(
        files(&output),
        [
            "Album",
            "Album/Seen",
            "Album/b.jpg",
            "Album/notes.txt",
            "Trip",
            "Trip/a.jpg",
            "top.jpg"
        ]
    );
    assert_eq!(
        files(&root),
        [".DS_Store", "Album", "Album/Seen", "Album/Seen/.DS_Store"]
    );
    assert_eq!(
        files(&trash),
        [
            "Trip",
            "Trip/._Thumbs.db",
            "Trip/Thumbs.db",
            "Trip/desktop.ini"
        ]
    );
}
