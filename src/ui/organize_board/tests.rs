use super::preview::sheet_tile;
use super::sidebar::PREVIEW_THUMBNAILS;
use super::*;

fn organize() -> (tempfile::TempDir, PhotoSortApp) {
    let (temp, mut app) = super::super::compact_cull_tests::fixture();
    app.step = Step::Organize;
    app.review = false;
    app.output = app.root.as_ref().unwrap().display().to_string();
    app.filter_dirty = true;
    app.refilter();
    (temp, app)
}

/// Makes a folder as the sidebar does, then names it.
fn new_folder(app: &mut PhotoSortApp, parent: &str, name: &str) {
    app.board_new_folder(parent, &[]);
    app.board.editing.as_mut().unwrap().1 = name.into();
    app.board_finish_edit();
}

#[test]
fn board_groups_items_by_destination_folder_in_view_order() {
    let (_temp, mut app) = organize();
    let paths: Vec<_> = app
        .board
        .sections
        .iter()
        .map(|s| s.path.clone().unwrap())
        .collect();
    assert_eq!(paths, vec!["A/Day1".to_owned(), "B".to_owned()]);
    assert_eq!(app.board.sections[0].range, 0..4);
    assert_eq!(app.board.sections[1].range, 4..9);
    // `filtered` follows the board, so arrows and the strip match it.
    for (position, &index) in app.filtered.iter().enumerate() {
        let folder = if position < 4 { "A/Day1" } else { "B" };
        assert!(app.media[index].path.parent().unwrap().ends_with(folder));
    }
    let folders: Vec<_> = app
        .board
        .folders
        .iter()
        .map(|f| (f.path.as_str(), f.direct, f.total))
        .collect();
    assert!(folders.contains(&("", 0, 9)));
    assert!(folders.contains(&("A", 0, 4)));
    assert!(folders.contains(&("A/Day1", 4, 4)));
    assert!(folders.contains(&("B", 5, 5)));
    app.runtime.shutdown();
}

#[test]
fn returning_from_another_step_mid_frame_draws_the_current_board() {
    let (_temp, mut app) = organize();
    assert_eq!(app.board.sections[1].range, 4..9);
    // Pick Best leaves its two-photo comparison in `filtered`; the board
    // keeps the sections it arranged on the last Organize visit.
    app.step = Step::PickBest;
    app.filtered.truncate(2);
    app.filter_dirty = false;
    app.board.focus = vec!["B".into()];
    // The step tabs switch steps after this frame's refilter has run,
    // so Organize draws before `filtered` is rebuilt.
    app.set_step(Step::Organize);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let _ = ctx.run_test(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1400.0, 900.0),
            )),
            ..Default::default()
        },
        |ui| app.organize_board(ui),
    );
    assert_eq!(app.filtered.len(), 9);
    assert_eq!(app.board.sections.last().unwrap().range.end, 9);
    app.runtime.shutdown();
}

#[test]
fn moving_creating_and_renaming_folders_edit_the_shared_draft_only() {
    let (_temp, mut app) = organize();
    let moved: Vec<MediaId> = app.filtered[..2].iter().map(|i| app.media[*i].id).collect();
    app.board_move(&moved, "B");
    assert!(app.organize_drafts.contains_key(&GroupMode::Current));
    app.check_board();
    app.refilter();
    let b = app
        .board
        .sections
        .iter()
        .find(|s| s.path.as_deref() == Some("B"))
        .unwrap();
    assert_eq!(b.range.len(), 7);
    // Creating and renaming a folder in place.
    new_folder(&mut app, "", "Trip");
    app.board.editing = Some(("Trip".into(), "Holiday".into()));
    app.board_finish_edit();
    app.board_move(&moved, "Holiday");
    app.check_board();
    app.refilter();
    assert!(
        app.board
            .sections
            .iter()
            .any(|s| s.path.as_deref() == Some("Holiday") && s.range.len() == 2)
    );
    // Nothing has moved on disk or been staged as a decision.
    assert!(app.decisions.is_empty());
    assert!(app.media.iter().all(|m| m.path.exists()));
    app.runtime.shutdown();
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    vec![egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }]
}

#[test]
fn shift_arrows_compare_the_selection_in_the_preview_and_escape_returns() {
    use super::super::compact_cull_tests::{frame, rendered};
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    app.select_library(ids[0], false, false);
    assert_eq!(app.comparison_ids(), vec![ids[0]]);
    for _ in 0..2 {
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::ArrowRight, egui::Modifiers::SHIFT),
        );
    }
    assert_eq!(app.selection.len(), 3);
    assert_eq!(app.comparison_ids(), ids[..3].to_vec());
    assert_eq!(app.active, Some(ids[2]));
    assert_eq!(app.focused_slot, 2);
    let nodes = rendered(&mut app, &ctx, size);
    assert!(nodes.iter().any(|(label, _)| label == "End comparison"));
    // Escape keeps the active photo and shows it alone.
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Escape, Default::default()),
    );
    assert_eq!(app.comparison_ids(), vec![ids[2]]);
    assert_eq!(
        app.selection.ordered(&app.media, &app.filtered),
        vec![ids[2]]
    );
    assert_eq!(app.focused_slot, 0);
    let nodes = rendered(&mut app, &ctx, size);
    assert!(!nodes.iter().any(|(label, _)| label == "End comparison"));
    // A second Escape clears the selection as before.
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Escape, Default::default()),
    );
    assert!(app.selection.is_empty() && app.active.is_none());
    app.runtime.shutdown();
}

#[test]
fn the_comparison_follows_board_order_and_stops_past_nine() {
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    // Clicked out of order, compared in the order the board shows them.
    app.select_library(ids[6], false, false);
    app.select_library(ids[1], true, false);
    app.select_library(ids[4], true, false);
    assert_eq!(app.comparison_ids(), vec![ids[1], ids[4], ids[6]]);
    assert_eq!(app.active, Some(ids[4]));
    assert_eq!(app.focused_slot, 1);
    app.selection.all(&app.media, &app.filtered);
    assert_eq!(app.comparison_ids(), ids);
    // A tenth selected item (one this view does not show) ends the
    // comparison; the preview shows the active photo.
    let mut selected = ids.clone();
    selected.push(MediaId(u64::MAX));
    app.selection.replace(selected);
    assert_eq!(app.comparison_ids(), vec![ids[4]]);
    // Other steps keep their own comparisons.
    app.select_library(ids[0], false, false);
    app.select_library(ids[1], true, false);
    app.step = Step::Cull;
    assert_eq!(app.comparison_ids(), vec![ids[1]]);
    app.runtime.shutdown();
}

#[test]
fn generated_groupings_hide_emptied_folders_but_keep_created_ones() {
    let (_temp, mut app) = organize();
    // Making a folder creates the draft, which knows every library folder.
    new_folder(&mut app, "", "Trip");
    app.check_board();
    app.refilter();
    let paths = |app: &PhotoSortApp| -> Vec<String> {
        app.board.folders.iter().map(|f| f.path.clone()).collect()
    };
    // Current folders lists them all, empty or not.
    assert_eq!(paths(&app), ["", "A", "A/Day1", "B", "Empty", "Trip"]);
    app.group_mode = GroupMode::Camera;
    new_folder(&mut app, "", "Picks");
    new_folder(&mut app, "Picks", "Best");
    app.check_board();
    app.refilter();
    assert_eq!(
        paths(&app),
        ["", "Picks", "Picks/Best", "Unknown camera"],
        "Camera lists the folders it fills and the ones made on the board"
    );
    // A folder emptied by a move disappears unless it was made by hand.
    let ids: Vec<MediaId> = app.media.iter().map(|m| m.id).collect();
    app.board_move(&ids, "Picks/Best");
    app.check_board();
    app.refilter();
    assert_eq!(paths(&app), ["", "Picks", "Picks/Best"]);
    app.runtime.shutdown();
}

fn labels(app: &mut PhotoSortApp, width: f32) -> Vec<(String, egui::Rect)> {
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    super::super::compact_cull_tests::rendered(app, &ctx, egui::vec2(width, 900.0))
}
fn shows(nodes: &[(String, egui::Rect)], text: &str) -> bool {
    nodes.iter().any(|(label, _)| label.contains(text))
}

#[test]
fn a_running_grouping_shows_progress_instead_of_a_result() {
    let (_temp, mut app) = organize();
    app.group_mode = GroupMode::Visual;
    app.visual_groups = None;
    app.filter_dirty = true;
    app.refilter();
    app.analysis_run = Some(7);
    app.organize_run = Some((7, GroupMode::Visual));
    app.progress = Some((27, 502));
    assert_eq!(app.organize_pending(), Some(Pending::Running));
    assert_eq!(unplaced_title(app.organize_pending()), "Grouping…");
    assert_eq!(app.board_counts(0, 9, true), "9 items · grouping…");
    assert_eq!(
        app.board_note().as_deref(),
        Some("Folders appear when grouping finishes")
    );
    let nodes = labels(&mut app, 1440.0);
    assert!(shows(&nodes, "Grouping by visual similarity"));
    assert!(shows(&nodes, "27 of 502 photos"));
    assert!(nodes.iter().any(|(label, _)| label == "Cancel"));
    assert!(shows(&nodes, "Photos not grouped yet"));
    let sentence = app.unplaced_sentence();
    for claim in ["Not moving", "9 not moving", sentence.as_str()] {
        assert!(!shows(&nodes, claim), "{claim} shown while grouping");
    }
    // Model preparation is named while it runs.
    {
        app.runtime
            .processing
            .preparing("dinov2", crate::core::processing::Backend::CoreMl);
        app.runtime.processing.step(
            "dinov2",
            crate::core::processing::Backend::CoreMl,
            crate::core::processing::PrepStep::Compiling,
        );
        let nodes = labels(&mut app, 1440.0);
        assert!(shows(&nodes, "Preparing the similarity model"));
        assert!(shows(&nodes, "Optimizing for this computer · 0:00"));
        app.runtime.processing.report(
            "dinov2",
            crate::core::processing::Backend::Cpu,
            "CPU fallback".into(),
        );
    }
    // Cancelled: says so and offers to group again; still no result.
    app.cancel_analysis();
    assert_eq!(app.organize_pending(), Some(Pending::Cancelled));
    // Cancelling another step's analysis later does not change that.
    app.analysis_run = Some(9);
    app.delete_run = Some(9);
    app.cancel_analysis();
    assert_eq!(app.organize_pending(), Some(Pending::Cancelled));
    let nodes = labels(&mut app, 1440.0);
    assert!(shows(&nodes, "Grouping cancelled"));
    assert!(nodes.iter().any(|(label, _)| label == "Group again"));
    assert!(!shows(&nodes, "27 of 502"));
    assert!(!shows(&nodes, &app.unplaced_sentence()));
    assert_eq!(
        app.board_note().as_deref(),
        Some("Grouping cancelled · no folders proposed")
    );
    // A failed run reads differently from a cancelled one.
    app.analysis_run = Some(8);
    app.organize_run = Some((8, GroupMode::Visual));
    app.halt_organize_run(); // As a failed run does.
    app.analysis_run = None;
    assert_eq!(app.organize_pending(), Some(Pending::Failed));
    assert!(
        app.organize_notice_text()
            .is_some_and(|(_, title, _, _)| title == "Grouping stopped")
    );
    // Finished without any group: now staying in place is the result.
    app.visual_groups = Some(HashMap::new());
    app.check_board();
    app.refilter();
    assert_eq!(app.organize_pending(), None);
    let nodes = labels(&mut app, 1440.0);
    assert!(shows(&nodes, "Not moving · "));
    let sentence = app.unplaced_sentence();
    assert!(
        sentence.ends_with("stay in their current folders"),
        "{sentence}"
    );
    assert!(shows(&nodes, &sentence));
    assert!(!shows(&nodes, "Grouping by visual similarity"));
    app.runtime.shutdown();
}

#[test]
fn another_steps_analysis_shows_its_own_card_and_the_grouping_waits() {
    let (_temp, mut app) = organize();
    app.group_mode = GroupMode::Faces;
    app.analysis_run = Some(3);
    app.delete_run = Some(3);
    app.progress = Some((4, 9));
    assert_eq!(app.organize_pending(), Some(Pending::Waiting));
    let nodes = labels(&mut app, 1440.0);
    assert!(shows(&nodes, "Looking for photos to clean up"));
    assert!(shows(&nodes, "4 of 9 photos"));
    assert!(shows(&nodes, "Grouping is next"));
    assert!(!shows(&nodes, &app.unplaced_sentence()));
    // Cull keeps its grid and gets the same card above it.
    app.delete_run = None;
    app.cull_group_run = Some(3);
    app.step = Step::Cull;
    app.filter_dirty = true;
    app.refilter();
    let nodes = labels(&mut app, 1440.0);
    assert!(shows(&nodes, "Analyzing the library"));
    assert!(shows(&nodes, "4 of 9 photos"));
    app.runtime.shutdown();
}

#[test]
fn board_counts_never_run_under_the_thumbnail_slider() {
    let (_temp, mut app) = organize();
    app.group_mode = GroupMode::Visual;
    app.visual_groups = Some(HashMap::new());
    app.filter_dirty = true;
    app.refilter();
    let summary = app.board_counts(0, 9, true);
    assert_eq!(summary, "0 folders · 9 items · 9 not moving");
    for width in [900.0, 1000.0, 1100.0, 1200.0, 1440.0, 1800.0] {
        let nodes = labels(&mut app, width);
        let rect = |name: &str| {
            nodes
                .iter()
                .find(|(label, _)| label == name)
                .unwrap_or_else(|| panic!("{name} missing at {width}"))
                .1
        };
        let (counts, slider) = (rect(&summary), rect("Photo size"));
        assert!(
            counts.right() <= slider.left(),
            "counts {counts:?} overlap the slider {slider:?} at {width}"
        );
        // Nor under the search and filter beside it.
        let filter = rect("Show");
        assert!(
            counts.right() <= filter.left(),
            "counts {counts:?} overlap the filter {filter:?} at {width}"
        );
    }
    app.runtime.shutdown();
}

#[test]
fn the_library_folder_is_one_row_that_shows_every_item() {
    let (_temp, mut app) = organize();
    let root = app.board_root_name();
    let nodes = labels(&mut app, 1400.0);
    let row = format!("{root}  ·  9");
    assert_eq!(nodes.iter().filter(|(label, _)| *label == row).count(), 1);
    assert!(!shows(&nodes, "All folders"));
    // Choosing it after a folder shows every item again.
    app.board_select_folder(Some("B".into()));
    app.board_select_folder(None);
    assert!(app.board.focus.is_empty());
    // Dropping onto it still moves photos to the library folder.
    let id = app.media[app.filtered[0]].id;
    app.board_move(&[id], "");
    assert_eq!(
        app.organize_drafts[&GroupMode::Current].labels().get(&id),
        Some(&String::new())
    );
    app.runtime.shutdown();
}

#[test]
fn details_say_where_a_photo_goes_on_disk_not_just_its_folder_name() {
    let (temp, mut app) = organize();
    let media = app.media[app.filtered[0]].clone();
    let placement = app.placement(&media);
    assert!(!placement.moves, "Current folders keep it where it is");
    assert_eq!(placement.folder, "photos / A / Day1");
    // Organized onto another drive, the same folder name still moves.
    let other = temp.path().join("SSD");
    app.output = other.display().to_string();
    let placement = app.placement(&media);
    assert!(placement.moves);
    assert_eq!(
        placement.folder,
        super::super::inspector::readable_path(&other.join("A").join("Day1"))
    );
    app.runtime.shutdown();
}

#[test]
fn the_sidebar_highlights_the_folder_you_are_looking_at() {
    use super::super::compact_cull_tests::{click_label, frame, rendered};
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    rendered(&mut app, &ctx, size);
    // Every folder is shown; the photo you pick (scrolled into view, as a
    // click or the arrow keys do) names its folder.
    app.select_library(ids[5], false, false);
    app.library_reveal = true;
    rendered(&mut app, &ctx, size);
    assert!(app.board.focus.is_empty());
    assert_eq!(app.board_highlight().as_deref(), Some("B"));
    app.select_library(ids[1], false, false);
    app.library_reveal = true;
    rendered(&mut app, &ctx, size);
    assert_eq!(app.board_highlight().as_deref(), Some("A/Day1"));
    // A folder clicked in the sidebar stays highlighted, though the
    // photos at the top belong to its subfolder ...
    click_label(&mut app, &ctx, size, "A  ·  4");
    rendered(&mut app, &ctx, size);
    assert!(app.board.focus.is_empty());
    assert_eq!(app.board_highlight().as_deref(), Some("A"));
    // ... until you move to another photo.
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::ArrowDown, Default::default()),
    );
    rendered(&mut app, &ctx, size);
    assert_eq!(app.board_highlight().as_deref(), Some("A/Day1"));
    app.runtime.shutdown();
}

#[test]
fn the_wheel_scrolls_the_board_and_it_stays_scrolled() {
    use super::super::compact_cull_tests::{frame, rendered};
    let (_temp, mut app) = organize();
    // Large photos, so nine of them run past the window.
    app.preferences.photo_size = *crate::core::preferences::PHOTO_SIZES.end();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    let name = app.media[app.filtered[0]].name();
    let nodes = rendered(&mut app, &ctx, size);
    // The tile, not the preview's title of the same name.
    let over = nodes
        .iter()
        .filter(|(label, _)| *label == name)
        .max_by(|a, b| a.1.area().total_cmp(&b.1.area()))
        .unwrap_or_else(|| panic!("{name} missing: {nodes:?}"))
        .1
        .center();
    assert_eq!(app.library_offset, 0.0);
    for _ in 0..4 {
        frame(
            &mut app,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(over),
                egui::Event::MouseWheel {
                    phase: egui::TouchPhase::Move,
                    unit: egui::MouseWheelUnit::Point,
                    delta: egui::vec2(0.0, -120.0),
                    modifiers: Default::default(),
                },
            ],
        );
    }
    for _ in 0..10 {
        rendered(&mut app, &ctx, size);
    }
    let scrolled = app.library_offset;
    assert!(scrolled > 200.0, "the board scrolled only {scrolled}");
    // Nothing pulls it back while the layout stays the same.
    rendered(&mut app, &ctx, size);
    assert_eq!(app.library_offset, scrolled);
    app.runtime.shutdown();
}

#[test]
fn arrow_keys_walk_on_across_folders_or_stay_in_the_folders_shown() {
    use super::super::compact_cull_tests::{click_label, frame};
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    let press = |app: &mut PhotoSortApp, pressed: egui::Key| {
        frame(app, &ctx, size, key(pressed, Default::default()));
    };
    // Clicking a folder starts at its first photo.
    click_label(&mut app, &ctx, size, "Day1  ·  4");
    assert_eq!(app.active, Some(ids[0]));
    for &id in &ids[1..4] {
        press(&mut app, egui::Key::ArrowDown);
        assert_eq!(app.active, Some(id));
    }
    // Past its last photo: the next folder's first, and the sidebar follows.
    press(&mut app, egui::Key::ArrowDown);
    assert_eq!(app.active, Some(ids[4]));
    assert_eq!(app.board_highlight().as_deref(), Some("B"));
    assert_eq!(
        app.selection.ordered(&app.media, &app.filtered),
        vec![ids[4]]
    );
    // And back.
    press(&mut app, egui::Key::ArrowUp);
    assert_eq!(app.active, Some(ids[3]));
    assert_eq!(app.board_highlight().as_deref(), Some("A/Day1"));
    assert!(app.board.focus.is_empty());
    // Showing only B, the arrows stay in its photos.
    app.board_choose_folder("B", egui::Modifiers::COMMAND);
    assert_eq!(app.active, Some(ids[4]));
    press(&mut app, egui::Key::ArrowUp);
    assert_eq!(app.active, Some(ids[4]));
    for _ in 0..8 {
        press(&mut app, egui::Key::ArrowDown);
    }
    assert_eq!(app.active, Some(ids[8]));
    assert_eq!(app.board.focus, ["B"]);
    app.runtime.shutdown();
}

/// A right-click on the labelled node.
fn context_click(app: &mut PhotoSortApp, ctx: &egui::Context, size: Vec2, label: &str) {
    use super::super::compact_cull_tests::{frame, rendered};
    let nodes = rendered(app, ctx, size);
    let pos = nodes
        .iter()
        .find(|(s, _)| s == label)
        .unwrap_or_else(|| panic!("Missing {label}"))
        .1
        .center();
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Secondary,
                    pressed,
                    modifiers: Default::default(),
                },
            ],
        );
    }
}

#[test]
fn new_folder_in_a_long_tree_is_made_in_the_plan_and_renamed_in_place() {
    use super::super::compact_cull_tests::{click_label, frame, rendered};
    let (_temp, mut app) = organize();
    // Enough folders that a new one lands far below the visible rows.
    for n in 1..=30 {
        new_folder(&mut app, "", &format!("M{n:02}"));
    }
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 420.0);
    let root = format!("{}  ·  9", app.board_root_name());
    context_click(&mut app, &ctx, size, &root);
    click_label(&mut app, &ctx, size, "New folder inside");
    let draft = &app.organize_drafts[&GroupMode::Current];
    assert!(draft.folders().contains("New folder"), "made in the plan");
    assert!(app.media.iter().all(|m| m.path.exists()));
    assert!(!app.root.as_ref().unwrap().join("New folder").exists());
    // Its name field is on screen, focused, with the name selected.
    let nodes = rendered(&mut app, &ctx, size);
    let field = nodes
        .iter()
        .find(|(label, _)| label == "New folder")
        .expect("the new folder's name field is shown");
    assert!(field.1.max.y <= size.y, "scrolled into view: {:?}", field.1);
    frame(&mut app, &ctx, size, vec![egui::Event::Text("Trip".into())]);
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Enter, Default::default()),
    );
    rendered(&mut app, &ctx, size);
    let draft = &app.organize_drafts[&GroupMode::Current];
    assert!(draft.folders().contains("Trip"));
    assert!(!draft.folders().contains("New folder"));
    assert!(app.board.editing.is_none());
    // A second one inside a folder takes the next free name.
    app.board_new_folder("Trip", &[]);
    app.board_new_folder("Trip", &[]);
    let folders = app.organize_drafts[&GroupMode::Current].folders();
    assert!(folders.contains("Trip/New folder") && folders.contains("Trip/New folder 2"));
    app.runtime.shutdown();
}

#[test]
fn show_in_finder_is_offered_only_for_folders_already_on_disk() {
    let (_temp, mut app) = organize();
    new_folder(&mut app, "", "Trip");
    app.check_board();
    app.refilter();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    let reveal = crate::core::desktop::Action::Reveal.label();
    context_click(&mut app, &ctx, size, "B  ·  5");
    let nodes = super::super::compact_cull_tests::rendered(&mut app, &ctx, size);
    assert!(shows(&nodes, reveal), "B exists on disk");
    assert!(shows(&nodes, "Move folder to Trash…"));
    assert_eq!(
        app.board_folder_on_disk("B"),
        Some(app.root.as_ref().unwrap().join("B"))
    );
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    context_click(&mut app, &ctx, size, "Trip  ·  0");
    let nodes = super::super::compact_cull_tests::rendered(&mut app, &ctx, size);
    assert!(!shows(&nodes, reveal), "Trip is only in the plan");
    assert!(shows(&nodes, "Remove folder"));
    assert!(!shows(&nodes, "Move folder to Trash"));
    assert_eq!(
        app.board_library_folder("B").as_deref(),
        app.catalog
            .as_ref()
            .unwrap()
            .directories
            .iter()
            .find(|d| d.ends_with("B"))
            .map(|d| d.as_path())
    );
    assert!(app.board_library_folder("").is_none());
    // A renamed folder is still found where it is on disk.
    app.board.editing = Some(("B".into(), "Beach".into()));
    app.board_finish_edit();
    assert_eq!(
        app.board_folder_on_disk("Beach"),
        Some(app.root.as_ref().unwrap().join("B"))
    );
    app.runtime.shutdown();
}

#[test]
fn a_trackpad_pinch_zooms_the_preview_around_the_pointer() {
    use super::super::compact_cull_tests::{frame, rendered};
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    app.select_library(ids[0], false, false);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    let nodes = rendered(&mut app, &ctx, size);
    let canvas = nodes
        .iter()
        .find(|(label, _)| label.starts_with("Photo 1:"))
        .expect("the preview pane shows the photo")
        .1;
    let pointer = canvas.center() + egui::vec2(60.0, 40.0);
    let before = app.view_for(0).zoom;
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::PointerMoved(pointer), egui::Event::Zoom(1.5)],
    );
    let view = app.view_for(0);
    assert!((view.zoom - before * 1.5).abs() < 1e-3, "{view:?}");
    // The point under the fingers stays put: the image moved away from it.
    assert!(view.pan[0] < 0.0 && view.pan[1] < 0.0, "{view:?}");
    // Pinching in again goes back to fit, centred.
    frame(
        &mut app,
        &ctx,
        size,
        vec![
            egui::Event::PointerMoved(pointer),
            egui::Event::Zoom(1.0 / 1.5),
        ],
    );
    let view = app.view_for(0);
    assert!((view.zoom - before).abs() < 1e-3 && view.pan == [0.0, 0.0]);
    // A pinch over the board does not zoom the preview.
    frame(
        &mut app,
        &ctx,
        size,
        vec![
            egui::Event::PointerMoved(egui::pos2(500.0, 400.0)),
            egui::Event::Zoom(1.5),
        ],
    );
    assert!((app.view_for(0).zoom - before).abs() < 1e-3);
    app.runtime.shutdown();
}

#[test]
fn holding_show_original_or_backslash_shows_the_folders_on_disk_until_release() {
    use super::super::compact_cull_tests::{frame, rendered};
    let (_temp, mut app) = organize();
    app.group_mode = GroupMode::Camera;
    app.filter_dirty = true;
    app.refilter();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    let sections = |app: &PhotoSortApp| -> Vec<Option<String>> {
        app.board.sections.iter().map(|s| s.path.clone()).collect()
    };
    let plan = vec![Some("Unknown camera".to_owned())];
    let disk = vec![Some("A/Day1".to_owned()), Some("B".to_owned())];
    rendered(&mut app, &ctx, size);
    assert_eq!(sections(&app), plan);
    let backslash = |pressed| egui::Event::Key {
        key: egui::Key::Backslash,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: Default::default(),
    };
    frame(&mut app, &ctx, size, vec![backslash(true)]);
    let nodes = rendered(&mut app, &ctx, size);
    assert!(app.board.original);
    assert_eq!(sections(&app), disk);
    assert!(
        shows(&nodes, "Empty  ·  0"),
        "empty folders on disk are listed"
    );
    assert!(!app.board_editable() && !app.photo_drag_enabled());
    // Edits wait for the plan to come back.
    let id = app.media[app.filtered[0]].id;
    app.board_move(&[id], "B");
    assert!(!app.organize_drafts.contains_key(&GroupMode::Camera));
    frame(&mut app, &ctx, size, vec![backslash(false)]);
    rendered(&mut app, &ctx, size);
    assert!(!app.board.original);
    assert_eq!(sections(&app), plan);
    // The button works the same way: down shows, up returns.
    let nodes = rendered(&mut app, &ctx, size);
    let pos = nodes
        .iter()
        .find(|(label, _)| label == "Show original")
        .expect("the Show original button")
        .1
        .center();
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
    rendered(&mut app, &ctx, size);
    assert_eq!(sections(&app), disk);
    frame(&mut app, &ctx, size, button(false));
    rendered(&mut app, &ctx, size);
    assert_eq!(sections(&app), plan);
    // A folder chosen in the sidebar is shown again afterwards.
    app.board.focus = vec!["Unknown camera".into()];
    frame(&mut app, &ctx, size, vec![backslash(true)]);
    assert!(app.board.focus.is_empty());
    frame(&mut app, &ctx, size, vec![backslash(false)]);
    assert_eq!(app.board.focus, ["Unknown camera"]);
    app.runtime.shutdown();
}

#[test]
fn dragging_on_empty_space_selects_every_photo_the_rectangle_touches() {
    use super::super::compact_cull_tests::rendered;
    let (_temp, mut app) = organize();
    // B's five photos fill one row and part of the next at this size.
    app.preferences.photo_size = 160.0;
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let names: Vec<String> = app.filtered.iter().map(|i| app.media[*i].name()).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 1100.0);
    let nodes = rendered(&mut app, &ctx, size);
    // The tile, not the preview's title of the same name.
    let tile = |name: &str| {
        nodes
            .iter()
            .filter(|(label, _)| label == name)
            .max_by(|a, b| a.1.area().total_cmp(&b.1.area()))
            .unwrap_or_else(|| panic!("tile {name}"))
            .1
    };
    let tiles: Vec<egui::Rect> = names.iter().map(|n| tile(n)).collect();
    let step = |app: &mut PhotoSortApp, pos: egui::Pos2, press: Option<bool>, keep: bool| {
        let modifiers = if keep {
            egui::Modifiers::SHIFT
        } else {
            Default::default()
        };
        let mut events = vec![
            egui::Event::ModifiersChanged(modifiers),
            egui::Event::PointerMoved(pos),
        ];
        if let Some(pressed) = press {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            });
        }
        ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| app.update_workspace(ui, false),
        );
    };
    let drag = |app: &mut PhotoSortApp, from: egui::Pos2, to: egui::Pos2, keep: bool| {
        step(app, from, Some(true), keep);
        step(app, from.lerp(to, 0.5), None, keep);
        step(app, to, None, keep);
        step(app, to, Some(false), keep);
    };
    // From the empty slot after B's last tile (B's five tiles fill one
    // row and part of the next), up and left to the middle of B's first
    // tile: it and the tiles between are selected.
    let columns = tiles[4..]
        .iter()
        .take_while(|t| t.top() == tiles[4].top())
        .count();
    assert!((2..5).contains(&columns), "B wraps: {columns} columns");
    let from = egui::pos2(tiles[4 + 5 % columns].center().x, tiles[8].center().y);
    let to = tiles[4].center();
    assert!(
        !tiles.iter().any(|t| t.contains(from)),
        "the drag starts on empty space"
    );
    drag(&mut app, from, to, false);
    let area = egui::Rect::from_two_pos(from, to);
    let expected: Vec<MediaId> = tiles
        .iter()
        .zip(&ids)
        .filter(|(t, _)| t.intersects(area))
        .map(|(_, id)| *id)
        .collect();
    assert!(expected.contains(&ids[4]) && expected.contains(&ids[8]));
    assert!(!expected.contains(&ids[0]));
    assert_eq!(app.selection.ordered(&app.media, &app.filtered), expected);
    assert!(app.active.is_some_and(|a| expected.contains(&a)));
    assert!(app.board.marquee.is_none(), "the rectangle ends on release");
    // Shift adds the next rectangle to the selection; this one starts
    // on A's header, which only takes clicks.
    let first = tiles[0];
    drag(
        &mut app,
        egui::pos2(first.left() + 10.0, first.top() - 8.0),
        first.center(),
        true,
    );
    let selected = app.selection.ordered(&app.media, &app.filtered);
    assert!(selected.contains(&ids[0]));
    assert!(expected.iter().all(|id| selected.contains(id)));
    // Without it, a new rectangle starts a new selection.
    drag(
        &mut app,
        egui::pos2(first.left() + 10.0, first.top() - 8.0),
        first.center(),
        false,
    );
    assert_eq!(
        app.selection.ordered(&app.media, &app.filtered),
        vec![ids[0]]
    );
    // Dragging a photo still drags the photo, not a rectangle.
    drag(&mut app, tiles[1].center(), tiles[2].center(), false);
    assert!(app.board.marquee.is_none());
    assert_eq!(
        app.selection.ordered(&app.media, &app.filtered),
        vec![ids[0]]
    );
    app.runtime.shutdown();
}

#[test]
fn folders_are_moved_by_drag_and_wrapped_removed_and_marked_from_their_menu() {
    use super::super::compact_cull_tests::{frame, rendered};
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    let labels = |app: &PhotoSortApp, id: MediaId| {
        app.organize_drafts[&GroupMode::Current].labels()[&id].clone()
    };
    // Marking a folder marks everything in it; again unmarks.
    app.board_folder_action(FolderAction::Trash("B".into()));
    assert!(ids[4..].iter().all(|id| app.decision(*id).rejected));
    assert!(ids[..4].iter().all(|id| !app.decision(*id).rejected));
    app.board_folder_action(FolderAction::Trash("B".into()));
    assert!(ids.iter().all(|id| !app.decision(*id).rejected));
    // Drag Day1 onto B in the sidebar.
    let nodes = rendered(&mut app, &ctx, size);
    let at = |label: &str| {
        nodes
            .iter()
            .find(|(l, _)| l == label)
            .unwrap_or_else(|| panic!("{label}"))
            .1
            .center()
    };
    let (from, to) = (at("Day1  ·  4"), at("B  ·  5"));
    let pointer = |pos, pressed: Option<bool>| {
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        events
    };
    frame(&mut app, &ctx, size, pointer(from, Some(true)));
    for t in [0.2, 0.5, 0.8, 1.0] {
        frame(&mut app, &ctx, size, pointer(from.lerp(to, t), None));
    }
    frame(&mut app, &ctx, size, pointer(to, Some(false)));
    rendered(&mut app, &ctx, size);
    assert!(ids[..4].iter().all(|id| labels(&app, *id) == "B/Day1"));
    assert!(app.board.folders.iter().any(|f| f.path == "B/Day1"));
    // Back to the top level from the menu.
    app.board_folder_action(FolderAction::Nest("B/Day1".into(), String::new()));
    assert!(ids[..4].iter().all(|id| labels(&app, *id) == "Day1"));
    // Put B inside a new folder, named in place.
    app.board_folder_action(FolderAction::Wrap("B".into()));
    assert_eq!(
        app.board.editing.as_ref().map(|(p, _)| p.as_str()),
        Some("New folder")
    );
    assert!(
        ids[4..]
            .iter()
            .all(|id| labels(&app, *id) == "New folder/B")
    );
    // A folder holding an existing one cannot be removed ...
    app.board_folder_action(FolderAction::Remove("New folder".into()));
    assert!(
        app.organize_drafts[&GroupMode::Current]
            .folders()
            .contains("New folder")
    );
    assert!(app.navigation_notice.is_some());
    // ... but a new one can: its photos go back where they were going,
    // A/Day1, which now sits at the top level as Day1.
    app.check_board();
    app.refilter();
    app.board_new_folder("", &ids[..2]);
    let made = app.board.editing.clone().unwrap().0;
    assert_eq!(made, "New folder 2");
    assert!(ids[..2].iter().all(|id| labels(&app, *id) == made));
    app.board_folder_action(FolderAction::Remove(made.clone()));
    assert!(ids[..2].iter().all(|id| labels(&app, *id) == "Day1"));
    assert!(
        !app.organize_drafts[&GroupMode::Current]
            .folders()
            .contains(&made)
    );
    // Start over forgets every folder edit.
    app.board_folder_action(FolderAction::StartOver);
    assert!(!app.organize_drafts.contains_key(&GroupMode::Current));
    // None of it touched the disk.
    assert!(app.media.iter().all(|m| m.path.exists()));
    let root = app.root.clone().unwrap();
    assert!(!root.join("New folder").exists() && !root.join("Day1").exists());
    assert!(app.decisions.values().all(|d| d.is_empty()));
    app.runtime.shutdown();
}

#[test]
fn f2_renames_the_current_photo_in_place_and_counts_as_a_change() {
    use super::super::compact_cull_tests::{frame, rendered};
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    app.select_library(ids[0], false, false);
    rendered(&mut app, &ctx, size);
    assert_eq!(app.board.changes, 0);
    assert_eq!(app.staged_change_count(), 0);
    frame(&mut app, &ctx, size, key(egui::Key::F2, Default::default()));
    let nodes = rendered(&mut app, &ctx, size);
    assert!(
        nodes.iter().any(|(label, _)| label == "photo-0"),
        "name field"
    );
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Text("Holiday".into())],
    );
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Enter, Default::default()),
    );
    let nodes = rendered(&mut app, &ctx, size);
    assert_eq!(
        app.file_names.get(&ids[0]).map(String::as_str),
        Some("Holiday")
    );
    assert!(shows(&nodes, "Holiday.png"));
    assert_eq!(app.board.changes, 1);
    // Moving two photos adds them; the title bar counts it all.
    app.board_move(&ids[1..3], "B");
    rendered(&mut app, &ctx, size);
    assert_eq!(app.board.changes, 3);
    assert_eq!(app.staged_change_count(), 3);
    assert!(
        app.board_note()
            .is_some_and(|n| n.starts_with("3 planned changes")),
        "{:?}",
        app.board_note()
    );
    // Escape keeps the name it had.
    frame(&mut app, &ctx, size, key(egui::Key::F2, Default::default()));
    rendered(&mut app, &ctx, size);
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Text("Other".into())],
    );
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Escape, Default::default()),
    );
    rendered(&mut app, &ctx, size);
    assert_eq!(
        app.file_names.get(&ids[0]).map(String::as_str),
        Some("Holiday")
    );
    assert!(app.board.renaming.is_none());
    assert!(app.media.iter().all(|m| m.path.exists()));
    app.runtime.shutdown();
}

/// A left click at `pos`.
fn click_at(app: &mut PhotoSortApp, ctx: &egui::Context, size: Vec2, pos: egui::Pos2) {
    use super::super::compact_cull_tests::frame;
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
}

fn found<'a>(nodes: &'a [(String, egui::Rect)], label: &str) -> Vec<&'a egui::Rect> {
    nodes
        .iter()
        .filter(|(s, _)| s == label)
        .map(|(_, rect)| rect)
        .collect()
}

#[test]
fn the_sidebar_board_and_preview_never_overlap_at_any_width() {
    use super::super::compact_cull_tests::rendered;
    let (_temp, mut app) = organize();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let panel =
        |id: &str| egui::PanelState::load(&ctx, egui::Id::new(id)).map(|p| p.outer_rect.x_range());
    for details in [false, true] {
        // One photo, then three compared side by side in the preview.
        for count in [1, 3] {
            app.gallery_select(count);
            for width in [600.0, 640.0, 760.0, 900.0, 1000.0, 1180.0, 1300.0, 1800.0] {
                app.inspector = details;
                let nodes = rendered(&mut app, &ctx, egui::vec2(width, 800.0));
                let case = format!("{count} selected at {width}, details {details}");
                let layout = app.board.layout;
                // The preview gives way first, then the sidebar; each
                // leaves a button on the board's header.
                assert!(layout.sidebar || !layout.preview, "{case}");
                assert_eq!(found(&nodes, "Folders").len(), !layout.sidebar as usize);
                assert_eq!(
                    found(&nodes, "Show preview").len(),
                    !layout.preview as usize
                );
                let left = if layout.sidebar {
                    panel("organize_folder_panel").unwrap().max
                } else {
                    0.0
                };
                let right = if layout.preview {
                    panel("organize_preview_panel").unwrap().min + BOARD_GAP
                } else {
                    width
                };
                let tiles: Vec<_> = nodes
                    .iter()
                    .filter(|(label, r)| label.starts_with("photo-") && r.height() > 40.0)
                    .collect();
                assert!(!tiles.is_empty(), "{case}");
                for (label, rect) in tiles {
                    assert!(
                        rect.left() >= left - 0.5 && rect.right() <= right + 0.5,
                        "{label} {rect:?} outside the board {left}..{right}, {case}"
                    );
                }
                // Nothing runs past the window, the preview's controls
                // included.
                for (label, rect) in &nodes {
                    assert!(rect.right() <= width + 0.5, "{label} {rect:?}, {case}");
                }
            }
        }
    }
    // A compact window still compares beside the board; a narrow one
    // keeps the sidebar; the narrowest gives the board the whole width.
    assert_eq!(board_layout(900.0 - 16.0, 240.0), BoardLayout::default());
    assert!(board_layout(640.0 - 16.0, 240.0).sidebar);
    assert!(!board_layout(600.0 - 16.0, 240.0).sidebar);
    app.runtime.shutdown();
}

#[test]
fn a_narrow_board_opens_the_preview_and_folders_from_its_header() {
    use super::super::compact_cull_tests::{click_label, frame, rendered};
    let (_temp, mut app) = organize();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    app.gallery_select(1);
    let size = egui::vec2(600.0, 800.0);
    let nodes = rendered(&mut app, &ctx, size);
    assert!(found(&nodes, "Close preview").is_empty());
    assert!(found(&nodes, "New folder").is_empty());
    // The preview opens over the board and closes again.
    click_label(&mut app, &ctx, size, "Show preview");
    let nodes = rendered(&mut app, &ctx, size);
    assert_eq!(found(&nodes, "Close preview").len(), 1);
    assert_eq!(found(&nodes, "Hide preview").len(), 1);
    click_label(&mut app, &ctx, size, "Close preview");
    assert!(!app.board.preview_open);
    // F2 renames in the preview's title, so it opens for it.
    frame(&mut app, &ctx, size, key(egui::Key::F2, Default::default()));
    assert!(app.board.renaming.is_some() && app.board.preview_open);
    app.board.renaming = None;
    app.board.preview_open = false;
    // A new folder is named in the sidebar, so its popover opens with
    // the name field focused.
    let command = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
    assert!(app.organize_shortcut(egui::Key::N, command));
    rendered(&mut app, &ctx, size);
    assert!(app.board.folders_open && app.board.editing.is_some());
    assert!(app.board.typing, "the name field has focus");
    app.board.editing = None;
    app.board.folders_open = false;
    // The folders open in a popover, with the sidebar's tools.
    click_label(&mut app, &ctx, size, "Folders");
    let nodes = rendered(&mut app, &ctx, size);
    assert_eq!(found(&nodes, "New folder").len(), 1);
    // A wider window brings the sidebar back and closes the popover.
    rendered(&mut app, &ctx, egui::vec2(1000.0, 800.0));
    assert!(app.board.layout.sidebar && !app.board.folders_open);
    app.runtime.shutdown();
}

#[test]
fn reset_changes_asks_then_drops_every_planned_change_but_keeps_marks() {
    use super::super::compact_cull_tests::{click_label, frame, rendered};
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    rendered(&mut app, &ctx, size);
    // Nothing planned: the button is there but does nothing.
    assert!(!app.organize_resettable());
    click_label(&mut app, &ctx, size, "Reset changes");
    assert!(!app.board.confirm_reset);
    // A move, a new folder, a rename, edits in another grouping, another
    // destination, plus a Trash mark and a rating from the other steps.
    app.board_move(&ids[..2], "B");
    new_folder(&mut app, "", "Trip");
    app.file_names.insert(ids[3], "Holiday".into());
    app.group_mode = GroupMode::Camera;
    new_folder(&mut app, "", "Picks");
    app.output = _temp.path().display().to_string();
    app.stage_manual([ids[4]], crate::core::decisions::Change::Trash(true));
    app.stage_manual([ids[5]], crate::core::decisions::Change::Rating(2));
    let decisions = app.decisions.clone();
    rendered(&mut app, &ctx, size);
    assert!(app.board.changes > 0 && app.organize_resettable());
    // Keep changes leaves everything as it was.
    click_label(&mut app, &ctx, size, "Reset changes");
    assert!(app.board.confirm_reset && app.modal_active());
    let nodes = rendered(&mut app, &ctx, size);
    assert!(shows(&nodes, "Trash marks, ratings and rotations stay."));
    click_label(&mut app, &ctx, size, "Keep changes");
    assert!(!app.board.confirm_reset);
    assert_eq!(app.organize_drafts.len(), 2);
    assert_eq!(app.group_mode, GroupMode::Camera);
    // So does Escape.
    click_label(&mut app, &ctx, size, "Reset changes");
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Escape, Default::default()),
    );
    rendered(&mut app, &ctx, size);
    assert!(!app.board.confirm_reset);
    assert_eq!(app.organize_drafts.len(), 2);
    // Confirming drops every planned change, in every grouping.
    click_label(&mut app, &ctx, size, "Reset changes");
    let nodes = rendered(&mut app, &ctx, size);
    let buttons = found(&nodes, "Reset changes");
    assert_eq!(buttons.len(), 2, "the bar's button and the dialog's");
    let confirm = buttons
        .into_iter()
        .max_by(|a, b| a.top().total_cmp(&b.top()))
        .unwrap()
        .center();
    click_at(&mut app, &ctx, size, confirm);
    rendered(&mut app, &ctx, size);
    assert!(!app.board.confirm_reset);
    assert!(app.organize_drafts.is_empty() && app.file_names.is_empty());
    assert_eq!(app.group_mode, GroupMode::Current);
    assert_eq!(app.output, app.root.as_ref().unwrap().display().to_string());
    assert_eq!(app.board.changes, 0);
    assert!(!app.organize_resettable());
    // The other steps' decisions stay, and so does every file.
    assert_eq!(app.decisions, decisions);
    assert_eq!(app.staged_change_count(), 2);
    assert!(app.media.iter().all(|m| m.path.exists()));
    app.runtime.shutdown();
}

#[test]
fn one_top_bar_holds_every_step_and_fits_every_width() {
    use super::super::compact_cull_tests::rendered;
    use super::super::top_bar::HEADER_HEIGHT;
    let (_temp, mut app) = organize();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let analyze = "Analyze library · Ctrl/Cmd+S";
    let cases: [(Step, &[&str]); 5] = [
        (
            Step::Organize,
            &["Show original", "Reset changes", analyze, "Details"],
        ),
        (Step::EasyDelete, &[analyze, "Details"]),
        (
            Step::Rotation,
            &["Suggestions only", "Detect rotations again · Ctrl/Cmd+S"],
        ),
        (Step::PickBest, &[analyze, "Details"]),
        (
            Step::Cull,
            &["Filters & sort", "Library navigator", analyze],
        ),
    ];
    for (step, controls) in cases {
        app.set_step(step);
        app.runtime.cancel_analysis();
        app.analysis_run = None;
        if step == Step::Cull {
            // A selection adds its menu to Cull's bar. Cull's list
            // comes from a worker, so wait until it shows photos.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while app.filtered.is_empty() && std::time::Instant::now() < deadline {
                rendered(&mut app, &ctx, egui::vec2(1280.0, 800.0));
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            app.select_library(app.media[app.filtered[0]].id, false, false);
            app.review = false;
        }
        for width in [640.0, 760.0, 1000.0, 1280.0, 1800.0] {
            let nodes = rendered(&mut app, &ctx, egui::vec2(width, 800.0));
            assert!(
                found(&nodes, "More actions").is_empty(),
                "{step:?} at {width}"
            );
            let mut rects: Vec<(&str, egui::Rect)> = ["Review changes", "Menu"]
                .iter()
                .chain(controls)
                .map(|name| {
                    let rects = found(&nodes, name);
                    assert_eq!(rects.len(), 1, "{name}, {step:?} at {width}");
                    (*name, *rects[0])
                })
                .collect();
            // One bar: every control sits on it, inside the window, and
            // none overlaps another.
            for (name, rect) in &rects {
                assert!(
                    rect.bottom() <= HEADER_HEIGHT && rect.left() >= 0.0 && rect.right() <= width,
                    "{name} {rect:?} off the bar, {step:?} at {width}"
                );
            }
            if step == Step::Cull {
                assert!(
                    nodes.iter().any(|(name, r)| name.starts_with("1 selected")
                        && r.bottom() <= HEADER_HEIGHT
                        && r.right() <= width),
                    "selection at {width}: {:?}",
                    nodes
                        .iter()
                        .filter(|(_, r)| r.bottom() <= HEADER_HEIGHT)
                        .collect::<Vec<_>>()
                );
            }
            // Nothing in the bar is cut off or drawn over another.
            rects = nodes
                .iter()
                .filter(|(_, r)| r.bottom() <= HEADER_HEIGHT && r.width() < width / 2.0)
                .map(|(name, r)| (name.as_str(), *r))
                .collect();
            rects.sort_by(|a, b| a.1.left().total_cmp(&b.1.left()));
            for (i, (_, a)) in rects.iter().enumerate() {
                for (_, b) in &rects[i + 1..] {
                    // A label inside its own slot is not an overlap.
                    let nested = a.expand(0.5).contains_rect(*b) || b.expand(0.5).contains_rect(*a);
                    assert!(
                        nested || a.intersect(*b).width() <= 0.5,
                        "{a:?} and {b:?} overlap, {step:?} at {width}: {rects:?}"
                    );
                }
            }
            if step == Step::Organize {
                // Search and the photo filter sit over the board.
                assert!(found(&nodes, "Show")[0].top() > HEADER_HEIGHT);
                // Wide windows run the folder sidebar up beside the bar.
                let folders = found(&nodes, "FOLDERS")[0];
                assert_eq!(folders.top() < HEADER_HEIGHT, width > 1180.0, "at {width}");
            }
        }
    }
    // Faces adds its own menu; a narrow bar moves it into a popover.
    app.set_step(Step::Organize);
    app.group_mode = GroupMode::Faces;
    for (width, inline) in [(1800.0, true), (640.0, false)] {
        let nodes = rendered(&mut app, &ctx, egui::vec2(width, 800.0));
        let menu = if inline {
            "Keep in their folders"
        } else {
            "More grouping options"
        };
        let rect = *found(&nodes, menu)[0];
        assert!(
            rect.bottom() <= HEADER_HEIGHT && rect.right() <= width,
            "{menu} at {width}"
        );
    }
    app.runtime.shutdown();
}

#[test]
fn board_search_filters_photos_and_stays_open_when_nothing_matches() {
    use super::super::compact_cull_tests::{click_label, frame, rendered};
    let search = egui::Id::new("library_search");
    for width in [900.0, 1440.0] {
        let (_temp, mut app) = organize();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(width, 800.0);
        let nodes = rendered(&mut app, &ctx, size);
        // A narrow board shows a magnifier until it is needed.
        assert_eq!(
            found(&nodes, "Search · Ctrl/Cmd+F").len(),
            usize::from(width < 1000.0),
            "at {width}"
        );
        frame(
            &mut app,
            &ctx,
            size,
            key(egui::Key::F, egui::Modifiers::COMMAND),
        );
        rendered(&mut app, &ctx, size);
        assert_eq!(ctx.memory(|m| m.focused()), Some(search), "at {width}");
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Text("photo-1".into())],
        );
        rendered(&mut app, &ctx, size);
        assert_eq!(app.query, "photo-1");
        assert_eq!(app.filtered.len(), 1, "at {width}");
        // No match keeps the board, the field and its focus.
        frame(&mut app, &ctx, size, vec![egui::Event::Text("x".into())]);
        let nodes = rendered(&mut app, &ctx, size);
        assert!(app.filtered.is_empty());
        assert!(shows(&nodes, "No photos match"), "at {width}");
        assert!(shows(&nodes, "Photo size"), "the board stays");
        assert_eq!(ctx.memory(|m| m.focused()), Some(search), "at {width}");
        click_label(&mut app, &ctx, size, "Clear search and filter");
        rendered(&mut app, &ctx, size);
        assert!(app.query.is_empty());
        assert_eq!(app.filtered.len(), 9);
        app.runtime.shutdown();
    }
}

#[test]
fn the_narrow_boards_magnifier_opens_a_focused_field_that_closes_when_left_empty() {
    use super::super::compact_cull_tests::{click_label, frame, rendered};
    let search = egui::Id::new("library_search");
    let (_temp, mut app) = organize();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(900.0, 800.0);
    click_label(&mut app, &ctx, size, "Search · Ctrl/Cmd+F");
    rendered(&mut app, &ctx, size);
    assert_eq!(ctx.memory(|m| m.focused()), Some(search));
    frame(&mut app, &ctx, size, vec![egui::Event::Text("B".into())]);
    rendered(&mut app, &ctx, size);
    assert_eq!(app.query, "B");
    // Emptied and left, it folds back into the magnifier.
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Backspace, Default::default()),
    );
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Escape, Default::default()),
    );
    let nodes = rendered(&mut app, &ctx, size);
    assert!(app.query.is_empty() && !app.board.searching);
    assert_eq!(found(&nodes, "Search · Ctrl/Cmd+F").len(), 1);
    app.runtime.shutdown();
}

#[test]
fn the_faces_bar_wraps_before_its_buttons_at_every_width() {
    use super::super::compact_cull_tests::rendered;
    let (_temp, mut app) = organize();
    app.group_mode = GroupMode::Faces;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    for (width, inspector) in [
        (900.0, false),
        (1000.0, false),
        (1100.0, false),
        (1150.0, false),
        (1200.0, false),
        (1280.0, false),
        (1300.0, true),
        (1440.0, true),
    ] {
        app.inspector = inspector;
        let nodes = rendered(&mut app, &ctx, egui::vec2(width, 800.0));
        let original = *found(&nodes, "Show original")[0];
        for name in ["Keep in their folders", "Organize into"] {
            let rect = nodes
                .iter()
                .find(|(s, _)| s.contains(name))
                .unwrap_or_else(|| panic!("{name} at {width}"))
                .1;
            assert!(
                rect.right() <= original.left()
                    || rect.top() >= original.bottom()
                    || rect.bottom() <= original.top(),
                "{name} {rect:?} runs under Show original {original:?} at {width}"
            );
        }
    }
    app.runtime.shutdown();
}

#[test]
fn the_board_filter_shows_marked_photos_and_lights_up_while_it_hides_some() {
    use super::super::compact_cull_tests::{click_label, rendered};
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    app.stage_manual([ids[2]], crate::core::decisions::Change::Trash(true));
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    click_label(&mut app, &ctx, size, "Show");
    click_label(&mut app, &ctx, size, Filter::Rejected.title());
    rendered(&mut app, &ctx, size);
    assert_eq!(app.filter, Filter::Rejected);
    assert_eq!(app.filtered.len(), 1);
    app.runtime.shutdown();
}

/// Organize grouped by `mode` with four photos the grouping leaves
/// where they are, which are returned.
fn grouped(mode: GroupMode) -> (tempfile::TempDir, PhotoSortApp, Vec<MediaId>) {
    let (temp, mut app) = organize();
    let ids: Vec<MediaId> = app.media.iter().map(|m| m.id).collect();
    match mode {
        GroupMode::Faces => app.gallery_organize_by_faces(),
        GroupMode::Visual => {
            app.visual_groups = Some(
                [0, 1, 4, 5, 8]
                    .into_iter()
                    .map(|i| (ids[i], i % 2))
                    .collect(),
            );
            app.group_mode = mode;
        }
        _ => {
            app.mixed_labels = Some(
                [0, 1, 4, 5, 8]
                    .into_iter()
                    .map(|i| (ids[i], format!("2023/Burst {}", i % 2)))
                    .collect(),
            );
            app.group_mode = mode;
        }
    }
    app.filter_dirty = true;
    app.refilter();
    let unplaced: Vec<MediaId> = app
        .media
        .iter()
        .filter(|m| app.organize_label(m).is_none())
        .map(|m| m.id)
        .collect();
    assert_eq!(unplaced.len(), 4, "{mode:?}");
    (temp, app, unplaced)
}

/// The board section an item is shown in; `None` is "Not moving".
fn section_of(app: &PhotoSortApp, id: MediaId) -> Option<String> {
    let position = app
        .filtered
        .iter()
        .position(|&i| app.media[i].id == id)
        .expect("shown");
    app.board
        .sections
        .iter()
        .find(|s| s.range.contains(&position))
        .expect("in a section")
        .path
        .clone()
}

/// Drags from one point to another with the pointer, as a person does.
fn drag(app: &mut PhotoSortApp, ctx: &egui::Context, size: Vec2, from: egui::Pos2, to: egui::Pos2) {
    use super::super::compact_cull_tests::frame;
    let pointer = |pos, pressed: Option<bool>| {
        let mut events = vec![egui::Event::PointerMoved(pos)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Default::default(),
            });
        }
        events
    };
    frame(app, ctx, size, pointer(from, Some(true)));
    for t in [0.2, 0.5, 0.8, 1.0] {
        frame(app, ctx, size, pointer(from.lerp(to, t), None));
    }
    frame(app, ctx, size, pointer(to, Some(false)));
}

fn center(nodes: &[(String, egui::Rect)], label: &str) -> egui::Pos2 {
    nodes
        .iter()
        .find(|(l, _)| l == label || l.starts_with(label))
        .unwrap_or_else(|| {
            panic!(
                "Missing {label}; labels {:?}",
                nodes.iter().map(|(l, _)| l).collect::<Vec<_>>()
            )
        })
        .1
        .center()
}

#[test]
fn items_not_moving_move_by_every_path_and_count_as_planned_changes() {
    use super::super::compact_cull_tests::{click_label, rendered};
    for mode in [GroupMode::Faces, GroupMode::Visual, GroupMode::Mixed] {
        let (_temp, mut app, unplaced) = grouped(mode);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1500.0, 1500.0);
        rendered(&mut app, &ctx, size);
        let target = app
            .board
            .folders
            .iter()
            .find(|f| !f.path.is_empty() && f.direct > 0)
            .unwrap()
            .path
            .clone();
        let name = |app: &PhotoSortApp, id: MediaId| app.media[app.index[&id]].name();
        // The grouping's own moves come first.
        let base = app.board.changes;
        let mut moved = 0;
        let mut check = |app: &mut PhotoSortApp, id: MediaId, how: &str| {
            rendered(app, &ctx, size);
            moved += 1;
            assert_eq!(section_of(app, id), Some(target.clone()), "{mode:?} {how}");
            assert_eq!(app.board.unplaced, 4 - moved, "{mode:?} {how}");
            assert_eq!(app.board.changes, base + moved, "{mode:?} {how}");
            assert!(app.organize_changes_pending(), "{mode:?} {how}");
        };
        // Dragged onto the folder in the sidebar.
        let nodes = rendered(&mut app, &ctx, size);
        let row = app.board.folders.iter().find(|f| f.path == target).unwrap();
        let row = format!("{}  ·  {}", leaf(&row.path), row.total);
        let (from, to) = (
            center(&nodes, &name(&app, unplaced[0])),
            center(&nodes, &row),
        );
        drag(&mut app, &ctx, size, from, to);
        check(&mut app, unplaced[0], "sidebar drag");
        // Dragged onto the folder's heading on the board.
        let nodes = rendered(&mut app, &ctx, size);
        let (from, to) = (
            center(&nodes, &name(&app, unplaced[1])),
            center(&nodes, &format!("Folder {target}")),
        );
        drag(&mut app, &ctx, size, from, to);
        check(&mut app, unplaced[1], "heading drag");
        // The tile's menu.
        let label = target.replace('/', " / ");
        let tile = name(&app, unplaced[2]);
        context_click(&mut app, &ctx, size, &tile);
        let nodes = rendered(&mut app, &ctx, size);
        click_at(&mut app, &ctx, size, center(&nodes, "Move item to folder"));
        click_label(&mut app, &ctx, size, &label);
        check(&mut app, unplaced[2], "tile menu");
        app.runtime.shutdown();
    }
}

#[test]
fn items_put_back_or_left_in_place_return_to_not_moving() {
    use super::super::compact_cull_tests::{click_label, frame, rendered};
    for mode in [GroupMode::Faces, GroupMode::Visual, GroupMode::Mixed] {
        let (_temp, mut app, unplaced) = grouped(mode);
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1500.0, 1500.0);
        rendered(&mut app, &ctx, size);
        let target = app
            .board
            .folders
            .iter()
            .find(|f| !f.path.is_empty() && f.direct > 0)
            .unwrap()
            .path
            .clone();
        let settle = |app: &mut PhotoSortApp| {
            rendered(app, &ctx, size);
        };
        // The grouping's own moves.
        let base = app.board.changes;
        // The sidebar's Move selected here, and moving back to the
        // folder it is in.
        app.selection.replace([unplaced[0]]);
        app.board_folder_action(FolderAction::MoveSelected(target.clone()));
        settle(&mut app);
        assert_eq!(
            section_of(&app, unplaced[0]),
            Some(target.clone()),
            "{mode:?}"
        );
        let home = app
            .current_label(&app.media[app.index[&unplaced[0]]])
            .unwrap();
        app.board_move(&[unplaced[0]], &home);
        settle(&mut app);
        assert_eq!(section_of(&app, unplaced[0]), None, "{mode:?} moved back");
        assert_eq!(
            (app.board.unplaced, app.board.changes),
            (4, base),
            "{mode:?}"
        );
        // The menu's Not moving.
        app.board_move(&[unplaced[1]], &target);
        settle(&mut app);
        let name = app.media[app.index[&unplaced[1]]].name();
        context_click(&mut app, &ctx, size, &name);
        let nodes = rendered(&mut app, &ctx, size);
        click_at(&mut app, &ctx, size, center(&nodes, "Move item to folder"));
        click_label(&mut app, &ctx, size, "Not moving  ·  stays in its folder");
        settle(&mut app);
        assert_eq!(section_of(&app, unplaced[1]), None, "{mode:?} menu");
        // Dragged onto the Not moving heading.
        app.board_move(&[unplaced[2]], &target);
        let nodes = rendered(&mut app, &ctx, size);
        let name = app.media[app.index[&unplaced[2]]].name();
        let (from, to) = (center(&nodes, &name), center(&nodes, "Not moving · "));
        drag(&mut app, &ctx, size, from, to);
        settle(&mut app);
        assert_eq!(section_of(&app, unplaced[2]), None, "{mode:?} heading");
        assert_eq!(
            (app.board.unplaced, app.board.changes),
            (4, base),
            "{mode:?}"
        );
        // A new folder made from it, then removed, leaves it in place.
        app.board_new_folder("", &[unplaced[3]]);
        app.board.editing = None;
        settle(&mut app);
        let made = section_of(&app, unplaced[3]).expect("in the new folder");
        app.board_folder_action(FolderAction::Remove(made));
        settle(&mut app);
        assert_eq!(
            section_of(&app, unplaced[3]),
            None,
            "{mode:?} removed folder"
        );
        // Show original, Start over and Reset treat it like any move.
        app.board_move(&[unplaced[0]], &target);
        settle(&mut app);
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Key {
                key: egui::Key::Backslash,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        settle(&mut app);
        assert!(app.board.original);
        assert_eq!(
            section_of(&app, unplaced[0]),
            Some(home.clone()),
            "{mode:?}"
        );
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::Key {
                key: egui::Key::Backslash,
                physical_key: None,
                pressed: false,
                repeat: false,
                modifiers: Default::default(),
            }],
        );
        settle(&mut app);
        assert_eq!(
            section_of(&app, unplaced[0]),
            Some(target.clone()),
            "{mode:?}"
        );
        app.board_folder_action(FolderAction::StartOver);
        settle(&mut app);
        assert_eq!(section_of(&app, unplaced[0]), None, "{mode:?} start over");
        app.board_move(&[unplaced[0]], &target);
        settle(&mut app);
        assert_eq!(app.board.changes, base + 1);
        app.reset_organize();
        settle(&mut app);
        assert!(app.organize_drafts.is_empty() && app.board.changes == 0);
        assert!(app.media.iter().all(|m| m.path.exists()));
        app.runtime.shutdown();
    }
}

/// Organize grouped by faces: three photos of person 001, two of 002,
/// two of both, two of nobody.
fn people_board() -> (tempfile::TempDir, PhotoSortApp, Vec<MediaId>) {
    let (temp, mut app) = organize();
    let ids: Vec<MediaId> = app.media.iter().map(|m| m.id).collect();
    let labels = [
        "Person 001",
        "Person 001",
        "Person 001",
        "People 001 + 002",
        "Person 002",
        "Person 002",
        "People 001 + 002",
        "",
        "",
    ];
    let mut output = crate::core::regional::grouping::Output {
        groups: HashMap::new(),
        folders: HashMap::new(),
    };
    for (id, label) in ids.iter().zip(labels) {
        let group = match label {
            "" => 0,
            "Person 001" => 1,
            "Person 002" => 2,
            _ => 3,
        };
        output.groups.insert(*id, group);
        output.folders.insert(
            *id,
            if label.is_empty() {
                "Unassigned"
            } else {
                label
            }
            .into(),
        );
    }
    app.face_groups = Some(output);
    app.set_group_mode(GroupMode::Faces);
    app.filter_dirty = true;
    app.refilter();
    (temp, app, ids)
}

#[test]
fn face_folders_show_their_persons_face_and_rows_stay_level() {
    use super::super::compact_cull_tests::{frame, rendered};
    let (_temp, mut app, ids) = people_board();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1500.0, 1500.0);
    rendered(&mut app, &ctx, size);
    let who = |app: &PhotoSortApp, path: &str| app.board.people.folders.get(path).cloned();
    assert_eq!(who(&app, "Person 001"), Some(vec!["001".to_owned()]));
    assert_eq!(
        who(&app, "People 001 + 002"),
        Some(vec!["001".to_owned(), "002".to_owned()])
    );
    assert_eq!(who(&app, ""), None);
    // Until a face is known, folders keep their icon.
    assert!(app.folder_faces("Person 001", 2.0).is_empty());
    let bbox = [0.3, 0.2, 0.7, 0.6];
    for (person, id) in [("001", ids[0]), ("002", ids[4])] {
        app.board.people.portraits.insert(
            person.into(),
            vec![super::super::people::Portrait { id, bbox }],
        );
    }
    // Faces appear once the thumbnails they are cut from are in.
    let started = Instant::now();
    while app.folder_faces("People 001 + 002", 2.0).len() < 2 {
        assert!(started.elapsed().as_secs() < 10, "thumbnails");
        rendered(&mut app, &ctx, size);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(app.folder_faces("Person 002", 2.0).len(), 1);
    // The hover card: each person by name, with their faces.
    app.board
        .people
        .portraits
        .get_mut("001")
        .unwrap()
        .push(super::super::people::Portrait { id: ids[1], bbox });
    let started = Instant::now();
    let card = loop {
        let card = app.folder_card("People 001 + 002", 2.0);
        if card.iter().map(|(_, faces)| faces.len()).sum::<usize>() == 3 {
            break card;
        }
        assert!(started.elapsed().as_secs() < 10, "thumbnails");
        rendered(&mut app, &ctx, size);
        std::thread::sleep(std::time::Duration::from_millis(5));
    };
    let names: Vec<&str> = card.iter().map(|(name, _)| name.as_str()).collect();
    assert_eq!(names, ["Person 001", "Person 002"]);
    assert_eq!(card[0].1.len(), 2);
    let nodes = rendered(&mut app, &ctx, size);
    let face = found(&nodes, "Person 001  ·  3")[0];
    let library = found(&nodes, "photos  ·  9")[0];
    assert_eq!(face.height(), library.height(), "rows stay the same height");
    assert!(shows(&nodes, "Folder Person 001"));
    // Show original lists the folders on disk, which have no faces.
    let backslash = |pressed| {
        vec![egui::Event::Key {
            key: egui::Key::Backslash,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        }]
    };
    frame(&mut app, &ctx, size, backslash(true));
    rendered(&mut app, &ctx, size);
    assert!(app.board.people.signature.is_none() && app.board.people.folders.is_empty());
    frame(&mut app, &ctx, size, backslash(false));
    rendered(&mut app, &ctx, size);
    assert_eq!(
        app.folder_faces("Person 002", 2.0).len(),
        1,
        "back with the plan"
    );
    // Photos dragged onto a face folder move there.
    let nodes = rendered(&mut app, &ctx, size);
    let name = app.media[app.index[&ids[7]]].name();
    let (from, to) = (center(&nodes, &name), center(&nodes, "Person 002  ·  2"));
    drag(&mut app, &ctx, size, from, to);
    rendered(&mut app, &ctx, size);
    assert_eq!(section_of(&app, ids[7]).as_deref(), Some("Person 002"));
    app.runtime.shutdown();
}

#[test]
fn naming_a_person_renames_every_folder_the_grouping_named_for_them() {
    use super::super::compact_cull_tests::rendered;
    let (_temp, mut app, _ids) = people_board();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    let name = |app: &mut PhotoSortApp, path: &str, to: &str| {
        app.board_rename_folder(path);
        app.board.editing.as_mut().unwrap().1 = to.into();
        app.board_finish_edit();
        rendered(app, &ctx, size);
    };
    let folders = |app: &PhotoSortApp| -> Vec<String> {
        app.board.folders.iter().map(|f| f.path.clone()).collect()
    };
    rendered(&mut app, &ctx, size);
    // A person's folder still called "Person 002" offers a name on hover.
    assert!(app.unnamed_person("Person 002") && !app.unnamed_person("People 001 + 002"));
    name(&mut app, "Person 002", "Ana");
    assert_eq!(folders(&app), ["", "Ana", "Person 001", "Person 001 + Ana"]);
    // The face follows the person into the renamed folders.
    assert_eq!(app.board.people.folders["Ana"], ["002"]);
    assert_eq!(app.board.people.folders["Person 001 + Ana"], ["001", "002"]);
    assert!(!app.unnamed_person("Ana"));
    // All or nothing: a name that would clash changes no folder.
    new_folder(&mut app, "", "Bo + Ana");
    rendered(&mut app, &ctx, size);
    name(&mut app, "Person 001", "Bo");
    assert!(folders(&app).contains(&"Person 001".to_owned()));
    assert!(folders(&app).contains(&"Person 001 + Ana".to_owned()));
    assert!(app.navigation_notice.is_some());
    app.board_folder_action(FolderAction::Remove("Bo + Ana".into()));
    rendered(&mut app, &ctx, size);
    // A name typed for a shared folder is kept.
    name(&mut app, "Person 001 + Ana", "Family");
    name(&mut app, "Person 001", "Bo");
    assert_eq!(folders(&app), ["", "Ana", "Bo", "Family"]);
    // Nothing on disk changed.
    assert!(app.media.iter().all(|m| m.path.exists()));
    app.runtime.shutdown();
}

/// Faces found three people, but 003 is really 001.
fn three_people_board() -> (tempfile::TempDir, PhotoSortApp) {
    let (temp, mut app, ids) = people_board();
    let groups = app.face_groups.as_mut().unwrap();
    for (index, label, group) in [(2, "Person 003", 4), (6, "People 002 + 003", 5)] {
        groups.groups.insert(ids[index], group);
        groups.folders.insert(ids[index], label.into());
    }
    app.filter_dirty = true;
    app.refilter();
    (temp, app)
}

#[test]
fn two_people_marked_the_same_combine_their_folders_as_a_staged_edit() {
    use super::super::compact_cull_tests::{frame, rendered};
    let folders = |app: &PhotoSortApp| -> Vec<String> {
        app.board.folders.iter().map(|f| f.path.clone()).collect()
    };
    let split = [
        "",
        "People 001 + 002",
        "People 002 + 003",
        "Person 001",
        "Person 002",
        "Person 003",
    ];
    let joined = ["", "People 001 + 002", "Person 001", "Person 002"];
    for how in ["menu", "drag"] {
        let (_temp, mut app) = three_people_board();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        let size = egui::vec2(1400.0, 1000.0);
        rendered(&mut app, &ctx, size);
        assert_eq!(folders(&app), split, "{how}");
        assert_eq!(
            app.same_person_choices("Person 003"),
            ["Person 001", "Person 002"]
        );
        assert!(app.same_person_choices("People 001 + 002").is_empty());
        if how == "menu" {
            context_click(&mut app, &ctx, size, "Person 003  ·  1");
            let nodes = rendered(&mut app, &ctx, size);
            click_at(&mut app, &ctx, size, center(&nodes, "Same person as…"));
            let nodes = rendered(&mut app, &ctx, size);
            // The submenu's entry, not the sidebar row of the same name.
            let entry = nodes
                .iter()
                .find(|(l, _)| l == "Person 001")
                .expect("menu entry")
                .1
                .center();
            click_at(&mut app, &ctx, size, entry);
        } else {
            let nodes = rendered(&mut app, &ctx, size);
            let (from, to) = (
                center(&nodes, "Person 003  ·  1"),
                center(&nodes, "Person 001  ·  2"),
            );
            drag(&mut app, &ctx, size, from, to);
        }
        rendered(&mut app, &ctx, size);
        assert_eq!(folders(&app), joined, "{how}");
        let count = |app: &PhotoSortApp, path: &str| {
            app.board
                .folders
                .iter()
                .find(|f| f.path == path)
                .unwrap()
                .total
        };
        assert_eq!(count(&app, "Person 001"), 3, "{how}");
        assert_eq!(count(&app, "People 001 + 002"), 2, "{how}");
        assert_eq!(app.board.people.folders["Person 001"], ["001"]);
        assert_eq!(
            app.board.people.merged.get("003").map(String::as_str),
            Some("001")
        );
        // Show original lists the folders on disk; the plan comes back.
        let backslash = |pressed| {
            vec![egui::Event::Key {
                key: egui::Key::Backslash,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: Default::default(),
            }]
        };
        frame(&mut app, &ctx, size, backslash(true));
        rendered(&mut app, &ctx, size);
        assert!(!folders(&app).contains(&"Person 001".to_owned()));
        frame(&mut app, &ctx, size, backslash(false));
        rendered(&mut app, &ctx, size);
        assert_eq!(folders(&app), joined, "{how}");
        // Start over undoes it with every other folder edit.
        app.board_folder_action(FolderAction::StartOver);
        rendered(&mut app, &ctx, size);
        assert_eq!(folders(&app), split, "{how}");
        assert!(app.board.people.merged.is_empty());
        assert!(app.media.iter().all(|m| m.path.exists()));
        app.runtime.shutdown();
    }
}

#[test]
fn a_named_person_marked_the_same_keeps_the_name_everywhere() {
    use super::super::compact_cull_tests::rendered;
    let (_temp, mut app) = three_people_board();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 1000.0);
    let folders = |app: &PhotoSortApp| -> Vec<String> {
        app.board.folders.iter().map(|f| f.path.clone()).collect()
    };
    rendered(&mut app, &ctx, size);
    app.board_rename_folder("Person 003");
    app.board.editing.as_mut().unwrap().1 = "Ana".into();
    app.board_finish_edit();
    rendered(&mut app, &ctx, size);
    assert_eq!(
        folders(&app),
        [
            "",
            "Ana",
            "People 001 + 002",
            "Person 001",
            "Person 002",
            "Person 002 + Ana"
        ]
    );
    app.board_folder_action(FolderAction::SamePerson("Ana".into(), "Person 001".into()));
    rendered(&mut app, &ctx, size);
    assert_eq!(folders(&app), ["", "Ana", "Ana + Person 002", "Person 002"]);
    let count = |path: &str| {
        app.board
            .folders
            .iter()
            .find(|f| f.path == path)
            .unwrap()
            .total
    };
    assert_eq!((count("Ana"), count("Ana + Person 002")), (3, 2));
    assert!(
        app.navigation_notice
            .as_deref()
            .is_some_and(|n| n.contains("is now Ana"))
    );
    // The same person twice, or a shared folder, is refused.
    app.board_folder_action(FolderAction::SamePerson("Ana".into(), "Ana".into()));
    assert!(app.navigation_notice.is_some());
    let before = app.organize_drafts[&GroupMode::Faces].clone();
    assert!(app.same_person("Ana + Person 002", "Person 002").is_err());
    assert_eq!(app.organize_drafts[&GroupMode::Faces], before);
    app.runtime.shutdown();
}

#[test]
fn the_selection_circle_adds_a_photo_and_shift_click_compares_the_range() {
    use super::super::compact_cull_tests::rendered;
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1500.0, 1500.0);
    let tile = |app: &mut PhotoSortApp, id: MediaId| {
        let name = app.media[app.index[&id]].name();
        *found(&rendered(app, &ctx, size), &name)
            .into_iter()
            .max_by(|a, b| a.area().total_cmp(&b.area()))
            .expect("tile")
    };
    // egui learns which keys are held from their own event.
    let click = |app: &mut PhotoSortApp, pos: egui::Pos2, modifiers: egui::Modifiers| {
        for pressed in [true, false] {
            super::super::compact_cull_tests::frame(
                app,
                &ctx,
                size,
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ],
            );
        }
        super::super::compact_cull_tests::frame(
            app,
            &ctx,
            size,
            vec![egui::Event::ModifiersChanged(Default::default())],
        );
    };
    // Shift-click selects the range and compares it in the preview.
    let first = tile(&mut app, ids[0]).center();
    click(&mut app, first, Default::default());
    assert_eq!(app.selection.ordered(&app.media, &app.filtered), [ids[0]]);
    let last = tile(&mut app, ids[3]).center();
    click(&mut app, last, egui::Modifiers::SHIFT);
    assert_eq!(app.selection.ordered(&app.media, &app.filtered), ids[..4]);
    assert_eq!(app.board_selection_comparison(), Some(ids[..4].to_vec()));
    rendered(&mut app, &ctx, size);
    assert_eq!(app.comparison_ids(), ids[..4]);
    // A click on the body selects only that photo again.
    click(&mut app, first, Default::default());
    assert_eq!(app.selection.ordered(&app.media, &app.filtered), [ids[0]]);
    // The round circle at a tile's corner adds that photo, and again
    // removes it.
    let other = tile(&mut app, ids[5]);
    let circle = egui::pos2(
        other.right() - 6.0 - S2 - 1.0,
        other.bottom() - library::CAPTION - S2 - 1.0,
    );
    click(&mut app, circle, Default::default());
    assert_eq!(
        app.selection.ordered(&app.media, &app.filtered),
        [ids[0], ids[5]]
    );
    // The tile kept its shape, so the same spot removes it again.
    assert_eq!(tile(&mut app, ids[5]), other);
    click(&mut app, circle, Default::default());
    assert_eq!(app.selection.ordered(&app.media, &app.filtered), [ids[0]]);
    app.runtime.shutdown();
}

#[test]
fn ctrl_or_cmd_click_on_folder_names_adds_and_removes_their_photos() {
    use super::super::compact_cull_tests::{frame, rendered};
    let (_temp, mut app, _) = grouped(GroupMode::Mixed);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1500.0, 1500.0);
    rendered(&mut app, &ctx, size);
    let sections: Vec<(String, Vec<MediaId>)> = app
        .board
        .sections
        .iter()
        .map(|s| {
            let title = match s.path.as_deref() {
                Some(path) => format!("Folder {path}"),
                None => "Not moving".into(),
            };
            let ids = app.filtered[s.range.clone()]
                .iter()
                .map(|i| app.media[*i].id)
                .collect();
            (title, ids)
        })
        .collect();
    assert!(sections.len() >= 3, "{:?}", sections);
    let click = |app: &mut PhotoSortApp, title: &str, modifiers: egui::Modifiers| {
        let pos = center(&rendered(app, &ctx, size), title);
        // egui learns which keys are held from their own event.
        for pressed in [true, false] {
            frame(
                app,
                &ctx,
                size,
                vec![
                    egui::Event::ModifiersChanged(modifiers),
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers,
                    },
                ],
            );
        }
        frame(
            app,
            &ctx,
            size,
            vec![egui::Event::ModifiersChanged(Default::default())],
        );
        for _ in 0..30 {
            frame(app, &ctx, size, Vec::new());
        }
    };
    let selected = |app: &PhotoSortApp| app.selection.ordered(&app.media, &app.filtered);
    let union = |parts: &[usize]| -> Vec<MediaId> {
        let mut ids: Vec<MediaId> = parts
            .iter()
            .flat_map(|i| sections[*i].1.iter().copied())
            .collect();
        ids.sort_by_key(|id| board_position(&sections, *id));
        ids
    };
    let (a, b, c) = (&sections[0].0, &sections[1].0, &sections[2].0);
    // A plain click selects that folder alone.
    click(&mut app, a, Default::default());
    assert_eq!(selected(&app), union(&[0]));
    // Cmd (macOS) or Ctrl adds another folder, and removes it again.
    click(&mut app, b, egui::Modifiers::COMMAND);
    assert_eq!(selected(&app), union(&[0, 1]));
    assert_eq!(app.active, sections[1].1.first().copied());
    click(&mut app, a, egui::Modifiers::CTRL);
    assert_eq!(selected(&app), union(&[1]));
    assert!(app.active.is_some_and(|id| app.selection.contains(id)));
    click(&mut app, a, egui::Modifiers::COMMAND);
    assert_eq!(selected(&app), union(&[0, 1]));
    // A plain click still selects just that folder.
    click(&mut app, b, Default::default());
    assert_eq!(selected(&app), union(&[1]));
    // Shift selects from the last folder clicked through this one, in
    // either direction, keeping both folders whole.
    click(&mut app, a, Default::default());
    click(&mut app, c, egui::Modifiers::SHIFT);
    assert_eq!(selected(&app), union(&[0, 1, 2]));
    click(&mut app, c, Default::default());
    click(&mut app, b, egui::Modifiers::SHIFT);
    assert_eq!(selected(&app), union(&[1, 2]));
    // Shift after a photo click runs from that photo, as photos do.
    app.select_library(sections[2].1[1], false, false);
    click(&mut app, b, egui::Modifiers::SHIFT);
    let mut expected = union(&[1]);
    expected.extend(&sections[2].1[..2]);
    assert_eq!(selected(&app), expected);
    app.runtime.shutdown();
}

/// Where `id` sits on the board, by the sections it was read from.
fn board_position(sections: &[(String, Vec<MediaId>)], id: MediaId) -> usize {
    sections
        .iter()
        .flat_map(|(_, ids)| ids)
        .position(|other| *other == id)
        .unwrap()
}

#[test]
fn a_folder_previews_its_contents_and_counts() {
    use super::super::compact_cull_tests::rendered;
    let (_temp, mut app) = organize();
    let preview = app.folder_preview("A", false);
    assert_eq!((preview.photos, preview.videos, preview.folders), (4, 0, 1));
    assert_eq!(preview.counts(), "4 photos · 1 folder");
    // Nothing is loaded yet: placeholders, and nothing asked for.
    assert!(preview.thumbnails.iter().all(Option::is_none));
    let requested = app.requested.len();
    let _ = app.folder_preview("", false);
    assert_eq!(app.requested.len(), requested);
    let all = app.folder_preview("", false);
    // A and B; the empty folder on disk is not on this board.
    assert_eq!((all.photos, all.folders), (9, 2));
    assert_eq!(app.folder_preview("Empty", false).counts(), "Empty");
    // Once the board has shown them, their thumbnails are used.
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1500.0, 1500.0);
    let started = Instant::now();
    while app
        .folder_preview("B", false)
        .thumbnails
        .iter()
        .flatten()
        .count()
        < 5
    {
        assert!(started.elapsed().as_secs() < 10, "thumbnails");
        rendered(&mut app, &ctx, size);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_eq!(
        app.folder_preview("", false).thumbnails.len(),
        PREVIEW_THUMBNAILS
    );
    app.runtime.shutdown();
    // The library row counts every item, those not moving included, and
    // so do its preview and its Trash mark.
    let (_temp, mut app, unplaced) = grouped(GroupMode::Visual);
    assert_eq!(app.folder_preview("", false).photos, 9);
    app.board_folder_action(FolderAction::Trash(String::new()));
    assert!(unplaced.iter().all(|id| app.decision(*id).rejected));
    app.runtime.shutdown();
}

#[test]
fn hovering_a_folder_asks_for_its_first_thumbnails_only_once_it_rests() {
    let (_temp, mut app) = organize();
    let in_b: Vec<MediaId> = app
        .media
        .iter()
        .filter(|m| m.path.parent().is_some_and(|p| p.ends_with("B")))
        .map(|m| m.id)
        .collect();
    // Passing over a folder asks for nothing; it shows placeholders.
    let passing = app.folder_preview("B", false);
    assert_eq!(passing.thumbnails.len(), 5);
    assert!(passing.thumbnails.iter().all(Option::is_none));
    assert!(app.requested.is_empty());
    // Resting on it asks for its photos, a few at a time.
    app.folder_preview("B", true);
    assert!(!app.requested.is_empty());
    assert!(app.requested.len() <= THUMBNAILS_IN_FLIGHT);
    assert!(
        app.requested
            .iter()
            .all(|(id, preview)| !preview && in_b.contains(id))
    );
    // Once loaded they are shown, and kept for the next hover.
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let started = Instant::now();
    while app
        .folder_preview("B", true)
        .thumbnails
        .iter()
        .any(Option::is_none)
    {
        assert!(started.elapsed().as_secs() < 10, "thumbnails");
        let _ = ctx.run_test(Default::default(), |ui| {
            let ctx = ui.ctx().clone();
            app.poll(&ctx);
        });
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(
        app.folder_preview("B", false)
            .thumbnails
            .iter()
            .all(Option::is_some)
    );
    app.runtime.shutdown();
}

#[test]
fn person_faces_are_cut_sharp_from_the_original_a_few_at_a_time() {
    use super::super::compact_cull_tests::rendered;
    let (_temp, mut app, ids) = people_board();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1500.0, 1500.0);
    rendered(&mut app, &ctx, size);
    let bbox = [0.25, 0.25, 0.75, 0.75];
    for (person, id) in [("001", ids[0]), ("002", ids[4])] {
        app.board.people.portraits.insert(
            person.into(),
            vec![super::super::people::Portrait { id, bbox }],
        );
    }
    // The crops arrive from the decoder and are kept.
    let started = Instant::now();
    while app.board.people.crops.len() < 2 {
        assert!(started.elapsed().as_secs() < 10, "sharp crops");
        rendered(&mut app, &ctx, size);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(app.board.people.crops.keys().any(|(id, _)| *id == ids[0]));
    // A 16 px photo is never decoded larger than it is.
    assert!(
        app.board
            .people
            .crops
            .values()
            .all(|t| t.size()[0] <= 16 && t.size()[1] <= 16)
    );
    // At most a few crops are asked for per frame.
    for n in 0..9 {
        app.board
            .people
            .wanted
            .push(crate::core::inspection::Request {
                slot: n,
                media: app.media[0].clone(),
                region: crate::core::inspection::Region::full(16, 16),
                output_edge: 16,
                allow_large: false,
            });
    }
    app.inspection_wanted.clear();
    app.request_sharp_faces();
    assert_eq!(app.inspection_wanted.len(), 4);
    assert!(app.board.people.wanted.is_empty());
    app.runtime.shutdown();
}

#[test]
fn contact_sheet_tiles_fit_the_panel_until_it_has_to_scroll() {
    // Fourteen photos in a 600 x 850 panel: four columns of 144.
    let tile = sheet_tile(14, 600.0, 850.0, 8.0);
    assert_eq!(tile, (600.0 - 3.0 * 8.0) / 4.0);
    // A few photos stop at a comfortable size; many scroll at the smallest.
    assert_eq!(sheet_tile(10, 2000.0, 2000.0, 8.0), 160.0);
    assert_eq!(sheet_tile(400, 600.0, 800.0, 8.0), 72.0);
    // Never wider than a narrow panel.
    assert_eq!(sheet_tile(12, 60.0, 800.0, 8.0), 60.0);
}

#[test]
fn more_than_nine_selected_show_as_a_contact_sheet_in_the_preview() {
    use super::super::compact_cull_tests::{click_label, fixture_with, frame, rendered};
    let (_temp, mut app) = fixture_with(14);
    app.step = Step::Organize;
    app.output = app.root.as_ref().unwrap().display().to_string();
    app.filter_dirty = true;
    app.refilter();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1500.0, 1000.0);
    rendered(&mut app, &ctx, size);
    // A few selected compare side by side, with no sheet.
    app.select_library(ids[0], false, false);
    app.select_library(ids[3], false, true);
    let nodes = rendered(&mut app, &ctx, size);
    assert!(!app.board_sheet() && !shows(&nodes, "Show one"));
    assert_eq!(app.comparison_ids().len(), 4);
    // All fourteen: every one shows in the preview's sheet.
    app.select_library(ids[13], false, true);
    assert_eq!(app.selection.len(), 14);
    let nodes = rendered(&mut app, &ctx, size);
    assert!(app.board_sheet());
    assert!(shows(&nodes, "14 selected") && shows(&nodes, "Mark all for Trash"));
    // The sheet's tiles are the right-most of each name.
    let names: HashMap<MediaId, String> = app.media.iter().map(|m| (m.id, m.name())).collect();
    let sheet = |nodes: &[(String, egui::Rect)], id: MediaId| -> egui::Rect {
        *found(nodes, &names[&id])
            .into_iter()
            .max_by(|a, b| a.center().x.total_cmp(&b.center().x))
            .unwrap()
    };
    let panel = sheet(&nodes, ids[0]).left() - 1.0;
    for id in &ids {
        let tile = sheet(&nodes, *id);
        assert!(
            tile.left() >= panel && tile.bottom() <= size.y,
            "{id:?} {tile:?}"
        );
    }
    // A click makes a photo active and keeps the selection; D then marks
    // only that photo.
    click_at(&mut app, &ctx, size, sheet(&nodes, ids[7]).center());
    assert_eq!(app.active, Some(ids[7]));
    assert_eq!(app.selection.len(), 14);
    frame(&mut app, &ctx, size, key(egui::Key::D, Default::default()));
    let marked: Vec<_> = ids
        .iter()
        .filter(|id| app.decision(**id).rejected)
        .collect();
    assert_eq!(marked, [&ids[7]]);
    frame(&mut app, &ctx, size, key(egui::Key::D, Default::default()));
    assert!(ids.iter().all(|id| !app.decision(*id).rejected));
    // Escape lets go of it: D marks every selected photo, and again
    // unmarks them.
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Escape, Default::default()),
    );
    assert!(app.board_sheet());
    frame(&mut app, &ctx, size, key(egui::Key::D, Default::default()));
    assert!(ids.iter().all(|id| app.decision(*id).rejected));
    frame(&mut app, &ctx, size, key(egui::Key::D, Default::default()));
    assert!(ids.iter().all(|id| !app.decision(*id).rejected));
    // Escape shows the active photo large, still with all selected.
    frame(
        &mut app,
        &ctx,
        size,
        key(egui::Key::Escape, Default::default()),
    );
    let nodes = rendered(&mut app, &ctx, size);
    assert!(!app.board_sheet());
    assert_eq!((app.active, app.selection.len()), (Some(ids[7]), 14));
    assert!(shows(&nodes, "Show all 14"));
    click_label(&mut app, &ctx, size, "Show all 14");
    assert!(app.board_sheet());
    // Double-click shows that photo large. Frames advance egui's clock
    // by a sixtieth of a second: let the earlier clicks age, or two more
    // count as a triple click.
    for _ in 0..15 {
        rendered(&mut app, &ctx, size);
    }
    let nodes = rendered(&mut app, &ctx, size);
    let pos = sheet(&nodes, ids[2]).center();
    let press = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: Default::default(),
    };
    for _ in 0..2 {
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerMoved(pos), press(true), press(false)],
        );
    }
    rendered(&mut app, &ctx, size);
    assert!(!app.board_sheet());
    assert_eq!((app.active, app.selection.len()), (Some(ids[2]), 14));
    assert!(app.media.iter().all(|m| m.path.exists()));
    app.runtime.shutdown();
}

#[test]
fn show_original_waits_for_a_plan_that_differs_from_disk() {
    use super::super::compact_cull_tests::{frame, rendered};
    let (_temp, mut app) = organize();
    let ids: Vec<MediaId> = app.filtered.iter().map(|i| app.media[*i].id).collect();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1400.0, 900.0);
    let backslash = |pressed| {
        vec![egui::Event::Key {
            key: egui::Key::Backslash,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: Default::default(),
        }]
    };
    let hold = |app: &mut PhotoSortApp| {
        frame(app, &ctx, size, backslash(true));
        rendered(app, &ctx, size);
        let shown = app.board.original;
        frame(app, &ctx, size, backslash(false));
        rendered(app, &ctx, size);
        shown
    };
    let held_button = |app: &mut PhotoSortApp| {
        let nodes = rendered(app, &ctx, size);
        let pos = center(&nodes, "Show original");
        let press = |pressed| {
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
        frame(app, &ctx, size, press(true));
        rendered(app, &ctx, size);
        let shown = app.board.original;
        frame(app, &ctx, size, press(false));
        rendered(app, &ctx, size);
        shown
    };
    // Current folders, nothing planned: the plan is the disk.
    rendered(&mut app, &ctx, size);
    assert!(!app.organize_planned() && !app.organize_resettable());
    assert!(!hold(&mut app), "the key does nothing");
    assert!(!held_button(&mut app), "the button is disabled");
    // A move makes a difference, and both work.
    app.board_move(&ids[..1], "B");
    rendered(&mut app, &ctx, size);
    assert!(app.organize_planned() && app.organize_resettable());
    assert!(hold(&mut app));
    assert!(held_button(&mut app));
    // Back to nothing planned after a reset.
    app.reset_organize();
    rendered(&mut app, &ctx, size);
    assert!(!app.organize_planned() && !hold(&mut app));
    // A grouping's own proposals differ from disk before any edit.
    app.set_group_mode(GroupMode::Camera);
    rendered(&mut app, &ctx, size);
    assert!(app.organize_drafts.is_empty());
    assert!(app.organize_planned() && hold(&mut app));
    app.runtime.shutdown();
}
