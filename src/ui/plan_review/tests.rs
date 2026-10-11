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
                if retry {
                    assert!(tree.nodes.iter().any(|(_, n)| {
                        n.value()
                            .is_some_and(|text| text.contains("Finish staged move"))
                    }));
                }
                for name in ["From  sample.ARW", "To    keepers.ARW"] {
                    assert!(
                        tree.nodes.iter().any(|(_, n)| n.value() == Some(name)),
                        "missing visible {name}"
                    );
                }
                for label in [
                    if retry {
                        "Save plan and quit"
                    } else {
                        "Keep reviewing"
                    },
                    if retry {
                        "Retry unfinished changes"
                    } else {
                        "Apply these changes"
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
    assert!(tree.nodes.iter().any(|(_, n)| {
        n.value().or(n.label()).is_some_and(|t| {
            t.contains("scan.bmp · re-encoded pixels · orientation 90 degrees clockwise")
        })
    }));
    app.runtime.shutdown();
}
