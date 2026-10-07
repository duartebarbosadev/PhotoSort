use super::*;
use crate::core::{
    startup::Options,
    views::{Bookmark, Saved},
};

fn fixture() -> (tempfile::TempDir, PathBuf, PathBuf) {
    // Relative paths are resolved against Cargo's cwd without changing the
    // process-wide cwd or touching the read-only Python reference.
    let dir = tempfile::tempdir_in(std::env::current_dir().unwrap()).unwrap();
    let root = dir.path().join("library");
    std::fs::create_dir(&root).unwrap();
    for name in ["a.png", "b.png"] {
        image::RgbaImage::new(2, 2).save(root.join(name)).unwrap();
    }
    let relative = PathBuf::from(dir.path().file_name().unwrap()).join("library");
    (dir, root.canonicalize().unwrap(), relative)
}
fn app(dir: &tempfile::TempDir) -> PhotoSortApp {
    PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None)
}
fn settle(app: &mut PhotoSortApp) {
    let until = Instant::now() + Duration::from_secs(10);
    let ctx = egui::Context::default();
    while app.scanning || app.busy {
        app.poll(&ctx);
        assert!(
            Instant::now() < until,
            "folder work did not finish: {:?}",
            app.error
        );
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
}

#[test]
fn relative_cli_folder_adopts_canonical_default_and_current_folders_is_a_noop() {
    for flag in [false, true] {
        let (dir, root, relative) = fixture();
        let mut args = Vec::new();
        if flag {
            args.push("--folder".into());
        }
        args.push(relative.into_os_string());
        let options = Options::parse(args).unwrap();
        let mut app = app(&dir);
        app.open(options.folder.unwrap());
        settle(&mut app);
        assert_eq!(app.root.as_ref(), Some(&root));
        assert_eq!(PathBuf::from(&app.output), root);
        assert_eq!(app.catalog.as_ref().unwrap().root, root);
        assert_eq!(app.preferences.last_existing_folder(), Some(root));
        assert_eq!(app.media.len(), 2);
        assert_eq!(app.stats.decodes, 0);
        app.organize();
        settle(&mut app);
        assert!(app.plan.as_ref().unwrap().actions.is_empty());
        app.plan_open = false;
        app.set_review_plan(None);
        app.switch_step(Step::Rotation);
        assert_eq!(app.step, Step::Rotation);
        assert!(app.organize_departure.is_none());
        assert_eq!(app.stats.decodes, 0);
    }
}

#[test]
fn destination_edited_during_discovery_is_preserved() {
    let (dir, root, relative) = fixture();
    for destination in [
        String::new(),
        dir.path().join("chosen-output").display().to_string(),
        "chosen-relative-output".into(),
    ] {
        let mut app = app(&dir);
        app.open(relative.clone());
        assert!(app.scanning);
        app.output.clone_from(&destination);
        settle(&mut app);
        assert_eq!(app.output, destination);
        assert_eq!(app.root.as_ref(), Some(&root));
    }
}

#[test]
fn alternate_open_spellings_reuse_session_bookmarks_and_one_recent_folder() {
    let (dir, root, relative) = fixture();
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let focus = catalog.media[1].id;
    let mut session = Session::default();
    session.decisions.insert(
        focus,
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    session.fingerprints = catalog
        .media
        .iter()
        .map(|m| (m.id, m.fingerprint))
        .collect();
    session.save(&root).unwrap();
    let mut views = Saved {
        step: Some(Step::Organize),
        ..Default::default()
    };
    views.bookmarks.insert(
        Step::Organize,
        Bookmark {
            active: Some(focus),
            selected: vec![focus],
            query: "b.png".into(),
            ..Default::default()
        },
    );
    let index = catalog
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    views.capture_catalog_fingerprints(&catalog, &index);
    views.save(&root).unwrap();
    let spellings = vec![
        relative.clone(),
        relative.join("."),
        relative.join("..").join("library"),
        root.clone(),
    ];
    #[cfg(unix)]
    let spellings = {
        let alias = dir.path().join("library-link");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        spellings.into_iter().chain([alias]).collect::<Vec<_>>()
    };
    let mut preferences = crate::core::preferences::Preferences::default();
    for spelling in spellings {
        let mut app = app(&dir);
        app.preferences = preferences;
        app.open(spelling);
        settle(&mut app);
        let until = Instant::now() + Duration::from_secs(10);
        while app.pending_bookmark.is_some() {
            app.refilter();
            app.restore_bookmark();
            assert!(Instant::now() < until, "saved view did not settle");
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(app.active, Some(focus));
        assert_eq!(app.query, "b.png");
        assert_eq!(app.decisions[&focus].rating, Some(4));
        assert_eq!(app.root.as_ref(), Some(&root));
        assert_eq!(PathBuf::from(&app.output), root);
        assert_eq!(app.preferences.recent_folders, vec![root.clone()]);
        assert_eq!(app.preferences.last_existing_folder(), Some(root.clone()));
        assert_eq!(app.stats.decodes, 0);
        preferences = app.preferences.clone();
    }
    // Opening from persisted recent history takes the same application path.
    let preferences_file = dir.path().join("preferences.json");
    preferences.save(&preferences_file).unwrap();
    let recent = crate::core::preferences::Preferences::load(&preferences_file).unwrap();
    let mut app = app(&dir);
    app.open(recent.last_existing_folder().unwrap());
    settle(&mut app);
    assert_eq!(
        app.saved_views.bookmarks[&Step::Organize].active,
        Some(focus)
    );
    assert_eq!(PathBuf::from(&app.output), root);
}

#[test]
fn opened_event_reuses_worker_identity_without_rescanning_or_resolving_on_ui_thread() {
    let (dir, root, relative) = fixture();
    let catalog = Arc::new(Catalog::scan(&root, || false, |_| {}).unwrap());
    // Resolution and scanning succeeded on the worker. Once that result exists,
    // accepting it must not do another filesystem lookup on the UI thread.
    std::fs::rename(&root, dir.path().join("temporarily-unavailable")).unwrap();
    let mut app = app(&dir);
    app.root = Some(relative.clone());
    app.output = relative.display().to_string();
    app.scanning = true;
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    app.runtime.events = rx;
    tx.send(crate::core::jobs::Envelope {
        epoch: app.runtime.epoch(),
        event: Event::Opened(catalog.clone(), Session::default(), Saved::default()),
    })
    .unwrap();
    app.poll(&egui::Context::default());
    assert_eq!(app.root.as_ref(), Some(&root));
    assert_eq!(PathBuf::from(&app.output), root);
    assert!(Arc::ptr_eq(app.catalog.as_ref().unwrap(), &catalog));
    assert_eq!(app.media.len(), 2);
    assert!(!app.scanning && app.error.is_none());
    assert_eq!(app.stats.decodes, 0);
    assert_eq!(app.stats.metadata_reads, 0);
}
