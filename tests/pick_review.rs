use photosort::core::{
    model::*,
    pick_review::{Cluster, Review},
};
use std::collections::HashMap;
fn id(n: u64) -> MediaId {
    MediaId(n)
}
/// A group where a higher number scores higher.
fn cluster(key: usize, ids: &[u64]) -> Cluster {
    Cluster::new(
        key,
        ids.iter().copied().map(id).collect(),
        ids.iter().map(|n| (id(*n), *n as f64)).collect(),
    )
}
fn review(clusters: Vec<Cluster>) -> Review {
    let mut r = Review::default();
    r.replace(clusters);
    r
}
fn rejected(d: &HashMap<MediaId, Decision>, n: u64) -> bool {
    d.get(&id(n)).is_some_and(|d| d.rejected)
}
#[test]
fn the_best_photo_is_kept_and_the_rest_are_staged_for_trash() {
    let mut r = review(vec![cluster(1, &[1, 2, 3]), cluster(2, &[4, 5])]);
    let mut d = HashMap::new();
    let marked = r.stage_new_suggestions(&mut d);
    assert_eq!(marked.len(), 3);
    assert!(rejected(&d, 1) && rejected(&d, 2) && !rejected(&d, 3));
    assert!(rejected(&d, 4) && !rejected(&d, 5));
    assert_eq!(r.group_of(id(4)), Some(1));
    assert_eq!(r.changed_count(&[0, 1], &d), 0);
}
#[test]
fn a_new_analysis_never_touches_staged_choices() {
    let mut r = review(vec![cluster(1, &[1, 2])]);
    let mut d = HashMap::new();
    r.stage_new_suggestions(&mut d);
    r.toggle(id(1), &mut d);
    let before = d.clone();
    // Same groups, and new ones that regroup the photos.
    r.replace(vec![cluster(1, &[1, 2])]);
    r.replace(vec![cluster(1, &[1, 2, 3])]);
    assert_eq!(d, before);
    // Only the photo never seen before is staged.
    assert_eq!(r.stage_new_suggestions(&mut d), Vec::<MediaId>::new());
    assert_eq!(d, before, "3 is the best photo: it stays kept");
}
#[test]
fn undo_and_accept_all_restore_the_suggestions() {
    let r = review(vec![cluster(1, &[1, 2, 3]), cluster(2, &[4, 5])]);
    let mut d = HashMap::new();
    r.keep_only(id(1), &mut d);
    r.toggle(id(5), &mut d);
    assert_eq!(r.changed_count(&[0, 1], &d), 2);
    r.accept(&[0], &mut d);
    assert!(!r.changed(0, &d) && r.changed(1, &d));
    r.accept(&[0, 1], &mut d);
    assert!(rejected(&d, 1) && rejected(&d, 2) && !rejected(&d, 3));
    assert!(rejected(&d, 4) && !rejected(&d, 5));
}
#[test]
fn groups_with_an_unscored_photo_are_left_out() {
    let mut broken = cluster(1, &[1, 2]);
    broken.scores.insert(id(2), f64::NAN);
    let r = review(vec![broken, cluster(2, &[3])]);
    assert!(r.clusters().is_empty());
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
fn adapter_requires_complete_quality_and_never_suggests_a_soft_photo() {
    use photosort::core::{analysis::Analysis, faces::FaceSummary, pick_review};
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
    analyses.get_mut(&id(3)).unwrap().score = 40.;
    let groups = media.iter().map(|m| (m.id, 1)).collect();
    let cs = pick_review::from_analysis(&media, &analyses, &groups, &HashMap::new());
    assert_eq!(cs[0].ordered, vec![id(3), id(1), id(4), id(2)]);
    assert_eq!(cs[0].reason, pick_review::Reason::Score);
    assert!(cs[0].soft.contains(&id(2)));
    analyses.get_mut(&id(4)).unwrap().aesthetic = None;
    assert!(pick_review::from_analysis(&media, &analyses, &groups, &HashMap::new()).is_empty());
}
