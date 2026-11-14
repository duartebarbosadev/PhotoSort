use super::*;

/// The copy being written in `folder`, under its temporary name.
fn partial_copy(folder: &Path) -> Option<PathBuf> {
    fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .find(|path| path.file_name().is_some_and(is_temporary_name))
}

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
        assert!(!destination.exists(), "a partial copy has the final name");
        let size = partial_copy(dir.path())
            .and_then(|partial| fs::metadata(partial).ok())
            .map_or(0, |m| m.len());
        observed.set(observed.get().max(size));
        size >= 1024 * 1024
    })
    .unwrap_err();
    assert!(error.is::<MoveCancelled>());
    assert_eq!(observed.get(), 1024 * 1024);
    assert!(!destination.exists() && partial_copy(dir.path()).is_none());
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
    let blocked = std::cell::RefCell::new(PathBuf::new());
    let result = copy_contents_exclusive(&source, &destination, &|| {
        if !stopped.get()
            && let Some(partial) = partial_copy(temp.path())
            && fs::metadata(&partial).is_ok_and(|m| m.len() >= 1024 * 1024)
        {
            fs::rename(&partial, &parked).unwrap();
            fs::create_dir(&partial).unwrap();
            blocked.replace(partial);
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
    assert!(blocked.borrow().is_dir() && !destination.exists());
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
fn copy_killed_midway_leaves_no_file_under_the_final_name_and_retry_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let output = root.join("Sorted");
    fs::create_dir(&output).unwrap();
    let source = root.join("photo.jpg");
    let bytes: Vec<u8> = (0..3 * 1024 * 1024 + 7).map(|i| (i % 249) as u8).collect();
    fs::write(&source, &bytes).unwrap();
    let destination = output.join("photo.jpg");
    let plan = Plan {
        actions: vec![Action::Move {
            files: vec![(
                Source::capture(source.clone()).unwrap(),
                destination.clone(),
            )],
        }],
        ..Default::default()
    };
    // Another volume, where only a copy can reach the destination. The app
    // dies with the copy half written: nothing gets to clean up after it.
    COPY_ONLY.set(true);
    let killed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        execute(
            &plan,
            &root.join("journal"),
            || {
                let written = partial_copy(&output)
                    .and_then(|partial| fs::metadata(partial).ok())
                    .map_or(0, |m| m.len());
                assert!(written < 1024 * 1024, "killed while copying");
                false
            },
            |_, _| {},
        )
    }));
    assert!(killed.is_err());
    assert!(!destination.exists(), "a partial copy has the final name");
    let partial = partial_copy(&output).expect("the partial copy keeps a temporary name");
    assert!(fs::metadata(&partial).unwrap().len() < bytes.len() as u64);
    assert_eq!(fs::read(&source).unwrap(), bytes);
    // Reopened: the leftover is not a photo, a companion or an Organize file.
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    assert!(catalog.media.iter().all(|media| media.path != partial));
    assert!(!catalog.inventory.contains(&partial));
    assert!(
        catalog
            .organize_files()
            .iter()
            .all(|(_, path)| *path != partial)
    );
    // Retry clears it when it next writes to that folder, and the move lands.
    let retry = execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
    COPY_ONLY.set(false);
    assert!(retry.errors.is_empty(), "{:?}", retry.errors);
    assert!(retry.remaining.actions.is_empty());
    assert_eq!(fs::read(&destination).unwrap(), bytes);
    assert!(!source.exists() && partial_copy(&output).is_none());
}

#[test]
fn fallback_copy_preserves_file_dates_and_contents() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.jpg");
    let destination = temp.path().join("destination.jpg");
    fs::write(&source, b"unchanged photo bytes").unwrap();
    let modified = std::time::UNIX_EPOCH + std::time::Duration::new(1_600_000_000, 123_456_789);
    let accessed = modified - std::time::Duration::from_secs(60);
    let times = fs::FileTimes::new()
        .set_modified(modified)
        .set_accessed(accessed);
    // Taken years before it was last edited, where the system keeps that date.
    #[cfg(any(target_os = "macos", windows))]
    let created = modified - std::time::Duration::from_secs(10 * 365 * 24 * 60 * 60);
    #[cfg(any(target_os = "macos", windows))]
    let times = {
        #[cfg(target_os = "macos")]
        use std::os::macos::fs::FileTimesExt;
        #[cfg(windows)]
        use std::os::windows::fs::FileTimesExt;
        times.set_created(created)
    };
    File::options()
        .write(true)
        .open(&source)
        .unwrap()
        .set_times(times)
        .unwrap();
    let before = fs::metadata(&source).unwrap();
    copy_contents_exclusive(&source, &destination, &|| false).unwrap();
    let after = fs::metadata(&destination).unwrap();
    assert_eq!(after.modified().unwrap(), before.modified().unwrap());
    assert_eq!(after.accessed().unwrap(), before.accessed().unwrap());
    #[cfg(any(target_os = "macos", windows))]
    {
        assert!(before.created().unwrap() < before.modified().unwrap());
        assert_eq!(
            after.created().unwrap(),
            before.created().unwrap(),
            "the copy lost its creation date"
        );
    }
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
            if let Some(metadata) =
                partial_copy(temp.path()).and_then(|partial| fs::metadata(partial).ok())
            {
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

/// On a volume without extended attributes macOS keeps a `._` file beside
/// each file, a temporary one included, and nothing locks it. A sweep by a
/// second PhotoSort, or by recovery, must not take it from a copy in progress.
#[test]
fn sweeping_a_folder_leaves_the_companion_of_a_temporary_file_in_use() {
    let temp = tempfile::tempdir().unwrap();
    let folder = temp.path();
    let companion = |path: &Path| {
        let name = path.file_name().unwrap().to_str().unwrap();
        path.with_file_name(format!("._{name}"))
    };
    let target = |name: &'static str| Some(std::ffi::OsStr::new(name));
    // A copy another PhotoSort is writing, and one a crash left behind.
    let (copying, in_use) = create_temporary(folder, target("IMG_1.JPG")).unwrap();
    let (crashed, left) = create_temporary(folder, target("IMG_2.JPG")).unwrap();
    drop(crashed);
    // Companions whose temporary file is gone: one just now, one long ago.
    let moving = folder.join("._.photosort-00000000000000aa.IMG_3.JPG.tmp");
    let orphan = folder.join("._.photosort-00000000000000bb.IMG_4.JPG.tmp");
    for path in [
        companion(&in_use),
        companion(&left),
        moving.clone(),
        orphan.clone(),
    ] {
        assert!(is_temporary_name(path.file_name().unwrap()));
        fs::write(path, b"attributes").unwrap();
    }
    File::options()
        .write(true)
        .open(&orphan)
        .unwrap()
        .set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
        .unwrap();
    sweep_temporaries(folder);
    assert!(in_use.exists(), "a temporary file in use was removed");
    assert!(
        companion(&in_use).exists(),
        "a copy in progress lost its attribute file"
    );
    assert!(!left.exists() && !companion(&left).exists());
    assert!(moving.exists() && !orphan.exists());
    // Once that copy is over or its PhotoSort is gone, both names are stale.
    drop(copying);
    sweep_temporaries(folder);
    assert!(!in_use.exists() && !companion(&in_use).exists());
}

/// exFAT and FAT on macOS have neither hard links nor an atomic exclusive
/// rename, so a name is reserved with an empty file before the rename. An
/// empty sidecar cannot go that way, and still arrives with its photos.
#[test]
fn an_empty_sidecar_moves_with_its_photos_where_the_volume_only_renames() {
    for reserved in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let sorted = root.join("Sorted");
        let files: Vec<_> = [
            ("IMG_1.CR2", &b"raw bytes"[..]),
            ("IMG_1.JPG", b"jpeg bytes"),
            ("IMG_1.xmp", b""),
        ]
        .into_iter()
        .map(|(name, bytes)| {
            fs::write(root.join(name), bytes).unwrap();
            (
                Source::capture(root.join(name)).unwrap(),
                sorted.join(name),
                bytes,
            )
        })
        .collect();
        let plan = Plan {
            actions: vec![Action::Move {
                files: files
                    .iter()
                    .map(|(source, destination, _)| (source.clone(), destination.clone()))
                    .collect(),
            }],
            ..Default::default()
        };
        NO_HARD_LINKS.set(true);
        NO_EXCLUSIVE_RENAME.set(reserved);
        let report = execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
        NO_HARD_LINKS.set(false);
        NO_EXCLUSIVE_RENAME.set(false);
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.remaining.actions.is_empty() && report.completed.len() == 1);
        for (source, destination, bytes) in &files {
            assert!(!source.path.exists(), "{}", source.path.display());
            assert_eq!(fs::read(destination).unwrap(), *bytes);
            // The photos were renamed, not copied with the empty file.
            #[cfg(unix)]
            if !bytes.is_empty() {
                assert_eq!(Fingerprint::read(destination).unwrap(), source.fingerprint);
            }
        }
        assert_eq!(fs::read_dir(&sorted).unwrap().count(), files.len());
    }
}

/// A staged move kept in a retry plan is finished in a later run, by then
/// perhaps under another device number: the saved one is from the old mount.
#[cfg(unix)]
#[test]
fn a_staged_move_is_proven_and_finished_after_its_drive_was_mounted_again() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    let source = root.join("photo.jpg");
    let destination = root.join("Sorted/photo.jpg");
    fs::create_dir(root.join("Sorted")).unwrap();
    fs::write(&source, b"photo bytes").unwrap();
    fs::hard_link(&source, &destination).unwrap();
    let mut saved = Source::capture(source.clone()).unwrap();
    saved.fingerprint.identity[0] += 12;
    // A copy has the source's size and dates but is another file: no proof.
    let copy = root.join("Sorted/copy.jpg");
    fs::copy(&source, &copy).unwrap();
    File::options()
        .write(true)
        .open(&copy)
        .unwrap()
        .set_modified(fs::metadata(&source).unwrap().modified().unwrap())
        .unwrap();
    assert!(verify_staged_move(&saved, &copy).is_err());
    // The link is, and what was proven is the file as this mount shows it.
    let proven = verify_staged_move(&saved, &destination).unwrap();
    assert_eq!(proven, Fingerprint::read(&destination).unwrap());
    let plan = Plan {
        actions: vec![Action::FinishMove {
            files: vec![(saved, destination.clone())],
        }],
        ..Default::default()
    };
    let report = execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(!source.exists());
    assert_eq!(fs::read(&destination).unwrap(), b"photo bytes");
}

#[cfg(unix)]
#[test]
fn same_volume_move_staging_still_uses_a_hard_link() {
    use std::os::unix::fs::MetadataExt;
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.jpg");
    let destination = temp.path().join("destination.jpg");
    fs::write(&source, b"photo bytes").unwrap();
    assert!(matches!(link_exclusive(&source, &destination), Link::Made));
    let before = fs::metadata(&source).unwrap();
    let after = fs::metadata(&destination).unwrap();
    assert_eq!((before.dev(), before.ino()), (after.dev(), after.ino()));
    assert_eq!(before.modified().unwrap(), after.modified().unwrap());
}

#[cfg(target_os = "macos")]
mod macos_attributes {
    use super::*;
    use std::{ffi::CString, os::fd::AsRawFd};

    unsafe extern "C" {
        fn fsetxattr(
            fd: i32,
            name: *const std::ffi::c_char,
            value: *const u8,
            size: usize,
            position: u32,
            options: i32,
        ) -> i32;
        fn fgetxattr(
            fd: i32,
            name: *const std::ffi::c_char,
            value: *mut u8,
            size: usize,
            position: u32,
            options: i32,
        ) -> isize;
    }
    fn set(file: &File, name: &str, bytes: &[u8]) {
        let name = CString::new(name).unwrap();
        assert_eq!(
            unsafe {
                fsetxattr(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    bytes.as_ptr(),
                    bytes.len(),
                    0,
                    0,
                )
            },
            0,
            "{}",
            std::io::Error::last_os_error()
        );
    }
    fn get(file: &File, name: &str) -> Option<Vec<u8>> {
        let name = CString::new(name).unwrap();
        let size = unsafe {
            fgetxattr(
                file.as_raw_fd(),
                name.as_ptr(),
                std::ptr::null_mut(),
                0,
                0,
                0,
            )
        };
        if size < 0 {
            return None;
        }
        let mut bytes = vec![0; size as usize];
        assert_eq!(
            unsafe {
                fgetxattr(
                    file.as_raw_fd(),
                    name.as_ptr(),
                    bytes.as_mut_ptr(),
                    bytes.len(),
                    0,
                    0,
                )
            },
            size
        );
        Some(bytes)
    }
    #[test]
    fn fallback_copy_preserves_finder_tags_resource_fork_and_custom_attributes() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photo.jpg");
        let destination = temp.path().join("destination.jpg");
        fs::write(&source, b"unchanged photo data fork").unwrap();
        let input = File::open(&source).unwrap();
        let mut finder_info = vec![0; 32];
        finder_info[..4].copy_from_slice(b"JPEG");
        let attributes = [
            ("com.apple.metadata:_kMDItemUserTags", b"<?xml version=\"1.0\"?><plist version=\"1.0\"><array><string>Keep\n6</string></array></plist>".to_vec()),
            ("com.apple.FinderInfo", finder_info),
            ("com.apple.ResourceFork", (0..2*1024*1024+19).map(|n| (n%251) as u8).collect()),
            ("dev.photosort.test", vec![0, 1, 255, 0, 42]),
            ("dev.photosort.empty", vec![]),
        ];
        for (name, bytes) in &attributes {
            set(&input, name, bytes);
        }
        let before = Source::capture(source.clone()).unwrap();
        copy_contents_exclusive(&source, &destination, &|| false).unwrap();
        let output = File::open(&destination).unwrap();
        assert_eq!(
            fs::read(&destination).unwrap(),
            b"unchanged photo data fork"
        );
        before.verify().unwrap();
        for (name, bytes) in attributes {
            assert_eq!(get(&output, name), Some(bytes.clone()), "lost {name}");
            assert_eq!(get(&input, name), Some(bytes), "changed source {name}");
        }
    }
    #[test]
    fn fallback_copy_cancels_attribute_transfer_and_rolls_back() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("photo.jpg");
        let destination = temp.path().join("destination.jpg");
        fs::write(&source, b"photo data").unwrap();
        let input = File::open(&source).unwrap();
        set(&input, "dev.photosort.first", b"first");
        set(&input, "dev.photosort.second", b"second");
        let before = Source::capture(source.clone()).unwrap();
        let saw_attribute = std::cell::Cell::new(false);
        let error = copy_contents_exclusive(&source, &destination, &|| {
            let partial = partial_copy(temp.path()).unwrap_or_default();
            let present = File::open(partial).is_ok_and(|f| {
                get(&f, "dev.photosort.first").is_some()
                    || get(&f, "dev.photosort.second").is_some()
            });
            saw_attribute.set(saw_attribute.get() || present);
            present
        })
        .unwrap_err();
        assert!(error.is::<MoveCancelled>(), "{error:#}");
        assert!(
            saw_attribute.get(),
            "did not reach attribute-copy cancellation"
        );
        assert!(!destination.exists());
        before.verify().unwrap();
        assert_eq!(get(&input, "dev.photosort.first"), Some(b"first".to_vec()));
        assert_eq!(
            get(&input, "dev.photosort.second"),
            Some(b"second".to_vec())
        );
        copy_contents_exclusive(&source, &destination, &|| false).unwrap();
        let output = File::open(&destination).unwrap();
        assert_eq!(get(&output, "dev.photosort.first"), Some(b"first".to_vec()));
        assert_eq!(
            get(&output, "dev.photosort.second"),
            Some(b"second".to_vec())
        );
    }
    #[test]
    fn fallback_copy_attribute_adapter_reports_write_failure_and_keeps_handles_open() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::write(&source, b"source bytes").unwrap();
        fs::write(&destination, b"existing bytes").unwrap();
        let input = File::open(source).unwrap();
        set(&input, "dev.photosort.test", b"value");
        // fsetxattr on macOS accepts read-only descriptors. A pipe is instead an
        // unsupported destination, exercising native error propagation safely.
        let (output, _writer) = std::io::pipe().unwrap();
        use std::os::fd::{FromRawFd, IntoRawFd};
        let output = unsafe { File::from_raw_fd(output.into_raw_fd()) };
        let error = super::super::macos_attributes::copy(&input, &output, &|| false).unwrap_err();
        assert!(!error.is::<MoveCancelled>());
        assert!(format!("{error:#}").contains("extended attributes"));
        assert!(input.metadata().is_ok());
        assert!(output.metadata().is_ok());
        assert_eq!(get(&input, "dev.photosort.test"), Some(b"value".to_vec()));
        assert_eq!(fs::read(&destination).unwrap(), b"existing bytes");
    }
    #[test]
    #[ignore = "Requires PHOTOSORT_SAMPLE_DIR and PHOTOSORT_MOVE_DESTINATION on a disposable second volume"]
    fn seven_samples_cross_volume_moves_preserve_attributes_and_originals() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let samples =
            PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("sample folder"));
        let target = PathBuf::from(
            std::env::var_os("PHOTOSORT_MOVE_DESTINATION").expect("disposable second volume"),
        );
        let source_dir = tempfile::tempdir().unwrap();
        let destination_dir = tempfile::tempdir_in(target).unwrap();
        assert_ne!(
            fs::metadata(source_dir.path()).unwrap().dev(),
            fs::metadata(destination_dir.path()).unwrap().dev(),
            "must use distinct volumes"
        );
        let mut originals = Vec::new();
        let mut expected = Vec::new();
        let mut actions = Vec::new();
        for entry in fs::read_dir(samples).unwrap() {
            let original = entry.unwrap().path();
            if !original.is_file() {
                continue;
            }
            let original_before = Source::capture(original.clone()).unwrap();
            let original_hash = content_hash(&original, || false).unwrap();
            let source = source_dir.path().join(original.file_name().unwrap());
            let destination = destination_dir.path().join(original.file_name().unwrap());
            fs::copy(&original, &source).unwrap();
            let file = File::open(&source).unwrap();
            set(&file, "dev.photosort.test", b"cross-volume sample");
            set(&file, "com.apple.metadata:_kMDItemUserTags", b"<?xml version=\"1.0\"?><plist version=\"1.0\"><array><string>Keep\n6</string></array></plist>");
            set(&file, "com.apple.ResourceFork", b"sample resource fork");
            fs::set_permissions(&source, fs::Permissions::from_mode(0o640)).unwrap();
            let metadata = file.metadata().unwrap();
            expected.push((
                destination.clone(),
                original_hash.clone(),
                metadata.modified().unwrap(),
                [
                    "dev.photosort.test",
                    "com.apple.metadata:_kMDItemUserTags",
                    "com.apple.ResourceFork",
                ]
                .map(|name| (name, get(&file, name).unwrap())),
            ));
            actions.push(Action::Move {
                files: vec![(Source::capture(source).unwrap(), destination)],
            });
            originals.push((original_before, original_hash));
        }
        assert_eq!(actions.len(), 7, "expected the seven authorized samples");
        let plan = Plan {
            actions,
            ..Default::default()
        };
        let report = execute(
            &plan,
            &source_dir.path().join("journals"),
            || false,
            |_, _| {},
        )
        .unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        assert!(report.remaining.actions.is_empty() && !report.cancelled);
        for (destination, hash, modified, attributes) in expected {
            assert_eq!(content_hash(&destination, || false).unwrap(), hash);
            let file = File::open(&destination).unwrap();
            assert_eq!(file.metadata().unwrap().modified().unwrap(), modified);
            assert_eq!(file.metadata().unwrap().permissions().mode() & 0o777, 0o640);
            for (name, bytes) in attributes {
                assert_eq!(get(&file, name), Some(bytes), "lost {name}");
            }
            assert!(
                !source_dir
                    .path()
                    .join(destination.file_name().unwrap())
                    .exists()
            );
        }
        for (before, hash) in originals {
            before.verify().unwrap();
            assert_eq!(content_hash(&before.path, || false).unwrap(), hash);
        }
    }
}
