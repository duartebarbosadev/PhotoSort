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
        eyes: Vec::new(),
        extra: Vec::new(),
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
        &[],
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
    let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &[near, formats], &[]);
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
    let rows = delete_review::from_analysis(&media, &a, &meta, &pairs, &[]);
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

mod eyes_closed {
    use super::*;
    use photosort::core::{
        eyes::{Blink, Frame, Person},
        faces::EyeState::{self, Closed, Open, Unknown},
        near_duplicates::batch::Kind,
    };
    fn person(x: f32, state: EyeState) -> Person {
        Person {
            bbox: [x, 0.4, x + 0.1, 0.5],
            eyes: Some([x + 0.02, 0.43, x + 0.08, 0.45]),
            state,
        }
    }
    fn frame(n: u64, at_ms: i64, people: &[(f32, EyeState)]) -> Frame {
        Frame {
            id: id(n),
            at_ms,
            people: people.iter().map(|(x, s)| person(*x, *s)).collect(),
        }
    }
    fn blink(shot: Frame, others: Vec<Frame>) -> Blink {
        Blink {
            frame: shot,
            others,
        }
    }
    fn library(n: u64) -> (Vec<Media>, HashMap<MediaId, Analysis>) {
        let media: Vec<_> = (1..=n).map(media).collect();
        let a = media.iter().map(|m| (m.id, Analysis::default())).collect();
        (media, a)
    }
    fn exact(discard: u64, keep: u64) -> photosort::core::near_duplicates::batch::Pair {
        pair(id(discard), id(keep))
    }
    fn cards(rows: &[Entry]) -> Vec<(Category, u64, Option<u64>)> {
        rows.iter()
            .map(|e| (e.category, e.subject.0, e.companion.map(|c| c.0)))
            .collect()
    }

    #[test]
    fn closed_eyes_with_a_better_shot_go_to_trash_and_show_both_eyes() {
        let (media, a) = library(2);
        let blinks = [blink(
            frame(1, 10_000, &[(0.2, Closed)]),
            vec![frame(2, 7_000, &[(0.21, Open)])],
        )];
        let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &[], &blinks);
        assert_eq!(cards(&rows), vec![(Category::EyesClosed, 1, Some(2))]);
        let card = &rows[0];
        assert_eq!(card.suggestion(), vec![false, true]);
        assert_eq!(card.tag, "Keep the shot with eyes open");
        assert_eq!(
            card.reason,
            "Eyes closed. 2.jpg shows the same moment with the eyes open. PhotoSort suggests moving the closed-eyes shot to Trash and keeping that one."
        );
        assert_eq!(card.keep_why, "Everyone's eyes are open here.");
        let crops: Vec<_> = card.eyes.iter().map(|c| (c.id.0, c.closed)).collect();
        assert_eq!(crops, vec![(1, true), (2, false)]);
        assert!(card.eyes.iter().all(|c| c.rect[0] < c.rect[2]));
        // Staged like any suggestion: the closed-eyes shot is marked.
        let mut review = Review::default();
        review.replace(rows);
        let mut d = HashMap::new();
        assert_eq!(review.stage_new_suggestions(&mut d), vec![id(1)]);
    }

    #[test]
    fn closed_eyes_with_no_better_shot_are_kept_and_say_so() {
        let (media, a) = library(3);
        // The only other shot of the moment has the eyes closed too, or is
        // of someone elsewhere in the frame.
        let blinks = [blink(
            frame(1, 10_000, &[(0.2, Closed)]),
            vec![
                frame(2, 12_000, &[(0.2, Unknown)]),
                frame(3, 12_000, &[(0.7, Open)]),
            ],
        )];
        let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &[], &blinks);
        assert_eq!(cards(&rows), vec![(Category::EyesClosed, 1, None)]);
        let card = &rows[0];
        assert!(card.informational());
        assert_eq!(card.suggestion(), vec![true]);
        assert_eq!(card.tag, "No shot with eyes open - kept");
        assert!(
            card.reason
                .starts_with("Eyes closed. No other photo of this moment"),
            "{}",
            card.reason
        );
        assert_eq!(card.eyes.len(), 1);
        let mut review = Review::default();
        review.replace(rows);
        let mut d = HashMap::new();
        assert!(review.stage_new_suggestions(&mut d).is_empty());
        // The user can still choose Trash, and Undo keeps it again.
        review.toggle(id(1), id(1), &mut d);
        assert!(rejected(&d, 1) && review.changed(id(1), &d));
        review.accept(&[id(1)], &mut d);
        assert!(!rejected(&d, 1));
    }

    #[test]
    fn in_a_group_one_blink_each_is_no_better_shot_for_either() {
        let (media, a) = library(2);
        let first = frame(1, 10_000, &[(0.1, Closed), (0.5, Open)]);
        let second = frame(2, 11_000, &[(0.1, Open), (0.5, Closed)]);
        let blinks = [
            blink(first.clone(), vec![second.clone()]),
            blink(second, vec![first]),
        ];
        let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &[], &blinks);
        assert_eq!(
            cards(&rows),
            vec![
                (Category::EyesClosed, 1, None),
                (Category::EyesClosed, 2, None)
            ]
        );
        assert!(rows[0].reason.starts_with("Eyes closed for 1 of 2 people."));
        assert!(rows.iter().all(|e| e.suggestion() == vec![true]));
    }

    #[test]
    fn a_better_shot_is_never_one_another_card_trashes() {
        let (media, mut a) = library(5);
        // 2 is blurry; 3 is the copy of 4 in an identical pair; 5 is fine
        // but further away in time than 4.
        a.get_mut(&id(2)).unwrap().issues = vec!["Possibly soft".into()];
        let open = |n, at_ms| frame(n, at_ms, &[(0.2, Open)]);
        let blinks = [blink(
            frame(1, 10_000, &[(0.2, Closed)]),
            vec![
                open(2, 10_500),
                open(3, 11_000),
                open(4, 11_000),
                open(5, 15_000),
            ],
        )];
        let rows =
            delete_review::from_analysis(&media, &a, &HashMap::new(), &[exact(3, 4)], &blinks);
        assert_eq!(
            cards(&rows),
            vec![
                (Category::Exact, 3, Some(4)),
                (Category::EyesClosed, 1, Some(4)),
                (Category::Blur, 2, None),
            ]
        );
        // The better shot can be the keeper of several cards: they agree.
        let mut review = Review::default();
        review.replace(rows);
        let mut d = HashMap::new();
        let mut marked = review.stage_new_suggestions(&mut d);
        marked.sort_by_key(|m| m.0);
        assert_eq!(marked, vec![id(1), id(2), id(3)]);
    }

    #[test]
    fn copies_of_a_closed_eyes_shot_share_one_card_with_the_better_shot() {
        // 1 and 2 are identical copies, both with closed eyes; 3 is a better
        // shot, and 4 + 5 are the JPEG and HEIC of a closed-eyes shot of
        // another moment with no better shot.
        let (media, a) = library(5);
        let better = frame(3, 12_000, &[(0.2, Open)]);
        let blinks = [
            blink(frame(1, 10_000, &[(0.2, Closed)]), vec![better.clone()]),
            blink(frame(2, 10_000, &[(0.2, Closed)]), vec![better]),
            blink(frame(4, 90_000, &[(0.2, Closed)]), Vec::new()),
            blink(frame(5, 90_000, &[(0.2, Closed)]), Vec::new()),
        ];
        let mut formats = exact(4, 5);
        formats.kind = Kind::Formats;
        let rows = delete_review::from_analysis(
            &media,
            &a,
            &HashMap::new(),
            &[exact(2, 1), formats],
            &blinks,
        );
        // The identical pair's card would keep 1, which the Eyes closed
        // card moves to Trash: the Eyes closed card wins, and holds both
        // copies, so the better shot is shown once. The JPEG + HEIC pair has
        // no better shot, so its card stays and no Eyes closed note repeats
        // either photo.
        assert_eq!(
            cards(&rows),
            vec![
                (Category::Formats, 4, Some(5)),
                (Category::EyesClosed, 1, Some(3)),
            ]
        );
        let card = rows[1].clone();
        assert_eq!(card.extra, vec![id(2)]);
        assert_eq!(card.candidates(), vec![id(1), id(2), id(3)]);
        assert_eq!(card.suggestion(), vec![false, false, true]);
        assert!(
            card.reason
                .starts_with("Eyes closed, in this photo and an identical copy. 3.jpg"),
            "{}",
            card.reason
        );
        assert!(
            card.reason
                .contains("moving the closed-eyes shots to Trash")
        );
        // Each copy shows its own closed eyes on the card.
        let crops: Vec<_> = card.eyes.iter().map(|c| (c.id.0, c.closed)).collect();
        assert_eq!(crops, vec![(1, true), (2, true), (3, false)]);
        // Both copies are staged, and either can be kept alone.
        let mut review = Review::default();
        review.replace(rows);
        let mut d = HashMap::new();
        let mut marked = review.stage_new_suggestions(&mut d);
        marked.sort_by_key(|m| m.0);
        assert_eq!(marked, vec![id(1), id(2)]);
        review.toggle(id(1), id(2), &mut d);
        assert!(rejected(&d, 1) && !rejected(&d, 2) && review.changed(id(1), &d));
        review.accept(&[id(1)], &mut d);
        assert!(rejected(&d, 1) && rejected(&d, 2));
    }

    #[test]
    fn different_shots_with_one_better_shot_share_its_card() {
        // Two frames of a burst blink; the third has the eyes open.
        let (media, a) = library(3);
        let better = frame(3, 12_000, &[(0.2, Open)]);
        let blinks = [
            blink(frame(1, 10_000, &[(0.2, Closed)]), vec![better.clone()]),
            blink(frame(2, 11_000, &[(0.2, Closed)]), vec![better]),
        ];
        let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &[], &blinks);
        assert_eq!(cards(&rows), vec![(Category::EyesClosed, 1, Some(3))]);
        assert_eq!(rows[0].extra, vec![id(2)]);
        assert!(
            rows[0]
                .reason
                .starts_with("Eyes closed in 2 shots of this moment."),
            "{}",
            rows[0].reason
        );
    }

    #[test]
    fn a_long_burst_of_blinks_is_split_into_cards_of_three() {
        let (media, a) = library(5);
        let better = frame(5, 12_000, &[(0.2, Open)]);
        let blinks: Vec<_> = (1..=4)
            .map(|n| {
                blink(
                    frame(n, 10_000 + n as i64 * 200, &[(0.2, Closed)]),
                    vec![better.clone()],
                )
            })
            .collect();
        let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &[], &blinks);
        let shots: Vec<_> = rows
            .iter()
            .map(|e| e.candidates().iter().map(|c| c.0).collect::<Vec<_>>())
            .collect();
        assert_eq!(shots, vec![vec![1, 2, 3, 5], vec![4, 5]]);
    }

    #[test]
    fn group_photos_show_every_closed_persons_eyes_and_their_open_eyes() {
        let (media, a) = library(2);
        // Two of three people blink; the better shot lists them in another
        // order.
        let blinks = [blink(
            frame(1, 10_000, &[(0.1, Closed), (0.4, Open), (0.7, Closed)]),
            vec![frame(2, 10_500, &[(0.71, Open), (0.1, Open), (0.41, Open)])],
        )];
        let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &[], &blinks);
        assert_eq!(cards(&rows), vec![(Category::EyesClosed, 1, Some(2))]);
        assert!(rows[0].reason.starts_with("Eyes closed for 2 of 3 people."));
        let crops: Vec<_> = rows[0]
            .eyes
            .iter()
            .map(|c| (c.id.0, c.closed, (c.rect[0] * 10.0).round() as i32))
            .collect();
        // The same two people, left and right, closed then open.
        assert_eq!(
            crops,
            vec![(1, true, 1), (1, true, 7), (2, false, 1), (2, false, 7)]
        );
    }

    #[test]
    fn eyes_closed_cards_outrank_near_duplicates_that_would_keep_the_blink() {
        // The near-duplicate pass kept the sharper photo, 1, whose eyes are
        // closed; 2 has them open.
        let (media, a) = library(2);
        let mut near = pair(id(2), id(1));
        near.kind = Kind::Near;
        let blinks = [blink(
            frame(1, 10_000, &[(0.2, Closed)]),
            vec![frame(2, 10_400, &[(0.2, Open)])],
        )];
        let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &[near], &blinks);
        assert_eq!(cards(&rows), vec![(Category::EyesClosed, 1, Some(2))]);
    }
}

/// Photos that sit in several groups at once (copies, formats, near
/// duplicates, closed eyes, blurry) still get one consistent decision each.
mod overlaps {
    use super::*;
    use photosort::core::{
        eyes::{Blink, Frame, Person},
        faces::EyeState,
        near_duplicates::batch::{Keeper, Kind},
    };
    use std::collections::HashSet;

    /// A small deterministic generator (xorshift), so failures replay.
    struct Dice(u64);
    impl Dice {
        fn roll(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }
    }
    fn person(state: EyeState) -> Person {
        Person {
            bbox: [0.2, 0.4, 0.3, 0.5],
            eyes: Some([0.22, 0.43, 0.28, 0.45]),
            state,
        }
    }

    /// The decision every card agrees on for each of its photos, after
    /// checking that no two cards disagree and that a photo going to Trash
    /// is on one card only.
    fn agreed(rows: &[Entry]) -> HashMap<MediaId, bool> {
        let mut keep: HashMap<MediaId, bool> = HashMap::new();
        let mut trash_cards: HashMap<MediaId, usize> = HashMap::new();
        let mut subjects = HashSet::new();
        for e in rows {
            assert!(
                subjects.insert(e.subject),
                "two cards for {:?}: {:#?}",
                e.subject,
                rows.iter()
                    .map(|e| (e.category, e.candidates(), e.suggestion()))
                    .collect::<Vec<_>>()
            );
            let ids = e.candidates();
            let unique: HashSet<_> = ids.iter().collect();
            assert_eq!(unique.len(), ids.len(), "a photo twice on one card: {e:?}");
            for (id, k) in ids.into_iter().zip(e.suggestion()) {
                if let Some(old) = keep.insert(id, k) {
                    assert_eq!(old, k, "cards disagree about {id:?}: {rows:?}");
                }
                if !k {
                    *trash_cards.entry(id).or_default() += 1;
                }
            }
        }
        for (id, n) in trash_cards {
            assert_eq!(n, 1, "{id:?} goes to Trash from {n} cards: {rows:?}");
        }
        keep
    }

    #[test]
    fn every_photo_gets_one_decision_whatever_groups_it_is_in() {
        // How often the cases that matter came up, so the test cannot pass
        // without exercising them.
        let (mut better, mut notes, mut shared, mut dropped) = (0, 0, 0, 0);
        for seed in 1..3000u64 {
            let mut dice = Dice(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
            let n = 3 + dice.roll(8);
            let media: Vec<_> = (1..=n).map(super::media).collect();
            let mut a: HashMap<_, _> = media.iter().map(|m| (m.id, Analysis::default())).collect();
            for m in &media {
                if dice.roll(6) == 0 {
                    let issue =
                        ["Possibly soft", "Very dark", "Overexposed"][dice.roll(3) as usize];
                    a.get_mut(&m.id).unwrap().issues = vec![issue.into()];
                }
            }
            let pick = |dice: &mut Dice| id(1 + dice.roll(n));
            let mut pairs = Vec::new();
            for _ in 0..dice.roll(5) {
                let (discard, keep) = (pick(&mut dice), pick(&mut dice));
                if discard == keep {
                    continue;
                }
                let mut p = pair(discard, keep);
                p.kind = [Kind::Exact, Kind::Near, Kind::Formats][dice.roll(3) as usize];
                p.keeper = Keeper::First;
                pairs.push(p);
            }
            let states = [EyeState::Open, EyeState::Closed, EyeState::Unknown];
            let frame = |n: MediaId, dice: &mut Dice, state| Frame {
                id: n,
                at_ms: 10_000 + dice.roll(5_000) as i64,
                people: vec![person(state)],
            };
            let mut blinks = Vec::new();
            for m in &media {
                if dice.roll(3) != 0 {
                    continue;
                }
                let mut others = Vec::new();
                for o in &media {
                    if o.id != m.id && dice.roll(2) == 0 {
                        let state = states[dice.roll(3) as usize];
                        others.push(frame(o.id, &mut dice, state));
                    }
                }
                blinks.push(Blink {
                    frame: frame(m.id, &mut dice, EyeState::Closed),
                    others,
                });
            }
            let rows = delete_review::from_analysis(&media, &a, &HashMap::new(), &pairs, &blinks);
            let keep = agreed(&rows);
            better += rows
                .iter()
                .filter(|e| e.category == Category::EyesClosed && e.companion.is_some())
                .count();
            notes += rows.iter().filter(|e| e.informational()).count();
            let mut seen = HashSet::new();
            shared += rows
                .iter()
                .flat_map(|e| e.candidates())
                .filter(|id| !seen.insert(*id))
                .count();
            let offered =
                pairs.len() + blinks.len() + a.values().filter(|x| !x.issues.is_empty()).count();
            dropped += usize::from(rows.len() < offered);
            for e in &rows {
                // A card with a keeper keeps it; only a single problem
                // photo goes to Trash with nothing kept beside it.
                if let Some(companion) = e.companion {
                    assert!(keep[&companion], "keeper trashed: {e:?}");
                } else if !e.informational() {
                    assert!(
                        matches!(
                            e.category,
                            Category::Blur | Category::Dark | Category::Bright
                        ),
                        "{e:?}"
                    );
                }
                // A better shot is never one another card trashes for a
                // reason the eyes do not outrank.
                if e.category == Category::EyesClosed
                    && let Some(better) = e.companion
                {
                    assert!(a[&better].issues.is_empty(), "blurry better shot: {e:?}");
                    assert!(
                        !pairs
                            .iter()
                            .any(|p| p.kind == Kind::Exact && p.discard == better),
                        "the copy of a pair chosen as better shot: {e:?}"
                    );
                }
            }
            // Staging marks exactly the agreed Trash photos, Accept all puts
            // the same decisions back in any card order, and Review changes
            // moves exactly those files.
            let trash: HashSet<MediaId> = keep
                .iter()
                .filter(|(_, k)| !**k)
                .map(|(id, _)| *id)
                .collect();
            let subjects: Vec<_> = rows.iter().map(|e| e.subject).collect();
            let mut review = Review::default();
            review.replace(rows.clone());
            let mut staged = HashMap::new();
            let marked: HashSet<_> = review
                .stage_new_suggestions(&mut staged)
                .into_iter()
                .collect();
            assert_eq!(marked, trash, "seed {seed}");
            assert_eq!(review.changed_count(&subjects, &staged), 0, "seed {seed}");
            let rejected: HashSet<_> = staged
                .iter()
                .filter(|(_, d)| d.rejected)
                .map(|(id, _)| *id)
                .collect();
            assert_eq!(rejected, trash, "seed {seed}");
            // Change every photo, then Accept all, forwards and backwards.
            for order in [subjects.clone(), subjects.iter().rev().copied().collect()] {
                let mut d = staged.clone();
                for e in &rows {
                    for c in e.candidates() {
                        review.toggle(e.subject, c, &mut d);
                    }
                }
                review.accept(&order, &mut d);
                let after: HashSet<_> = d
                    .iter()
                    .filter(|(_, d)| d.rejected)
                    .map(|(id, _)| *id)
                    .collect();
                assert_eq!(after, trash, "seed {seed}");
            }
            // A photo shared by several cards changes all of them at once.
            for e in &rows {
                for c in e.candidates() {
                    let mut d = staged.clone();
                    review.toggle(e.subject, c, &mut d);
                    for other in rows.iter().filter(|o| o.candidates().contains(&c)) {
                        assert!(review.changed(other.subject, &d), "seed {seed}: {other:?}");
                    }
                }
            }
        }
        assert!(
            better > 300 && notes > 300 && shared > 100 && dropped > 300,
            "better {better}, notes {notes}, shared {shared}, dropped {dropped}"
        );
    }
}
