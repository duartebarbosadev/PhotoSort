use super::*;

fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("photos");
    std::fs::create_dir_all(root.join("A/Day1")).unwrap();
    std::fs::create_dir_all(root.join("B")).unwrap();
    std::fs::create_dir_all(root.join("Empty")).unwrap();
    for i in 0..9 {
        let folder = if i < 4 { "A/Day1" } else { "B" };
        let path = root.join(folder).join(format!("photo-{i}.png"));
        image::RgbaImage::new(16, 16).save(&path).unwrap();
        // Thumbnail workers may publish metadata while these real UI frames run.
        // Keep on-disk and seeded ratings identical regardless of worker timing.
        crate::core::metadata::write_sidecar(&path, Some(if i == 0 { 5 } else { 3 }), None)
            .unwrap();
    }
    let catalog = Arc::new(Catalog::scan(&root, || false, |_| {}).unwrap());
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.preferences.updates.automatic = false;
    app.root = Some(root.canonicalize().unwrap());
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    for m in &app.media {
        app.metadata.insert(
            m.id,
            Metadata {
                width: 16,
                height: 16,
                orientation: 1,
                rating: if m.path.file_name().unwrap() == "photo-0.png" {
                    5
                } else {
                    3
                },
                captured: "2023:04:27 22:46:40".into(),
                ..Default::default()
            },
        );
    }
    app.catalog = Some(catalog);
    app.step = Step::Cull;
    app.review = false;
    app.inspector = false;
    app.navigator.enabled = true;
    settle(&mut app);
    (temp, app)
}
fn settle(app: &mut PhotoSortApp) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        app.refilter();
        if app.browse_run.is_none() {
            break;
        }
        assert!(Instant::now() < deadline, "browse query did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn frame(
    app: &mut PhotoSortApp,
    ctx: &egui::Context,
    size: Vec2,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
    let out = ctx.run(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ctx| app.update_workspace(ctx, false),
    );
    out.platform_output
        .accesskit_update
        .unwrap()
        .nodes
        .into_iter()
        .filter_map(|(_, n)| {
            let label = n.label().or_else(|| n.value())?.to_owned();
            let b = n.bounds()?;
            Some((
                label,
                egui::Rect::from_min_max(
                    egui::pos2(b.x0 as f32, b.y0 as f32),
                    egui::pos2(b.x1 as f32, b.y1 as f32),
                ),
            ))
        })
        .collect()
}
fn rendered(app: &mut PhotoSortApp, ctx: &egui::Context, size: Vec2) -> Vec<(String, egui::Rect)> {
    let mut nodes = Vec::new();
    for _ in 0..3 {
        nodes = frame(app, ctx, size, Vec::new());
    }
    nodes
}

#[test]
fn compact_cull_workspace_keeps_photos_visible_with_navigator_and_filters() {
    let (_temp, mut app) = fixture();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    for size in [
        egui::vec2(640., 600.),
        egui::vec2(800., 600.),
        egui::vec2(1000., 600.),
        egui::vec2(1440., 900.),
    ] {
        for layout in library::Layout::ALL {
            app.set_library_layout(layout);
            let nodes = rendered(&mut app, &ctx, size);
            let footer = nodes
                .iter()
                .find(|(s, _)| s == "< Pick Best")
                .unwrap()
                .1
                .top();
            let photos: Vec<_> = nodes
                .iter()
                .filter(|(s, _)| s.starts_with("photo-") && s.ends_with(".png"))
                .collect();
            assert!(
                photos.iter().any(|(_, b)| b.top() >= 0.
                    && b.bottom() < footer
                    && b.left() >= 0.
                    && b.right() <= size.x),
                "No complete photo row visible at {size:?}/{layout:?}; footer {footer}; photo bounds {photos:?}"
            );
        }
    }
    assert!(app.decisions.is_empty());
    assert_eq!(app.media.len(), 9);
}

fn click_label(app: &mut PhotoSortApp, ctx: &egui::Context, size: Vec2, label: &str) {
    let nodes = rendered(app, ctx, size);
    let pos = nodes
        .iter()
        .find(|(s, _)| s == label)
        .unwrap_or_else(|| {
            panic!(
                "Missing {label}; labels {:?}",
                nodes.iter().map(|(s, _)| s).collect::<Vec<_>>()
            )
        })
        .1
        .center();
    assert!(
        egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains(pos),
        "{label} is outside the window"
    );
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
    settle(app);
}

#[test]
fn compact_filter_panel_keeps_nested_rating_folder_choices_and_keyboard_ownership() {
    use crate::core::browsing::RatingFilter;
    let (_temp, mut app) = fixture();
    let catalog = app.catalog.clone().unwrap();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(640., 600.);
    click_label(&mut app, &ctx, size, "Filters & sort");
    assert!(app.browse_filters_open);
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Key {
            key: egui::Key::D,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert!(
        app.decisions.is_empty(),
        "Filter interaction marked a photo"
    );
    click_label(&mut app, &ctx, size, "Any rating");
    assert!(
        app.browse_filters_open,
        "Opening a nested chooser closed the parent panel"
    );
    let nodes = rendered(&mut app, &ctx, size);
    let first = nodes
        .iter()
        .find(|(s, _)| s == "Unrated")
        .unwrap()
        .1
        .center();
    frame(
        &mut app,
        &ctx,
        size,
        vec![
            egui::Event::PointerMoved(first),
            egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., -600.),
                modifiers: Default::default(),
            },
        ],
    );
    click_label(&mut app, &ctx, size, "Exactly 5 stars");
    assert_eq!(app.browse.rating, RatingFilter::Exactly(5));
    assert_eq!(app.filtered.len(), 1);
    app.browse_filters_open = true;
    click_label(&mut app, &ctx, size, "All folders");
    click_label(&mut app, &ctx, size, "A/Day1");
    assert_eq!(
        app.browse.folder.path,
        Some(app.root.as_ref().unwrap().join("A/Day1"))
    );
    assert_eq!(app.filtered.len(), 1);
    app.browse_filters_open = true;
    click_label(&mut app, &ctx, size, "Clear browse filters");
    assert_eq!(app.filtered.len(), 9);
    assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
    assert_eq!(app.metadata.len(), 9);
    assert!(app.decisions.is_empty());
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert!(!app.browse_filters_open, "Escape did not dismiss the panel");
    app.browse_filters_open = true;
    rendered(&mut app, &ctx, egui::vec2(1440., 900.));
    assert!(
        !app.browse_filters_open,
        "Resizing left an invisible keyboard blocker"
    );
    app.browse_filters_open = true;
    app.set_step(Step::Organize);
    assert!(
        !app.browse_filters_open,
        "A Cull panel survived a workflow change"
    );
}

#[test]
fn compact_navigator_selects_scope_and_metadata_cancel_stays_outside_filters() {
    let (_temp, mut app) = fixture();
    let catalog = app.catalog.clone().unwrap();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(640., 600.);
    click_label(&mut app, &ctx, size, "Browse folders");
    let nodes = rendered(&mut app, &ctx, size);
    for label in ["Folders", "Dates", "Groups", "A  ·  4", "Empty  ·  0"] {
        let b = nodes
            .iter()
            .find(|(s, _)| s == label)
            .unwrap_or_else(|| panic!("Missing {label}"))
            .1;
        assert!(
            b.min.x >= 0. && b.min.y >= 0. && b.max.x <= size.x && b.max.y <= size.y,
            "Navigator control {label} is outside the window: {b:?}"
        );
    }
    click_label(&mut app, &ctx, size, "A  ·  4");
    assert_eq!(app.filtered.len(), 4);
    assert_eq!(
        app.browse.folder.path,
        Some(app.root.as_ref().unwrap().join("A"))
    );
    // Dismiss through the public UI: the browser and its nested context menu
    // deliberately have independent popup owners.
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }],
    );
    assert!(!app.navigator_popup_open(&ctx));
    app.metadata_run = Some(73);
    app.metadata_progress = Some((1, 9));
    let nodes = rendered(&mut app, &ctx, size);
    assert!(!app.browse_filters_open);
    let b = nodes
        .iter()
        .find(|(s, _)| s == "Cancel metadata reading")
        .unwrap()
        .1;
    let footer = nodes
        .iter()
        .find(|(s, _)| s == "< Pick Best")
        .unwrap()
        .1
        .top();
    assert!(
        b.bottom() < footer,
        "Metadata cancellation is hidden below the footer"
    );
    click_label(&mut app, &ctx, size, "Cancel metadata reading");
    assert!(app.metadata_run.is_none());
    assert_eq!(app.filtered.len(), 4);
    assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
    assert!(app.decisions.is_empty());
}

#[test]
fn navigator_activation_checkbox_reveals_browser_from_review_at_both_sizes() {
    for size in [egui::vec2(640., 600.), egui::vec2(1440., 960.)] {
        let (_temp, mut app) = fixture();
        app.review = true;
        app.navigator.enabled = false;
        let active = app.media[0].id;
        app.active = Some(active);
        app.comparison = vec![active, app.media[1].id];
        let comparison = app.comparison.clone();
        app.decisions.insert(
            active,
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
        let catalog = app.catalog.clone().unwrap();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        click_label(&mut app, &ctx, size, "Library navigator");
        assert!(
            app.navigator.enabled && !app.review,
            "Navigator remained hidden in Review at {size:?}"
        );
        let nodes = rendered(&mut app, &ctx, size);
        let expected = if size.x < 1000. {
            "Browse folders"
        } else {
            "Counts cover the whole library"
        };
        assert!(
            nodes.iter().any(|(label, _)| label == expected),
            "Missing visible navigator at {size:?}"
        );
        assert_eq!(app.active, Some(active));
        assert_eq!(app.comparison, comparison);
        assert_eq!(app.decisions[&active].rating, Some(4));
        assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
        let saved = app.capture_bookmark();
        assert!(saved.navigator.enabled && !saved.review);
        click_label(&mut app, &ctx, size, "Library navigator");
        assert!(!app.navigator.enabled && !app.review);
        // Returning to Review hides the tree but retains its saved enabled state.
        // The checkbox reflects visibility and opens it again in one click.
        app.navigator.enabled = true;
        app.review = true;
        click_label(&mut app, &ctx, size, "Library navigator");
        assert!(app.navigator.enabled && !app.review);
        assert_eq!(app.decisions[&active].rating, Some(4));
    }
}
