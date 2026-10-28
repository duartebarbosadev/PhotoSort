use photosort::core::{
    analysis::{self, Analysis, Thresholds},
    delete_review::{self, Category, Entry, FormatChoice, Review},
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
        tag: "Identical copy".into(),
        keep_why: String::new(),
        reason: "copy".into(),
    }
}
fn formats(jpeg: u64, heic: u64) -> Entry {
    Entry {
        category: Category::Formats,
        tag: "JPEG + HEIC".into(),
        keep_why: String::new(),
        ..entry(jpeg, Some(heic))
    }
}
fn queue() -> Review {
    let mut r = Review::default();
    r.replace(vec![entry(1, Some(2)), entry(3, None), formats(5, 6)]);
    r
}
fn rejected(d: &HashMap<MediaId, Decision>, n: u64) -> bool {
    d.get(&id(n)).is_some_and(|d| d.rejected)
}
#[test]
fn suggestions_are_staged_as_the_default_decisions() {
    let mut r = queue();
    let mut d = HashMap::new();
    let mut marked = r.stage_new_suggestions(&mut d);
    marked.sort_by_key(|id| id.0);
    // The copy and the single problem photo go to Trash; keepers and both
    // formats of one shot stay.
    assert_eq!(marked, vec![id(1), id(3)]);
    assert!(rejected(&d, 1) && rejected(&d, 3));
    assert!(!rejected(&d, 2) && !rejected(&d, 5) && !rejected(&d, 6));
    for n in [1, 3, 5] {
        assert!(
            !r.changed(id(n), &d),
            "card {n} differs from its suggestion"
        );
    }
    assert_eq!(r.format_choice(&[id(5)], &d), Some(FormatChoice::Both));
}
#[test]
fn a_photo_is_staged_once_so_later_analyses_keep_the_users_choices() {
    let mut r = queue();
    let mut d = HashMap::new();
    r.stage_new_suggestions(&mut d);
    // The user keeps the copy; a new analysis brings the same suggestion back.
    r.toggle(id(1), id(1), &mut d);
    assert!(!rejected(&d, 1));
    assert!(r.changed(id(1), &d));
    r.replace(vec![entry(1, Some(2)), entry(3, None), entry(7, Some(8))]);
    assert_eq!(r.stage_new_suggestions(&mut d), vec![id(7)]);
    assert!(!rejected(&d, 1), "a later analysis re-trashed a kept photo");
    // Persisted with the session, so a restart does not stage it again.
    let mut restored = Review::default();
    restored.restore_suggested(r.suggested().clone());
    restored.replace(vec![entry(1, Some(2))]);
    assert!(restored.stage_new_suggestions(&mut d).is_empty());
    assert!(!rejected(&d, 1));
}
#[test]
fn staging_never_unmarks_and_switching_keeps_ratings_and_rotations() {
    let mut r = queue();
    let mut d = HashMap::from([(
        id(2),
        Decision {
            rejected: true,
            rating: Some(4),
            turns: 2,
        },
    )]);
    r.stage_new_suggestions(&mut d);
    // A photo marked elsewhere stays marked, even as the suggested keeper.
    assert!(rejected(&d, 2));
    assert!(r.changed(id(1), &d));
    r.toggle(id(1), id(2), &mut d);
    assert!(!rejected(&d, 2));
    assert_eq!((d[&id(2)].rating, d[&id(2)].turns), (Some(4), 2));
    // Photos outside the card are never switched.
    r.toggle(id(1), id(3), &mut d);
    assert!(rejected(&d, 3));
}
#[test]
fn undo_and_accept_put_cards_back_to_their_suggestion_only_where_asked() {
    let mut r = queue();
    let mut d = HashMap::new();
    r.stage_new_suggestions(&mut d);
    r.toggle(id(1), id(1), &mut d);
    r.toggle(id(1), id(2), &mut d);
    r.toggle(id(3), id(3), &mut d);
    assert_eq!(r.changed_count(&[id(1), id(3), id(5)], &d), 2);
    // A group's bulk action (here the exact copies) leaves other groups alone.
    r.accept(&[id(1)], &mut d);
    assert!(rejected(&d, 1) && !rejected(&d, 2));
    assert!(!rejected(&d, 3));
    // Accept all suggestions.
    r.accept(&[id(1), id(3), id(5)], &mut d);
    assert!(rejected(&d, 3));
    assert_eq!(r.changed_count(&[id(1), id(3), id(5)], &d), 0);
    // After Discard every mark is gone, and nothing is staged again.
    d.clear();
    assert!(r.stage_new_suggestions(&mut d).is_empty());
    assert_eq!(r.changed_count(&[id(1), id(3), id(5)], &d), 2);
}
#[test]
fn one_format_choice_applies_to_every_jpeg_heic_pair() {
    let mut r = Review::default();
    r.replace(vec![formats(1, 2), formats(3, 4), entry(5, Some(6))]);
    let all = [id(1), id(3), id(5)];
    let mut d = HashMap::new();
    r.stage_new_suggestions(&mut d);
    assert_eq!(r.format_choice(&all, &d), Some(FormatChoice::Both));
    r.choose_formats(&all, FormatChoice::Heic, &mut d);
    assert!(rejected(&d, 1) && rejected(&d, 3));
    assert!(!rejected(&d, 2) && !rejected(&d, 4));
    // Only the format pairs: the exact copy keeps its own decision.
    assert!(rejected(&d, 5) && !rejected(&d, 6));
    assert_eq!(r.format_choice(&all, &d), Some(FormatChoice::Heic));
    r.choose_formats(&all, FormatChoice::Jpeg, &mut d);
    assert!(!rejected(&d, 1) && rejected(&d, 2) && rejected(&d, 4));
    assert_eq!(r.format_choice(&all, &d), Some(FormatChoice::Jpeg));
    // One pair changed by hand (back to both): no single choice is selected.
    r.toggle(id(1), id(2), &mut d);
    assert_eq!(r.format_choice(&all, &d), None);
    assert!(!r.changed(id(1), &d) && r.changed(id(3), &d));
    r.choose_formats(&all, FormatChoice::Both, &mut d);
    assert!(!r.changed(id(1), &d) && !r.changed(id(3), &d));
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
        kind: photosort::core::near_duplicates::batch::Kind::Exact,
        keeper: photosort::core::near_duplicates::batch::Keeper::First,
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
            .ends_with(" The suggested discard is also possibly soft."),
        "{}",
        rows[0].reason
    );
}
#[test]
fn two_formats_of_one_shot_become_their_own_group_keeping_both() {
    let media: Vec<_> = (1..=4).map(media).collect();
    let a = media.iter().map(|m| (m.id, Analysis::default())).collect();
    let mut formats = pair(id(1), id(2));
    formats.kind = photosort::core::near_duplicates::batch::Kind::Formats;
    let mut near = pair(id(3), id(4));
    near.reason = "No visible subject change detected · suggest keeping the sharper photo".into();
    near.kind = photosort::core::near_duplicates::batch::Kind::Near;
    near.keeper = photosort::core::near_duplicates::batch::Keeper::Sharper;
    let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &[near, formats]);
    assert_eq!(
        rows.iter().map(|e| e.category).collect::<Vec<_>>(),
        vec![Category::Formats, Category::Near]
    );
    assert_eq!((rows[0].subject, rows[0].companion), (id(1), Some(id(2))));
    assert_eq!(rows[0].suggestion(), vec![true, true]);
    assert_eq!(rows[1].tag, "Sharper kept");
    // The keeper's mark explains itself; both formats are kept, so no mark.
    assert_eq!(rows[1].keep_why, "It is the sharper photo of the two.");
    assert!(rows[0].keep_why.is_empty());
    assert_eq!(rows[1].suggestion(), vec![false, true]);
    // Every sentence of the reason starts with a capital letter.
    assert_eq!(
        rows[1].reason,
        "No visible subject change detected. Suggest keeping the sharper photo."
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
