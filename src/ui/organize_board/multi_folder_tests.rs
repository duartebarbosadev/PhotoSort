//! Sidebar clicks on the board: a plain click scrolls to a folder, Ctrl/Cmd
//! and Shift choose the folders shown, and moves, ranges and marks keep to
//! the folders shown.
use super::super::compact_cull_tests::{frame, rendered};
use super::*;

const SIZE: Vec2 = egui::vec2(1500.0, 1000.0);

/// The board with A/Day1 (4 photos), B (3) and C (2), in that order.
fn board() -> (tempfile::TempDir, PhotoSortApp, Vec<MediaId>) {
    let (temp, app, ids) = board_with(9, 2);
    let sections: Vec<_> = app
        .board
        .sections
        .iter()
        .map(|s| (s.path.clone().unwrap(), s.range.clone()))
        .collect();
    assert_eq!(
        sections,
        [
            ("A/Day1".to_owned(), 0..4),
            ("B".to_owned(), 4..7),
            ("C".to_owned(), 7..9)
        ]
    );
    (temp, app, ids)
}
/// `count` photos: four in A/Day1, then B's, then the last `in_c` in C.
fn board_with(count: usize, in_c: usize) -> (tempfile::TempDir, PhotoSortApp, Vec<MediaId>) {
    let (temp, mut app) = super::super::compact_cull_tests::fixture_with(count);
    app.step = Step::Organize;
    app.review = false;
    app.output = app.root.as_ref().unwrap().display().to_string();
    app.filter_dirty = true;
    app.refilter();
    let ids = board_ids(&app);
    app.board_new_folder("", &ids[count - in_c..]);
    app.board.editing.as_mut().unwrap().1 = "C".into();
    app.board_finish_edit();
    app.filter_dirty = true;
    app.refilter();
    let ids = board_ids(&app);
    (temp, app, ids)
}
fn board_ids(app: &PhotoSortApp) -> Vec<MediaId> {
    app.filtered.iter().map(|i| app.media[*i].id).collect()
}
fn context() -> egui::Context {
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    ctx
}
/// A click on the sidebar row or board item labelled `label`.
fn click(app: &mut PhotoSortApp, ctx: &egui::Context, label: &str, modifiers: egui::Modifiers) {
    let nodes = rendered(app, ctx, SIZE);
    let pos = nodes
        .iter()
        .find(|(l, _)| l == label)
        .unwrap_or_else(|| panic!("Missing {label}; labels {:?}", nodes))
        .1
        .center();
    // egui learns which keys are held from their own event.
    for pressed in [true, false] {
        frame(
            app,
            ctx,
            SIZE,
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
        ctx,
        SIZE,
        vec![egui::Event::ModifiersChanged(Default::default())],
    );
}
fn press(app: &mut PhotoSortApp, ctx: &egui::Context, key: egui::Key, modifiers: egui::Modifiers) {
    let event = egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    };
    frame(app, ctx, SIZE, vec![event]);
}
/// The folders on the board, by their headers.
fn shown(app: &mut PhotoSortApp, ctx: &egui::Context) -> Vec<String> {
    let mut shown: Vec<String> = rendered(app, ctx, SIZE)
        .into_iter()
        .filter_map(|(label, _)| label.strip_prefix("Folder ").map(str::to_owned))
        .collect();
    shown.sort_unstable();
    shown
}
fn sorted(focus: &[String]) -> Vec<&str> {
    let mut focus: Vec<&str> = focus.iter().map(String::as_str).collect();
    focus.sort_unstable();
    focus
}
fn highlighted(app: &PhotoSortApp) -> Vec<String> {
    let highlight = app.board_highlight();
    let rows = app.board.folders.iter().map(|f| f.path.clone());
    rows.filter(|path| app.board_highlighted(path, highlight.as_deref()))
        .collect()
}

#[test]
fn sidebar_clicks_choose_one_or_several_folders_to_show() {
    let (_temp, mut app, _) = board();
    let ctx = context();
    let none = egui::Modifiers::default();
    let (a, day1, b, c) = ("A  ·  4", "Day1  ·  4", "B  ·  3", "C  ·  2");
    // A plain click hides nothing: it highlights that folder.
    click(&mut app, &ctx, b, none);
    assert!(app.board.focus.is_empty());
    assert_eq!(highlighted(&app), ["B"]);
    // Cmd (macOS) or Ctrl shows only that folder, and adds another; the
    // board shows both together, the sidebar highlights both and the
    // header counts them.
    click(&mut app, &ctx, b, egui::Modifiers::COMMAND);
    assert_eq!(app.board.focus, ["B"]);
    assert_eq!(shown(&mut app, &ctx), ["B"]);
    click(&mut app, &ctx, c, egui::Modifiers::COMMAND);
    assert_eq!(app.board.focus, ["B", "C"]);
    assert_eq!(shown(&mut app, &ctx), ["B", "C"]);
    assert_eq!(highlighted(&app), ["B", "C"]);
    let nodes = rendered(&mut app, &ctx, SIZE);
    let root = app.board_root_name();
    assert!(
        nodes
            .iter()
            .any(|(label, _)| *label == format!("{root} / 2 folders")),
        "{nodes:?}"
    );
    assert!(
        nodes
            .iter()
            .any(|(label, _)| label == "2 folders · 5 items")
    );
    // A plain click on a folder shown keeps them; on one they leave out,
    // every folder shows again.
    click(&mut app, &ctx, c, none);
    assert_eq!(app.board.focus, ["B", "C"]);
    click(&mut app, &ctx, day1, none);
    assert!(app.board.focus.is_empty());
    // Cmd/Ctrl-click on a folder shown takes it away; the last one taken
    // away shows every folder again.
    click(&mut app, &ctx, b, egui::Modifiers::COMMAND);
    click(&mut app, &ctx, c, egui::Modifiers::COMMAND);
    click(&mut app, &ctx, b, egui::Modifiers::CTRL);
    assert_eq!(app.board.focus, ["C"]);
    assert_eq!(highlighted(&app), ["C"]);
    click(&mut app, &ctx, c, egui::Modifiers::COMMAND);
    assert!(app.board.focus.is_empty());
    assert_eq!(shown(&mut app, &ctx), ["A/Day1", "B", "C"]);
    // Shift shows every folder from the last one clicked through this one.
    click(&mut app, &ctx, a, none);
    click(&mut app, &ctx, c, egui::Modifiers::SHIFT);
    assert_eq!(sorted(&app.board.focus), ["A", "B", "C"]);
    assert!(
        rendered(&mut app, &ctx, SIZE)
            .iter()
            .any(|(label, _)| *label == format!("{root} / 3 folders"))
    );
    // And again from the same folder, the other way, replacing them.
    click(&mut app, &ctx, b, egui::Modifiers::SHIFT);
    assert_eq!(sorted(&app.board.focus), ["A", "B"]);
    // Cmd/Ctrl with Shift adds the range to the folders shown.
    let library = format!("{root}  ·  9");
    click(&mut app, &ctx, &library, none);
    click(&mut app, &ctx, day1, egui::Modifiers::COMMAND);
    click(&mut app, &ctx, c, egui::Modifiers::COMMAND);
    click(
        &mut app,
        &ctx,
        b,
        egui::Modifiers::COMMAND | egui::Modifiers::SHIFT,
    );
    assert_eq!(sorted(&app.board.focus), ["A/Day1", "B", "C"]);
    // A folder inside one shown is shown with it already.
    click(&mut app, &ctx, a, egui::Modifiers::COMMAND);
    assert_eq!(sorted(&app.board.focus), ["A", "B", "C"]);
    click(&mut app, &ctx, day1, egui::Modifiers::COMMAND);
    assert_eq!(sorted(&app.board.focus), ["A", "B", "C"]);
    // The library row shows all, with any key held, and so does the back
    // arrow.
    click(&mut app, &ctx, &library, egui::Modifiers::COMMAND);
    assert!(app.board.focus.is_empty());
    click(&mut app, &ctx, b, egui::Modifiers::COMMAND);
    click(&mut app, &ctx, c, egui::Modifiers::COMMAND);
    click(&mut app, &ctx, "Back to all folders", none);
    assert!(app.board.focus.is_empty());
    // The right-click menu shows only one folder, then every folder again.
    app.board_folder_action(FolderAction::ShowOnly(Some("C".into())));
    assert_eq!(app.board.focus, ["C"]);
    app.board_folder_action(FolderAction::ShowOnly(None));
    assert!(app.board.focus.is_empty());
    app.runtime.shutdown();
}

#[test]
fn several_folders_shown_keep_arrows_ranges_and_marks_to_their_photos() {
    let (_temp, mut app, ids) = board();
    let ctx = context();
    let none = egui::Modifiers::default();
    // A/Day1 and C, but not B between them.
    app.board_choose_folder("A/Day1", egui::Modifiers::COMMAND);
    app.board_choose_folder("C", egui::Modifiers::COMMAND);
    assert_eq!(app.board.focus, ["A/Day1", "C"]);
    rendered(&mut app, &ctx, SIZE);
    // The arrow keys step over B and stop at C's end.
    app.select_library(ids[3], false, false);
    press(&mut app, &ctx, egui::Key::ArrowRight, none);
    assert_eq!(app.active, Some(ids[7]));
    press(&mut app, &ctx, egui::Key::ArrowRight, none);
    press(&mut app, &ctx, egui::Key::ArrowRight, none);
    assert_eq!(app.active, Some(ids[8]));
    press(&mut app, &ctx, egui::Key::ArrowLeft, none);
    press(&mut app, &ctx, egui::Key::ArrowLeft, none);
    assert_eq!(app.active, Some(ids[3]));
    assert_eq!(app.board.focus, ["A/Day1", "C"]);
    let selected = |app: &PhotoSortApp| app.selection.ordered(&app.media, &app.filtered);
    // A Shift range of photos holds no photo of B.
    app.select_library(ids[1], false, false);
    app.select_library(ids[8], false, true);
    assert_eq!(selected(&app), [ids[1], ids[2], ids[3], ids[7], ids[8]]);
    // Nor does a Shift range of folder headers.
    app.board_select_section(0, none);
    app.board_select_section(2, egui::Modifiers::SHIFT);
    assert_eq!(selected(&app), [&ids[..4], &ids[7..]].concat());
    // Ctrl/Cmd+A selects what is shown, and D marks those for Trash.
    app.selection.clear();
    press(&mut app, &ctx, egui::Key::A, egui::Modifiers::COMMAND);
    assert_eq!(selected(&app), [&ids[..4], &ids[7..]].concat());
    press(&mut app, &ctx, egui::Key::D, none);
    let marked: Vec<MediaId> = ids
        .iter()
        .copied()
        .filter(|id| app.decision(*id).rejected)
        .collect();
    assert_eq!(marked, [&ids[..4], &ids[7..]].concat());
    app.runtime.shutdown();
}

#[test]
fn ctrl_cmd_a_selects_the_photos_of_one_folder_shown_or_every_item() {
    let (_temp, mut app, ids) = board();
    let ctx = context();
    let none = egui::Modifiers::default();
    let selected = |app: &PhotoSortApp| app.selection.ordered(&app.media, &app.filtered);
    // One folder shown: only its photos, even with a photo of another
    // folder active before.
    app.select_library(ids[8], false, false);
    app.board_choose_folder("B", egui::Modifiers::COMMAND);
    rendered(&mut app, &ctx, SIZE);
    press(&mut app, &ctx, egui::Key::A, egui::Modifiers::COMMAND);
    assert_eq!(selected(&app), &ids[4..7]);
    assert!(app.active.is_some_and(|id| ids[4..7].contains(&id)));
    // D marks those, not B's neighbours.
    press(&mut app, &ctx, egui::Key::D, none);
    let marked: Vec<MediaId> = ids
        .iter()
        .copied()
        .filter(|id| app.decision(*id).rejected)
        .collect();
    assert_eq!(marked, &ids[4..7]);
    // Every item shown: every photo.
    app.board_select_folder(None);
    rendered(&mut app, &ctx, SIZE);
    press(&mut app, &ctx, egui::Key::A, egui::Modifiers::COMMAND);
    assert_eq!(selected(&app), ids);
    app.runtime.shutdown();
}

#[test]
fn folders_shown_together_follow_renames_moves_and_searches() {
    let (_temp, mut app, ids) = board();
    let ctx = context();
    app.board_choose_folder("A/Day1", egui::Modifiers::COMMAND);
    app.board_choose_folder("C", egui::Modifiers::COMMAND);
    // Renamed, a folder shown stays shown.
    app.board_rename_folder("C");
    app.board.editing.as_mut().unwrap().1 = "Trip".into();
    app.board_finish_edit();
    assert_eq!(app.board.focus, ["A/Day1", "Trip"]);
    assert_eq!(shown(&mut app, &ctx), ["A/Day1", "Trip"]);
    // Moved inside B, it is still shown, B's own photos still are not.
    app.board_folder_action(FolderAction::Nest("Trip".into(), "B".into()));
    assert_eq!(app.board.focus, ["A/Day1", "B/Trip"]);
    assert_eq!(shown(&mut app, &ctx), ["A/Day1", "B/Trip"]);
    // Photos dragged into a folder shown appear with it.
    app.board_move(&[ids[4]], "B/Trip");
    assert_eq!(shown(&mut app, &ctx), ["A/Day1", "B/Trip"]);
    assert_eq!(
        app.board_shown_order().map(|order| order.len()),
        Some(7),
        "four in A/Day1 and three in B/Trip"
    );
    // A search narrows the photos shown, not the folders chosen.
    app.query = "photo-8".into();
    app.filter_dirty = true;
    app.refilter();
    rendered(&mut app, &ctx, SIZE);
    assert_eq!(app.board.focus, ["A/Day1", "B/Trip"]);
    assert_eq!(shown(&mut app, &ctx), ["B/Trip"]);
    app.query.clear();
    app.filter_dirty = true;
    app.refilter();
    // The original folders show every folder; the plan's come back after.
    app.show_original(true);
    assert!(app.board.focus.is_empty());
    app.show_original(false);
    assert_eq!(app.board.focus, ["A/Day1", "B/Trip"]);
    // A folder taken away from the plan stops being shown.
    app.board_folder_action(FolderAction::Remove("B/Trip".into()));
    app.filter_dirty = true;
    app.refilter();
    assert_eq!(app.board.focus, ["A/Day1"]);
    // New folders go inside the folder shown last clicked.
    app.board_choose_folder("B", egui::Modifiers::COMMAND);
    assert_eq!(app.board_focus_folder(), "B");
    app.runtime.shutdown();
}

#[test]
fn a_folder_taken_away_leaves_none_of_its_photos_selected() {
    let (_temp, mut app, ids) = board();
    let ctx = context();
    app.board_choose_folder("B", egui::Modifiers::COMMAND);
    app.board_choose_folder("C", egui::Modifiers::COMMAND);
    rendered(&mut app, &ctx, SIZE);
    app.select_library(ids[4], false, false);
    app.select_library(ids[7], true, false);
    assert_eq!(app.active, Some(ids[7]));
    app.board_choose_folder("B", egui::Modifiers::COMMAND);
    assert_eq!(app.board.focus, ["C"]);
    assert_eq!(app.selection.ordered(&app.media, &app.filtered), [ids[7]]);
    // So D marks only what is shown.
    press(&mut app, &ctx, egui::Key::D, Default::default());
    assert!(!app.decision(ids[4]).rejected);
    assert!(app.decision(ids[7]).rejected);
    app.runtime.shutdown();
}

#[test]
fn the_contact_sheet_filters_and_group_by_work_on_several_folders() {
    // A/Day1 (4), B (4) and C (6): A/Day1 and C make ten, a contact sheet.
    let (_temp, mut app, ids) = board_with(14, 6);
    let ctx = context();
    let size = SIZE;
    app.board_choose_folder("A/Day1", egui::Modifiers::COMMAND);
    app.board_choose_folder("C", egui::Modifiers::COMMAND);
    rendered(&mut app, &ctx, size);
    let shown_ids = [&ids[..4], &ids[8..]].concat();
    frame(
        &mut app,
        &ctx,
        size,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        }],
    );
    let nodes = rendered(&mut app, &ctx, size);
    assert!(app.board_sheet());
    assert_eq!(app.selection.ordered(&app.media, &app.filtered), shown_ids);
    assert!(nodes.iter().any(|(label, _)| label.contains("10 selected")));
    // Marking the sheet marks those ten and none of B's.
    press(&mut app, &ctx, egui::Key::D, Default::default());
    let marked: Vec<MediaId> = ids
        .iter()
        .copied()
        .filter(|id| app.decision(*id).rejected)
        .collect();
    assert_eq!(marked, shown_ids);
    // A filter narrows the photos shown and keeps the folders chosen.
    press(&mut app, &ctx, egui::Key::D, Default::default());
    app.selection.clear();
    app.reject(ids[0]);
    app.filter = Filter::Unmarked;
    app.filter_dirty = true;
    app.refilter();
    rendered(&mut app, &ctx, size);
    assert_eq!(app.board.focus, ["A/Day1", "C"]);
    let order = app.board_shown_order().unwrap();
    let order: Vec<MediaId> = order.iter().map(|i| app.media[*i].id).collect();
    assert_eq!(order, shown_ids[1..]);
    app.filter = Filter::All;
    // Another grouping has none of these folders: every folder shows.
    app.set_group_mode(GroupMode::Camera);
    rendered(&mut app, &ctx, size);
    assert!(app.board.focus.is_empty());
    app.set_group_mode(GroupMode::Current);
    rendered(&mut app, &ctx, size);
    assert!(app.board.focus.is_empty());
    assert_eq!(app.board.sections.len(), 3);
    app.runtime.shutdown();
}

#[test]
fn a_folder_click_scrolls_the_board_to_it_and_the_sidebar_follows_your_scrolling() {
    let (_temp, mut app, _) = board();
    // Large photos, so the folders run past the window.
    app.preferences.photo_size = *crate::core::preferences::PHOTO_SIZES.end();
    let ctx = context();
    set_reduce_motion(&ctx, false);
    let none = egui::Modifiers::default();
    let top = |app: &mut PhotoSortApp, folder: &str| {
        let label = format!("Folder {folder}");
        rendered(app, &ctx, SIZE)
            .into_iter()
            .find(|(l, _)| *l == label)
            .map(|(_, rect)| rect.top())
    };
    let board_top = top(&mut app, "A/Day1").unwrap();
    // A click eases the board down until B's header is at the top, hiding
    // nothing; the sidebar keeps B highlighted on the way.
    click(&mut app, &ctx, "B  ·  3", none);
    assert!(app.board.focus.is_empty());
    let gliding = app.library_offset;
    assert!(gliding > 0.0 && app.board.glide.is_some(), "{gliding}");
    for _ in 0..60 {
        rendered(&mut app, &ctx, SIZE);
        assert_eq!(highlighted(&app), ["B"]);
    }
    assert!(app.board.glide.is_none());
    let landed = top(&mut app, "B").expect("B's header on screen");
    assert!((landed - board_top).abs() < 1.0, "{landed} vs {board_top}");
    // An empty folder has nothing to scroll to: the board stays put while
    // the sidebar highlights it.
    app.board_new_folder("A/Day1", &[]);
    app.board.editing.as_mut().unwrap().1 = "Empty".into();
    app.board_finish_edit();
    rendered(&mut app, &ctx, SIZE);
    let before = app.library_offset;
    click(&mut app, &ctx, "Empty  ·  0", none);
    for _ in 0..30 {
        rendered(&mut app, &ctx, SIZE);
    }
    assert_eq!(app.library_offset, before);
    assert_eq!(highlighted(&app), ["A/Day1/Empty"]);
    assert_eq!(top(&mut app, "B"), Some(landed));
    // Scrolling by hand: the sidebar follows the folder at the top.
    let wheel = |app: &mut PhotoSortApp, dy: f32| {
        let over = egui::pos2(SIZE.x / 2.0, SIZE.y / 2.0);
        for _ in 0..6 {
            frame(
                app,
                &ctx,
                SIZE,
                vec![
                    egui::Event::PointerMoved(over),
                    egui::Event::MouseWheel {
                        phase: egui::TouchPhase::Move,
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, dy),
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        for _ in 0..30 {
            rendered(app, &ctx, SIZE);
        }
    };
    wheel(&mut app, 400.0);
    assert_eq!(app.library_offset, 0.0);
    assert_eq!(highlighted(&app), ["A/Day1"]);
    // Scrolling during a glide stops it where you left it.
    click(&mut app, &ctx, "C  ·  2", none);
    wheel(&mut app, 400.0);
    assert!(app.board.glide.is_none());
    assert_eq!(app.library_offset, 0.0);
    // Reduce motion jumps straight there.
    set_reduce_motion(&ctx, true);
    click(&mut app, &ctx, "B  ·  3", none);
    assert!(app.board.glide.is_none());
    assert_eq!(top(&mut app, "B"), Some(landed));
    app.runtime.shutdown();
}
