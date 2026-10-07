use photosort::core::{
    catalog::Catalog,
    model::*,
    operations::{self, Action, Plan},
    session::Session,
};
use std::{cell::Cell, collections::HashMap, path::Path};
fn fixture() -> (tempfile::TempDir, Catalog, Plan) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir(root.join("Before")).unwrap();
    for name in ["a.jpg", "b.jpg"] {
        std::fs::write(root.join("Before").join(name), name).unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let labels = catalog
        .media
        .iter()
        .map(|m| (m.id, "After".into()))
        .collect();
    let plan = operations::organize_plan(&catalog, &labels, &root, true).unwrap();
    (dir, catalog, plan)
}
fn run(plan: &Plan, journals: &Path) -> operations::Report {
    operations::execute(plan, journals, || false, |_, _| {}).unwrap()
}
fn moves(plan: &Plan) -> Vec<(std::path::PathBuf, std::path::PathBuf)> {
    plan.actions
        .iter()
        .flat_map(|a| match a {
            Action::Move { files } => files
                .iter()
                .map(|(s, d)| (s.path.clone(), d.clone()))
                .collect(),
            _ => vec![],
        })
        .collect()
}
#[test]
fn cancellation_retains_exact_unattempted_moves_and_cleanup_dependencies() {
    let (_dir, c, plan) = fixture();
    let stop = Cell::new(false);
    let report = operations::execute(
        &plan,
        &c.root.join("journal"),
        || stop.get(),
        |done, _| stop.set(done == 1),
    )
    .unwrap();
    assert!(report.cancelled);
    assert_eq!(report.completed.len(), 1);
    assert_eq!(moves(&report.remaining), moves(&plan)[1..]);
    assert_eq!(report.remaining.output_root, plan.output_root);
    let retry = run(&report.remaining, &c.root.join("journal"));
    assert!(retry.errors.is_empty() && retry.remaining.actions.is_empty());
    assert!(!c.root.join("Before").exists());
    for name in ["a.jpg", "b.jpg"] {
        assert_eq!(
            std::fs::read_to_string(c.root.join("After").join(name)).unwrap(),
            name
        );
    }
}
#[test]
fn failed_move_does_not_repeat_successes_or_choose_a_new_collision_suffix() {
    let (_dir, c, plan) = fixture();
    std::fs::create_dir(c.root.join("After")).unwrap();
    let conflict = c.root.join("After/b.jpg");
    std::fs::write(&conflict, b"unrelated").unwrap();
    let report = run(&plan, &c.root.join("journal"));
    assert_eq!(report.errors.len(), 1);
    assert_eq!(
        moves(&report.remaining),
        vec![(c.root.join("Before/b.jpg"), conflict.clone())]
    );
    assert!(
        report
            .remaining
            .actions
            .iter()
            .any(|a| matches!(a, Action::RemoveEmptyDirectory { .. }))
    );
    let still_blocked = run(&report.remaining, &c.root.join("journal"));
    assert_eq!(std::fs::read(&conflict).unwrap(), b"unrelated");
    assert_eq!(still_blocked.errors.len(), 1);
    assert!(!c.root.join("After/b_1.jpg").exists());
    std::fs::remove_file(&conflict).unwrap();
    let retry = run(&still_blocked.remaining, &c.root.join("journal"));
    assert!(retry.errors.is_empty() && retry.remaining.actions.is_empty());
    assert!(!c.root.join("Before").exists());
    assert_eq!(std::fs::read_dir(c.root.join("After")).unwrap().count(), 2);
}
#[test]
fn retries_keep_original_source_fingerprints_and_companions_together() {
    let (_dir, c, _) = fixture();
    std::fs::write(c.root.join("Before/b.ARW"), b"raw").unwrap();
    std::fs::write(c.root.join("Before/b.xmp"), b"sidecar").unwrap();
    let c = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    let labels = c.media.iter().map(|m| (m.id, "After".into())).collect();
    let plan = operations::organize_plan(&c, &labels, &c.root, true).unwrap();
    std::fs::write(c.root.join("Before/b.jpg"), b"changed after review").unwrap();
    let report = run(&plan, &c.root.join("journal"));
    assert_eq!(report.errors.len(), 1);
    assert_eq!(moves(&report.remaining).len(), 3);
    let retry = run(&report.remaining, &c.root.join("journal"));
    assert_eq!(retry.errors.len(), 1);
    for name in ["b.jpg", "b.ARW", "b.xmp"] {
        assert!(c.root.join("Before").join(name).exists());
        assert!(!c.root.join("After").join(name).exists());
    }
}
#[test]
fn move_migrates_decisions_and_new_fingerprints_without_marking_them_applied() {
    let (_dir, c, plan) = fixture();
    let mut session = Session {
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: c
            .media
            .iter()
            .map(|m| {
                (
                    m.id,
                    Decision {
                        rating: Some(4),
                        rejected: true,
                        turns: 1,
                    },
                )
            })
            .collect(),
        fingerprints: c.media.iter().map(|m| (m.id, m.fingerprint)).collect(),
    };
    let report = run(&plan, &c.root.join("journal"));
    assert!(report.applied_decisions(&c.media).is_empty());
    session.apply_report(&report, &c.media);
    let after = Catalog::scan(&c.root, || false, |_| {}).unwrap();
    session.reconcile(&after.media);
    assert_eq!(session.decisions.len(), 2);
    for media in &after.media {
        assert_eq!(session.decisions[&media.id].rating, Some(4));
        assert!(session.decisions[&media.id].rejected);
        assert_eq!(session.decisions[&media.id].turns, 1);
    }
    let metadata = Plan {
        actions: vec![Action::Metadata {
            source: operations::Source::capture(after.media[0].path.clone()).unwrap(),
            sidecar_before: None,
            target: Default::default(),
            rating: Some(4),
            orientation: None,
        }],
        ..Default::default()
    };
    let applied = run(&metadata, &c.root.join("journal"));
    assert!(applied.errors.is_empty());
    session.apply_report(&applied, &after.media);
    assert_eq!(session.decisions.len(), 1);
}
#[test]
fn session_merge_preserves_preexisting_destination_decisions() {
    let (_dir, c, plan) = fixture();
    let report = run(&plan, &c.root.join("journal"));
    let mut moved = Session {
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: HashMap::from([(
            c.media[0].id,
            Decision {
                rating: Some(3),
                ..Default::default()
            },
        )]),
        fingerprints: Default::default(),
    };
    moved.apply_report(&report, &c.media);
    let existing = MediaId(900);
    let mut destination = Session {
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: HashMap::from([(
            existing,
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    destination.merge(moved);
    assert_eq!(destination.decisions.len(), 2);
    assert_eq!(destination.decisions[&existing].rating, Some(5));
}

#[test]
fn equivalent_folder_spellings_share_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let id = MediaId(7);
    let session = Session {
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: HashMap::from([(
            id,
            Decision {
                rating: Some(2),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    session.save(&root.join("")).unwrap();
    assert_eq!(Session::load(&root).unwrap().decisions[&id].rating, Some(2));
    assert_eq!(
        Session::load(&root.join(".")).unwrap().decisions[&id].rating,
        Some(2)
    );
}

#[test]
fn rotation_review_follows_successful_moves_without_losing_confirmation() {
    use photosort::core::rotation::Review;
    let (_dir, c, plan) = fixture();
    let id = c.media[0].id;
    let mut review = Review::default();
    review.ensure(id, 1);
    review.confirm(id);
    let report = run(&plan, &c.root.join("journal"));
    review.complete(&report.applied_decisions(&c.media));
    review.relocate(&report.relocated);
    let new_id = MediaId::for_path(&c.root.join("After/a.jpg"));
    assert_eq!(review.pending_count(), 1);
    assert!(review.choice(id).is_none());
    assert!(review.choice(new_id).unwrap().confirmed);
    assert_eq!(review.pending()[&new_id].turns, 1);
}

#[test]
fn shutdown_drains_accepted_source_and_destination_session_saves() {
    use photosort::core::jobs::{IoWork, Runtime};
    let source = tempfile::tempdir().unwrap();
    let target = tempfile::tempdir().unwrap();
    let id = MediaId(88);
    let session = Session {
        manual_groups: Default::default(),
        scene_groups: Default::default(),
        decisions: HashMap::from([(
            id,
            Decision {
                rating: Some(4),
                ..Default::default()
            },
        )]),
        ..Default::default()
    };
    // More scan batches than the event queue can hold guarantees the file
    // worker is occupied until shutdown cancels it; queued saves cannot race
    // ahead and make this test pass without actually draining them.
    for n in 0..1280 {
        std::fs::write(source.path().join(format!("{n}.jpg")), b"scan fixture").unwrap();
    }
    let mut runtime = Runtime::with_cache(|| {}, source.path().join("cache"));
    runtime
        .io(IoWork::Scan(source.path().to_path_buf()))
        .unwrap();
    runtime
        .events
        .recv_timeout(std::time::Duration::from_secs(5))
        .unwrap();
    runtime
        .io(IoWork::Save {
            root: source.path().to_path_buf(),
            session: session.clone(),
        })
        .unwrap();
    runtime
        .io(IoWork::MergeSession {
            root: target.path().to_path_buf(),
            session,
        })
        .unwrap();
    runtime.shutdown();
    assert_eq!(
        Session::load(source.path()).unwrap().decisions[&id].rating,
        Some(4)
    );
    assert_eq!(
        Session::load(target.path()).unwrap().decisions[&id].rating,
        Some(4)
    );
}
