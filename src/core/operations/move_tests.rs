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
            let present = File::open(&destination).is_ok_and(|f| {
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
