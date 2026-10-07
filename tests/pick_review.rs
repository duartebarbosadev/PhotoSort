use photosort::core::{
    model::*,
    pick_review::{Cluster, Review},
};
use std::collections::HashMap;
fn id(n: u64) -> MediaId {
    MediaId(n)
}
fn cluster(key: usize, ids: &[u64]) -> Cluster {
    Cluster {
        key,
        ordered: ids.iter().copied().map(id).collect(),
        scores: ids.iter().map(|n| (id(*n), *n as f64)).collect(),
    }
}
fn review(ids: &[u64]) -> Review {
    let mut r = Review::default();
    r.replace(vec![cluster(1, ids)], &mut HashMap::new());
    r
}
fn rejected(d: &HashMap<MediaId, Decision>, n: u64) -> bool {
    d.get(&id(n)).is_some_and(|d| d.rejected)
}
#[test]
fn local_choices_stay_provisional_and_confirmed_pair_advances_without_starring() {
    let mut r = review(&[1, 2, 3]);
    let mut d = HashMap::new();
    assert_eq!(r.comparison().unwrap().ai_pick, id(2));
    r.toggle(id(1), &mut d);
    assert!(d.is_empty());
    assert!(r.unfinished());
    assert!(r.confirm(false, &mut d));
    assert!(!rejected(&d, 1));
    assert!(!rejected(&d, 2));
    assert_eq!(r.comparison().unwrap().paths, vec![id(2), id(3)]);
    assert!(
        r.comparison().unwrap().keeps(id(2)),
        "Carry explicit keep into the next matchup"
    );
    assert!(d.values().all(|d| d.rating.is_none()));
    assert!(!r.any_completed());
    r.confirm(false, &mut d);
    assert!(r.any_completed());
    assert!(
        !r.confirm(false, &mut d),
        "Repeated confirm must not undo the decision"
    );
}
#[test]
fn rejecting_both_starts_a_fresh_pair_if_two_challengers_remain() {
    let mut r = review(&[1, 2, 3, 4, 5]);
    assert_eq!(r.current().unwrap().upcoming(), &[id(3), id(4), id(5)]);
    let mut d = HashMap::new();
    r.toggle(id(2), &mut d);
    r.confirm(false, &mut d);
    assert!(rejected(&d, 1) && rejected(&d, 2));
    assert_eq!(r.comparison().unwrap().paths, vec![id(3), id(4)]);
    assert_eq!(r.current().unwrap().upcoming(), &[id(5)]);
    r.confirm(true, &mut d);
    assert_eq!(r.comparison().unwrap().paths, vec![id(4), id(5)]);
    assert!(r.comparison().unwrap().keeps(id(4)));
    assert!(!rejected(&d, 3));
}
#[test]
fn a_last_challenger_does_not_resurrect_a_rejected_carrier() {
    let mut r = review(&[4, 5, 1]);
    let mut d = HashMap::new();
    r.toggle(id(5), &mut d);
    r.confirm(false, &mut d);
    let c = r.comparison().unwrap();
    assert_eq!(c.ai_pick, id(5));
    assert!(!c.keeps(id(5)));
    assert!(!c.keeps(id(1)));
    r.confirm(false, &mut d);
    assert!(r.any_completed());
    assert!([1, 4, 5].iter().all(|n| rejected(&d, *n)));
}
#[test]
fn editing_an_earlier_round_restores_prior_marks_and_invalidates_dependent_rounds() {
    let mut r = review(&[1, 2, 3, 4]);
    let mut d = HashMap::from([(
        id(4),
        Decision {
            rejected: true,
            rating: Some(3),
            turns: 2,
        },
    )]);
    r.confirm(false, &mut d);
    r.confirm(false, &mut d);
    r.confirm(false, &mut d);
    assert!(r.any_completed());
    assert!(r.select_comparison(1));
    r.toggle(id(2), &mut d);
    assert!(!r.any_completed());
    assert_eq!(r.current().unwrap().comparisons.len(), 2);
    assert_eq!(r.current().unwrap().upcoming(), &[id(4)]);
    assert!(rejected(&d, 1));
    assert!(!rejected(&d, 2));
    assert!(rejected(&d, 4));
    assert_eq!(d[&id(4)].rating, Some(3));
    assert_eq!(d[&id(4)].turns, 2);
    r.reset_current(&mut d);
    r.confirm(false, &mut d);
    assert!(
        r.comparison().unwrap().paths.contains(&id(4)),
        "The invalidated challenger must be reviewed again"
    );
}
#[test]
fn reset_all_restores_baseline_and_rebuilds_immutable_defaults() {
    let mut r = review(&[1, 2, 3]);
    let mut d = HashMap::from([(
        id(2),
        Decision {
            rejected: true,
            ..Default::default()
        },
    )]);
    r.confirm(true, &mut d);
    r.confirm(true, &mut d);
    r.reset_all(&mut d);
    assert!(rejected(&d, 2));
    assert!(!rejected(&d, 1));
    assert!(!rejected(&d, 3));
    assert_eq!(r.current().unwrap().comparisons.len(), 1);
    assert!(!r.unfinished());
    assert!(!r.any_completed());
}
#[test]
fn completion_advances_to_next_unfinished_cluster_and_navigation_guards_edits() {
    let mut r = Review::default();
    let mut d = HashMap::new();
    r.replace(vec![cluster(1, &[1, 2]), cluster(2, &[3, 4])], &mut d);
    r.toggle(id(1), &mut d);
    assert!(!r.select_cluster(1));
    r.confirm(false, &mut d);
    assert_eq!(r.current_index(), 1);
    r.confirm(false, &mut d);
    assert!(r.tournaments().iter().all(|t| t.finalized));
    r.select_cluster(0);
    r.reset_current(&mut d);
    assert!(!r.current().unwrap().finalized);
}
#[test]
fn identical_results_preserve_review_but_replacement_restores_old_marks() {
    let mut r = review(&[1, 2]);
    let mut d = HashMap::new();
    r.confirm(false, &mut d);
    r.replace(vec![cluster(1, &[1, 2])], &mut d);
    assert!(r.any_completed());
    r.replace(vec![cluster(2, &[3, 4])], &mut d);
    assert!(!rejected(&d, 1));
    assert!(!r.any_completed());
}
#[test]
fn shared_discard_and_external_decisions_cannot_replay_stale_undo() {
    let mut r = review(&[1, 2]);
    let mut d = HashMap::new();
    r.confirm(false, &mut d);
    d.get_mut(&id(1)).unwrap().rejected = false;
    d.get_mut(&id(2)).unwrap().rejected = true;
    r.reconcile_external(&d);
    r.reset_all(&mut d);
    assert!(!rejected(&d, 1));
    assert!(rejected(&d, 2));
    r.confirm(false, &mut d);
    d.clear();
    r.discard_confirmations();
    r.reset_all(&mut d);
    assert!(d.is_empty());
}
#[test]
fn whole_image_sharpness_uses_reflected_borders_and_keeps_featureless_images_at_zero() {
    use photosort::core::{analysis, pipeline::Asset};
    use std::sync::Arc;
    let pixels = Arc::new(image::RgbaImage::from_fn(3, 3, |x, y| {
        let v = if x == 1 && y == 1 { 255 } else { 0 };
        image::Rgba([v, v, v, 255])
    }));
    let asset = Asset {
        decoder_warning: None,
        pixels: pixels.clone(),
        thumbnail: pixels,
        metadata: Metadata::default(),
    };
    assert!((analysis::quality_sharpness(&asset) - 218355.5555555).abs() < 0.001);
}
#[test]
fn adapter_requires_complete_quality_and_reuses_anchor_embeddings_and_cluster_gate() {
    use photosort::core::{analysis::Analysis, faces::FaceSummary, pick_review};
    use std::sync::Arc;
    let media: Vec<_> = (1..=4)
        .map(|n| Media {
            id: id(n),
            path: format!("/{n}.jpg").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        })
        .collect();
    let mut analyses: HashMap<_, _> = media
        .iter()
        .map(|m| {
            (
                m.id,
                Analysis {
                    aesthetic: Some(0.5),
                    faces: Some(FaceSummary::default()),
                    quality_sharpness: Some(100.),
                    score: 30.,
                    ..Default::default()
                },
            )
        })
        .collect();
    analyses.get_mut(&id(2)).unwrap().quality_sharpness = Some(20.);
    analyses.get_mut(&id(2)).unwrap().score = 90.;
    let groups = media.iter().map(|m| (m.id, 1)).collect();
    let embeddings = HashMap::from([
        (id(1), Arc::new(vec![1., 0.])),
        (id(2), Arc::new(vec![0., 1.])),
        (id(3), Arc::new(vec![0.9, 0.1])),
    ]);
    let cs = pick_review::from_analysis(&media, &analyses, &groups, &HashMap::new(), &embeddings);
    assert_eq!(cs[0].ordered, vec![id(1), id(3), id(2), id(4)]);
    assert!(cs[0].scores[&id(2)] < cs[0].scores[&id(3)]);
    analyses.get_mut(&id(4)).unwrap().aesthetic = None;
    assert!(
        pick_review::from_analysis(&media, &analyses, &groups, &HashMap::new(), &embeddings)
            .is_empty()
    );
}

#[test]
fn saved_pair_restores_fresh_review_without_replaying_discarded_marks() {
    let mut original = review(&[1, 2, 3, 4]);
    let mut decisions = HashMap::new();
    original.confirm(false, &mut decisions);
    original.confirm(false, &mut decisions);
    let pair = original.comparison().unwrap().paths.clone();
    assert_eq!(pair, vec![id(3), id(4)]);
    decisions.clear();
    original.discard_confirmations();
    let mut reopened = review(&[1, 2, 3, 4]);
    reopened.restore_pair(Some(id(4)), &pair);
    assert_eq!(reopened.comparison().unwrap().paths, pair);
    assert!(!reopened.comparison().unwrap().confirmed);
    assert!(!reopened.any_completed());
    assert!(decisions.is_empty());
    assert_eq!(reopened.current().unwrap().upcoming(), &[id(1), id(2)]);
    reopened.confirm(false, &mut decisions);
    let before = decisions.clone();
    let current = reopened.comparison().unwrap().paths.clone();
    reopened.restore_pair(Some(id(4)), &[id(1), id(3)]);
    assert_eq!(
        reopened.comparison().unwrap().paths,
        current,
        "do not reorder an existing confirmed tournament"
    );
    assert_eq!(decisions, before);
}
