use photosort::core::{catalog::Catalog, model::*, operations, rotation::Review};
use std::collections::HashSet;
fn id(n: u64) -> MediaId {
    MediaId(n)
}
fn queue() -> Review {
    let mut r = Review::default();
    r.replace_suggestions(vec![(id(1), 1), (id(2), 2), (id(3), 3), (id(4), 0)]);
    r
}
#[test]
fn suggestions_and_manual_previews_never_queue_files_without_confirmation() {
    let mut r = queue();
    assert_eq!(r.ids(), &[id(1), id(2), id(3)]);
    assert_eq!(r.choice(id(3)).unwrap().degrees(), 270);
    assert!(r.pending().is_empty());
    assert!(!r.unfinished(Some(id(1))));
    r.rotate(id(1), 1);
    assert!(r.unfinished(Some(id(1))));
    assert!(r.pending().is_empty());
    assert_eq!(r.confirm(id(1)), Some(id(2)));
    assert_eq!(r.pending()[&id(1)].turns, 2);
    assert!(!r.unfinished(Some(id(1))));
    assert_eq!(r.confirm(id(1)), None);
    assert!(r.pending().is_empty());
    assert!(r.unfinished(Some(id(1))));
}
#[test]
fn selecting_original_and_zero_degree_choices_follow_reference_rules() {
    let mut r = queue();
    r.select(id(1), false);
    r.confirm(id(1));
    assert!(r.pending().is_empty());
    r.rotate(id(1), 3); // Original selection starts from 0, not the suggestion.
    assert_eq!(r.choice(id(1)).unwrap().turns(), 3);
    assert!(!r.choice(id(1)).unwrap().confirmed);
    r.rotate(id(1), 1);
    assert!(!r.choice(id(1)).unwrap().selected);
    assert_eq!(r.choice(id(1)).unwrap().turns(), 0);
    r.select(id(1), true); // Re-selecting correction restores the suggested angle.
    assert_eq!(r.choice(id(1)).unwrap().turns(), 1);
    assert!(!r.choice(id(1)).unwrap().manual());
}
#[test]
fn confirm_skips_reviewed_items_without_wrapping_and_changes_revoke_confirmation() {
    let mut r = queue();
    assert_eq!(r.confirm(id(2)), Some(id(3)));
    assert_eq!(r.confirm(id(1)), Some(id(3)));
    assert_eq!(r.confirm(id(3)), None);
    r.select(id(2), false);
    assert!(!r.pending().contains_key(&id(2)));
    assert!(r.unfinished(Some(id(2))));
    r.reset(id(2));
    assert!(!r.unfinished(Some(id(2))));
    r.rotate(id(1), 3);
    assert_eq!(r.choice(id(1)).unwrap().turns(), 0);
    r.confirm_all();
    assert_eq!(r.pending().len(), 3);
    assert_eq!(r.pending()[&id(1)].turns, 1);
    r.reset_all();
    assert!(r.pending().is_empty());
    assert!(r.ids().iter().all(|id| !r.choice(*id).unwrap().manual()));
}
#[test]
fn identical_results_preserve_review_and_successes_only_are_removed() {
    let mut r = queue();
    r.rotate(id(2), 1);
    r.confirm(id(2));
    r.replace_suggestions(vec![(id(1), 1), (id(2), 2), (id(3), 3)]);
    assert_eq!(r.pending()[&id(2)].turns, 3);
    r.confirm(id(1));
    r.complete(&HashSet::from([id(1)]));
    assert_eq!(r.ids(), &[id(2), id(3)]);
    assert_eq!(r.pending()[&id(2)].turns, 3); // failed / unsubmitted correction retained
    r.discard();
    assert!(r.pending().is_empty());
    assert!(!r.choice(id(2)).unwrap().selected);
    r.reset_all();
    assert!(r.choice(id(2)).unwrap().selected);
}
#[test]
fn only_confirmed_selected_corrections_reach_shared_file_planner() {
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
    r.confirm(ids[0]);
    r.select(ids[1], false);
    r.confirm(ids[1]);
    r.rotate(ids[2], 1); // unconfirmed manual preview must not leak into a write
    let plan = operations::decisions_plan(&catalog, &r.pending(), false).unwrap();
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
    r.complete(
        &report
            .completed
            .iter()
            .flat_map(|a| a.sources())
            .map(MediaId::for_path)
            .collect(),
    );
    assert!(r.pending().is_empty());
    assert!(r.choice(ids[0]).is_none());
    assert!(r.choice(ids[1]).is_some());
    assert!(r.unfinished(Some(ids[2])));
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
    review.confirm_all();
    let plan = operations::decisions_plan(&catalog, &review.pending(), false).unwrap();
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
    review.confirm_all();
    let plan = operations::decisions_plan(&catalog, &review.pending(), false).unwrap();
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
    assert_eq!(review.pending()[&catalog.media[1].id].turns, 1);
}

#[test]
fn uncertain_predictions_start_original_and_require_individual_confirmation() {
    let mut review = Review::default();
    review.replace_predictions(vec![(id(1), 2, true), (id(2), 1, false)]);
    assert!(!review.choice(id(1)).unwrap().selected);
    assert!(!review.unfinished(Some(id(1))));
    review.confirm_all();
    assert_eq!(review.pending().len(), 1);
    assert!(!review.choice(id(1)).unwrap().confirmed);
    review.select(id(1), true);
    assert!(review.unfinished(Some(id(1))));
    review.confirm_all();
    assert!(
        !review.pending().contains_key(&id(1)),
        "Bulk confirmation must not approve uncertain manual choices"
    );
    review.confirm(id(1));
    assert_eq!(review.pending()[&id(1)].turns, 2);
    review.replace_predictions(vec![(id(1), 2, true), (id(2), 1, false)]);
    assert_eq!(review.pending()[&id(1)].turns, 2);
    review.reset_all();
    assert!(!review.choice(id(1)).unwrap().selected);
    assert!(review.choice(id(2)).unwrap().selected);
    assert!(!review.unfinished(Some(id(1))));
    review.rotate(id(1), 1);
    assert_eq!(
        review.choice(id(1)).unwrap().turns(),
        1,
        "Manual rotation starts from original"
    );
    review.reset(id(1));
    review.confirm(id(1));
    assert!(
        !review.pending().contains_key(&id(1)),
        "Confirming original must not write"
    );
    review.replace_predictions(vec![(id(1), 2, false), (id(2), 1, false)]);
    assert!(review.choice(id(1)).unwrap().selected);
    assert!(
        !review.choice(id(1)).unwrap().confirmed,
        "Changed certainty invalidates old decisions"
    );
}
