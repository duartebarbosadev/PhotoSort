use super::*;
use crate::core::operations::{Action, Source};
use crate::test_support::worker_patience;
use std::sync::{
    Condvar, Mutex,
    atomic::{AtomicBool, Ordering},
    mpsc,
};

type Gate = Arc<(Mutex<bool>, Condvar)>;
struct Release(Gate);
impl Release {
    fn release(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.release();
    }
}

fn wait(gate: &Gate) {
    let (lock, wake) = &**gate;
    let mut released = lock.lock().unwrap();
    while !*released {
        released = wake.wait(released).unwrap();
    }
}

/// Two photos in a folder of their own, scanned.
fn library() -> (tempfile::TempDir, PathBuf, Arc<Catalog>) {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap().join("photos");
    std::fs::create_dir(&root).unwrap();
    for name in ["a.png", "b.png"] {
        image::RgbaImage::new(2, 2).save(root.join(name)).unwrap();
    }
    let catalog = Arc::new(Catalog::scan(&root, || false, |_| {}).unwrap());
    (temp, root, catalog)
}

fn fixture(step: Step) -> (tempfile::TempDir, PhotoSortApp, mpsc::Receiver<()>, Gate) {
    let (temp, root, catalog) = library();
    let gate = Arc::new((Mutex::new(false), Condvar::new()));
    let worker_gate = gate.clone();
    let once = AtomicBool::new(false);
    let (notice, ready) = mpsc::sync_channel(1);
    let runtime = Runtime::with_cache(
        move || {
            // Block at the first published file progress event, after one committed
            // action. This makes Stop deterministic without timing races or sleeps.
            if std::thread::current().name() == Some("photosort-files")
                && !once.swap(true, Ordering::AcqRel)
            {
                let _ = notice.send(());
                wait(&worker_gate);
            }
        },
        temp.path().join("cache"),
    );
    let app = staged(runtime, &root, catalog, step);
    (temp, app, ready, gate)
}

/// The library open on `step`, both photos rated and a move of each listed.
fn staged(
    runtime: Runtime,
    root: &std::path::Path,
    catalog: Arc<Catalog>,
    step: Step,
) -> PhotoSortApp {
    let mut app = PhotoSortApp::with_runtime(runtime, None);
    app.preferences.updates.automatic = false;
    app.root = Some(root.to_path_buf());
    app.output = root.display().to_string();
    app.step = step;
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.active = Some(app.media[0].id);
    app.catalog = Some(catalog);
    let plan = Plan {
        actions: app
            .media
            .iter()
            .map(|m| Action::Move {
                files: vec![(
                    Source::capture(m.path.clone()).unwrap(),
                    root.join("Sorted").join(m.name()),
                )],
            })
            .collect(),
        output_root: Some(root.to_path_buf()),
        ..Default::default()
    };
    for m in &app.media {
        app.decisions.insert(
            m.id,
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        );
    }
    app.set_review_plan(Some(plan));
    app
}
fn settle(app: &mut PhotoSortApp) {
    let until = Instant::now() + worker_patience();
    while app.busy || app.scanning {
        app.poll(&egui::Context::default());
        assert!(Instant::now() < until, "file work stalled: {:?}", app.error);
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
}
fn frame(
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
        |ui| app.footer(ui),
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

#[test]
fn every_workflow_can_stop_from_compact_footer_and_retry_without_losing_intent() {
    for step in Step::ALL {
        let (_temp, mut app, ready, gate) = fixture(step);
        let release = Release(gate); // Drop before app, including during panic.
        let root = app.root.clone().unwrap();
        app.apply_reviewed_plan();
        ready.recv_timeout(worker_patience()).unwrap();
        assert!(app.busy && app.applying_plan.is_some());
        let token = app.apply_stop.as_ref().unwrap().clone();
        let epoch = app.runtime.epoch();
        let ctx = egui::Context::default();
        theme_install(&ctx);
        ctx.enable_accesskit();
        theme_install(&ctx);
        app.status = "A deliberately long operation status that must never push Stop outside a small window ".repeat(8);
        for size in [
            egui::vec2(600., 500.),
            egui::vec2(640., 600.),
            egui::vec2(1440., 900.),
        ] {
            for _ in 0..2 {
                frame(&mut app, &ctx, size, Vec::new());
            }
            let nodes = frame(&mut app, &ctx, size, Vec::new());
            let button = nodes.iter().find(|(s, _)| s == "Stop").unwrap().1;
            assert!(button.left() >= 0. && button.right() <= size.x && button.bottom() <= size.y);
        }
        let size = egui::vec2(640., 600.);
        frame(&mut app, &ctx, size, Vec::new());
        let nodes = frame(&mut app, &ctx, size, Vec::new());
        let pos = nodes.iter().find(|(s, _)| s == "Stop").unwrap().1.center();
        frame(
            &mut app,
            &ctx,
            size,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
        );
        frame(
            &mut app,
            &ctx,
            size,
            vec![egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        assert!(token.load(Ordering::Acquire));
        assert_eq!(
            epoch,
            app.runtime.epoch(),
            "Stop must not discard the completion event"
        );
        assert!(app.busy && app.applying_plan.is_some());
        assert!(
            frame(&mut app, &ctx, size, Vec::new())
                .iter()
                .any(|(s, _)| s == "Stopping…")
        );
        release.release();
        settle(&mut app);
        assert!(app.apply_stop.is_none() && app.applying_plan.is_none());
        assert_eq!(app.retry_plan.as_ref().unwrap().actions.len(), 1);
        assert!(app.recovery_ticket.is_some());
        assert!(root.join("Sorted/a.png").exists() && root.join("b.png").exists());
        assert!(!root.join("a.png").exists() && !root.join("Sorted/b.png").exists());
        assert_eq!(
            app.decisions[&MediaId::for_path(&root.join("Sorted/a.png"))].rating,
            Some(4)
        );
        assert_eq!(
            app.decisions[&MediaId::for_path(&root.join("b.png"))].rating,
            Some(4)
        );
        assert_eq!(app.stats.decodes, 0);
        app.set_review_plan(app.retry_plan.clone());
        app.apply_reviewed_plan();
        assert!(
            !app.apply_stop.as_ref().unwrap().load(Ordering::Acquire),
            "Retry owns a fresh Stop token"
        );
        settle(&mut app);
        assert!(app.retry_plan.is_none() && app.apply_stop.is_none());
        assert!(root.join("Sorted/b.png").exists() && !root.join("b.png").exists());
        assert_eq!(
            app.decisions[&MediaId::for_path(&root.join("Sorted/b.png"))].rating,
            Some(4)
        );
        assert_eq!(app.stats.decodes, 0);
    }
}

#[test]
fn stop_control_clears_on_worker_failure_and_ignores_unrelated_busy_work() {
    let (_temp, mut app, _ready, gate) = fixture(Step::Organize);
    let _release = Release(gate);
    let token = Arc::new(AtomicBool::new(false));
    app.apply_stop = Some(token.clone());
    app.busy = true;
    app.stop_file_operations();
    assert!(
        !token.load(Ordering::Acquire),
        "No accepted file plan is active"
    );
    app.applying_plan = app.plan.take();
    let (tx, rx) = mpsc::sync_channel(1);
    app.runtime.events = rx;
    tx.send(crate::core::jobs::Envelope {
        epoch: app.runtime.epoch(),
        event: Event::Error("journal unavailable".into()),
    })
    .unwrap();
    app.poll(&egui::Context::default());
    assert!(app.apply_stop.is_none() && app.retry_plan.is_some() && !app.busy);
    app.stop_file_operations();
    assert!(!token.load(Ordering::Acquire));
}

#[test]
fn review_apply_never_waits_for_the_image_worker_and_closing_cancels_its_wait() {
    let (temp, root, catalog) = library();
    let (files, images): (Gate, Gate) = Default::default();
    let (files_gate, images_gate) = (files.clone(), images.clone());
    let (files_notice, files_held) = mpsc::sync_channel(1);
    let (images_notice, images_held) = mpsc::sync_channel(1);
    let once = AtomicBool::new(false);
    let runtime = Runtime::with_cache(
        move || match std::thread::current().name() {
            // Held at whatever it publishes, as it is while a model compiles.
            Some("photosort-images") => {
                let _ = images_notice.try_send(());
                wait(&images_gate);
            }
            // Held once, at the save that goes before the plan.
            Some("photosort-files") if !once.swap(true, Ordering::AcqRel) => {
                let _ = files_notice.send(());
                wait(&files_gate);
            }
            _ => {}
        },
        temp.path().join("cache"),
    );
    let mut app = staged(runtime, &root, catalog, Step::Review);
    let (release_files, _release_images) = (Release(files), Release(images.clone()));
    // Only the two ratings are listed, no moves.
    app.list_review(Plan::default());
    assert!(!app.busy && !app.plan_rows.is_empty());
    let photo = app.media[0].clone();
    app.runtime.image(ImageWork::Asset(photo, false)).unwrap();
    images_held.recv_timeout(worker_patience()).unwrap();

    app.apply_review();
    files_held.recv_timeout(worker_patience()).unwrap();
    assert!(app.busy && app.apply_planned);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(640., 600.);
    frame(&mut app, &ctx, size, Vec::new());
    let shown = |app: &mut PhotoSortApp, label: &str| {
        frame(app, &ctx, size, Vec::new())
            .iter()
            .any(|(s, _)| s == label)
    };
    assert!(shown(&mut app, "Cancel planning"));
    // Closing the window cancels the wait instead of being ignored.
    app.request_departure(Destination::Close);
    assert!(shown(&mut app, "Cancelling…"));
    release_files.release();
    settle(&mut app);
    // Nothing was applied or lost: the list is as it was, and closing asks
    // about the staged choices as it always does.
    assert!(matches!(app.departure, Some(Destination::Close)));
    assert!(!app.close_authorized && !app.review_applying() && app.plan_stop.is_none());
    assert_eq!(app.staged_choice_count(), 2);
    assert!(app.plan.is_some() && !app.plan_rows.is_empty());

    // Cancel in that dialog, then Apply again: it finishes while the image
    // worker is still held.
    app.departure = None;
    app.apply_review();
    settle(&mut app);
    assert!(!*images.0.lock().unwrap(), "the image worker never let go");
    assert_eq!(app.staged_choice_count(), 0);
    assert!(app.review_outcome.as_ref().is_some_and(|o| o.done));
}

#[test]
fn saving_staged_choices_shares_the_catalogs_identities_and_the_drafts_labels() {
    let (_temp, mut app, _ready, gate) = fixture(Step::Organize);
    let _release = Release(gate);
    let catalog = app.catalog.clone().unwrap();
    let mode = app.group_mode;
    let draft = crate::core::organize::Draft::from_catalog(&catalog, app.organize_labels());
    app.organize_drafts.insert(mode, draft);
    let labels = |session: &Session| {
        std::ptr::from_ref(session.organize.as_ref().unwrap().drafts[&mode].labels())
    };
    let (first, second) = (app.session_snapshot(), app.session_snapshot());
    assert_eq!(first.fingerprints.len(), app.media.len());
    // One map of fingerprints and one of labels, however often it is saved.
    assert!(std::ptr::eq(&*first.fingerprints, &*second.fingerprints));
    assert_eq!(labels(&first), labels(&second));
    assert_eq!(
        labels(&first),
        std::ptr::from_ref(app.organize_drafts[&mode].labels())
    );
    // An edit made while a save still holds the labels leaves that save as
    // it was.
    let moved = app.media[0].id;
    let before = first.organize.as_ref().unwrap().drafts[&mode].labels()[&moved].clone();
    let draft = app.organize_drafts.get_mut(&mode).unwrap();
    draft.create_folder("", "Elsewhere").unwrap();
    draft.move_photos(&[moved], "Elsewhere").unwrap();
    assert_eq!(app.organize_drafts[&mode].labels()[&moved], "Elsewhere");
    assert_eq!(
        first.organize.as_ref().unwrap().drafts[&mode].labels()[&moved],
        before
    );
    // The same folder read again is another catalog: its files are read anew.
    let root = catalog.root.clone();
    app.catalog = Some(Arc::new(Catalog::scan(&root, || false, |_| {}).unwrap()));
    let reread = app.session_snapshot();
    assert!(!std::ptr::eq(&*first.fingerprints, &*reread.fingerprints));
    assert_eq!(*first.fingerprints, *reread.fingerprints);
}

/// The accessible texts of what `draw` shows, after a few frames, with
/// `events` sent to the last one.
fn texts(
    app: &mut PhotoSortApp,
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    draw: fn(&mut PhotoSortApp, &mut egui::Ui),
) -> Vec<(String, egui::Rect)> {
    let size = egui::vec2(1280., 800.);
    let mut run = |events| {
        ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                events,
                ..Default::default()
            },
            |ui| draw(app, ui),
        )
    };
    run(Vec::new());
    run(Vec::new())
        .platform_output
        .accesskit_update
        .map(|_| ())
        .unwrap();
    let out = run(events);
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
fn page(app: &mut PhotoSortApp, ctx: &egui::Context) -> Vec<String> {
    texts(app, ctx, Vec::new(), |app, ui| app.review_page(ui))
        .into_iter()
        .map(|(text, _)| text)
        .collect()
}
/// Opens what is left as the "did not finish" card's button does, and
/// clicks Retry in the dialog.
fn retry_from_the_dialog(app: &mut PhotoSortApp, ctx: &egui::Context) {
    let dialog = |app: &mut PhotoSortApp, ui: &mut egui::Ui| {
        let ctx = ui.ctx().clone();
        app.plan_review_dialog(&ctx)
    };
    assert!(app.review_remaining_operations() && app.plan_open);
    let shown = texts(app, ctx, Vec::new(), dialog);
    let pos = shown
        .iter()
        .find(|(text, _)| text == "Retry unfinished changes")
        .unwrap_or_else(|| panic!("{shown:?}"))
        .1
        .center();
    for pressed in [true, false] {
        texts(
            app,
            ctx,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                },
            ],
            dialog,
        );
    }
}

/// Two moves were left unfinished. Retry runs them and is stopped after the
/// first; a second Retry finishes. While Retry works the page says so
/// instead of "did not finish"; what is left is one number in the status
/// bar, on the card and in the list; and a Retry that finishes ends with
/// what it did, not with "Nothing to apply yet".
#[test]
fn retry_shows_it_is_running_counts_what_is_left_once_and_ends_with_a_result() {
    let (_temp, mut app, ready, gate) = fixture(Step::Review);
    let release = Release(gate);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    // As found when the folder is opened after an Apply that was cut short.
    app.retry_plan = app.plan.clone();
    let left = |app: &mut PhotoSortApp| {
        let unfinished = app.unfinished().unwrap();
        (unfinished.status(), unfinished.sentence())
    };
    assert_eq!(
        left(&mut app),
        (
            "2 unfinished changes · Review changes to resolve".to_owned(),
            "2 changes did not finish. Retry the rest, choose another destination or discard them."
                .to_owned()
        )
    );
    // The Review tab in the footer says that number too.
    assert_eq!(app.staged_change_count(), 2);
    let shown = page(&mut app, &ctx);
    assert!(
        shown.iter().any(|t| t == "Some changes did not finish"),
        "{shown:?}"
    );
    retry_from_the_dialog(&mut app, &ctx);
    ready.recv_timeout(worker_patience()).unwrap();
    assert!(app.busy && app.retry_plan.is_some());
    let shown = page(&mut app, &ctx);
    assert!(
        shown.iter().any(|t| t == "Retrying the unfinished changes")
            && !shown.iter().any(|t| t == "Some changes did not finish"),
        "{shown:?}"
    );
    app.stop_file_operations();
    release.release();
    settle(&mut app);
    // One photo moved and one is left: the same count everywhere.
    assert_eq!(app.retry_plan.as_ref().unwrap().actions.len(), 1);
    let (status, sentence) = left(&mut app);
    assert_eq!(status, "1 unfinished change · Review changes to resolve");
    assert_eq!(app.status, status);
    assert_eq!(app.staged_change_count(), 1);
    assert_eq!(
        sentence,
        "1 change finished and 1 did not. Retry the rest, choose another destination or discard them."
    );
    let shown = page(&mut app, &ctx);
    assert!(shown.contains(&sentence), "{shown:?}");
    retry_from_the_dialog(&mut app, &ctx);
    assert_eq!(
        app.plan_rows.total(),
        1,
        "the list counts what the status counted"
    );
    settle(&mut app);
    assert!(app.retry_plan.is_none());
    assert_eq!(
        app.review_outcome.as_ref().map(|o| (o.sentence(), o.done)),
        Some(("Moved 1 photo.".to_owned(), true))
    );
    app.runtime.shutdown();
}
