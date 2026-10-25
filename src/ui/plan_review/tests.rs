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
    // A photo with its RAW and sidecar is one photo; renames in place keep their card.
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
        "2 photos in library get new names"
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
        "You're about to rename 2 photos, create 1 empty folder and send 1 photo to the Trash."
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
            "Nothing in library has changed yet.",
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
            assert!(texts.iter().any(|t| t.starts_with(
                "Folders left empty afterwards are removed: 2025-10-25 / group-6, 2025-10-25 / group-2, 2025-10-25"
            )), "{texts:?}");
        }
        assert!(!texts.iter().any(|t| t.contains("Remove folder if empty")
            || t.contains("/library/")
            || t.starts_with("Apply")));
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
    rows.replace(Some(&plan), &[]);
    assert_eq!(rows.folders.len(), 1);
    assert_eq!(rows.folders[0].photos(), 100_000);
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
    index.replace(Some(&plan), &[]);
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
        "Ratings and rotations",
        "scan.bmp: Rotate right (re-saves the image)",
        "Rotate 1 photo",
    ] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    app.runtime.shutdown();
}

/// One review covers Organize's moves and the staged ratings: one Apply
/// applies the moves, then plans and applies the ratings for the moved files
/// without opening a second review.
#[test]
fn one_review_applies_moves_then_staged_choices_without_a_second_review() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("Before")).unwrap();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/samples/jpg_sample.jpg"),
        root.join("Before/a.jpg"),
    )
    .unwrap();
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
    app.step = Step::Organize;
    let id = app.media[0].id;
    app.file_names.insert(id, "b".into());
    app.decisions.insert(
        id,
        Decision {
            rating: Some(3),
            ..Default::default()
        },
    );
    let settle = |app: &mut PhotoSortApp| {
        let until = std::time::Instant::now() + crate::test_support::worker_patience();
        loop {
            let expired = std::time::Instant::now() >= until;
            app.poll(&egui::Context::default());
            if !app.busy && !app.scanning && app.metadata_run.is_none() {
                break;
            }
            assert!(!expired, "worker did not finish");
            std::thread::sleep(Duration::from_millis(5));
        }
    };
    app.review_changes();
    settle(&mut app);
    assert!(app.plan_open && app.error.is_none(), "{:?}", app.error);
    // The staged rating is in the same review, under its own heading.
    assert_eq!(app.plan_rows.updates.len(), 1);
    assert_eq!(
        app.review_sentence(app.plan.as_ref().unwrap(), &app.plan_rows),
        "You're about to rename 1 photo and rate 1 photo."
    );
    assert_eq!(app.plan_rows.summary.apply_label(), "Apply all changes");
    app.apply_reviewed_plan();
    settle(&mut app);
    assert!(app.error.is_none(), "{:?}", app.error);
    assert!(app.retry_plan.is_none() && !app.plan_open && !app.apply_planned);
    // Each apply journals on its own; a finished chain leaves none behind.
    assert!(app.recovery_ticket.is_none());
    assert!(root.join("Before/b.jpg").exists());
    // The rating was applied too: nothing is left staged.
    assert!(
        app.decisions.values().all(|d| d.is_empty()),
        "{:?}",
        app.decisions
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
