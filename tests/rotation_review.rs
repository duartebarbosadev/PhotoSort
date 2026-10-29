use photosort::core::rotation::{Review, set_turns, turns};
use photosort::core::{catalog::Catalog, model::*, operations};
use std::collections::{HashMap, HashSet};
fn id(n: u64) -> MediaId {
    MediaId(n)
}
fn queue() -> Review {
    let mut r = Review::default();
    r.replace_suggestions(vec![(id(1), 1), (id(2), 2), (id(3), 3), (id(4), 0)]);
    r
}
#[test]
fn suggestions_are_staged_once_and_photos_the_model_keeps_are_left_out() {
    let mut r = queue();
    assert_eq!(r.ids(), &[id(1), id(2), id(3)]);
    let mut decisions = HashMap::new();
    assert_eq!(r.stage_new_suggestions(&mut decisions).len(), 3);
    assert_eq!(turns(&decisions, id(3)), 3);
    assert!(!decisions.contains_key(&id(4)));
    assert!(r.ids().iter().all(|id| !r.changed(*id, &decisions)));
    // Turning right then back left is no change.
    set_turns(&mut decisions, id(1), 2);
    assert!(r.changed(id(1), &decisions));
    set_turns(&mut decisions, id(1), 1);
    assert!(!r.changed(id(1), &decisions));
    // Kept as it is: nothing staged, and a new detection respects it.
    set_turns(&mut decisions, id(2), 0);
    assert!(!decisions.contains_key(&id(2)));
    r.replace_suggestions(vec![(id(1), 1), (id(2), 2), (id(3), 3)]);
    assert!(r.stage_new_suggestions(&mut decisions).is_empty());
    assert!(!decisions.contains_key(&id(2)));
    r.undo(id(2), &mut decisions);
    assert_eq!(turns(&decisions, id(2)), 2);
}
#[test]
fn applied_suggestions_leave_the_review_and_failed_ones_stay() {
    let mut r = queue();
    r.stage_new_suggestions(&mut HashMap::new());
    r.complete(&HashSet::from([id(1)]));
    assert_eq!(r.ids(), &[id(2), id(3)]);
    assert!(r.suggestion(id(1)).is_none());
    assert_eq!(r.default_turns(id(2)), 2);
}
#[test]
fn only_staged_turns_reach_the_shared_file_planner() {
    let root = tempfile::tempdir().unwrap();
    for name in ["a.png", "b.png", "c.png"] {
        image::RgbImage::new(16, 8)
            .save(root.path().join(name))
            .unwrap();
    }
    let catalog = Catalog::scan(root.path(), || false, |_| {}).unwrap();
    let ids: Vec<_> = catalog.media.iter().map(|m| m.id).collect();
    let mut r = Review::default();
    r.replace_suggestions(ids.iter().map(|id| (*id, 1)).collect());
    let mut decisions = HashMap::new();
    r.stage_new_suggestions(&mut decisions);
    // Kept as it is, and turned and turned back: neither is written.
    set_turns(&mut decisions, ids[1], 0);
    set_turns(&mut decisions, ids[2], 2);
    set_turns(&mut decisions, ids[2], 0);
    let plan = operations::decisions_plan(&catalog, &decisions, false).unwrap();
    assert_eq!(plan.actions.len(), 1);
    match &plan.actions[0] {
        operations::Action::Metadata {
            source,
            orientation,
            rating,
            ..
        } => {
            assert_eq!(source.path, catalog.media[0].path);
            assert_eq!(*orientation, Some(6));
            assert_eq!(*rating, None);
        }
        _ => panic!("Rotation-only application must not move or trash photos"),
    }
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty());
    r.complete(&report.affected_media(&catalog.media));
    assert!(r.suggestion(ids[0]).is_none());
    assert!(r.suggestion(ids[1]).is_some());
}

#[test]
fn shared_sidecar_success_removes_both_companion_reviews() {
    let root = tempfile::tempdir().unwrap();
    for name in ["pair.png", "pair.jpg"] {
        image::RgbImage::new(16, 8)
            .save(root.path().join(name))
            .unwrap();
    }
    let catalog = Catalog::scan(root.path(), || false, |_| {}).unwrap();
    let mut review = Review::default();
    review.replace_suggestions(catalog.media.iter().map(|m| (m.id, 1)).collect());
    let mut decisions = HashMap::new();
    review.stage_new_suggestions(&mut decisions);
    let plan = operations::decisions_plan(&catalog, &decisions, false).unwrap();
    assert_eq!(
        plan.actions.len(),
        1,
        "Companions share one metadata operation"
    );
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty());
    let affected = report.affected_media(&catalog.media);
    assert_eq!(affected.len(), 2);
    review.complete(&affected);
    assert!(
        review.ids().is_empty(),
        "Neither companion may receive the same correction twice"
    );
}

#[test]
fn partial_application_keeps_failed_rotation_for_retry() {
    let root = tempfile::tempdir().unwrap();
    for name in ["a.png", "b.png"] {
        image::RgbImage::new(16, 8)
            .save(root.path().join(name))
            .unwrap();
    }
    let catalog = Catalog::scan(root.path(), || false, |_| {}).unwrap();
    let mut review = Review::default();
    review.replace_suggestions(catalog.media.iter().map(|m| (m.id, 1)).collect());
    let mut decisions = HashMap::new();
    review.stage_new_suggestions(&mut decisions);
    let plan = operations::decisions_plan(&catalog, &decisions, false).unwrap();
    std::fs::write(
        photosort::core::metadata::sidecar(&catalog.media[1].path),
        b"Changed externally after review",
    )
    .unwrap();
    let journal = tempfile::tempdir().unwrap();
    let report = operations::execute(&plan, journal.path(), || false, |_, _| {}).unwrap();
    assert_eq!(report.completed.len(), 1);
    assert_eq!(report.errors.len(), 1);
    review.complete(&report.affected_media(&catalog.media));
    assert_eq!(review.ids(), &[catalog.media[1].id]);
    assert_eq!(review.default_turns(catalog.media[1].id), 1);
}

#[test]
fn uncertain_predictions_are_listed_but_never_staged_on_their_own() {
    let mut review = Review::default();
    review.replace_predictions(vec![(id(1), 2, true), (id(2), 1, false)]);
    assert_eq!(review.ids(), &[id(1), id(2)]);
    let mut decisions = HashMap::new();
    review.stage_new_suggestions(&mut decisions);
    assert_eq!(turns(&decisions, id(1)), 0);
    assert_eq!(turns(&decisions, id(2)), 1);
    assert_eq!(review.default_turns(id(1)), 0);
    // A later, certain detection stages it once.
    review.replace_predictions(vec![(id(1), 2, false), (id(2), 1, false)]);
    review.stage_new_suggestions(&mut decisions);
    assert_eq!(turns(&decisions, id(1)), 2);
}
