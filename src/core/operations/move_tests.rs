use super::*;

#[test]
fn cancellation_during_companion_staging_keeps_all_sources_and_retry_plan() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().canonicalize().unwrap().join("input");
    let output = dir.path().canonicalize().unwrap().join("output");
    fs::create_dir(&input).unwrap();
    fs::create_dir(&output).unwrap();
    let mut files = Vec::new();
    for name in ["photo.jpg", "photo.raw", "photo.xmp"] {
        let path = input.join(name);
        fs::write(&path, format!("original {name}")).unwrap();
        files.push((Source::capture(path).unwrap(), output.join(name)));
    }
    let plan = Plan {
        actions: vec![Action::Move {
            files: files.clone(),
        }],
        ..Default::default()
    };
    let cancelled = std::cell::Cell::new(false);
    let report = execute(
        &plan,
        &dir.path().join("journal"),
        || {
            cancelled.set(cancelled.get() || files[0].1.exists());
            cancelled.get()
        },
        |_, _| {},
    )
    .unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(report.cancelled, "staging cancellation must stop the move");
    assert_eq!(report.remaining.actions.len(), 1);
    for (source, destination) in &files {
        source.verify().unwrap();
        assert!(!destination.exists(), "staged destination was left behind");
    }
    let retry = execute(
        &report.remaining,
        &dir.path().join("retry"),
        || false,
        |_, _| {},
    )
    .unwrap();
    assert!(retry.errors.is_empty());
    assert!(retry.remaining.actions.is_empty());
    for (source, destination) in files {
        assert!(!source.path.exists());
        assert_eq!(
            fs::read_to_string(&destination).unwrap(),
            format!(
                "original {}",
                destination.file_name().unwrap().to_string_lossy()
            )
        );
    }
}

#[test]
fn fallback_copy_cancels_after_one_chunk_without_leaving_partial_file() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("large-video.mov");
    let destination = dir.path().join("copied.mov");
    let bytes: Vec<u8> = (0..4 * 1024 * 1024 + 19).map(|i| (i % 251) as u8).collect();
    fs::write(&source, &bytes).unwrap();
    let before = Source::capture(source.clone()).unwrap();
    let observed = std::cell::Cell::new(0);
    let error = copy_contents_exclusive(&source, &destination, &|| {
        let size = fs::metadata(&destination).map_or(0, |m| m.len());
        observed.set(observed.get().max(size));
        size >= 1024 * 1024
    })
    .unwrap_err();
    assert!(error.is::<MoveCancelled>());
    assert_eq!(observed.get(), 1024 * 1024);
    assert!(!destination.exists());
    before.verify().unwrap();
    assert_eq!(fs::read(&source).unwrap(), bytes);
    copy_contents_exclusive(&source, &destination, &|| false).unwrap();
    assert_eq!(fs::read(&destination).unwrap(), bytes);
}

#[test]
fn fallback_copy_never_removes_an_existing_destination_or_creates_one_when_cancelled() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("source");
    let destination = dir.path().join("destination");
    fs::write(&source, "source bytes").unwrap();
    fs::write(&destination, "other bytes").unwrap();
    assert!(copy_contents_exclusive(&source, &destination, &|| false).is_err());
    assert_eq!(fs::read_to_string(&destination).unwrap(), "other bytes");
    let absent = dir.path().join("absent");
    assert!(
        copy_contents_exclusive(&source, &absent, &|| true)
            .unwrap_err()
            .is::<MoveCancelled>()
    );
    assert!(!absent.exists());
    assert_eq!(fs::read_to_string(&source).unwrap(), "source bytes");
}

#[test]
fn cancelled_move_staging_restores_a_certain_retry_without_losing_decisions() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("library");
    fs::create_dir(&root).unwrap();
    fs::write(root.join("photo.jpg"), "photo bytes").unwrap();
    fs::write(root.join("photo.xmp"), "sidecar bytes").unwrap();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let id = catalog.media[0].id;
    let mut session = session::Session::default();
    session.decisions.insert(
        id,
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    let context = recovery::Context::capture(
        root.clone(),
        false,
        &catalog.media,
        &HashMap::from([(id, 0)]),
        session,
        super::super::views::Saved::default(),
    );
    let destination = root.join("Sorted");
    let files: Vec<_> = ["photo.jpg", "photo.xmp"]
        .into_iter()
        .map(|name| {
            (
                Source::capture(root.join(name)).unwrap(),
                destination.join(name),
            )
        })
        .collect();
    let plan = Plan {
        actions: vec![Action::Move {
            files: files.clone(),
        }],
        output_root: Some(root.clone()),
        ..Default::default()
    };
    let journals = temp.path().canonicalize().unwrap().join("journals");
    let store = recovery::Store::new(journals.clone());
    let stopped = std::cell::Cell::new(false);
    let report = store
        .execute(
            &context,
            &plan,
            None,
            || {
                stopped.set(stopped.get() || files[0].1.exists());
                stopped.get()
            },
            |_, _| {},
        )
        .unwrap();
    assert!(report.cancelled && report.errors.is_empty());
    assert!(report.completed.is_empty() && report.relocated.is_empty());
    for (source, destination) in &files {
        source.verify().unwrap();
        assert!(!destination.exists());
    }
    drop(store);
    let store = recovery::Store::new(journals);
    let restored = store.restore(&root, &|| false).unwrap().unwrap();
    assert_eq!(restored.uncertain, 0);
    assert!(restored.errors.is_empty());
    assert_eq!(
        serde_json::to_value(&restored.plan.actions).unwrap(),
        serde_json::to_value(&plan.actions).unwrap()
    );
    assert_eq!(
        session::Session::load(&root).unwrap().decisions[&id].rating,
        Some(4)
    );
    let retried = store
        .execute(
            &context,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(retried.errors.is_empty() && retried.remaining.actions.is_empty());
    let moved_id = MediaId::for_path(&destination.join("photo.jpg"));
    assert_eq!(
        session::Session::load(&root).unwrap().decisions[&moved_id].rating,
        Some(4)
    );
    assert!(store.restore(&root, &|| false).unwrap().is_none());
    for (source, destination) in files {
        assert!(!source.path.exists());
        assert!(destination.exists());
    }
}

#[test]
fn failed_partial_copy_cleanup_is_not_reported_as_clean_cancellation() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    let parked = temp.path().join("parked");
    fs::write(&source, vec![7u8; 2 * 1024 * 1024]).unwrap();
    let stopped = std::cell::Cell::new(false);
    let result = copy_contents_exclusive(&source, &destination, &|| {
        if !stopped.get() && fs::metadata(&destination).is_ok_and(|m| m.len() >= 1024 * 1024) {
            fs::rename(&destination, &parked).unwrap();
            fs::create_dir(&destination).unwrap();
            stopped.set(true);
        }
        stopped.get()
    })
    .unwrap_err();
    assert!(!result.is::<MoveCancelled>());
    assert!(
        result
            .to_string()
            .contains("could not remove partial destination")
    );
    assert!(destination.is_dir());
    assert!(source.exists());
}

#[test]
fn stop_before_execution_persists_the_exact_plan_without_applying_files() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("photos");
    fs::create_dir(&root).unwrap();
    let source = root.join("a.jpg");
    fs::write(&source, b"source bytes").unwrap();
    let destination = root.join("Sorted/a.jpg");
    let plan = Plan {
        actions: vec![Action::Move {
            files: vec![(
                Source::capture(source.clone()).unwrap(),
                destination.clone(),
            )],
        }],
        output_root: Some(root.clone()),
        ..Default::default()
    };
    let context = recovery::Context {
        root: root.clone(),
        rotation_only: false,
        media: Vec::new(),
        session: session::Session::default(),
        views: super::super::views::Saved::default(),
    };
    let journals = temp.path().canonicalize().unwrap().join("journals");
    let store = recovery::Store::new(journals.clone());
    let stopped = store
        .execute_stoppable(&context, &plan, None, || false, || true, |_, _| {})
        .unwrap();
    assert!(stopped.cancelled && stopped.errors.is_empty() && stopped.completed.is_empty());
    assert!(stopped.recovery.is_some());
    assert_eq!(fs::read(&source).unwrap(), b"source bytes");
    assert!(!destination.exists());
    drop(store);
    let store = recovery::Store::new(journals);
    let restored = store.restore(&root, &|| false).unwrap().unwrap();
    assert_eq!(restored.uncertain, 0);
    assert_eq!(
        serde_json::to_value(&restored.plan.actions).unwrap(),
        serde_json::to_value(&plan.actions).unwrap()
    );
    let retry = store
        .execute_stoppable(
            &context,
            &restored.plan,
            Some(&restored.ticket),
            || false,
            || false,
            |_, _| {},
        )
        .unwrap();
    assert!(retry.errors.is_empty() && retry.remaining.actions.is_empty());
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"source bytes");
}

#[test]
fn fallback_copy_preserves_file_dates_and_contents() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.jpg");
    let destination = temp.path().join("destination.jpg");
    fs::write(&source, b"unchanged photo bytes").unwrap();
    let modified = std::time::UNIX_EPOCH + std::time::Duration::new(1_600_000_000, 123_456_789);
    let accessed = modified - std::time::Duration::from_secs(60);
    File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(
            fs::FileTimes::new()
                .set_modified(modified)
                .set_accessed(accessed),
        )
        .unwrap();
    let before = fs::metadata(&source).unwrap();
    copy_contents_exclusive(&source, &destination, &|| false).unwrap();
    let after = fs::metadata(&destination).unwrap();
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(after.accessed().unwrap(), before.accessed().unwrap());
    assert_eq!(fs::read(&destination).unwrap(), b"unchanged photo bytes");
    assert_eq!(fs::read(&source).unwrap(), b"unchanged photo bytes");
}

#[cfg(unix)]
#[test]
fn fallback_copy_keeps_partial_files_private_and_preserves_source_mode() {
    use std::os::unix::fs::PermissionsExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.jpg");
    let destination = temp.path().join("destination.jpg");
    fs::write(&source, vec![3u8; 2 * 1024 * 1024]).unwrap();
    for mode in [0o600, 0o640, 0o444] {
        fs::set_permissions(&source, fs::Permissions::from_mode(mode)).unwrap();
        let saw_partial = std::cell::Cell::new(false);
        let stopped = copy_contents_exclusive(&source, &destination, &|| {
            if let Ok(metadata) = fs::metadata(&destination) {
                assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
                if metadata.len() >= 1024 * 1024 {
                    saw_partial.set(true);
                    return true;
                }
            }
            false
        })
        .unwrap_err();
        assert!(stopped.is::<MoveCancelled>());
        assert!(saw_partial.get());
        assert!(!destination.exists());
        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode() & 0o777,
            mode
        );
        copy_contents_exclusive(&source, &destination, &|| false).unwrap();
        assert_eq!(
            fs::metadata(&destination).unwrap().permissions().mode() & 0o777,
            mode
        );
        assert_eq!(fs::read(&destination).unwrap(), fs::read(&source).unwrap());
        fs::remove_file(&destination).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn same_volume_move_staging_still_uses_a_hard_link() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.jpg");
    let destination = temp.path().join("destination.jpg");
    fs::write(&source, b"photo bytes").unwrap();
    copy_exclusive(&source, &destination, &|| false).unwrap();
    let before = fs::metadata(&source).unwrap();
    let after = fs::metadata(&destination).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
}
