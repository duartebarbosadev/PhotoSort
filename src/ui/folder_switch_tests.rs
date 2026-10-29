//! Opening a different library starts every step with a clean view; the same
//! library keeps its view, and every library keeps its own staged changes.
use super::*;
use crate::core::{
    browsing::{RatingFilter, Sort},
    delete_review::Category,
    preferences::{Faceless, StripPosition},
    views::{Bookmark, Saved},
};
use crate::test_support::worker_patience;

fn library(dir: &tempfile::TempDir, name: &str) -> PathBuf {
    let root = dir.path().join(name);
    std::fs::create_dir(&root).unwrap();
    for photo in ["a.png", "b.png"] {
        image::RgbaImage::new(2, 2).save(root.join(photo)).unwrap();
    }
    root.canonicalize().unwrap()
}
fn app(dir: &tempfile::TempDir) -> PhotoSortApp {
    PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None)
}
fn settle(app: &mut PhotoSortApp) {
    let until = Instant::now() + worker_patience();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    loop {
        let expired = Instant::now() >= until;
        app.poll(&ctx);
        if !app.scanning && !app.busy {
            app.refilter();
            app.restore_bookmark();
            if app.pending_bookmark.is_none() && !app.filter_dirty && app.browse_run.is_none() {
                break;
            }
        }
        assert!(!expired, "folder work did not finish: {:?}", app.error);
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
}
fn rating(stars: u8) -> Decision {
    Decision {
        rating: Some(stars),
        ..Default::default()
    }
}
/// Everything a user can change about how a step looks, away from its default.
fn change_every_view(app: &mut PhotoSortApp) {
    let [first, second] = [app.media[0].id, app.media[1].id];
    app.active = Some(second);
    app.selection.restore([first, second], Some(first));
    app.comparison = vec![first, second];
    app.single_view_stash = vec![first];
    app.review = app.step == Step::Organize;
    app.inspector = false;
    app.library_layout = crate::core::views::Layout::List;
    app.query = "b".into();
    app.filter = Filter::Rated;
    app.browse.rating = RatingFilter::AtLeast(2);
    app.browse.sort = Sort::Newest;
    app.browse.folder.path = Some(PathBuf::from("/Trip"));
    app.browse.folder.also = vec![PathBuf::from("/Home")];
    app.browse_filters_open = true;
    app.group_mode = GroupMode::Visual;
    app.delete_current = Some(first);
    app.delete_categories = [Category::Blur].into_iter().collect();
    app.delete_panel = true;
    app.rotation_queue_only = true;
    app.pick_current = Some(first);
    app.pick_panel = true;
    app.inspection_view.zoom = 3.0;
    app.independent_views
        .insert(1, crate::core::inspection::View::default());
    app.sync_views = false;
    app.focused_slot = 1;
    app.library_offset = 120.0;
    app.board.focus = vec!["Trip".into(), "Home".into()];
    app.board.folder_anchor = Some("Home".into());
    app.board.original = true;
    app.board.focus_before = Some(Vec::new());
    app.board.single = Some(2);
    app.board.searching = true;
    app.board.renaming = Some((first, "x".into()));
    app.board.confirm_reset = true;
    app.board.people.merged.insert("003".into(), "001".into());
}
/// How a step looks the first time a library opens in it: the first photo
/// focused, and at most that photo selected (some steps select the focused
/// photo). `review` follows the step when the step changes, so it is checked
/// where the folder opens.
fn assert_clean(app: &PhotoSortApp) {
    let first = app.media.first().map(|m| m.id);
    let (selected, anchor) = app.selection.snapshot();
    assert!(
        selected.iter().all(|id| Some(*id) == first) && anchor.is_none_or(|id| Some(id) == first)
    );
    assert_eq!(
        app.capture_bookmark(),
        Bookmark {
            active: first,
            selected,
            anchor,
            review: app.review,
            ..Default::default()
        },
        "{:?}",
        app.step
    );
    assert!(app.single_view_stash.is_empty() && app.focused_slot == 0);
    assert!(!app.browse_filters_open && !app.delete_panel);
    let board = &app.board;
    assert!(board.focus.is_empty() && !board.original && board.focus_before.is_none());
    assert!(board.folder_anchor.is_none());
    assert!(board.single.is_none() && !board.searching && board.renaming.is_none());
    assert!(!board.confirm_reset && board.people.merged.is_empty());
}

#[test]
fn another_folder_opens_with_clean_views_in_every_step_and_keeps_app_wide_choices() {
    let dir = tempfile::tempdir().unwrap();
    let a = library(&dir, "a");
    let b = library(&dir, "b");
    // B was opened before: it staged a rating and saved its own views.
    let b_catalog = Catalog::scan(&b, || false, |_| {}).unwrap();
    let b_photo = b_catalog.media[0].id;
    let mut session = Session::default();
    session.decisions.insert(b_photo, rating(3));
    session.fingerprints = b_catalog
        .media
        .iter()
        .map(|m| (m.id, m.fingerprint))
        .collect();
    session.save(&b).unwrap();
    let mut views = Saved {
        step: Some(Step::Organize),
        ..Default::default()
    };
    views.bookmarks.insert(
        Step::Organize,
        Bookmark {
            query: "a".into(),
            group_mode: GroupMode::Date,
            ..Default::default()
        },
    );
    views.save(&b).unwrap();

    let mut app = app(&dir);
    app.open_now(a.clone());
    settle(&mut app);
    let a_photo = app.media[0].id;
    app.decisions.insert(a_photo, rating(5));
    app.preferences.strip_position = StripPosition::Left;
    app.preferences.show_filenames = true;
    app.preferences.faceless = Faceless::Folder;
    app.preferences.easy_delete_auto = false;
    app.board.tile = Some(222.0);
    app.delete_slot = Some(333.0);
    app.delete_panel_width = Some(444.0);
    for step in Step::ALL {
        app.step = step;
        change_every_view(&mut app);
    }
    app.step = Step::Cull;

    app.open_now(b.clone());
    // Clean while B is still being discovered, not only once it opens.
    assert_eq!(app.group_mode, GroupMode::Current);
    assert!(app.query.is_empty() && app.board.focus.is_empty());
    settle(&mut app);

    assert_eq!(app.root.as_ref(), Some(&b));
    assert_eq!(PathBuf::from(&app.output), b);
    // The step stays where the user was.
    assert_eq!(app.step, Step::Cull);
    assert!(app.review);
    assert!(app.saved_views.bookmarks.is_empty());
    for step in Step::ALL {
        app.step = step;
        app.prepare_bookmark();
        assert_clean(&app);
    }
    assert!(app.forget_scroll);
    // Staged changes stay with their own folder.
    assert!(!app.decisions.contains_key(&a_photo));
    assert_eq!(app.decisions[&b_photo].rating, Some(3));
    assert!(app.organize_drafts.is_empty());
    // App-wide choices are not about a folder.
    assert_eq!(app.preferences.strip_position, StripPosition::Left);
    assert!(app.preferences.show_filenames);
    assert_eq!(app.preferences.faceless, Faceless::Folder);
    assert!(!app.preferences.easy_delete_auto);
    assert_eq!(app.board.tile, Some(222.0));
    assert_eq!(app.delete_slot, Some(333.0));
    assert_eq!(app.delete_panel_width, Some(444.0));
}

#[test]
fn the_same_folder_keeps_its_view_and_returning_to_it_restores_only_its_staged_changes() {
    let dir = tempfile::tempdir().unwrap();
    let a = library(&dir, "a");
    let b = library(&dir, "b");
    let mut app = app(&dir);
    app.open_now(a.clone());
    settle(&mut app);
    let a_photo = app.media[1].id;
    let viewed = |app: &mut PhotoSortApp| {
        app.group_mode = GroupMode::Date;
        app.query = "b".into();
        app.browse.sort = Sort::Newest;
        app.filter_dirty = true;
        settle(app);
    };
    viewed(&mut app);
    app.decisions.insert(a_photo, rating(4));

    // Opening it again, e.g. from Recent folders.
    app.open_now(a.clone());
    assert_eq!(app.group_mode, GroupMode::Date);
    settle(&mut app);
    assert_eq!(app.group_mode, GroupMode::Date);
    assert_eq!(app.query, "b");
    assert_eq!(app.browse.sort, Sort::Newest);
    assert_eq!(app.decisions[&a_photo].rating, Some(4));

    // The reload after Apply.
    app.checkpoint_bookmarks();
    app.save_session();
    app.scan_folder(a.clone());
    settle(&mut app);
    assert_eq!(app.group_mode, GroupMode::Date);
    assert_eq!(app.query, "b");
    assert_eq!(app.decisions[&a_photo].rating, Some(4));

    // Away and back: A's staged rating returns, its old view does not.
    app.open_now(b.clone());
    settle(&mut app);
    assert!(app.decisions.is_empty());
    app.open_now(a.clone());
    settle(&mut app);
    assert_eq!(app.root.as_ref(), Some(&a));
    assert_eq!(app.decisions[&a_photo].rating, Some(4));
    assert_clean(&app);

    // A relaunch opening A restores the view A saved.
    viewed(&mut app);
    app.checkpoint_bookmarks();
    app.save_session();
    app.runtime.shutdown();
    drop(app);
    let mut relaunched = self::app(&dir);
    relaunched.open_now(a.clone());
    settle(&mut relaunched);
    assert_eq!(relaunched.group_mode, GroupMode::Date);
    assert_eq!(relaunched.query, "b");
    assert_eq!(relaunched.decisions[&a_photo].rating, Some(4));
}

#[test]
fn a_folder_chosen_while_changes_were_applied_still_opens_clean() {
    let dir = tempfile::tempdir().unwrap();
    let a = library(&dir, "a");
    let b = library(&dir, "b");
    let mut app = app(&dir);
    app.open_now(a.clone());
    settle(&mut app);
    app.group_mode = GroupMode::Date;
    app.query = "b".into();
    app.filter_dirty = true;
    settle(&mut app);
    app.checkpoint_bookmarks();
    // Applying reloads A first, then continues to the folder that was chosen.
    app.after_apply = Some(Destination::Folder(b.clone()));
    app.scan_folder(a.clone());
    settle(&mut app);
    assert_eq!(app.root.as_ref(), Some(&b));
    assert_clean(&app);
}
