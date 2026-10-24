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
    assert!(app.plan_rows.lines.is_empty() && app.plan_rows.folders.is_empty());
    app.runtime.shutdown();
}

#[test]
fn plain_view_counts_photos_and_groups_them_by_folder() {
    let mut index = Rows::default();
    index.replace(Some(&plan()));
    // A photo with its RAW and sidecar is one photo; renames in place keep their card.
    assert_eq!(
        index.lines,
        [
            Line::Into(0),
            Line::Moved(0),
            Line::Moved(1),
            Line::Into(1),
            Line::Trash,
            Line::Trashed(2),
        ]
    );
    assert!(!index.folders[0].new && !index.folders[0].arrivals);
    assert!(index.folders[1].new && index.folders[1].photos == 0);
    assert_eq!(
        index
            .summary
            .parts()
            .into_iter()
            .map(|(p, _)| p)
            .collect::<Vec<_>>(),
        [
            "Rename 2 photos",
            "Create 1 empty folder",
            "Move 1 photo to the Trash"
        ]
    );
    assert_eq!(index.summary.changes(), 4);
    let text = |line| {
        plain_line(&plan(), &index, line, Some(Path::new("/library")))
            .into_iter()
            .map(|(t, ..)| t.trim().to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(text(Line::Moved(0)), "sample.JPG → keepers.JPG + RAW, XMP");
    assert_eq!(text(Line::Trashed(2)), "rejected.jpg + XMP in library");
    assert_eq!(text(Line::Into(1)), "New folder Empty stays empty for now");
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
        let texts: Vec<String> = tree
            .nodes
            .iter()
            .filter_map(|(_, n)| n.value().or(n.label()).map(str::to_string))
            .collect();
        for expected in [
            "Move 3 photos into 1 new folder",
            "Nothing in library has changed yet.",
            "Into 2025-10-25 / group-2 / h new folder 3 photos",
            "a.heic from 2025-10-25 / group-6",
            "c.heic from 2025-10-25 / group-2",
            "Back",
            "Apply 3 changes",
        ] {
            assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
        }
        // Folder cleanup is one quiet, conditional line, never one row per folder.
        assert!(texts.iter().any(|t| t.starts_with(
            "3 folders left empty afterwards are removed: 2025-10-25 / group-6, 2025-10-25 / group-2, 2025-10-25"
        )));
        assert!(!texts.iter().any(|t| t.contains("Remove folder if empty")
            || t.contains("/library/")
            || t.contains("Library:")));
    }
    app.runtime.shutdown();
}

#[test]
fn plain_view_virtualizes_hundreds_of_thousands_of_photos() {
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
    let mut index = Rows::default();
    index.replace(Some(&plan));
    assert_eq!(index.lines.len(), 100_001);
    assert_eq!(index.folders.len(), 1);
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
                        let shown =
                            plain_rows(ui, &plan, &index, Some(Path::new("/library")), 220.0);
                        assert!(shown.inner <= 10, "{} rendered lines", shown.inner);
                        scroll = Some((shown.id, shown.state));
                    });
                },
            ));
        }
        let tree = output.unwrap().platform_output.accesskit_update.unwrap();
        let expected = if at_end {
            "99999.jpg from library"
        } else {
            "Into Sorted 100000 photos"
        };
        assert!(
            tree.nodes
                .iter()
                .any(|(_, n)| n.value().or(n.label()) == Some(expected)),
            "{expected}"
        );
        assert!(tree.nodes.len() < 100, "{} nodes", tree.nodes.len());
    }
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
    index.replace(Some(&plan));
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
                    &[
                        "sample.JPG → keepers.JPG + RAW, XMP",
                        "To the Trash 1 photo",
                    ]
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
                        "Apply 4 changes"
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
    let mut output = None;
    for _ in 0..3 {
        output = Some(ctx.run_test(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 700.0),
                )),
                ..Default::default()
            },
            |ui| {
                let ctx = &ui.ctx().clone();
                app.plan_review_dialog(ctx)
            },
        ));
    }
    let tree = output.unwrap().platform_output.accesskit_update.unwrap();
    assert!(
        tree.nodes
            .iter()
            .any(|(_, n)| n.value() == Some("Rotate 1 photo"))
    );
    assert!(tree.nodes.iter().any(|(_, n)| {
        n.value().or(n.label()) == Some("scan.bmp rotate right (re-saves the image)")
    }));
    app.runtime.shutdown();
}
