use photosort::core::{
    analysis::{self, Analysis, Thresholds},
    delete_review::{self, Category, Entry, Review},
    model::*,
    pipeline::Asset,
};
use std::{collections::HashMap, sync::Arc};
fn id(n: u64) -> MediaId {
    MediaId(n)
}
fn entry(n: u64, pair: Option<u64>) -> Entry {
    Entry {
        subject: id(n),
        companion: pair.map(id),
        category: Category::Exact,
        reason: "copy".into(),
    }
}
fn queue() -> Review {
    let mut r = Review::default();
    r.replace(vec![entry(1, Some(2)), entry(3, None)]);
    r
}
fn rejected(d: &HashMap<MediaId, Decision>, n: u64) -> bool {
    d.get(&id(n)).is_some_and(|d| d.rejected)
}
#[test]
fn choices_are_provisional_and_revoking_restores_prior_marks_without_touching_metadata() {
    let mut r = queue();
    let mut d = HashMap::from([(
        id(2),
        Decision {
            rejected: true,
            rating: Some(4),
            turns: 2,
        },
    )]);
    let initial = d.clone();
    r.toggle(id(1), id(1), &mut d);
    assert_eq!(d, initial);
    assert!(r.unfinished(Some(id(1))));
    assert!(r.confirm(id(1), &mut d));
    assert!(!rejected(&d, 1));
    assert!(!rejected(&d, 2));
    assert_eq!(d[&id(2)].rating, Some(4));
    assert_eq!(d[&id(2)].turns, 2);
    assert!(!r.unfinished(Some(id(1))));
    assert!(!r.confirm(id(1), &mut d));
    assert!(rejected(&d, 2));
    assert!(r.unfinished(Some(id(1))));
    r.reset(id(1), &mut d);
    assert!(!r.unfinished(Some(id(1))));
    assert!(rejected(&d, 2));
}
#[test]
fn editing_confirmed_choices_revokes_before_changing_pending_state() {
    let mut r = queue();
    let mut d = HashMap::new();
    r.confirm(id(1), &mut d);
    assert!(rejected(&d, 1));
    r.toggle(id(1), id(2), &mut d);
    assert!(!r.confirmed(id(1)));
    assert!(!rejected(&d, 1));
    assert!(!rejected(&d, 2));
    assert!(!r.keeps(id(1), id(2)));
    r.confirm(id(1), &mut d);
    assert!(rejected(&d, 1));
    assert!(rejected(&d, 2));
}
#[test]
fn resetting_overlapping_confirmations_unwinds_in_reverse_order() {
    let mut r = Review::default();
    r.replace(vec![entry(1, Some(2)), entry(2, Some(3))]);
    let mut d = HashMap::from([(
        id(3),
        Decision {
            rejected: true,
            ..Default::default()
        },
    )]);
    r.confirm(id(1), &mut d);
    r.confirm(id(2), &mut d);
    assert!(rejected(&d, 2));
    assert!(!rejected(&d, 3));
    r.reset_all(&mut d);
    assert!(!rejected(&d, 1));
    assert!(!rejected(&d, 2));
    assert!(rejected(&d, 3));
    assert!(!r.confirmed(id(1)));
    assert!(!r.confirmed(id(2)));
}
#[test]
fn external_decisions_are_not_overwritten_by_stale_review_history() {
    let mut r = queue();
    let mut d = HashMap::new();
    r.confirm(id(1), &mut d);
    d.get_mut(&id(1)).unwrap().rejected = false;
    d.get_mut(&id(2)).unwrap().rejected = true;
    r.reconcile_external(&d);
    r.reset_all(&mut d);
    assert!(!rejected(&d, 1));
    assert!(rejected(&d, 2));
    assert!(!r.confirmed(id(1)));
}
#[test]
fn filtered_defaults_leave_other_reviews_alone_and_retain_prebatch_marks() {
    let mut r = Review::default();
    r.replace(vec![entry(1, Some(2)), entry(2, Some(3)), entry(4, None)]);
    let mut d = HashMap::new();
    r.accept_defaults(&[id(1), id(2)], &mut d);
    assert!(r.confirmed(id(1)));
    assert!(r.confirmed(id(2)));
    assert!(!r.confirmed(id(4)));
    assert!(rejected(&d, 1));
    assert!(rejected(&d, 2));
    assert!(!rejected(&d, 4));
    // Cancel the second row alone: its prior state predates the entire batch.
    r.confirm(id(2), &mut d);
    assert!(!rejected(&d, 2));
    r.reset_all(&mut d);
    assert!(d.values().all(Decision::is_empty));
}
#[test]
fn repeating_results_preserves_review_and_discard_forgets_old_undo_history() {
    let mut r = queue();
    let mut d = HashMap::new();
    r.confirm(id(1), &mut d);
    r.replace(vec![entry(1, Some(2)), entry(3, None)]);
    assert!(r.confirmed(id(1)));
    d.clear();
    r.discard_confirmations();
    r.reset_all(&mut d);
    assert!(d.is_empty());
    assert!(!r.confirmed(id(1)));
}
fn media(n: u64) -> Media {
    Media {
        id: id(n),
        path: format!("/{n}.jpg").into(),
        kind: MediaKind::Photo,
        fingerprint: Fingerprint::default(),
    }
}
fn pair(discard: MediaId, keep: MediaId) -> photosort::core::near_duplicates::batch::Pair {
    photosort::core::near_duplicates::batch::Pair {
        discard,
        keep,
        assessment: photosort::core::near_duplicates::Comparator::default().assess(
            None,
            None,
            true,
            |_| None,
            || false,
        ),
        reason: "Exact copy".into(),
    }
}
#[test]
fn duplicate_pairs_outrank_quality_issues_and_invalid_pairs_are_rejected() {
    let media: Vec<_> = (1..=5).map(media).collect();
    let mut a: HashMap<_, _> = media
        .iter()
        .map(|m| {
            (
                m.id,
                Analysis {
                    sharpness: 200.,
                    ..Default::default()
                },
            )
        })
        .collect();
    a.get_mut(&id(2)).unwrap().issues = vec!["Possibly soft".into()];
    a.get_mut(&id(2)).unwrap().sharpness = 10.;
    let rows = delete_review::from_analysis(
        &media,
        &a,
        &HashMap::new(),
        &[pair(id(3), id(3)), pair(id(4), id(99)), pair(id(2), id(1))],
    );
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].subject, id(2));
    assert_eq!(rows[0].companion, Some(id(1)));
    assert_ne!(rows[0].category, Category::Blur);
    assert!(
        rows[0]
            .reason
            .ends_with("· the suggested discard is also possibly soft"),
        "{}",
        rows[0].reason
    );
}
#[test]
fn capture_sorting_changes_order_without_changing_pair_ownership() {
    let media: Vec<_> = (1..=4).map(media).collect();
    let a = media.iter().map(|m| (m.id, Analysis::default())).collect();
    let pairs = [pair(id(1), id(2)), pair(id(2), id(1)), pair(id(3), id(4))];
    let mut meta = HashMap::new();
    meta.insert(
        id(2),
        Metadata {
            captured: "2000:01:01".into(),
            ..Default::default()
        },
    );
    meta.insert(
        id(3),
        Metadata {
            captured: "2001:01:01".into(),
            ..Default::default()
        },
    );
    let rows = delete_review::from_analysis(&media, &a, &meta, &pairs);
    assert_eq!(
        rows.iter().map(|e| e.subject).collect::<Vec<_>>(),
        vec![id(3), id(1)]
    );
    assert_eq!(rows[1].companion, Some(id(2)));
}
fn analyze_gray(pixel: impl Fn(u32, u32) -> u8, thresholds: Thresholds) -> Analysis {
    let pixels = Arc::new(image::RgbaImage::from_fn(640, 480, |x, y| {
        let v = pixel(x, y);
        image::Rgba([v, v, v, 255])
    }));
    analysis::analyze(
        &Asset {
            decoder_warning: None,
            thumbnail: pixels.clone(),
            pixels,
            metadata: Metadata::default(),
        },
        thresholds,
    )
}
#[test]
fn recoverable_low_light_is_not_proposed_for_deletion_even_with_low_sharpness() {
    let a = analyze_gray(|_, _| 12, Thresholds::default());
    assert!(a.brightness < 15. && a.sharpness < 100.);
    assert!(a.issues.is_empty());
    let black = analyze_gray(|_, _| 0, Thresholds::default());
    assert_eq!(black.issues, vec!["Very dark"]);
}
#[test]
fn issue_precedence_is_dark_then_blur_then_bright() {
    let white = analyze_gray(|_, _| 255, Thresholds::default());
    assert_eq!(white.issues, vec!["Possibly soft"]);
    let white = analyze_gray(
        |_, _| 255,
        Thresholds {
            blur: 0.,
            ..Default::default()
        },
    );
    assert_eq!(white.issues, vec!["Overexposed"]);
}
