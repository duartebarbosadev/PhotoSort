use super::*;
use crate::test_support::worker_patience;

pub(super) fn fixture() -> (tempfile::TempDir, PhotoSortApp) {
    fixture_with(9)
}
/// [`fixture`] with `count` photos: four in A/Day1, the rest in B.
pub(super) fn fixture_with(count: usize) -> (tempfile::TempDir, PhotoSortApp) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("photos");
    std::fs::create_dir_all(root.join("A/Day1")).unwrap();
    std::fs::create_dir_all(root.join("B")).unwrap();
    std::fs::create_dir_all(root.join("Empty")).unwrap();
    for i in 0..count {
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
    // Seed exactly what the metadata workers will read. Thumbnail work
    // publishes metadata while the test's frames run; a seed that differs
    // (a made-up capture date, say) re-sorts the grid and rebuilds the
    // navigator between a press and its release.
    for m in &app.media {
        let mut metadata = crate::core::metadata::read(&m.path);
        // Without native metadata a PNG has no size until its thumbnail is
        // decoded, and the workers then publish it: seed it now, so tiles
        // keep their shape instead of changing between a press and release.
        if metadata.width == 0 || metadata.height == 0 {
            (metadata.width, metadata.height) = image::image_dimensions(&m.path).unwrap();
        }
        assert_eq!(
            metadata.rating,
            if m.path.ends_with("photo-0.png") {
                5
            } else {
                3
            }
        );
        app.metadata.insert(m.id, metadata);
    }
    app.catalog = Some(catalog);
    app.step = Step::Cull;
    app.review = false;
    app.inspector = false;
    app.navigator.enabled = true;
    settle(&mut app);
    (temp, app)
}
pub(super) fn settle(app: &mut PhotoSortApp) {
    let deadline = Instant::now() + worker_patience();
    loop {
        let expired = Instant::now() >= deadline;
        app.refilter();
        if app.browse_run.is_none() {
            break;
        }
        assert!(!expired, "browse query did not finish");
        std::thread::sleep(Duration::from_millis(1));
    }
}
pub(super) fn frame(
    app: &mut PhotoSortApp,
    ctx: &egui::Context,
    size: Vec2,
    events: Vec<egui::Event>,
) -> Vec<(String, egui::Rect)> {
    let out = ctx.run_test(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
            events,
            ..Default::default()
        },
        |ui| app.update_workspace(ui, false),
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
pub(super) fn rendered(
    app: &mut PhotoSortApp,
    ctx: &egui::Context,
    size: Vec2,
) -> Vec<(String, egui::Rect)> {
    let mut nodes = Vec::new();
    for _ in 0..3 {
        nodes = frame(app, ctx, size, Vec::new());
    }
    nodes
}

#[test]
fn wide_review_gives_photos_space_and_reuses_the_existing_filter_panel() {
    use crate::core::browsing::RatingFilter;
    let (_temp, mut app) = fixture();
    app.review = true;
    app.navigator.enabled = false;
    app.inspector = true;
    let active = app.media[0].id;
    app.active = Some(active);
    let catalog = app.catalog.clone().unwrap();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1440., 990.);
    let nodes = rendered(&mut app, &ctx, size);
    let canvas = nodes
        .iter()
        .find(|(label, _)| label.starts_with("Photo 1:"))
        .unwrap()
        .1;
    eprintln!("wide review canvas before/after policy: {canvas:?}");
    assert!(
        nodes.iter().any(|(label, _)| label == "Filters & sort"),
        "Wide review should share the compact filter panel"
    );
    assert!(
        canvas.height() >= 300.,
        "Photo canvas lost height to browse controls: {canvas:?}"
    );
    assert!(!nodes.iter().any(|(label, _)| label == "Any rating"));
    click_label(&mut app, &ctx, size, "Filters & sort");
    assert!(app.browse_filters_open);
    choose_five_stars(&mut app, &ctx, size);
    assert_eq!(app.browse.rating, RatingFilter::Exactly(5));
    assert_eq!(app.filtered.len(), 1);
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
    assert!(!app.browse_filters_open);
    let nodes = rendered(&mut app, &ctx, size);
    assert!(nodes.iter().any(|(label, _)| label == "Filters & sort · 1"));
    assert_eq!(app.active, Some(active));
    assert!(app.decisions.is_empty());
    assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
    app.metadata_run = Some(73);
    app.metadata_progress = Some((1, 9));
    let nodes = rendered(&mut app, &ctx, size);
    assert!(
        nodes
            .iter()
            .any(|(label, _)| label == "Cancel metadata reading")
    );
    assert!(!app.browse_filters_open);
    click_label(&mut app, &ctx, size, "Cancel metadata reading");
    assert!(app.metadata_run.is_none());
    // Return to the wide browser: the same panel and its filter survive.
    app.show_grid();
    let nodes = rendered(&mut app, &ctx, size);
    assert!(nodes.iter().any(|(label, _)| label == "Filters & sort · 1"));
    assert!(!nodes.iter().any(|(label, _)| label == "Any rating"));
    assert_eq!(app.filtered.len(), 1);
    app.runtime.shutdown();
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
        for review in [false, true] {
            app.review = review;
            let nodes = rendered(&mut app, &ctx, size);
            let footer = size.y - navigation::STATUS_HEIGHT;
            let photos: Vec<_> = nodes
                .iter()
                .filter(|(s, _)| s.starts_with("photo-") && s.ends_with(".png"))
                .collect();
            assert!(
                photos.iter().any(|(_, b)| b.top() >= 0.
                    && b.bottom() < footer
                    && b.left() >= 0.
                    && b.right() <= size.x),
                "No complete photo row visible at {size:?}/review {review}; footer {footer}; photo bounds {photos:?}"
            );
        }
    }
    assert!(app.decisions.is_empty());
    assert_eq!(app.media.len(), 9);
}

pub(super) fn click_label(app: &mut PhotoSortApp, ctx: &egui::Context, size: Vec2, label: &str) {
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

fn choose_five_stars(app: &mut PhotoSortApp, ctx: &egui::Context, size: Vec2) {
    click_label(app, ctx, size, "Any rating");
    assert!(
        app.browse_filters_open,
        "Opening a nested chooser closed the parent panel"
    );
    let nodes = rendered(app, ctx, size);
    let first = nodes
        .iter()
        .find(|(s, _)| s == "Unrated")
        .unwrap()
        .1
        .center();
    frame(
        app,
        ctx,
        size,
        vec![
            egui::Event::PointerMoved(first),
            egui::Event::MouseWheel {
                phase: egui::TouchPhase::Move,
                unit: egui::MouseWheelUnit::Point,
                delta: egui::vec2(0., -600.),
                modifiers: Default::default(),
            },
        ],
    );
    click_label(app, ctx, size, "Exactly 5 stars");
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
    choose_five_stars(&mut app, &ctx, size);
    assert_eq!(app.browse.rating, RatingFilter::Exactly(5));
    assert_eq!(app.filtered.len(), 1);
    app.browse_filters_open = true;
    click_label(&mut app, &ctx, size, "All folders");
    // Folder labels use the platform's separator (A\Day1 on Windows).
    let day = std::path::Path::new("A").join("Day1");
    click_label(&mut app, &ctx, size, &day.display().to_string());
    assert_eq!(
        app.browse.folder.path,
        Some(app.root.as_ref().unwrap().join(&day))
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
    let nodes = rendered(&mut app, &ctx, egui::vec2(1440., 900.));
    assert!(
        !app.browse_filters_open
            || nodes
                .iter()
                .any(|(label, _)| label == "Clear browse filters"),
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
    for label in ["Previous", "Next", "A  ·  4", "Empty  ·  0"] {
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
    // A folder scrolls the photos to it rather than hiding the others.
    click_label(&mut app, &ctx, size, "A  ·  4");
    assert_eq!(app.filtered.len(), 9);
    assert_eq!(app.browse.folder.path, None);
    assert_eq!(
        app.spy.current,
        Some(crate::core::browsing::tree::Key::Folder(
            app.root.as_ref().unwrap().join("A")
        ))
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
    // It lives in the status bar, so it must be fully inside the window.
    assert!(
        b.top() >= size.y - navigation::STATUS_HEIGHT && b.bottom() <= size.y,
        "Metadata cancellation is outside the status bar: {b:?}"
    );
    click_label(&mut app, &ctx, size, "Cancel metadata reading");
    assert!(app.metadata_run.is_none());
    assert_eq!(app.filtered.len(), 9);
    assert!(Arc::ptr_eq(&catalog, app.catalog.as_ref().unwrap()));
    assert!(app.decisions.is_empty());
}

#[test]
fn navigator_opens_beside_review_and_follows_between_views_at_both_sizes() {
    for size in [egui::vec2(640., 600.), egui::vec2(1440., 960.)] {
        let (_temp, mut app) = fixture();
        app.preferences.strip_position = crate::core::preferences::StripPosition::Left;
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
            app.navigator.enabled && app.review,
            "Opening the navigator left Review at {size:?}"
        );
        let nodes = rendered(&mut app, &ctx, size);
        let expected = if size.x < 1000. {
            "Browse folders"
        } else {
            "Previous"
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
        assert!(saved.navigator.enabled && saved.review);
        // Switching to a grid view and back keeps it open in both.
        app.show_grid();
        assert!(app.navigator.enabled);
        app.open_review();
        assert!(app.navigator.enabled && app.review);
        click_label(&mut app, &ctx, size, "Library navigator");
        assert!(!app.navigator.enabled && app.review);
        assert_eq!(app.decisions[&active].rating, Some(4));
    }
}

#[test]
fn loading_grid_animates_then_comes_to_rest() {
    const IDLE: Duration = Duration::from_millis(900);
    let (_temp, mut app) = fixture();
    app.navigator.enabled = false;
    app.active = Some(app.media[0].id);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    // Real motion: shimmer, fade-in, hover and ring must all come to rest.
    set_reduce_motion(&ctx, false);
    let size = egui::vec2(1200., 800.);
    let tile = egui::pos2(100., 300.);
    let mut animated = false;
    let mut delay = Duration::ZERO;
    let deadline = Instant::now() + Duration::from_secs(20);
    for frame in 0u32.. {
        assert!(
            Instant::now() < deadline,
            "the grid never settled: delay {delay:?}, {} textures, requested {:?}, causes {:?}",
            app.textures.len(),
            app.requested,
            ctx.repaint_causes()
        );
        // Hover a tile for a moment, then leave the window.
        // Any input event repaints at once, so later frames send none.
        let events = match frame {
            5..=20 => vec![egui::Event::PointerMoved(tile)],
            21 => vec![egui::Event::PointerGone],
            _ => Vec::new(),
        };
        let out = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                time: Some(f64::from(frame) / 60.0),
                events,
                ..Default::default()
            },
            |ui| app.update_workspace(ui, false),
        );
        delay = out.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        animated |= delay.is_zero();
        let loaded = !app.textures.is_empty() && app.requested.is_empty();
        if frame > 40 && loaded && delay >= IDLE {
            break;
        }
        settle(&mut app);
        if !loaded {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    assert!(animated, "no frame asked for animation");
    assert!(app.requested.is_empty(), "{:?}", app.requested);
    // The workspace keeps a one-second heartbeat for view checkpoints;
    // anything sooner means an animation never came to rest.
    assert!(delay >= IDLE, "an idle window repaints every {delay:?}");
}

#[test]
fn details_slide_out_never_doubles_or_outlives_its_folder() {
    let (_temp, mut app) = fixture();
    app.inspector = true;
    app.active = Some(app.media[0].id);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    set_reduce_motion(&ctx, false);
    // The reveal button is named per platform (Finder, Explorer, file manager).
    let reveal = crate::core::desktop::Action::Reveal.label();
    let run = |app: &mut PhotoSortApp, width: f32, frame: u32| {
        let out = ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 900.),
                )),
                time: Some(f64::from(frame) / 60.0),
                ..Default::default()
            },
            |ui| app.update_workspace(ui, false),
        );
        out.platform_output
            .accesskit_update
            .unwrap()
            .nodes
            .iter()
            .filter(|(_, n)| n.label() == Some(reveal))
            .count()
    };
    let mut frame = 0;
    for _ in 0..30 {
        frame += 1;
        assert!(run(&mut app, 1440., frame) <= 1);
    }
    frame += 1;
    assert_eq!(run(&mut app, 1440., frame), 1, "Details is not showing");
    // Narrowing hands Details to the drawer while the panel slides away.
    for _ in 0..30 {
        frame += 1;
        assert!(run(&mut app, 1000., frame) <= 1, "two inspectors at once");
    }
    for _ in 0..30 {
        frame += 1;
        run(&mut app, 1440., frame);
    }
    // Closing the folder mid-session slides an empty panel out.
    app.catalog = None;
    for _ in 0..30 {
        frame += 1;
        assert_eq!(run(&mut app, 1440., frame), 0);
    }
    app.runtime.shutdown();
}

#[test]
fn status_bar_metadata_controls_never_overlap_the_status_text() {
    for width in [360., 640., 1440.] {
        let (_temp, mut app) = fixture();
        app.metadata_run = Some(73);
        app.metadata_progress = Some((120, 6500));
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(width, 700.);
        let nodes = rendered(&mut app, &ctx, size);
        let footer = size.y - navigation::STATUS_HEIGHT;
        let bar: Vec<_> = nodes
            .iter()
            .filter(|(label, r)| {
                r.top() >= footer - 1.0 && r.bottom() <= size.y + 1.0 && !label.is_empty()
            })
            .collect();
        assert!(
            bar.iter().any(|(l, _)| l == "Cancel metadata reading"),
            "{width}: {bar:?}"
        );
        for (i, (a, ra)) in bar.iter().enumerate() {
            assert!(ra.right() <= width + 0.5, "{width}: {a} leaves the window");
            for (b, rb) in &bar[i + 1..] {
                let overlap = ra.intersect(*rb);
                assert!(
                    overlap.width() <= 0.5 || ra.contains_rect(*rb) || rb.contains_rect(*ra),
                    "{width}: {a} {ra:?} overlaps {b} {rb:?}"
                );
            }
        }
        app.runtime.shutdown();
    }
}

#[test]
fn a_restored_filter_says_how_many_items_are_hidden_and_clears_in_one_click() {
    let (_temp, mut app) = fixture();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1440., 900.);
    // No filter: no notice.
    let nodes = rendered(&mut app, &ctx, size);
    assert!(!nodes.iter().any(|(label, _)| label == "Clear filters"));
    // A folder filter, as a saved view would restore it.
    app.browse.folder.path = Some(app.root.clone().unwrap().join("B"));
    app.filter_dirty = true;
    settle(&mut app);
    assert_eq!(app.filtered.len(), 5);
    let nodes = rendered(&mut app, &ctx, size);
    assert!(
        nodes
            .iter()
            .any(|(label, _)| label == "Showing 5 of 9 items"),
        "{:?}",
        nodes.iter().map(|(l, _)| l).collect::<Vec<_>>()
    );
    assert!(nodes.iter().any(|(label, _)| label == "Folder B"));
    click_label(&mut app, &ctx, size, "Clear filters");
    settle(&mut app);
    assert_eq!(app.filtered.len(), 9);
    assert!(app.active_cull_filters().is_empty());
    let nodes = rendered(&mut app, &ctx, size);
    assert!(!nodes.iter().any(|(label, _)| label == "Clear filters"));
    assert!(app.decisions.is_empty());
    app.runtime.shutdown();
}

#[test]
fn compact_strip_fits_more_photos_and_best_links_its_similar_group() {
    let size = egui::vec2(1440., 900.);
    let (_temp, mut app) = fixture_with(24);
    app.preferences.strip_position = crate::core::preferences::StripPosition::Left;
    app.review = true;
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    // The second shown photo is the best of a group of three.
    for (id, score) in ids.iter().zip([60.0, 90.0, 70.0]) {
        app.analysis.insert(
            *id,
            crate::core::analysis::Analysis {
                score,
                ..Default::default()
            },
        );
    }
    app.publish_groups(Arc::new(ids[..3].iter().map(|id| (*id, 5)).collect()));
    settle(&mut app);
    app.active = Some(ids[0]);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let window = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let shown = |app: &PhotoSortApp, nodes: &[(String, egui::Rect)]| {
        ids.iter()
            .filter(|id| {
                let name = app.media[app.index[id]].name();
                nodes
                    .iter()
                    .any(|(label, rect)| *label == name && window.contains_rect(*rect))
            })
            .count()
    };
    app.preferences.compact_strip = false;
    let nodes = rendered(&mut app, &ctx, size);
    let large = shown(&app, &nodes);
    click_label(
        &mut app,
        &ctx,
        size,
        "Large thumbnails · click for compact rows",
    );
    assert!(app.preferences.compact_strip);
    let nodes = rendered(&mut app, &ctx, size);
    let compact = shown(&app, &nodes);
    assert!(
        compact >= large * 2,
        "Compact rows show {compact} photos, large tiles {large}"
    );
    // A photo that lost says where it stands and goes to the winner.
    let chip = nodes
        .iter()
        .find(|(label, _)| label.contains("of 3 · Go to best"))
        .map(|(label, _)| label.clone())
        .expect("group chip on a photo that lost");
    assert!(chip.starts_with("3rd of 3"), "{chip}");
    // With the best photo hidden by the filters, it only says so.
    let shown = app.filtered.clone();
    app.filtered.retain(|i| app.media[*i].id != ids[1]);
    let nodes = rendered(&mut app, &ctx, size);
    assert!(nodes.iter().any(|(label, _)| label == "3rd of 3"));
    click_label(&mut app, &ctx, size, "3rd of 3");
    assert_eq!(app.active, Some(ids[0]));
    app.filtered = shown;
    click_label(&mut app, &ctx, size, &chip);
    assert_eq!(app.active, Some(ids[1]));
    // The winner offers to compare the whole group.
    let nodes = rendered(&mut app, &ctx, size);
    let chip = nodes
        .iter()
        .find(|(label, _)| label.contains("Best of 3 · Compare all"))
        .map(|(label, _)| label.clone())
        .expect("group chip on the best photo");
    click_label(&mut app, &ctx, size, &chip);
    assert_eq!(app.comparison, ids[..3].to_vec());
    assert!(app.review);
}

#[test]
fn a_photo_sorted_away_between_press_and_release_is_not_clicked() {
    let size = egui::vec2(1200., 800.);
    let (_temp, mut app) = fixture();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    app.set_active(ids[0]);
    let nodes = rendered(&mut app, &ctx, size);
    let name = app.media[app.index[&ids[2]]].name();
    let pos = nodes.iter().find(|(l, _)| *l == name).unwrap().1.center();
    let button = |pressed| {
        vec![
            egui::Event::PointerMoved(pos),
            egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            },
        ]
    };
    frame(&mut app, &ctx, size, button(true));
    // A late result re-sorts the photos: another one takes the third place.
    app.filtered.swap(2, 3);
    frame(&mut app, &ctx, size, button(false));
    assert_eq!(app.active, Some(ids[0]));
    assert_eq!(app.selection.ordered(&app.media, &app.filtered), [ids[0]]);
    app.runtime.shutdown();
}

#[test]
fn a_folder_clicked_in_the_navigator_scrolls_the_bottom_filmstrip() {
    let size = egui::vec2(1200., 800.);
    let (_temp, mut app) = fixture_with(24);
    app.preferences.strip_position = crate::core::preferences::StripPosition::Bottom;
    app.review = true;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    // Frames here take no time, so the strip jumps rather than glides.
    ctx.global_style_mut(|style| style.scroll_animation = egui::style::ScrollAnimation::none());
    let first = app.media[app.filtered[0]].name();
    let strip = |app: &mut PhotoSortApp| {
        rendered(app, &ctx, size)
            .into_iter()
            .any(|(label, _)| label == first)
    };
    assert!(strip(&mut app));
    let b = crate::core::browsing::tree::Key::Folder(app.root.clone().unwrap().join("B"));
    app.go_to(b);
    // The four photos of A/Day1 scroll out on the left.
    assert!(!strip(&mut app));
    assert!(app.spy.scroll_to.is_none());
    app.runtime.shutdown();
}
