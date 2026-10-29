use super::*;
use crate::test_support::worker_patience;
fn app() -> PhotoSortApp {
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.step = Step::Organize;
    app.group_mode = GroupMode::Mixed;
    app.catalog = Some(Arc::new(Catalog::default()));
    app.analysis_run = Some(41);
    app.organize_run = Some((41, GroupMode::Mixed));
    app
}
fn deliver(app: &mut PhotoSortApp, events: Vec<Event>) {
    let (sender, receiver) = std::sync::mpsc::sync_channel(events.len());
    let original = std::mem::replace(&mut app.runtime.events, receiver);
    for event in events {
        sender
            .send(crate::core::jobs::Envelope {
                epoch: app.runtime.epoch(),
                event,
            })
            .unwrap();
    }
    app.poll(&egui::Context::default());
    app.runtime.events = original;
}
#[test]
fn folder_trash_cancelled_plans_never_open_late_review_or_drop_kept_edits() {
    let mut app = app();
    app.file_names.insert(MediaId(1), "Keep".into());
    app.busy = true;
    app.plan_stop = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
    deliver(&mut app, vec![Event::Planned(Plan::default())]);
    assert!(!app.busy && app.plan.is_none() && app.plan_stop.is_none());
    assert_eq!(app.file_names[&MediaId(1)], "Keep");
    app.busy = true;
    app.plan_stop = Some(Arc::new(std::sync::atomic::AtomicBool::new(true)));
    deliver(&mut app, vec![Event::PlanningCancelled]);
    assert!(!app.busy && app.plan.is_none() && app.error.is_none());
    assert_eq!(app.file_names[&MediaId(1)], "Keep");
    app.busy = true;
    app.plan_stop = Some(Arc::new(std::sync::atomic::AtomicBool::new(false)));
    deliver(&mut app, vec![Event::Planned(Plan::default())]);
    assert!(!app.busy && app.plan.is_some() && app.plan_open);
    assert_eq!(app.status, "File plan ready · review your changes");
}
#[test]
fn regional_results_are_atomic_run_guarded_and_keep_subject_groups_separate() {
    let mut app = app();
    app.organize_run = None;
    app.group_mode = GroupMode::Visual;
    let id = MediaId(1);
    app.visual_groups = Some(HashMap::from([(id, 8)]));
    Arc::make_mut(&mut app.cull_groups).insert(id, 9);
    deliver(
        &mut app,
        vec![Event::GeneralGroups {
            automatic: Arc::default(),
            choices: Vec::new(),
            run: 40,
            groups: HashMap::from([(id, 1)]),
        }],
    );
    assert_eq!(app.visual_groups.as_ref().unwrap()[&id], 8);
    deliver(
        &mut app,
        vec![Event::GeneralGroups {
            automatic: Arc::default(),
            choices: Vec::new(),
            run: 41,
            groups: HashMap::from([(id, 2)]),
        }],
    );
    assert_eq!(app.visual_groups.as_ref().unwrap()[&id], 2);
    assert_eq!(app.cull_groups[&id], 9);
    app.organize_run = Some((41, GroupMode::Mixed));
    deliver(
        &mut app,
        vec![Event::GeneralGroups {
            automatic: Arc::default(),
            choices: Vec::new(),
            run: 41,
            groups: HashMap::new(),
        }],
    );
    assert_eq!(app.visual_groups.as_ref().unwrap()[&id], 2);
    app.organize_run = None;
    app.cull_group_run = Some(41);
    deliver(
        &mut app,
        vec![Event::GeneralGroups {
            automatic: Arc::default(),
            choices: Vec::new(),
            run: 41,
            groups: HashMap::new(),
        }],
    );
    assert_eq!(app.visual_groups.as_ref().unwrap()[&id], 2);
}
#[test]
fn subject_publication_is_atomic_shared_by_pick_and_cull_and_rejects_stale_runs() {
    for step in [Step::Cull, Step::PickBest] {
        let mut app = app();
        app.step = step;
        app.organize_run = None;
        app.cull_group_run = Some(41);
        let id = MediaId(1);
        app.visual_groups = Some(HashMap::from([(id, 77)]));
        deliver(
            &mut app,
            vec![Event::SubjectGroups {
                run: 40,
                groups: HashMap::from([(id, 9)]),
            }],
        );
        assert!(app.cull_groups.is_empty());
        deliver(
            &mut app,
            vec![Event::SubjectGroups {
                run: 41,
                groups: HashMap::from([(id, 3)]),
            }],
        );
        assert_eq!(app.cull_groups.get(&id), Some(&3));
        assert_eq!(
            app.visual_groups
                .as_ref()
                .and_then(|groups| groups.get(&id)),
            Some(&77),
            "Subject groups must not overwrite Organize groups"
        );
        app.analysis_run = None;
        deliver(
            &mut app,
            vec![Event::SubjectGroups {
                run: 41,
                groups: HashMap::from([(id, 99)]),
            }],
        );
        assert_eq!(app.cull_groups.get(&id), Some(&3));
    }
    let mut app = app();
    deliver(
        &mut app,
        vec![Event::SubjectGroups {
            run: 41,
            groups: HashMap::from([(MediaId(1), 7)]),
        }],
    );
    assert!(
        app.visual_groups.is_none(),
        "Organize must not accept Cull's group namespace"
    );
}
#[test]
fn face_results_are_isolated_atomic_and_preview_only_after_completion() {
    let mut app = app();
    app.organize_run = Some((41, GroupMode::Faces));
    app.group_mode = GroupMode::Faces;
    let id = MediaId(1);
    app.media = vec![Media {
        id,
        path: "fixture.jpg".into(),
        kind: MediaKind::Photo,
        fingerprint: Default::default(),
    }];
    app.visual_groups = Some(HashMap::from([(id, 8)]));
    Arc::make_mut(&mut app.cull_groups).insert(id, 9);
    app.organize();
    assert!(app.organize_preview_requested);
    assert!(!app.busy);
    deliver(
        &mut app,
        vec![Event::FaceGroups {
            run: 40,
            groups: crate::core::regional::grouping::Output {
                groups: HashMap::from([(id, 1)]),
                folders: HashMap::from([(id, "Person 001".into())]),
            },
        }],
    );
    assert!(app.face_groups.is_none());
    deliver(
        &mut app,
        vec![Event::FaceGroups {
            run: 41,
            groups: crate::core::regional::grouping::Output {
                groups: HashMap::from([(id, 2)]),
                folders: HashMap::from([(id, "People 001 + 003".into())]),
            },
        }],
    );
    assert_eq!(app.face_groups.as_ref().unwrap().groups[&id], 2);
    assert_eq!(app.label_for(&app.media[0]), "People 001 + 003");
    assert_eq!(app.organize_labels()[&id], "People 001 + 003");
    assert_eq!(app.visual_groups.as_ref().unwrap()[&id], 8);
    assert_eq!(app.cull_groups[&id], 9);
    assert!(!app.busy);
    deliver(
        &mut app,
        vec![Event::AnalysisDone {
            run: 41,
            stats: Default::default(),
        }],
    );
    assert!(app.busy);
    assert!(!app.organize_preview_requested);
}
#[test]
fn cancelled_face_results_keep_previous_groups_without_opening_preview() {
    let mut app = app();
    app.organize_run = Some((41, GroupMode::Faces));
    app.group_mode = GroupMode::Faces;
    let id = MediaId(1);
    app.media = vec![Media {
        id,
        path: "fixture.jpg".into(),
        kind: MediaKind::Photo,
        fingerprint: Default::default(),
    }];
    app.face_groups = Some(crate::core::regional::grouping::Output {
        groups: HashMap::from([(id, 7)]),
        folders: HashMap::from([(id, "People 001 + 003".into())]),
    });
    app.cancel_analysis();
    deliver(
        &mut app,
        vec![
            Event::FaceGroups {
                run: 41,
                groups: crate::core::regional::grouping::Output {
                    groups: HashMap::new(),
                    folders: HashMap::new(),
                },
            },
            Event::AnalysisDone {
                run: 41,
                stats: Default::default(),
            },
        ],
    );
    assert_eq!(app.face_groups.as_ref().unwrap().groups[&id], 7);
    assert_eq!(app.label_for(&app.media[0]), "People 001 + 003");
    assert!(!app.busy && !app.plan_open);
}

fn visual_fixture() -> (tempfile::TempDir, PhotoSortApp) {
    let dir = tempfile::tempdir().unwrap();
    let library = dir.path().join("library");
    std::fs::create_dir(&library).unwrap();
    for name in ["good.jpg", "unreadable.jpg", "clip.mov"] {
        std::fs::write(library.join(name), name).unwrap();
    }
    let catalog = Arc::new(Catalog::scan(&library, || false, |_| {}).unwrap());
    let mut app =
        PhotoSortApp::with_runtime(Runtime::with_cache(|| {}, dir.path().join("cache")), None);
    app.step = Step::Organize;
    app.group_mode = GroupMode::Visual;
    app.root = Some(library.clone());
    app.output = library.display().to_string();
    app.media = catalog.media.clone();
    app.index = app
        .media
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();
    app.catalog = Some(catalog);
    app.analysis_run = Some(41);
    app.organize_run = Some((41, GroupMode::Visual));
    (dir, app)
}
fn visual_completed(groups: HashMap<MediaId, usize>) -> Vec<Event> {
    vec![
        Event::GeneralGroups {
            automatic: Arc::default(),
            choices: Vec::new(),
            run: 41,
            groups,
        },
        Event::AnalysisDone {
            run: 41,
            stats: Default::default(),
        },
    ]
}
#[test]
fn visual_preview_ignores_technical_groups_waits_once_and_plans_only_verified_members() {
    let (_dir, mut app) = visual_fixture();
    let id = app
        .media
        .iter()
        .find(|m| m.name() == "good.jpg")
        .unwrap()
        .id;
    app.record_analysis_group(40, id, 9);
    assert!(!app.organize_grouping_ready() && !app.can_edit_folders());
    assert!(app.organize_labels().is_empty());
    app.organize();
    app.organize();
    assert_eq!(
        app.analysis_run,
        Some(41),
        "Repeated preview must reuse the accepted analysis"
    );
    assert!(app.organize_preview_requested && !app.busy && !app.plan_open);
    deliver(
        &mut app,
        vec![Event::GeneralGroups {
            automatic: Arc::default(),
            choices: Vec::new(),
            run: 40,
            groups: HashMap::from([(id, 7)]),
        }],
    );
    assert!(app.visual_groups.is_none());
    deliver(
        &mut app,
        vec![Event::GeneralGroups {
            automatic: Arc::default(),
            choices: Vec::new(),
            run: 41,
            groups: HashMap::from([(id, 3)]),
        }],
    );
    app.organize();
    assert!(!app.busy, "Publication must finish before planning");
    assert_eq!(
        app.organize_labels(),
        HashMap::from([(id, "Scene 003".into())])
    );
    deliver(
        &mut app,
        vec![Event::AnalysisDone {
            run: 41,
            stats: Default::default(),
        }],
    );
    let deadline = Instant::now() + worker_patience();
    while app.busy {
        assert!(Instant::now() < deadline);
        app.poll(&egui::Context::default());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.plan_open && !app.organize_preview_requested);
    let sources: Vec<_> = app
        .plan
        .as_ref()
        .unwrap()
        .actions
        .iter()
        .flat_map(|a| a.sources())
        .collect();
    assert_eq!(sources.len(), 1);
    assert_eq!(sources[0].file_name().unwrap(), "good.jpg");
    assert_eq!(app.stats.decodes, 0, "Planning must not repeat image work");
    assert!(app.analysis_run.is_none());
}
#[test]
fn empty_visual_results_are_complete_and_never_move_videos_or_failed_photos() {
    let (_dir, mut app) = visual_fixture();
    deliver(&mut app, visual_completed(HashMap::new()));
    assert!(app.organize_grouping_ready());
    assert!(app.organize_labels().is_empty());
    assert!(
        app.media
            .iter()
            .all(|m| app.label_for(m).contains("kept in place"))
    );
    app.organize();
    assert!(
        app.busy && app.analysis_run.is_none(),
        "Empty complete results must not restart AI"
    );
    let deadline = Instant::now() + worker_patience();
    while app.busy {
        assert!(Instant::now() < deadline);
        app.poll(&egui::Context::default());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.plan.as_ref().unwrap().actions.is_empty());
    assert_eq!(app.stats.decodes, 0);
}
#[test]
fn visual_preview_cancellation_errors_and_departure_reject_late_open_requests() {
    for action in 0..4 {
        let (_dir, mut app) = visual_fixture();
        let id = app.media[0].id;
        app.visual_groups = Some(HashMap::from([(id, 8)]));
        app.organize();
        assert!(app.organize_preview_requested && !app.busy);
        match action {
            0 => app.cancel_analysis(),
            1 => deliver(&mut app, vec![Event::Error("test analysis failure".into())]),
            2 => {
                app.set_group_mode(GroupMode::Current);
                app.set_group_mode(GroupMode::Visual);
            }
            _ => {
                app.set_step(Step::Cull);
                app.set_step(Step::Organize);
            }
        }
        assert!(!app.organize_preview_requested);
        deliver(&mut app, visual_completed(HashMap::from([(id, 3)])));
        assert!(!app.busy && !app.plan_open);
        assert_eq!(
            app.visual_groups.as_ref().unwrap()[&id],
            if action < 2 { 8 } else { 3 }
        );
    }
}
#[test]
fn preview_starts_the_matching_mode_once_and_manual_drafts_need_no_model_work() {
    for mode in [GroupMode::Visual, GroupMode::Faces, GroupMode::Mixed] {
        let (_dir, mut app) = visual_fixture();
        app.group_mode = mode;
        app.analysis_run = None;
        app.organize_run = None;
        app.organize();
        let run = app.analysis_run.expect("Preview starts analysis");
        assert_eq!(app.organize_run, Some((run, mode)));
        app.organize();
        assert_eq!(app.analysis_run, Some(run));
        assert!(app.organize_preview_requested && !app.busy);
        app.cancel_analysis();
    }
    let (_dir, mut app) = visual_fixture();
    app.analysis_run = None;
    app.organize_run = None;
    let id = app
        .media
        .iter()
        .find(|m| m.name() == "good.jpg")
        .unwrap()
        .id;
    app.organize_drafts.insert(
        GroupMode::Visual,
        crate::core::organize::Draft::from_catalog(
            app.catalog.as_ref().unwrap(),
            HashMap::from([(id, "Chosen".into())]),
        ),
    );
    assert!(app.can_edit_folders());
    app.organize();
    assert!(app.busy && app.analysis_run.is_none());
}
fn completed() -> Vec<Event> {
    vec![
        Event::MixedGroups {
            run: 41,
            labels: HashMap::new(),
        },
        Event::AnalysisDone {
            run: 41,
            stats: Default::default(),
        },
    ]
}
#[test]
fn mixed_move_preview_waits_for_complete_analysis_and_then_uses_shared_planner() {
    let mut app = app();
    app.organize();
    assert!(app.organize_preview_requested);
    assert!(
        !app.busy && !app.plan_open,
        "Never preview provisional date groups"
    );
    deliver(&mut app, completed());
    assert!(app.mixed_labels.is_some());
    assert!(!app.organize_preview_requested);
    assert!(
        app.busy,
        "Completed groups dispatch to the existing file-plan worker"
    );
}
#[test]
fn cancelled_or_replaced_mixed_results_cannot_open_a_move_preview() {
    let mut app = app();
    app.organize();
    app.analysis_run = None;
    app.organize_run = None;
    app.organize_preview_requested = false;
    deliver(&mut app, completed());
    assert!(app.mixed_labels.is_none());
    assert!(!app.busy && !app.plan_open);
}
#[test]
fn switching_workflow_or_grouping_mode_does_not_open_a_late_preview() {
    for leave_workflow in [false, true] {
        let mut app = app();
        app.organize();
        if leave_workflow {
            app.step = Step::Cull;
        } else {
            app.group_mode = GroupMode::Current;
        }
        deliver(&mut app, completed());
        assert!(
            app.mixed_labels.is_some(),
            "Completed reusable results are retained"
        );
        assert!(!app.busy && !app.plan_open);
        assert!(!app.organize_preview_requested);
    }
}
#[test]
fn current_folders_default_to_in_place_and_do_not_invent_root_moves() {
    let folder = tempfile::tempdir().unwrap();
    std::fs::create_dir(folder.path().join("nested")).unwrap();
    for name in ["root.jpg", "nested/child.jpg"] {
        image::RgbImage::new(4, 4)
            .save(folder.path().join(name))
            .unwrap();
    }
    let mut app = PhotoSortApp::with_runtime(Runtime::new(|| {}), None);
    app.open_now(folder.path().to_owned());
    assert_eq!(PathBuf::from(&app.output), folder.path());
    let ctx = egui::Context::default();
    theme_install(&ctx);
    let deadline = Instant::now() + worker_patience();
    while app.scanning && Instant::now() < deadline {
        app.poll(&ctx);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(!app.scanning && app.catalog.is_some());
    let labels: HashMap<_, _> = app.media.iter().map(|m| (m.id, app.label_for(m))).collect();
    assert!(labels.values().any(String::is_empty));
    assert!(labels.values().any(|label| label == "nested"));
    let plan = crate::core::operations::organize_plan(
        app.catalog.as_ref().unwrap(),
        &labels,
        &PathBuf::from(&app.output),
        true,
    )
    .unwrap();
    assert!(
        plan.actions.is_empty(),
        "Current folders must not stage moves without edits"
    );
}
#[test]
fn a_failure_from_a_superseded_run_does_not_stop_the_current_run() {
    let (_dir, mut app) = visual_fixture();
    app.analysis_run = None;
    app.organize_run = None;
    app.organize();
    let run = app.analysis_run.expect("Preview starts analysis");
    let stale = Event::RunError {
        run: run.wrapping_sub(1),
        error: "Similarity grouping failed: stale".into(),
    };
    deliver(&mut app, vec![stale]);
    assert_eq!(app.analysis_run, Some(run));
    assert!(app.organize_preview_requested && app.error.is_none());
    let current = Event::RunError {
        run,
        error: "Similarity grouping failed: current".into(),
    };
    deliver(&mut app, vec![current]);
    assert!(app.analysis_run.is_none() && !app.organize_preview_requested);
    assert_eq!(
        app.error.as_deref(),
        Some("Similarity grouping failed: current")
    );
}
fn face_groups(labels: &[(MediaId, usize, &str)]) -> crate::core::regional::grouping::Output {
    crate::core::regional::grouping::Output {
        groups: labels.iter().map(|(id, group, _)| (*id, *group)).collect(),
        folders: labels
            .iter()
            .map(|(id, _, label)| (*id, (*label).to_owned()))
            .collect(),
    }
}
fn faces_done(run: u64, groups: crate::core::regional::grouping::Output) -> Vec<Event> {
    vec![
        Event::FaceGroups { run, groups },
        Event::AnalysisDone {
            run,
            stats: Default::default(),
        },
    ]
}
#[test]
fn choosing_faces_starts_finding_faces_with_progress_or_says_why_it_waits() {
    // The reported bug: choosing Faces changed the mode and nothing else.
    let (_dir, mut app) = visual_fixture();
    app.group_mode = GroupMode::Current;
    app.analysis_run = None;
    app.organize_run = None;
    app.set_group_mode(GroupMode::Faces);
    let run = app
        .analysis_run
        .expect("choosing Faces starts finding faces");
    assert_eq!(app.organize_run, Some((run, GroupMode::Faces)));
    assert!(
        !app.organize_preview_requested,
        "choosing a mode only groups"
    );
    assert_eq!(app.workflow_status(), "Finding faces · 0 of 3");
    deliver(
        &mut app,
        vec![Event::Progress {
            run,
            done: 2,
            total: 3,
        }],
    );
    assert_eq!(app.workflow_status(), "Finding faces · 2 of 3");
    // Choosing it again while it runs does not restart it.
    app.set_group_mode(GroupMode::Faces);
    assert_eq!(app.analysis_run, Some(run));
    app.cancel_analysis();
    assert_eq!(app.organize_halted, Some(GroupMode::Faces));
    // Entering Organize again does not restart a cancelled grouping...
    app.set_step(Step::Organize);
    assert!(app.analysis_run.is_none());
    // ...but choosing it again does.
    app.set_group_mode(GroupMode::Faces);
    assert!(app.analysis_run.is_some());
    app.cancel_analysis();

    // Another analysis is running: Faces waits for it and then starts.
    let (_dir, mut app) = visual_fixture();
    app.group_mode = GroupMode::Current;
    app.organize_run = None;
    app.pick_run = Some(41);
    app.set_group_mode(GroupMode::Faces);
    assert_eq!(app.analysis_run, Some(41));
    assert_eq!(
        app.workflow_status(),
        "Grouping by faces starts when the current analysis finishes"
    );
    app.pick_run = None;
    deliver(
        &mut app,
        vec![Event::AnalysisDone {
            run: 41,
            stats: Default::default(),
        }],
    );
    let run = app.analysis_run.expect("Faces starts after the other run");
    assert_eq!(app.organize_run, Some((run, GroupMode::Faces)));
    app.cancel_analysis();

    // Missing models: say so and point to AI models instead of staying silent.
    let (_dir, mut app) = visual_fixture();
    app.group_mode = GroupMode::Current;
    app.analysis_run = None;
    app.organize_run = None;
    app.model_issue = Some(crate::core::model_assets::Issue {
        missing: vec![crate::core::model_assets::Id::People],
        ..Default::default()
    });
    app.set_group_mode(GroupMode::Faces);
    assert!(app.analysis_run.is_none());
    assert_eq!(
        app.workflow_status(),
        "Grouping by faces needs AI models · open AI models to set them up"
    );
    // Models reported missing by the worker after the start.
    let (_dir, mut app) = visual_fixture();
    app.group_mode = GroupMode::Faces;
    app.organize_run = Some((41, GroupMode::Faces));
    app.models_needed(
        41,
        crate::core::model_assets::Issue {
            missing: vec![crate::core::model_assets::Id::People],
            ..Default::default()
        },
    );
    assert!(app.model_dialog && app.analysis_run.is_none());
    assert_eq!(
        app.workflow_status(),
        "Grouping by faces needs AI models · download them in AI models"
    );
    app.runtime.shutdown();
}
#[test]
fn finished_face_grouping_says_when_nothing_or_nobody_repeats() {
    let (_dir, mut app) = visual_fixture();
    app.group_mode = GroupMode::Faces;
    app.organize_run = Some((41, GroupMode::Faces));
    app.organize_preview_requested = true;
    let photos: Vec<&Media> = app
        .media
        .iter()
        .filter(|m| m.kind == MediaKind::Photo)
        .collect();
    let ids: Vec<MediaId> = photos.iter().map(|m| m.id).collect();
    let first = photos[0].clone();
    let none = face_groups(&[(ids[0], 0, "Unassigned"), (ids[1], 0, "Unassigned")]);
    deliver(&mut app, faces_done(41, none));
    assert_eq!(app.workflow_status(), "No faces found in these photos");
    assert!(
        app.organize_labels().is_empty(),
        "photos without a recognized person stay in place"
    );
    assert_eq!(app.label_for(&first), "No person found · kept in place");
    assert!(
        !app.busy && !app.plan_open,
        "no preview when nothing would move"
    );
    assert!(
        app.analysis_run.is_none(),
        "a finished grouping is not rerun"
    );

    app.face_groups = None;
    app.analysis_run = Some(42);
    app.organize_run = Some((42, GroupMode::Faces));
    let some = face_groups(&[(ids[0], 1, "Person 001"), (ids[1], 0, "Unassigned")]);
    deliver(&mut app, faces_done(42, some));
    let labels = app.organize_labels();
    assert_eq!(labels.get(&ids[0]).map(String::as_str), Some("Person 001"));
    assert!(!labels.contains_key(&ids[1]), "only recognized people move");

    app.face_groups = None;
    app.analysis_run = Some(44);
    app.organize_run = Some((44, GroupMode::Faces));
    let once = face_groups(&[(ids[0], 1, "Person 001"), (ids[1], 2, "Person 002")]);
    deliver(&mut app, faces_done(42, once));
    assert_eq!(
        app.workflow_status(),
        "Faces found, but no person appears in more than one photo"
    );

    app.face_groups = None;
    app.analysis_run = Some(43);
    app.organize_run = Some((43, GroupMode::Faces));
    let twice = face_groups(&[(ids[0], 1, "Person 001"), (ids[1], 3, "People 001 + 002")]);
    deliver(&mut app, faces_done(43, twice));
    assert_eq!(
        app.workflow_status(),
        "People groups ready · 1 person in more than one photo"
    );
    app.runtime.shutdown();
}
#[test]
fn a_failed_face_grouping_shows_the_error_and_waits_to_be_chosen_again() {
    let (_dir, mut app) = visual_fixture();
    app.group_mode = GroupMode::Faces;
    app.organize_run = Some((41, GroupMode::Faces));
    deliver(
        &mut app,
        vec![Event::RunError {
            run: 41,
            error: "Similarity grouping failed: broken".into(),
        }],
    );
    assert_eq!(
        app.error.as_deref(),
        Some("Similarity grouping failed: broken")
    );
    assert_eq!(
        app.workflow_status(),
        "Grouping by faces failed · choose it again to retry"
    );
    assert!(app.analysis_run.is_none() && app.organize_run.is_none());
    // Returning to Organize does not repeat the failure on its own.
    app.error = None;
    app.set_step(Step::Organize);
    assert!(app.analysis_run.is_none() && app.error.is_none());
    app.set_group_mode(GroupMode::Faces);
    let run = app.analysis_run.expect("choosing Faces again retries");
    assert_eq!(app.organize_run, Some((run, GroupMode::Faces)));
    app.cancel_analysis();
    app.runtime.shutdown();
}
#[test]
fn photos_without_a_recognized_person_stay_or_go_to_one_folder_as_chosen() {
    use crate::core::preferences::{Faceless, NO_PEOPLE};
    let (_dir, mut app) = visual_fixture();
    app.group_mode = GroupMode::Faces;
    app.analysis_run = None;
    app.organize_run = None;
    let photos: Vec<Media> = app
        .media
        .iter()
        .filter(|m| m.kind == MediaKind::Photo)
        .cloned()
        .collect();
    let video = app
        .media
        .iter()
        .find(|m| m.kind == MediaKind::Video)
        .unwrap()
        .id;
    app.face_groups = Some(face_groups(&[
        (photos[0].id, 1, "Person 001"),
        (photos[1].id, 0, "Unassigned"),
    ]));
    // By default they stay in their folders, and the preview says why on hover.
    assert_eq!(app.preferences.faceless, Faceless::Stay);
    let labels = app.organize_labels();
    assert_eq!(
        labels.get(&photos[0].id).map(String::as_str),
        Some("Person 001")
    );
    assert!(!labels.contains_key(&photos[1].id));
    assert_eq!(app.unplaced_reason(&photos[1]), "no recognized person");
    assert!(!app.placement(&photos[1]).moves);
    assert_eq!(
        app.unplaced_phrase(1, 1),
        "1 photo without a recognized person and 1 video"
    );
    // Chosen: one "No people" folder; videos still stay.
    app.change_faceless(Faceless::Folder);
    assert_eq!(app.preferences.faceless, Faceless::Folder);
    let labels = app.organize_labels();
    assert_eq!(
        labels.get(&photos[1].id).map(String::as_str),
        Some(NO_PEOPLE)
    );
    assert!(!labels.contains_key(&video));
    let placement = app.placement(&photos[1]);
    assert!(placement.moves && placement.folder.ends_with(NO_PEOPLE));
    // With folder edits, switching back asks before replacing them.
    app.organize_drafts.insert(
        GroupMode::Faces,
        crate::core::organize::Draft::new(app.organize_labels()),
    );
    app.change_faceless(Faceless::Stay);
    assert_eq!(app.pending_faceless, Some(Faceless::Stay));
    assert_eq!(app.preferences.faceless, Faceless::Folder);
    assert!(app.organize_drafts.contains_key(&GroupMode::Faces));
    app.accept_faceless(Faceless::Stay);
    app.pending_faceless = None;
    assert_eq!(app.preferences.faceless, Faceless::Stay);
    assert!(!app.organize_drafts.contains_key(&GroupMode::Faces));
    assert!(!app.organize_labels().contains_key(&photos[1].id));
    // Nothing on disk moved.
    assert!(app.media.iter().all(|m| m.path.exists()));
    app.runtime.shutdown();
}
#[test]
fn the_no_people_folder_reaches_the_reviewed_plan_and_nothing_moves_on_disk() {
    use crate::core::{
        operations::Action,
        preferences::{Faceless, NO_PEOPLE},
    };
    let (_dir, mut app) = visual_fixture();
    app.group_mode = GroupMode::Faces;
    app.analysis_run = None;
    app.organize_run = None;
    let photos: Vec<Media> = app
        .media
        .iter()
        .filter(|m| m.kind == MediaKind::Photo)
        .cloned()
        .collect();
    app.face_groups = Some(face_groups(&[
        (photos[0].id, 1, "Person 001"),
        (photos[1].id, 0, "Unassigned"),
    ]));
    app.preferences.faceless = Faceless::Folder;
    app.organize();
    let deadline = Instant::now() + worker_patience();
    while app.busy {
        assert!(Instant::now() < deadline);
        app.poll(&egui::Context::default());
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(app.plan_open && app.error.is_none(), "{:?}", app.error);
    let moves: Vec<(PathBuf, PathBuf)> = app
        .plan
        .as_ref()
        .unwrap()
        .actions
        .iter()
        .flat_map(|action| match action {
            Action::Move { files } => files
                .iter()
                .map(|(source, target)| (source.path.clone(), target.clone()))
                .collect(),
            _ => Vec::new(),
        })
        .collect();
    // Temporary folders may be spelled through a symlink (/var and
    // /private/var on macOS), so compare the last folder and file names.
    let lands = |photo: &Media, folder: &str| {
        let name = photo.path.file_name().unwrap();
        moves.iter().any(|(source, target)| {
            source.file_name() == Some(name)
                && target.file_name() == Some(name)
                && target.parent().and_then(|p| p.file_name()) == Some(folder.as_ref())
        })
    };
    assert!(lands(&photos[1], NO_PEOPLE));
    assert!(lands(&photos[0], "Person 001"));
    let root = PathBuf::from(&app.output);
    assert!(
        moves
            .iter()
            .all(|(source, _)| source.extension().is_none_or(|x| x != "mov")),
        "videos stay in their folders"
    );
    assert!(app.media.iter().all(|m| m.path.exists()));
    assert!(!root.join(NO_PEOPLE).exists());
    app.runtime.shutdown();
}
