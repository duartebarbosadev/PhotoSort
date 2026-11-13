use super::*;
use crate::core::operations::Source;

fn source(name: &str) -> Source {
    Source {
        path: Path::new("/library").join(name),
        fingerprint: Fingerprint::default(),
    }
}
fn plan() -> Plan {
    Plan {
        actions: vec![
            Action::Move {
                files: ["ARW", "JPG", "xmp"]
                    .into_iter()
                    .map(|ext| {
                        (
                            source(&format!("sample.{ext}")),
                            Path::new("/library").join(format!("keepers.{ext}")),
                        )
                    })
                    .collect(),
            },
            Action::Move {
                files: vec![(source("notes.txt"), "/library/Trip notes.txt".into())],
            },
            Action::Trash {
                files: vec![source("rejected.jpg"), source("rejected.xmp")],
            },
            Action::CreateDirectory {
                path: "/library/Empty".into(),
            },
        ],
        output_root: Some("/library".into()),
        ..Default::default()
    }
}

/// Waits for the details of a list just made: the file worker's answers.
fn listed(app: &mut PhotoSortApp) {
    let until = std::time::Instant::now() + crate::test_support::worker_patience();
    while app.busy {
        assert!(std::time::Instant::now() < until, "worker did not finish");
        app.poll(&egui::Context::default());
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The accessible texts of the review dialog after a few frames at `size`.
fn dialog_texts(app: &mut PhotoSortApp, ctx: &egui::Context, size: egui::Vec2) -> Vec<String> {
    let mut output = None;
    for _ in 0..3 {
        output = Some(ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.plan_review_dialog(ctx)
            },
        ));
    }
    let tree = output.unwrap().platform_output.accesskit_update.unwrap();
    tree.nodes
        .iter()
        .filter_map(|(_, n)| n.value().or(n.label()).map(str::to_string))
        .collect()
}

#[test]
fn review_rows_cover_every_companion_and_refresh_on_plan_replacement() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.set_review_plan(Some(plan()));
    assert_eq!(app.plan_rows.len(), 7);
    assert_eq!(
        (0..7).map(|i| app.plan_rows.at(i)).collect::<Vec<_>>(),
        [(0, 0), (0, 1), (0, 2), (1, 0), (2, 0), (2, 1), (3, 0)]
    );
    let generation = app.plan_rows.generation;
    // Same group count but different cohort sizes must never reuse stale offsets.
    let mut changed = plan();
    changed.actions[0] = Action::Trash {
        files: vec![source("only-one.jpg")],
    };
    app.set_review_plan(Some(changed));
    assert_eq!(app.plan_rows.len(), 5);
    assert_ne!(app.plan_rows.generation, generation);
    assert_eq!(app.plan_rows.at(1), (1, 0));
    app.set_review_plan(None);
    assert_eq!(app.plan_rows.len(), 0);
    assert!(app.plan_rows.folders.is_empty() && app.plan_rows.trash.is_empty());
    app.runtime.shutdown();
}

#[test]
fn plain_view_counts_photos_and_groups_them_by_folder() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some("/library".into());
    app.set_review_plan(Some(plan()));
    let rows = &app.plan_rows;
    // A photo with its RAW and sidecar is one photo, and a notes file is a
    // file, never a photo; renames in place keep their card.
    assert_eq!(rows.folders.len(), 2);
    assert_eq!(
        rows.folders[0].groups,
        [("/library".into(), vec![Item::Group(0), Item::Group(1)])]
    );
    assert!(!rows.folders[0].new && !rows.folders[0].arrivals);
    assert!(rows.folders[1].new && rows.folders[1].groups.is_empty());
    let plan = app.plan.as_ref().unwrap();
    assert_eq!(
        app.card_sentence(plan, &rows.folders[0]),
        "1 photo and 1 other file in library get new names"
    );
    assert_eq!(
        app.card_sentence(plan, &rows.folders[1]),
        "Empty is created and stays empty for now"
    );
    assert_eq!(rows.trash, [Item::Group(2)]);
    assert_eq!(
        lead_and_extras(&[Path::new("a.ARW"), Path::new("a.JPG"), Path::new("a.xmp")]),
        (1, "+ RAW, XMP".to_string())
    );
    // The sentence and the button read the same counts.
    assert_eq!(
        app.review_sentence(app.plan.as_ref().unwrap(), rows),
        "You're about to rename 1 photo and 1 other file, create 1 empty folder and send 1 photo to the Trash."
    );
    assert_eq!(rows.summary.apply_label(), "Apply all changes");
    app.runtime.shutdown();
}

/// The Organize plan from a sample_images_2 screenshot: photos from two
/// folders go into a new subfolder, and the folders they leave may be removed.
fn organize_plan() -> Plan {
    let root = Path::new("/library");
    let day = root.join("2025-10-25");
    let new = day.join("group-2/h");
    let mut actions: Vec<Action> = ["group-6/a.heic", "group-6/b.heic", "group-2/c.heic"]
        .into_iter()
        .map(|from| Action::Move {
            files: vec![(
                source(&format!("2025-10-25/{from}")),
                new.join(Path::new(from).file_name().unwrap()),
            )],
        })
        .collect();
    actions.push(Action::CreateDirectory { path: new.clone() });
    for folder in [day.join("group-6"), day.join("group-2"), day.clone()] {
        actions.push(Action::RemoveEmptyDirectory {
            path: folder,
            root: root.into(),
            after: Vec::new(),
            leftovers: Vec::new(),
        });
    }
    Plan {
        actions,
        output_root: Some(root.into()),
        ..Default::default()
    }
}

#[test]
fn organize_review_reads_as_plain_sentences_without_full_paths() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some("/library".into());
    app.set_review_plan(Some(organize_plan()));
    app.plan_open = true;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    for size in [egui::vec2(600.0, 500.0), egui::vec2(1280.0, 800.0)] {
        let texts = dialog_texts(&mut app, &ctx, size);
        for expected in [
            "You're about to move 3 photos into a new folder \"h\".",
            "Moving",
            // The card says where its photos go and where they come from,
            // then shows them by source folder.
            "h (in 2025-10-25 / group-2) gets 3 photos from 2 folders, new folder",
            "From 2025-10-25 / group-2 · 1 photo",
            "From 2025-10-25 / group-6 · 2 photos",
            "a.heic",
            "c.heic",
            "Back",
            // The button says what it does, with the sentence's count.
            "Move 3 photos",
        ] {
            assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
        }
        if size.y > 600.0 {
            // Folder cleanup is one quiet, conditional line, never one row per folder.
            assert!(
                texts
                    .iter()
                    .any(|t| t.starts_with("Folders left empty are removed: 2025-10-25")),
                "{texts:?}"
            );
        }
        assert!(!texts.iter().any(|t| t.contains("Remove folder if empty")
            || t.contains("/library/")
            || t.starts_with("Apply")));
    }
    // A folder that keeps Finder or NAS leftovers is not "left empty": the
    // list says it goes to the Trash and what goes with it.
    let mut plan = organize_plan();
    for action in &mut plan.actions {
        if let Action::RemoveEmptyDirectory {
            path, leftovers, ..
        } = action
            && path.ends_with("group-6")
        {
            *leftovers = [".DS_Store", "._.DS_Store", "._@eaDir", "@eaDir"]
                .map(String::from)
                .to_vec();
        }
    }
    app.set_review_plan(Some(plan));
    let texts = dialog_texts(&mut app, &ctx, egui::vec2(1280.0, 800.0));
    for expected in [
        "You're about to move 3 photos into a new folder \"h\" and send 1 folder to the Trash.",
        "Sent to the Trash: 2025-10-25 / group-6, with .DS_Store and @eaDir. Folders left empty are removed: 2025-10-25",
        "Apply all changes",
    ] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    assert_eq!(app.stats.decodes, 0);
    app.runtime.shutdown();
}

#[test]
fn plain_view_virtualizes_hundreds_of_thousands_of_photos() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some("/library".into());
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let plan = Plan {
        actions: (0..100_000)
            .map(|i| Action::Move {
                files: vec![(
                    source(&format!("{i}.jpg")),
                    Path::new("/library/Sorted").join(format!("{i}.jpg")),
                )],
            })
            .collect(),
        output_root: Some("/library".into()),
        ..Default::default()
    };
    let mut rows = Rows::default();
    rows.replace(Some(&plan), &[], &HashSet::new(), &HashSet::new());
    assert_eq!(rows.folders.len(), 1);
    assert_eq!(rows.folders[0].tally().photos, 100_000);
    let mut scroll = None;
    for at_end in [false, true] {
        if at_end {
            let (id, mut state): (egui::Id, egui::scroll_area::State) = scroll.take().unwrap();
            state.offset.y = 1.0e9;
            state.store(&ctx, id);
        }
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let shown = app.plain_view(ui, &plan, &mut rows, 300.0);
                        assert!(shown.inner <= 8, "{} rendered pieces", shown.inner);
                        scroll = Some((shown.id, shown.state));
                    });
                },
            ));
        }
        let tree = output.unwrap().platform_output.accesskit_update.unwrap();
        let expected = if at_end {
            "99999.jpg"
        } else {
            "100000 photos from library move to Sorted"
        };
        assert!(
            tree.nodes
                .iter()
                .any(|(_, n)| n.value().or(n.label()) == Some(expected)),
            "{expected}"
        );
        assert!(tree.nodes.len() < 100, "{} nodes", tree.nodes.len());
    }
    // Rows past the end were laid out once, never drawn.
    assert!(rows.layout.len() > 10_000);
    app.runtime.shutdown();
}

#[test]
fn review_virtualization_reaches_last_file_without_materializing_other_rows() {
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let mut plan = plan();
    plan.actions = vec![Action::Move {
        files: (0..100_000)
            .map(|i| {
                (
                    source(&format!("before-{i}.jpg")),
                    PathBuf::from(format!("/library/after-{i}.jpg")),
                )
            })
            .collect(),
    }];
    let mut index = Rows::default();
    index.replace(Some(&plan), &[], &HashSet::new(), &HashSet::new());
    assert_eq!(index.ends.len(), 1);
    let mut scroll = None;
    for at_end in [false, true] {
        if at_end {
            let (id, mut state): (egui::Id, egui::scroll_area::State) = scroll.take().unwrap();
            // Past the end; the scroll area clamps to the last row whatever the row height.
            state.offset.y = 1.0e9;
            state.store(&ctx, id);
        }
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(640.0, 500.0),
                    )),
                    ..Default::default()
                },
                |ui| {
                    egui::CentralPanel::default().show(ui, |ui| {
                        let shown =
                            file_rows(ui, &plan, &index, Some(Path::new("/library")), 220.0);
                        assert!(shown.inner <= 6, "{} rendered rows", shown.inner);
                        scroll = Some((shown.id, shown.state));
                    });
                },
            ));
        }
        let tree = output.unwrap().platform_output.accesskit_update.unwrap();
        let expected = if at_end {
            "To    after-99999.jpg"
        } else {
            "To    after-0.jpg"
        };
        assert!(
            tree.nodes.iter().any(|(_, n)| n.value() == Some(expected)),
            "{expected}"
        );
        assert!(
            tree.nodes.len() < 100,
            "virtualization emitted {} nodes",
            tree.nodes.len()
        );
    }
}

#[test]
fn shared_review_exposes_names_and_keeps_apply_visible_in_compact_and_retry_views() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some("/library".into());
    app.set_review_plan(Some(plan()));
    app.plan_open = true;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    for step in Step::ALL {
        app.step = step;
        for retry in [false, true] {
            app.retry_plan = retry.then(plan);
            let mut reviewed = plan();
            if retry {
                reviewed.actions[0] = Action::FinishMove {
                    files: vec![(source("sample.ARW"), "/library/keepers.ARW".into())],
                };
            }
            app.set_review_plan(Some(reviewed));
            app.recovery_ticket = retry.then(|| crate::core::operations::recovery::Ticket {
                journal: "test".into(),
                fingerprint: Default::default(),
            });
            app.after_apply = retry.then_some(Destination::Close);
            app.error = retry.then(|| "A file changed since review. ".repeat(500));
            for size in [
                egui::vec2(600.0, 500.0),
                egui::vec2(640.0, 600.0),
                egui::vec2(1280.0, 800.0),
            ] {
                let mut output = None;
                for _ in 0..3 {
                    output = Some(ctx.run_test(
                        egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                            ..Default::default()
                        },
                        |ui| {
                            let ctx = &ui.ctx().clone();
                            app.plan_review_dialog(ctx)
                        },
                    ));
                }
                let tree = output.unwrap().platform_output.accesskit_update.unwrap();
                // Retries open every file operation; a first review shows photos.
                let names: &[&str] = if retry {
                    &["From  sample.ARW", "To    keepers.ARW"]
                } else {
                    &["Going to the Trash", "rejected.jpg"]
                };
                if retry {
                    assert!(tree.nodes.iter().any(|(_, n)| {
                        n.value()
                            .is_some_and(|text| text.contains("Finish staged move"))
                    }));
                }
                for name in names {
                    assert!(
                        tree.nodes
                            .iter()
                            .any(|(_, n)| n.value().or(n.label()) == Some(name)),
                        "missing visible {name}"
                    );
                }
                for label in [
                    if retry { "Save plan and quit" } else { "Back" },
                    if retry {
                        "Retry unfinished changes"
                    } else {
                        "Apply all changes"
                    },
                ] {
                    let n = tree
                        .nodes
                        .iter()
                        .find(|(_, n)| n.label() == Some(label))
                        .expect(label);
                    let bounds = n.1.bounds().unwrap();
                    assert!(
                        bounds.y0 >= 0.0
                            && bounds.y1 <= f64::from(size.y)
                            && bounds.x0 >= 0.0
                            && bounds.x1 <= f64::from(size.x),
                        "{step:?} {size:?} retry={retry}: {label} {bounds:?}"
                    );
                }
                // Plan paths outside the library have no thumbnails to load.
                assert!(
                    app.requested.is_empty()
                        && app.analysis_run.is_none()
                        && app.rotation_run.is_none()
                );
                assert_eq!(app.stats.decodes, 0);
                assert!(app.decisions.is_empty());
            }
        }
    }
    app.runtime.shutdown();
}

#[test]
fn review_names_lossless_pixel_rotations() {
    use crate::core::metadata::WriteTarget;
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some("/library".into());
    app.set_review_plan(Some(Plan {
        actions: vec![Action::Metadata {
            source: source("scan.bmp"),
            sidecar_before: None,
            target: WriteTarget::Pixels,
            rating: None,
            orientation: Some(6),
        }],
        ..Default::default()
    }));
    app.plan_open = true;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let texts = dialog_texts(&mut app, &ctx, egui::vec2(900.0, 700.0));
    for expected in [
        "You're about to rotate 1 photo.",
        "Rotating",
        "scan.bmp: Rotate right (re-saves the image)",
        "Rotate 1 photo",
    ] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    app.runtime.shutdown();
}

/// A rotation reads "before → after": the photo as it is, then turned, and
/// its caption names the turn from how it looks now, not the tag it ends
/// with ("Set upright" for a photo tagged sideways reads "Rotate left").
#[test]
fn review_shows_rotations_before_and_after_named_by_the_turn() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some("/library".into());
    let metadata = |name: &str, orientation, rating| Action::Metadata {
        source: source(name),
        sidecar_before: None,
        target: crate::core::metadata::WriteTarget::Embedded,
        rating,
        orientation,
    };
    for (name, orientation) in [("o1.jpg", 6), ("o6.jpg", 1)] {
        app.metadata.insert(
            MediaId::for_path(&Path::new("/library").join(name)),
            Metadata {
                orientation,
                ..Default::default()
            },
        );
    }
    app.set_review_plan(Some(Plan {
        actions: vec![
            metadata("o1.jpg", Some(1), None),
            metadata("o6.jpg", Some(6), Some(3)),
        ],
        ..Default::default()
    }));
    app.plan_open = true;
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let texts = dialog_texts(&mut app, &ctx, egui::vec2(900.0, 700.0));
    // A photo rotated and rated is in both sections, each saying its part.
    for expected in ["o1.jpg: Rotate left", "o6.jpg: Rotate right", "o6.jpg: ★★★"] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    assert!(
        !texts.iter().any(|t| t.contains("Set upright")),
        "{texts:?}"
    );
    assert_eq!(app.plan_rows.slot(List::Rotate), PAIR);
    assert_eq!(app.plan_rows.slot(List::Rate), TILE.x);
    assert_eq!(app.plan_rows.slot(List::Trash), TILE.x);
    // A mirror or an unknown orientation is still named, without a pair.
    assert_eq!(turns_between(2, 7), Some(1));
    assert_eq!(turns_between(1, 2), None);
    app.runtime.shutdown();
}

/// The Review step lists Organize's renames with the choices staged in the
/// other steps, and one Apply applies the moves, then plans and applies the
/// staged choices for the moved files without a second review.
#[test]
fn one_review_applies_moves_then_staged_choices_without_a_second_review() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("Before")).unwrap();
    for name in ["a.jpg", "c.jpg", "d.jpg"] {
        std::fs::copy(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/samples/jpg_sample.jpg"),
            root.join("Before").join(name),
        )
        .unwrap();
    }
    // The marked photo's sidecar, listed with its mark.
    std::fs::write(root.join("Before/d.xmp"), "<x:xmpmeta/>").unwrap();
    let catalog = crate::core::catalog::Catalog::scan(&root, || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.join("cache")), None);
    app.root = Some(root.clone());
    app.output = root.display().to_string();
    app.media = catalog.media.clone();
    app.catalog = Some(Arc::new(catalog));
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    let id = |app: &PhotoSortApp, name: &str| {
        app.media
            .iter()
            .find(|m| m.path.ends_with(name))
            .unwrap()
            .id
    };
    let (a, c, d) = (id(&app, "a.jpg"), id(&app, "c.jpg"), id(&app, "d.jpg"));
    // Renames in Organize, a rating and a Trash mark in Cull and a turn in
    // Fix Rotation.
    app.step = Step::Organize;
    app.file_names.insert(a, "b".into());
    app.file_names.insert(d, "e".into());
    app.switch_step(Step::Cull);
    app.rating(a, 3);
    app.reject(d);
    app.switch_step(Step::Rotation);
    crate::core::rotation::set_turns(&mut app.decisions, c, 1);
    app.changed_item(c);
    assert_eq!(app.staged_change_count(), 5);
    let settle = |app: &mut PhotoSortApp| {
        let until = std::time::Instant::now() + crate::test_support::worker_patience();
        loop {
            let expired = std::time::Instant::now() >= until;
            app.poll(&egui::Context::default());
            app.refresh_review();
            if !app.busy && !app.scanning && app.metadata_run.is_none() && !app.review_stale {
                break;
            }
            assert!(!expired, "worker did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    // Shift+Enter, the bar's button and the footer tab all lead here.
    app.review_changes();
    assert_eq!(app.step, Step::Review);
    settle(&mut app);
    assert!(!app.plan_open && app.error.is_none(), "{:?}", app.error);
    assert_eq!(
        (
            app.plan_rows.ratings.len(),
            app.plan_rows.rotations.len(),
            app.plan_rows.trash.len()
        ),
        (1, 1, 1)
    );
    // The moves are planned first; the Trash mark waits for the moved file.
    assert!(
        app.plan
            .as_ref()
            .unwrap()
            .actions
            .iter()
            .all(|a| !matches!(a, Action::Trash { .. } | Action::TrashDirectory(_)))
    );
    assert_eq!(
        app.review_sentence(app.plan.as_ref().unwrap(), &app.plan_rows),
        "You're about to rename 1 photo, change the rating or rotation of 2 photos and send 1 photo to the Trash."
    );
    assert_eq!(app.plan_rows.apply_label(), "Apply all changes");
    assert_eq!(app.plan_rows.followers[&d], "+ XMP");
    // Nothing has changed on disk while the changes are only listed.
    assert!(root.join("Before/a.jpg").is_file() && root.join("Before/d.jpg").is_file());
    app.apply_review();
    // The moves are applied: the listed choices follow the moved files.
    let until = std::time::Instant::now() + crate::test_support::worker_patience();
    while !app
        .review_after_moves
        .as_ref()
        .is_some_and(|chain| chain.root.is_some())
    {
        assert!(std::time::Instant::now() < until, "moves did not finish");
        app.poll(&egui::Context::default());
        std::thread::sleep(Duration::from_millis(5));
    }
    let moved = MediaId::for_path(&root.join("Before/e.jpg"));
    let chain = app.review_after_moves.as_mut().unwrap();
    assert!(chain.decisions[&moved].rejected && !chain.decisions.contains_key(&d));
    // Only the sidecar the mark listed may follow it, under its new name.
    assert_eq!(
        chain.followers,
        HashSet::from([MediaId::for_path(&root.join("Before/e.xmp"))])
    );
    assert_eq!(
        chain.decisions[&MediaId::for_path(&root.join("Before/b.jpg"))].rating,
        Some(3)
    );
    // Kept out of the computer's Trash: the chain applies only what it holds.
    chain.decisions.remove(&moved);
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.retry_plan.is_none() && !app.plan_open && !app.apply_planned);
    // Each apply journals on its own; a finished chain leaves none behind.
    assert!(app.recovery_ticket.is_none());
    assert!(root.join("Before/b.jpg").exists() && root.join("Before/e.jpg").exists());
    // The rating and the turn were applied too: only the mark taken out of
    // the chain is left staged, and the page says what was done.
    assert!(
        app.decisions
            .iter()
            .all(|(id, d)| d.is_empty() || (*id == moved && d.rejected)),
        "{:?}",
        app.decisions
    );
    assert_eq!(app.step, Step::Review);
    assert_eq!(app.staged_change_count(), 1);
    assert_eq!(
        app.review_outcome,
        Some(review_step::Outcome {
            sentence:
                "Renamed 1 photo, rotated 1 photo, rated 1 photo and sent 1 photo to the Trash."
                    .into(),
            done: true,
            left_behind: None,
            kept_folders: Vec::new(),
        })
    );
    app.runtime.shutdown();
}
#[test]
fn destination_editor_virtualizes_large_plans_and_keeps_compact_controls_visible() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some("/library".into());
    let mut plan = plan();
    plan.actions = (0..100_000)
        .map(|i| Action::Move {
            files: vec![(
                source(&format!("{i}.jpg")),
                Path::new("/library/Sorted").join(format!("{i}.jpg")),
            )],
        })
        .collect();
    app.retry_plan = Some(plan.clone());
    app.recovery_ticket = Some(crate::core::operations::recovery::Ticket {
        journal: "test".into(),
        fingerprint: Default::default(),
    });
    app.set_review_plan(Some(plan));
    app.plan_open = true;
    app.begin_destination_edit();
    app.retry_edit.as_mut().unwrap().error = Some("A source changed. ".repeat(100));
    app.retry_edit.as_mut().unwrap().stem = "Reviewed name".into();
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    for size in [egui::vec2(600., 500.), egui::vec2(1280., 800.)] {
        let mut output = None;
        for _ in 0..3 {
            output = Some(ctx.run_test(
                egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, size)),
                    ..Default::default()
                },
                |ui| {
                    let ctx = &ui.ctx().clone();
                    app.plan_review_dialog(ctx)
                },
            ));
        }
        let tree = output.unwrap().platform_output.accesskit_update.unwrap();
        for label in ["Cancel", "Save destination"] {
            let bounds = tree
                .nodes
                .iter()
                .find(|(_, n)| n.label() == Some(label))
                .unwrap()
                .1
                .bounds()
                .unwrap();
            assert!(
                bounds.y0 >= 0.
                    && bounds.y1 <= f64::from(size.y)
                    && bounds.x0 >= 0.
                    && bounds.x1 <= f64::from(size.x),
                "{label}: {bounds:?}"
            );
        }
        assert!(tree.nodes.len() < 100);
    }
    assert_eq!(app.stats.decodes, 0);
    app.set_review_plan(None);
    assert!(app.retry_edit.is_none() && app.plan_rows.movable.is_empty());
    app.runtime.shutdown();
}

#[test]
fn card_sentences_name_one_source_or_count_several() {
    let temp = tempfile::tempdir().unwrap();
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, temp.path().join("cache")), None);
    app.root = Some("/library".into());
    let mut plan = organize_plan();
    // Only the group-6 photos: one source folder.
    plan.actions.remove(2);
    app.set_review_plan(Some(plan));
    let rows = &app.plan_rows;
    assert_eq!(
        app.card_sentence(app.plan.as_ref().unwrap(), &rows.folders[0]),
        "2 photos from 2025-10-25 / group-6 move to h (in 2025-10-25 / group-2)"
    );
    // One source is named by the sentence, so the card has no source heading.
    assert!(rows.layout.is_empty());
    app.plan_rows.lay_out(800.0);
    assert!(
        !app.plan_rows
            .layout
            .iter()
            .any(|p| matches!(p.block, Block::SourceHead(..)))
    );
    app.runtime.shutdown();
}

/// The Review step lists the changes made in several steps by what happens
/// to the photos, and each photo's Undo takes back its change alone.
#[test]
fn the_review_step_lists_changes_from_every_step_and_undoes_one_at_a_time() {
    use super::super::compact_cull_tests::{click_label, fixture, frame, rendered};
    let (_temp, mut app) = fixture();
    app.output = app.root.as_ref().unwrap().display().to_string();
    let ids: Vec<MediaId> = app.media.iter().map(|m| m.id).collect();
    let name = |i: usize| app.media[i].name();
    let names: Vec<String> = (1..5).map(name).collect();
    // Cull: a Trash mark and a rating.
    app.stage_manual([ids[1]], crate::core::decisions::Change::Trash(true));
    app.rating(ids[2], 4);
    // Organize: a new name. Fix Rotation: a turn.
    app.switch_step(Step::Organize);
    app.file_names.insert(ids[3], "Keeper".into());
    app.switch_step(Step::Rotation);
    crate::core::rotation::set_turns(&mut app.decisions, ids[4], 1);
    app.changed_item(ids[4]);
    let ctx = egui::Context::default();
    theme_install(&ctx);
    ctx.enable_accesskit();
    let size = egui::vec2(1440.0, 900.0);
    let list = |app: &mut PhotoSortApp| {
        let until = std::time::Instant::now() + crate::test_support::worker_patience();
        loop {
            frame(app, &ctx, size, Vec::new());
            if !app.busy && !app.review_stale {
                break;
            }
            assert!(std::time::Instant::now() < until, "the list did not come");
            std::thread::sleep(Duration::from_millis(5));
        }
        rendered(app, &ctx, size)
    };
    app.switch_step(Step::Review);
    let nodes = list(&mut app);
    let has = |nodes: &[(String, egui::Rect)], label: &str| nodes.iter().any(|(l, _)| l == label);
    for label in [
        "You're about to rename 1 photo, change the rating or rotation of 2 photos and send 1 photo to the Trash.",
        "Apply all changes",
        &format!("Keep {}", names[0]),
        &format!("Don't change the rating of {}", names[1]),
        &format!("Leave {} where it is", names[2]),
        &format!("Don't rotate {}", names[3]),
    ] {
        assert!(
            has(&nodes, label),
            "{label}: {:?}",
            nodes.iter().map(|n| &n.0).collect::<Vec<_>>()
        );
    }
    assert!(!app.plan_open);
    // Undo on one photo: only its Trash mark goes.
    click_label(&mut app, &ctx, size, &format!("Keep {}", names[0]));
    assert!(!app.decision(ids[1]).rejected);
    assert_eq!(app.decision(ids[2]).rating, Some(4));
    // The list is made again without it, from the moves already planned:
    // nothing is sent to the file worker, and Apply applies what is left.
    let planned = |app: &PhotoSortApp| app.plan.as_ref().map_or(0, |plan| plan.actions.len());
    assert!(!app.review_stale && !app.busy && planned(&app) > 0);
    frame(&mut app, &ctx, size, Vec::new());
    assert!(!app.review_stale && !app.busy && planned(&app) > 0);
    assert_eq!(
        app.review_listed,
        app.staged_choices().into_iter().collect::<HashMap<_, _>>()
    );
    assert!(!app.review_listed.contains_key(&ids[1]));
    let nodes = list(&mut app);
    assert!(!has(&nodes, &format!("Keep {}", names[0])));
    // Undo on a rename plans the folder changes again, without it.
    click_label(
        &mut app,
        &ctx,
        size,
        &format!("Leave {} where it is", names[2]),
    );
    let nodes = list(&mut app);
    assert!(app.file_names.is_empty());
    assert!(!has(&nodes, &format!("Leave {} where it is", names[2])));
    assert!(has(&nodes, &format!("Don't rotate {}", names[3])));
    assert_eq!(app.staged_change_count(), 2);
    // Nothing changed on disk.
    assert!(app.media.iter().all(|m| m.path.is_file()));
    app.runtime.shutdown();
}

/// A photo marked for Trash lists the files Apply sends to the Trash with
/// it: its RAW twin and XMP sidecar. A RAW the user kept stays out of them
/// until Apply has asked.
#[test]
fn a_listed_trash_mark_names_the_raw_and_xmp_that_go_with_it() {
    let dir = tempfile::tempdir().unwrap();
    // Both sidecar spellings: "IMG_1.xmp" is shared, the others name one file.
    for name in [
        "IMG_1.JPG",
        "IMG_1.CR2",
        "IMG_1.xmp",
        "IMG_1.JPG.xmp",
        "IMG_1.CR2.xmp",
        "IMG_2.JPG",
    ] {
        std::fs::write(dir.path().join(name), name).unwrap();
    }
    let catalog = crate::core::catalog::Catalog::scan(dir.path(), || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(crate::core::jobs::Runtime::new(|| {}), None);
    app.root = Some(catalog.root.clone());
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.catalog = Some(std::sync::Arc::new(catalog));
    let id = |name: &str| {
        app.media
            .iter()
            .find(|m| m.name() == name)
            .map(|m| m.id)
            .unwrap()
    };
    let (jpeg, other, raw) = (id("IMG_1.JPG"), id("IMG_2.JPG"), id("IMG_1.CR2"));
    let sidecar = |name: &str| MediaId::for_path(&app.root.as_ref().unwrap().join(name));
    let (shared, of_jpeg, of_raw) = (
        sidecar("IMG_1.xmp"),
        sidecar("IMG_1.JPG.xmp"),
        sidecar("IMG_1.CR2.xmp"),
    );
    for photo in [jpeg, other] {
        app.decisions.insert(
            photo,
            Decision {
                rejected: true,
                ..Default::default()
            },
        );
    }
    app.step = Step::Review;
    app.set_review_plan(Some(Plan::default()));
    assert_eq!(
        app.plan_rows.followers.get(&jpeg).map(String::as_str),
        Some("+ RAW, XMP")
    );
    assert!(!app.plan_rows.followers.contains_key(&other));
    assert_eq!(
        app.plan_rows.follower_ids(),
        HashSet::from([raw, shared, of_jpeg, of_raw])
    );
    // A RAW nobody chose follows without a question.
    assert_eq!(app.plan_rows.wanted_raws(), 0);
    // Kept in Pick Best over its JPEG: it stays, with the XMP they share,
    // until the user says otherwise.
    let scores = HashMap::from([(raw, 0.9), (jpeg, 0.5)]);
    app.pick_review
        .replace(vec![crate::core::pick_review::Cluster::new(
            0,
            vec![raw, jpeg],
            scores,
        )]);
    app.list_review(Plan::default());
    assert_eq!(app.plan_rows.wanted_raws(), 1);
    // The JPEG's own sidecar still goes with it; the RAW keeps the other two.
    assert_eq!(
        app.plan_rows.followers.get(&jpeg).map(String::as_str),
        Some("+ XMP")
    );
    assert_eq!(app.plan_rows.follower_ids(), HashSet::from([of_jpeg]));
    app.pick_review.replace(Vec::new());
    // The same for a RAW with a rating of its own, which is listed.
    app.decisions.entry(raw).or_default().rating = Some(4);
    listed(&mut app);
    app.list_review(Plan::default());
    assert_eq!(app.plan_rows.follower_ids(), HashSet::from([of_jpeg]));
    assert_eq!(app.plan_rows.ratings.len(), 1);
    // Apply waits for the list's details, then asks first, and applies
    // nothing without an answer.
    listed(&mut app);
    app.apply_review();
    assert_eq!(
        app.raw_question,
        Some(review_step::RawQuestion {
            count: 1,
            quiet: false
        })
    );
    app.answer_raw_question(None);
    assert!(app.raw_question.is_none() && !app.busy && app.review_outcome.is_none());
    // "Trash them" with "Don't ask again": the RAW and XMP are named on the
    // mark, the rating is no longer listed or counted, and the answer is
    // remembered. The plan is never polled, so nothing reaches the Trash.
    app.apply_review();
    app.raw_question.as_mut().unwrap().quiet = true;
    app.answer_raw_question(Some(crate::core::preferences::WantedRaw::Trash));
    assert_eq!(
        app.plan_rows.followers.get(&jpeg).map(String::as_str),
        Some("+ RAW, XMP")
    );
    assert_eq!(
        app.plan_rows.follower_ids(),
        HashSet::from([raw, shared, of_jpeg, of_raw])
    );
    assert!(app.plan_rows.ratings.is_empty());
    assert_eq!(
        app.review_outcome.as_ref().map(|o| o.sentence.as_str()),
        Some("Sent 2 photos to the Trash.")
    );
    assert!(app.busy && app.raw_question.is_none());
    assert_eq!(
        app.preferences.limit_prompts.wanted_raw,
        Some(crate::core::preferences::WantedRaw::Trash)
    );
    app.runtime.shutdown();
}

/// A photo marked for Trash that Organize also moves is listed once, under
/// Trash, with where it moves first and the files that follow it. Keeping
/// it lists it as an ordinary move again, from the moves already planned.
#[test]
fn a_photo_that_moves_and_goes_to_the_trash_is_listed_once_under_trash() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("Before")).unwrap();
    for name in ["IMG_1.JPG", "IMG_1.CR2", "IMG_1.xmp", "IMG_2.JPG"] {
        std::fs::write(root.join("Before").join(name), name).unwrap();
    }
    let catalog = crate::core::catalog::Catalog::scan(&root, || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.join("cache")), None);
    app.root = Some(root.clone());
    app.output = root.display().to_string();
    app.media = catalog.media.clone();
    app.catalog = Some(Arc::new(catalog));
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    let id = |name: &str| MediaId::for_path(&root.join("Before").join(name));
    let (jpeg, raw, other) = (id("IMG_1.JPG"), id("IMG_1.CR2"), id("IMG_2.JPG"));
    app.step = Step::Organize;
    app.organize_drafts.insert(
        app.group_mode,
        crate::core::organize::Draft::new(HashMap::from([
            (jpeg, "Trips/Gone".to_owned()),
            (raw, "Trips/Gone".to_owned()),
            (other, "Kept".to_owned()),
        ])),
    );
    app.decisions.entry(jpeg).or_default().rejected = true;
    app.changed_item(jpeg);
    let settle = |app: &mut PhotoSortApp| {
        let until = std::time::Instant::now() + crate::test_support::worker_patience();
        loop {
            let expired = std::time::Instant::now() >= until;
            app.poll(&egui::Context::default());
            app.refresh_review();
            if !app.busy && !app.scanning && app.metadata_run.is_none() && !app.review_stale {
                break;
            }
            assert!(!expired, "worker did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    app.review_changes();
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    // What the Moving section shows: each photo's name and the files with it.
    let moving = |app: &PhotoSortApp| -> Vec<(String, String)> {
        let (plan, rows) = (app.plan.as_ref().unwrap(), &app.plan_rows);
        let mut shown = Vec::new();
        for (f, folder) in rows.folders.iter().enumerate() {
            for (g, (_, items)) in folder.groups.iter().enumerate() {
                for item in items {
                    let thumb = app.thumb(plan, rows, *item, List::Folder(f, g)).unwrap();
                    shown.push((thumb.file, thumb.extras));
                }
            }
        }
        shown
    };
    let sentence =
        |app: &PhotoSortApp| app.review_sentence(app.plan.as_ref().unwrap(), &app.plan_rows);
    // Once, under Trash: the plan moves it with its RAW and XMP all the same.
    assert_eq!(moving(&app), [("IMG_2.JPG".to_owned(), String::new())]);
    assert_eq!(
        sentence(&app),
        "You're about to move 1 photo into the folder \"Kept\" and send 1 photo to the Trash."
    );
    assert_eq!(app.plan_rows.apply_label(), "Move 1 and trash 1");
    let moves = app
        .plan
        .as_ref()
        .unwrap()
        .actions
        .iter()
        .filter(|action| matches!(action, Action::Move { .. }))
        .count();
    assert_eq!(moves, 2);
    let mark = app.plan_rows.trash[0];
    let tile = app
        .thumb(
            app.plan.as_ref().unwrap(),
            &app.plan_rows,
            mark,
            List::Trash,
        )
        .unwrap();
    assert_eq!(
        (
            tile.file.as_str(),
            tile.extras.as_str(),
            tile.first.as_str()
        ),
        ("IMG_1.JPG", "+ RAW, XMP", "Gone")
    );
    assert!(
        tile.hover.ends_with("Moves to Trips / Gone first"),
        "{}",
        tile.hover
    );
    assert_eq!(app.plan_rows.wanted_raws(), 0);
    // A RAW the user rated stays until Apply has asked: it is listed as
    // moving, with the XMP it keeps, and the question counts it.
    app.decisions.entry(raw).or_default().rating = Some(4);
    app.changed_item(raw);
    settle(&mut app);
    assert_eq!(app.plan_rows.wanted_raws(), 1);
    assert_eq!(
        moving(&app),
        [
            ("IMG_2.JPG".to_owned(), String::new()),
            ("IMG_1.CR2".to_owned(), "+ XMP".to_owned())
        ]
    );
    app.decisions.remove(&raw);
    app.changed_item(raw);
    settle(&mut app);
    // Keep: an ordinary moving photo again, and nothing is planned again.
    // The list stays where it was scrolled to: it is the same scroll area.
    let scrolled = app.plan_rows.generation;
    app.undo_review_item(List::Trash, mark);
    assert_eq!(app.plan_rows.generation, scrolled);
    assert!(!app.busy && !app.review_stale && app.plan.is_some());
    assert!(app.plan_rows.trash.is_empty() && app.plan_rows.moves_first.is_empty());
    assert_eq!(
        moving(&app),
        [
            ("IMG_2.JPG".to_owned(), String::new()),
            ("IMG_1.JPG".to_owned(), "+ RAW, XMP".to_owned())
        ]
    );
    assert_eq!(
        sentence(&app),
        "You're about to move 2 photos into 2 folders."
    );
    assert_eq!(app.staged_change_count(), 2);
    app.runtime.shutdown();
}

/// With no move at all: Trash marks that take the last photos out of a
/// folder that was there before remove it too, and "Tidying up" names it
/// first. A folder left with only Finder's leftovers goes to the Trash with
/// them, a parent emptied by its only subfolder goes as well, and a folder
/// that keeps a notes file or another photo stays, like the library folder.
#[test]
fn trash_marks_alone_remove_the_folders_they_empty_and_the_list_names_them_first() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let trash = tempfile::tempdir().unwrap();
    crate::core::operations::trash_stand_in(&root, trash.path());
    for name in [
        "Solo/a.jpg",
        "Solo/.DS_Store",
        "Deep/Inner/b.jpg",
        "Notes/c.jpg",
        "Notes/notes.txt",
        "Mixed/d.jpg",
        "Mixed/e.jpg",
    ] {
        std::fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
        std::fs::write(root.join(name), name).unwrap();
    }
    let catalog = crate::core::catalog::Catalog::scan(&root, || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.join("cache")), None);
    app.root = Some(root.clone());
    app.output = root.display().to_string();
    app.media = catalog.media.clone();
    app.catalog = Some(Arc::new(catalog));
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.step = Step::Cull;
    for name in [
        "Solo/a.jpg",
        "Deep/Inner/b.jpg",
        "Notes/c.jpg",
        "Mixed/d.jpg",
    ] {
        app.reject(MediaId::for_path(&root.join(name)));
    }
    let settle = |app: &mut PhotoSortApp| {
        let until = std::time::Instant::now() + crate::test_support::worker_patience();
        loop {
            let expired = std::time::Instant::now() >= until;
            app.poll(&egui::Context::default());
            app.refresh_review();
            if !app.busy && !app.scanning && app.metadata_run.is_none() && !app.review_stale {
                break;
            }
            assert!(!expired, "worker did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    app.review_changes();
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    let plan = app.plan.as_ref().unwrap();
    assert!(plan.actions.is_empty(), "nothing moves");
    assert_eq!(
        cleanup_text(plan, &app.plan_rows, Some(&root), 3, false),
        "Sent to the Trash: Solo, with .DS_Store. Folders left empty are removed: Deep"
    );
    // The folder that goes to the Trash is counted, as the sentence says.
    assert_eq!(
        app.review_sentence(plan, &app.plan_rows),
        "You're about to send 4 photos and 1 folder to the Trash."
    );
    assert_eq!(app.staged_change_count(), 5);
    // Nothing has changed on disk while the changes are only listed.
    assert!(root.join("Solo/a.jpg").is_file() && root.join("Deep/Inner").is_dir());
    app.apply_review();
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.retry_plan.is_none() && !app.review_applying());
    assert_eq!(
        app.review_outcome.as_ref().map(|o| o.sentence.as_str()),
        Some("Sent 4 photos and 1 folder to the Trash.")
    );
    // What the list named is gone, and nothing else.
    assert!(!root.join("Solo").exists() && !root.join("Deep").exists());
    assert!(trash.path().join("Solo/.DS_Store").is_file());
    assert!(root.join("Notes/notes.txt").is_file());
    assert!(root.join("Mixed/e.jpg").is_file() && !root.join("Mixed/d.jpg").exists());
    assert!(root.is_dir());
    app.runtime.shutdown();
}

/// A photo renamed where it is leaves its folder as full as it was: the RAW
/// with its XMP files, a plain JPEG, one beside a photo that moves away and
/// one beside a photo marked for Trash. None of these folders is planned for
/// removal or named in "Tidying up", and all are there after Apply. A folder
/// whose only photo is renamed and marked for Trash does end up empty.
#[test]
fn a_folder_whose_photos_are_only_renamed_is_not_tidied_away() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let trash = tempfile::tempdir().unwrap();
    crate::core::operations::trash_stand_in(&root, trash.path());
    for name in [
        "raw/R1.ARW",
        "raw/R1.ARW.xmp",
        "raw/R1.xmp",
        "plain/P1.JPG",
        "mix/M1.JPG",
        "mix/M2.JPG",
        "marks/K1.JPG",
        "marks/K2.JPG",
        "both/B1.JPG",
        "gone/G1.JPG",
    ] {
        std::fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
        std::fs::write(root.join(name), name).unwrap();
    }
    let catalog = Arc::new(crate::core::catalog::Catalog::scan(&root, || false, |_| {}).unwrap());
    let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.join("cache")), None);
    app.root = Some(root.clone());
    app.output = root.display().to_string();
    app.media = catalog.media.clone();
    app.catalog = Some(catalog.clone());
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    let id = |name: &str| MediaId::for_path(&root.join(name));
    app.step = Step::Organize;
    for (name, stem) in [
        ("raw/R1.ARW", "R1NEW"),
        ("plain/P1.JPG", "P1NEW"),
        ("mix/M1.JPG", "M1NEW"),
        ("marks/K1.JPG", "K1NEW"),
        ("both/B1.JPG", "B1NEW"),
    ] {
        crate::core::organize::set_file_name(
            &catalog,
            &mut app.file_names,
            id(name),
            stem,
            app.companions,
        )
        .unwrap();
    }
    app.organize_drafts.insert(
        app.group_mode,
        crate::core::organize::Draft::new(
            [("mix/M2.JPG", "plain"), ("gone/G1.JPG", "plain")]
                .into_iter()
                .map(|(name, folder)| (id(name), folder.to_owned()))
                .collect(),
        ),
    );
    let marked = [id("marks/K2.JPG"), id("both/B1.JPG")];
    for mark in marked {
        app.decisions.entry(mark).or_default().rejected = true;
    }
    app.changed_items(marked);
    let settle = |app: &mut PhotoSortApp| {
        let until = std::time::Instant::now() + crate::test_support::worker_patience();
        loop {
            let expired = std::time::Instant::now() >= until;
            app.poll(&egui::Context::default());
            app.refresh_review();
            if !app.busy && !app.scanning && app.metadata_run.is_none() && !app.review_stale {
                break;
            }
            assert!(!expired, "worker did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    app.review_changes();
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    let plan = app.plan.as_ref().unwrap();
    let removed: Vec<_> = plan
        .actions
        .iter()
        .filter_map(|action| match action {
            Action::RemoveEmptyDirectory { path, .. } => Some(path.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(removed, [root.join("gone")]);
    assert_eq!(
        cleanup_text(plan, &app.plan_rows, Some(&root), 3, false),
        "Folders left empty are removed: gone, both"
    );
    app.apply_review();
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.retry_plan.is_none() && !app.review_applying());
    for kept in [
        "raw/R1NEW.ARW",
        "raw/R1NEW.ARW.xmp",
        "raw/R1NEW.xmp",
        "plain/P1NEW.JPG",
        "plain/M2.JPG",
        "plain/G1.JPG",
        "mix/M1NEW.JPG",
        "marks/K1NEW.JPG",
    ] {
        assert!(root.join(kept).is_file(), "{kept}");
    }
    assert!(!root.join("marks/K2.JPG").exists());
    assert!(!root.join("gone").exists() && !root.join("both").exists());
    app.runtime.shutdown();
}

/// The Review badge, the sentence and the Apply button say one number. A
/// JPEG with its RAW and XMP is one photo wherever it is counted, a notes
/// file that follows its renamed folder is a file and not a photo, and
/// "Tidying up" names only the folder that really ends up empty.
#[test]
fn the_review_badge_says_the_number_the_sentence_and_the_button_say() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    for name in [
        "trip/EX_1.JPG",
        "trip/EX_2.JPG",
        "trip/EX_2.ARW",
        "trip/notes.txt",
        "raw/EXR_1.JPG",
        "raw/EXR_1.ARW",
        "raw/EXR_1.xmp",
        "raw/EXR_2.JPG",
        "raw/EXR_2.ARW",
        "raw/EXR_2.xmp",
    ] {
        std::fs::create_dir_all(root.join(name).parent().unwrap()).unwrap();
        std::fs::write(root.join(name), name).unwrap();
    }
    let catalog = crate::core::catalog::Catalog::scan(&root, || false, |_| {}).unwrap();
    let mut app = PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, root.join("cache")), None);
    app.root = Some(root.clone());
    app.output = root.display().to_string();
    app.media = catalog.media.clone();
    app.catalog = Some(Arc::new(catalog));
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    let id = |name: &str| MediaId::for_path(&root.join(name));
    // "trip" is renamed "trip2" with everything in it, one photo is dragged
    // there with its RAW, and the other photo of "raw" is marked for Trash.
    app.step = Step::Organize;
    let labels = [
        ("trip/EX_1.JPG", "trip2"),
        ("trip/EX_2.JPG", "trip2"),
        ("trip/EX_2.ARW", "trip2"),
        ("trip/notes.txt", "trip2"),
        ("raw/EXR_2.JPG", "trip2"),
        ("raw/EXR_2.ARW", "trip2"),
        ("raw/EXR_1.JPG", "raw"),
        ("raw/EXR_1.ARW", "raw"),
    ];
    app.organize_drafts.insert(
        app.group_mode,
        crate::core::organize::Draft::new(
            labels
                .into_iter()
                .map(|(name, folder)| (id(name), folder.to_owned()))
                .collect(),
        ),
    );
    let marked = id("raw/EXR_1.JPG");
    app.decisions.entry(marked).or_default().rejected = true;
    app.changed_item(marked);
    app.review_changes();
    let until = std::time::Instant::now() + crate::test_support::worker_patience();
    loop {
        let expired = std::time::Instant::now() >= until;
        app.poll(&egui::Context::default());
        app.refresh_review();
        if !app.busy && !app.scanning && app.metadata_run.is_none() && !app.review_stale {
            break;
        }
        assert!(!expired, "worker did not finish");
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(app.error.is_none(), "{:?}", app.error);
    let plan = app.plan.as_ref().unwrap();
    assert_eq!(
        app.review_sentence(plan, &app.plan_rows),
        "You're about to move 3 photos and 1 other file into the folder \"trip2\" and send 1 photo to the Trash."
    );
    assert_eq!(app.plan_rows.apply_label(), "Move 4 and trash 1");
    assert_eq!(app.plan_rows.total(), 5);
    assert_eq!(app.staged_change_count(), 5);
    assert_eq!(app.plan_rows.followers[&marked], "+ RAW, XMP");
    // The moves empty "trip"; the marked photo, with its RAW and XMP, is
    // the last thing in "raw", so the Trash mark empties that one.
    assert_eq!(
        cleanup_text(plan, &app.plan_rows, Some(&root), 3, false),
        "Folders left empty are removed: trip, raw"
    );
    app.runtime.shutdown();
}
