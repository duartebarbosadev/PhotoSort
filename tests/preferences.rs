use photosort::core::{
    jobs::{IoWork, Runtime},
    model::Step,
    preferences::{Preferences, folder_labels},
    startup::Options,
};
use std::ffi::OsString;
#[test]
fn endpoints_and_navigation_share_one_visibility_policy() {
    let mut p = Preferences {
        hidden_steps: Step::ALL.into_iter().collect(),
        ..Default::default()
    };
    p.normalize();
    assert!(p.visible(Step::Organize) && p.visible(Step::Cull));
    assert_eq!(p.neighbor(Step::Organize, true), Some(Step::Cull));
    assert_eq!(p.neighbor(Step::Cull, false), Some(Step::Organize));
    assert_eq!(p.neighbor(Step::Cull, true), None);
    assert_eq!(p.neighbor(Step::Organize, false), None);
}
#[test]
fn history_is_bounded_deduplicated_and_last_folder_skips_missing_entries() {
    let dir = tempfile::tempdir().unwrap();
    let mut p = Preferences::default();
    for i in 0..12 {
        let root = dir.path().join(i.to_string());
        std::fs::create_dir(&root).unwrap();
        p.remember(&root);
    }
    p.remember(&dir.path().join("8/."));
    assert_eq!(p.recent_folders.len(), 10);
    assert_eq!(p.recent_folders[0], dir.path().join("8"));
    std::fs::remove_dir(dir.path().join("8")).unwrap();
    assert_eq!(p.last_existing_folder(), Some(dir.path().join("11")));
}
#[test]
fn folders_sharing_a_name_show_just_enough_parent_path_to_tell_apart() {
    let sep = std::path::MAIN_SEPARATOR_STR;
    let root = std::path::Path::new(sep);
    let paths = [
        root.join("2024/photos/samples"),
        root.join("2023/photos/samples"),
        root.join("Desktop/samples"),
        root.join("trips"),
        root.join("x/samples"),
        root.join("y/x/samples"),
    ];
    assert_eq!(
        folder_labels(&paths),
        [
            format!("samples · 2024{sep}photos"),
            format!("samples · 2023{sep}photos"),
            "samples · Desktop".into(),
            "trips".into(),
            "samples · x".into(),
            format!("samples · y{sep}x"),
        ]
    );
}
#[test]
fn accepted_preference_writes_survive_folder_epochs_and_shutdown_without_touching_bookmarks() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preferences.json");
    let sentinel = dir.path().join("views.json");
    std::fs::write(&sentinel, b"existing view").unwrap();
    let mut p = Preferences {
        show_shortcuts: false,
        ..Default::default()
    };
    p.hidden_steps.insert(Step::Rotation);
    p.remember(dir.path());
    let mut rt = Runtime::with_cache(|| {}, dir.path().join("cache"));
    rt.io(IoWork::SavePreferences {
        path: path.clone(),
        preferences: p.clone(),
    })
    .unwrap();
    rt.next_epoch();
    rt.shutdown();
    assert_eq!(Preferences::load(&path).unwrap(), p);
    assert_eq!(std::fs::read(sentinel).unwrap(), b"existing view");
}
#[test]
fn preferences_report_corruption_and_unknown_versions_without_replacing_them() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("prefs.json");
    assert_eq!(Preferences::load(&path).unwrap(), Preferences::default());
    for bytes in [b"invalid".as_slice(), b"[99, {}]"] {
        std::fs::write(&path, bytes).unwrap();
        assert!(Preferences::load(&path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
    }
}
fn parse(args: &[&str]) -> Result<Options, String> {
    Options::parse(args.iter().map(OsString::from))
}
#[test]
fn startup_options_reject_ambiguous_or_incomplete_folder_requests() {
    assert_eq!(
        parse(&["--folder", "/photos"]).unwrap().folder,
        Some("/photos".into())
    );
    assert_eq!(parse(&["/photos"]).unwrap().folder, Some("/photos".into()));
    assert!(parse(&["--last-folder"]).unwrap().last_folder);
    assert!(parse(&["--help"]).unwrap().help);
    assert!(parse(&["--version"]).unwrap().version);
    for args in [
        vec!["--folder"],
        vec!["--folder", "--last-folder"],
        vec!["--last-folder", "/photos"],
        vec!["a", "b"],
        vec!["--unknown"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
    assert_eq!(
        parse(&["--", "--photos"]).unwrap().folder,
        Some("--photos".into())
    );
}
#[cfg(unix)]
#[test]
fn startup_keeps_non_unicode_folder_paths_intact() {
    use std::os::unix::ffi::OsStringExt;
    let path = OsString::from_vec(b"/photos/\xff".to_vec());
    assert_eq!(
        Options::parse([OsString::from("--folder"), path.clone()])
            .unwrap()
            .folder,
        Some(path.into())
    );
}

#[test]
fn smoke_startup_validates_delay_and_preserves_folder_selection() {
    assert_eq!(
        parse(&["--smoke-test"]).unwrap().smoke_delay(),
        Some(std::time::Duration::from_secs(2))
    );
    assert_eq!(parse(&[]).unwrap().smoke_delay(), None);
    let options = parse(&[
        "--smoke-delay-ms",
        "500",
        "--smoke-test",
        "--folder",
        "/photos",
    ])
    .unwrap();
    assert_eq!(
        options.smoke_delay(),
        Some(std::time::Duration::from_millis(500))
    );
    assert_eq!(options.folder, Some("/photos".into()));
    assert!(
        parse(&["--smoke-test", "--last-folder"])
            .unwrap()
            .last_folder
    );
    for args in [
        vec!["--smoke-delay-ms", "500"],
        vec!["--smoke-test", "--smoke-delay-ms"],
        vec!["--smoke-test", "--smoke-delay-ms", "-1"],
        vec!["--smoke-test", "--smoke-delay-ms", "0"],
        vec!["--smoke-test", "--smoke-delay-ms", "199"],
        vec!["--smoke-test", "--smoke-delay-ms", "60001"],
        vec!["--smoke-test", "--smoke-delay-ms", "18446744073709551616"],
        vec!["--smoke-test", "--smoke-delay-ms", "1.5"],
        vec![
            "--smoke-test",
            "--smoke-delay-ms",
            "500",
            "--smoke-delay-ms",
            "600",
        ],
        vec!["--smoke-test", "--diagnostics"],
        vec!["--check-runtime", "--smoke-test"],
    ] {
        assert!(parse(&args).is_err(), "{args:?}");
    }
    for value in ["200", "60000"] {
        assert!(parse(&["--smoke-test", "--smoke-delay-ms", value]).is_ok());
    }
}

#[test]
fn a_partial_preference_file_gains_defaults_and_every_setting_round_trips() {
    use photosort::core::{geography::Layout, preferences::StripPosition, processing};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("preferences.json");
    // A file holding one setting: the rest take their defaults.
    std::fs::write(&path, br#"[1, {"show_shortcuts": false}]"#).unwrap();
    let mut p = Preferences::load(&path).unwrap();
    assert!(!p.show_shortcuts);
    assert!(!p.intro_shown);
    assert_eq!(p.easy_delete.blur, 100.0);
    assert_eq!(p.navigation, Default::default());
    assert_eq!(p.location_layout, Layout::CountryRegionCity);
    assert!(p.keep_companions, "companions stay together by default");
    assert_eq!(p.strip_position, StripPosition::Bottom);
    assert!(p.show_filenames);
    assert_eq!(p.processing.mode, processing::Mode::Balanced);
    p.intro_shown = true;
    p.easy_delete.blur = 250.0;
    p.easy_delete.dark = 8.0;
    p.easy_delete.bright = 252.0;
    p.navigation = photosort::core::navigation::Preferences {
        rating: Some(4),
        skip_singletons: true,
        cycle_similarity: true,
    };
    p.keep_companions = false;
    p.strip_position = StripPosition::Left;
    p.show_filenames = false;
    p.processing = processing::Preferences {
        mode: processing::Mode::Custom,
        custom_threads: processing::available(),
        ..Default::default()
    };
    for layout in [
        Layout::Country,
        Layout::CountryCity,
        Layout::CountryRegionCity,
    ] {
        p.location_layout = layout;
        p.save(&path).unwrap();
        assert_eq!(Preferences::load(&path).unwrap(), p);
    }
    // A rating the filters do not offer is dropped.
    p.navigation.rating = Some(9);
    p.normalize();
    assert_eq!(p.navigation.rating, None);
}
