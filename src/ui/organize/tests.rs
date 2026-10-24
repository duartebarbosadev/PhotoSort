use super::*;
use crate::core::organize::Draft;
use crate::test_support::worker_patience;
#[test]
fn location_layout_reuses_metadata_for_the_board_and_inspector_without_image_work() {
    use crate::core::geography::{Layout, Location};
    let mut app = app();
    app.group_mode = GroupMode::Location;
    for media in &app.media {
        app.metadata.insert(
            media.id,
            Metadata {
                location: Some(Location {
                    country: "Japan".into(),
                    region: "Shizuoka".into(),
                    city: "Fuji".into(),
                }),
                ..Default::default()
            },
        );
    }
    for (layout, expected) in [
        (Layout::Country, "Japan"),
        (Layout::CountryCity, "Japan/Fuji"),
        (Layout::CountryRegionCity, "Japan/Shizuoka/Fuji"),
    ] {
        app.change_location_layout(layout);
        assert_eq!(app.label_for(&app.media[0]), expected);
        assert!(
            app.organize_labels()
                .values()
                .all(|label| label == expected)
        );
        let draft = Draft::from_catalog(app.catalog.as_ref().unwrap(), app.organize_labels());
        assert!(draft.folders().contains(expected));
        assert!(
            app.requested.is_empty()
                && app.metadata_run.is_none()
                && app.analysis_run.is_none()
                && !app.scanning
                && !app.busy
        );
        assert_eq!(app.stats.decodes, 0);
        assert_eq!(app.metadata.len(), 3);
    }
}
#[test]
fn location_layout_requires_explicit_replacement_and_preserves_other_intent() {
    use crate::core::geography::Layout;
    let mut app = app();
    app.group_mode = GroupMode::Location;
    let draft = Draft::new([(MediaId(0), "Custom location".into())].into());
    app.organize_drafts
        .insert(GroupMode::Location, draft.clone());
    app.organize_drafts.insert(GroupMode::Camera, draft.clone());
    app.file_names.insert(MediaId(0), "Keeper".into());
    app.decisions.insert(
        MediaId(0),
        Decision {
            rating: Some(4),
            ..Default::default()
        },
    );
    app.change_location_layout(Layout::Country);
    assert_eq!(app.preferences.location_layout, Layout::CountryRegionCity);
    assert!(app.modal_active());
    app.resolve_location_layout(false);
    assert_eq!(app.organize_drafts[&GroupMode::Location], draft);
    assert_eq!(app.preferences.location_layout, Layout::CountryRegionCity);
    app.change_location_layout(Layout::CountryCity);
    app.resolve_location_layout(true);
    assert_eq!(app.preferences.location_layout, Layout::CountryCity);
    assert!(!app.organize_drafts.contains_key(&GroupMode::Location));
    assert_eq!(app.organize_drafts[&GroupMode::Camera], draft);
    assert_eq!(app.file_names[&MediaId(0)], "Keeper");
    assert_eq!(app.decisions[&MediaId(0)].rating, Some(4));
    assert!(app.requested.is_empty() && app.analysis_run.is_none() && app.plan.is_none());
    for state in 0..5 {
        app.step = if state == 0 {
            Step::Cull
        } else {
            Step::Organize
        };
        app.busy = state == 1;
        app.scanning = state == 2;
        app.analysis_run = (state == 3).then_some(1);
        app.plan_open = state == 4;
        app.change_location_layout(Layout::Country);
        assert_eq!(app.preferences.location_layout, Layout::CountryCity);
    }
}
fn metadata_fixture(count: usize) -> (PhotoSortApp, tempfile::TempDir) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("photos");
    std::fs::create_dir(&root).unwrap();
    for i in 0..count {
        image::RgbImage::new(4, 4)
            .save(root.join(format!("photo-{i:04}.png")))
            .unwrap();
    }
    let catalog = Arc::new(Catalog::scan(&root, || false, |_| {}).unwrap());
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some(catalog.root.clone());
    app.output = catalog.root.display().to_string();
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.catalog = Some(catalog);
    (app, temp)
}
fn wait_metadata(app: &mut PhotoSortApp) {
    let deadline = Instant::now() + worker_patience();
    while app.metadata_run.is_some() || app.busy {
        app.poll(&egui::Context::default());
        assert!(
            Instant::now() < deadline,
            "metadata or planning did not finish"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn metadata_organize_modes_coalesce_one_read_and_wait_for_offscreen_photos() {
    let (mut app, _temp) = metadata_fixture(129);
    let catalog = app.catalog.clone().unwrap();
    app.set_group_mode(GroupMode::Date);
    let run = app.metadata_run.unwrap();
    assert!(!app.organize_grouping_ready() && !app.can_edit_folders());
    app.organize();
    assert!(app.plan.is_none() && !app.busy);
    for mode in [GroupMode::Location, GroupMode::Camera] {
        app.set_group_mode(mode);
        assert_eq!(app.metadata_run, Some(run));
    }
    wait_metadata(&mut app);
    assert!(app.organize_grouping_ready() && app.can_edit_folders());
    assert_eq!(app.metadata.len(), 129);
    assert_eq!(app.stats.metadata_reads, 129);
    assert_eq!(app.stats.decodes, 0);
    assert!(app.analysis_run.is_none() && app.analysis.is_empty() && app.requested.is_empty());
    assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
    for mode in [GroupMode::Date, GroupMode::Location, GroupMode::Camera] {
        app.set_group_mode(mode);
        assert!(app.metadata_run.is_none());
    }
    let labels = app.organize_labels();
    assert_eq!(labels.len(), 129);
    assert!(labels.values().all(|label| label == "Unknown camera"));
    app.organize();
    wait_metadata(&mut app);
    assert_eq!(app.plan.as_ref().unwrap().actions.len(), 129);
    assert_eq!(app.stats.metadata_reads, 129);
    assert!(app.media.iter().all(|m| m.path.is_file()));
}
#[test]
fn metadata_organize_cancel_rejects_late_batches_and_resume_reuses_completed_data() {
    let (mut app, _temp) = metadata_fixture(129);
    app.set_group_mode(GroupMode::Date);
    let old = app.metadata_run.unwrap();
    app.set_group_mode(GroupMode::Current);
    assert!(app.metadata_run.is_none());
    // Drain any already-published old batch. It must not revive the cancelled run.
    for _ in 0..10 {
        app.poll(&egui::Context::default());
    }
    assert!(app.metadata.is_empty());
    app.set_group_mode(GroupMode::Camera);
    assert_ne!(app.metadata_run, Some(old));
    wait_metadata(&mut app);
    let reads = app.stats.metadata_reads;
    app.cancel_metadata_index();
    app.start_metadata_index();
    assert!(app.metadata_run.is_none());
    assert_eq!(app.metadata.len(), 129);
    assert_eq!(app.stats.metadata_reads, reads);
    assert_eq!(app.stats.decodes, 0);
    assert!(app.plan.is_none() && app.analysis.is_empty());
}
#[test]
fn metadata_organize_missing_file_blocks_proposals_until_shared_retry_succeeds() {
    let (mut app, temp) = metadata_fixture(2);
    let path = app.media[0].path.clone();
    let backup = temp.path().join("temporarily-unavailable.png");
    std::fs::rename(&path, &backup).unwrap();
    app.set_group_mode(GroupMode::Location);
    wait_metadata(&mut app);
    assert_eq!(app.metadata.len(), 1);
    assert_eq!(app.metadata_failures.len(), 1);
    assert!(!app.organize_grouping_ready() && !app.can_edit_folders());
    assert!(app.plan.is_none());
    std::fs::rename(backup, path).unwrap();
    app.start_metadata_index();
    wait_metadata(&mut app);
    assert_eq!(app.metadata.len(), 2);
    assert_eq!(app.stats.metadata_reads, 2);
    assert!(app.metadata_failures.is_empty() && app.can_edit_folders());
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn metadata_organize_departure_waits_for_complete_metadata_before_reviewing_names() {
    let (mut app, _temp) = metadata_fixture(2);
    app.group_mode = GroupMode::Camera;
    app.file_names.insert(app.media[0].id, "Keeper".into());
    app.request_departure(Destination::Step(Step::Cull));
    assert!(app.organize_departure.is_some() && app.metadata_run.is_some());
    assert!(app.plan.is_none() && !app.busy && app.step == Step::Organize);
    wait_metadata(&mut app);
    assert!(app.plan.is_some() && app.organize_departure.is_some());
    assert_eq!(app.stats.metadata_reads, 2);
    assert_eq!(app.stats.decodes, 0);
    assert!(app.media.iter().all(|m| m.path.is_file()));
    app.resolve_organize_departure(super::super::transitions::OrganizeResolution::Cancel);
    assert!(app.plan.is_none() && app.file_names.len() == 1);
}
fn app() -> PhotoSortApp {
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    // Nonexistent paths prove folder editing needs no rescan or image load.
    let media: Vec<_> = [
        "/missing/trip/a.jpg",
        "/missing/trip/a.ARW",
        "/missing/trip/b.jpg",
    ]
    .into_iter()
    .enumerate()
    .map(|(i, path)| Media {
        id: MediaId(i as u64),
        path: path.into(),
        kind: MediaKind::Photo,
        fingerprint: Default::default(),
    })
    .collect();
    app.catalog = Some(Arc::new(Catalog {
        root: "/missing".into(),
        media: media.clone(),
        ..Default::default()
    }));
    app.metadata = media.iter().map(|m| (m.id, Metadata::default())).collect();
    app.media = media;
    app
}
#[test]
fn board_moves_take_a_photos_companions_along_without_touching_decisions_or_images() {
    let mut app = app();
    app.step = Step::Organize;
    app.companions = true;
    app.board_new_folder("", &[MediaId(0)]);
    let labels = app.organize_drafts[&GroupMode::Current].labels();
    assert_eq!(labels[&MediaId(0)], "New folder");
    assert_eq!(labels[&MediaId(1)], "New folder", "a.ARW goes with a.jpg");
    assert_eq!(labels[&MediaId(2)], "trip");
    assert!(app.decisions.is_empty());
    assert_eq!(app.stats.decodes, 0);
    assert!(!app.scanning && app.analysis_run.is_none());
    // Switched off, only the photo moves.
    app.companions = false;
    app.board_move(&[MediaId(0)], "trip");
    let labels = app.organize_drafts[&GroupMode::Current].labels();
    assert_eq!(labels[&MediaId(0)], "trip");
    assert_eq!(labels[&MediaId(1)], "New folder");
}
