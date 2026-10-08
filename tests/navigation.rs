use photosort::core::{model::*, navigation::next_visible};
use std::collections::HashMap;
fn media() -> Vec<Media> {
    (0..5)
        .map(|i| Media {
            id: MediaId(i),
            path: format!("/missing/{i}.jpg").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        })
        .collect()
}
#[test]
fn traversal_uses_visible_order_and_skips_runs_of_trash_marks_in_both_directions() {
    let media = media();
    let order = [4, 1, 3, 0, 2];
    let marks = HashMap::from([
        (
            MediaId(1),
            Decision {
                rejected: true,
                ..Default::default()
            },
        ),
        (
            MediaId(3),
            Decision {
                rejected: true,
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(
        next_visible(&media, &order, Some(MediaId(4)), 1, &marks, true),
        Some(MediaId(0))
    );
    assert_eq!(
        next_visible(&media, &order, Some(MediaId(0)), -1, &marks, true),
        Some(MediaId(4))
    );
    assert_eq!(
        next_visible(&media, &order, Some(MediaId(4)), 1, &marks, false),
        Some(MediaId(1))
    );
    assert_eq!(
        next_visible(&media, &order, Some(MediaId(0)), -1, &marks, false),
        Some(MediaId(3))
    );
    assert_eq!(
        order,
        [4, 1, 3, 0, 2],
        "navigation must not hide marked photos"
    );
}
#[test]
fn empty_all_marked_and_endpoints_do_not_wrap_or_return_ineligible_photos() {
    let media = media();
    let order = [0, 1, 2, 3, 4];
    let marks = media
        .iter()
        .map(|m| {
            (
                m.id,
                Decision {
                    rejected: true,
                    ..Default::default()
                },
            )
        })
        .collect();
    assert_eq!(next_visible(&media, &[], None, 1, &marks, true), None);
    assert_eq!(next_visible(&media, &order, None, -1, &marks, true), None);
    assert_eq!(
        next_visible(&media, &order, Some(MediaId(2)), 1, &marks, true),
        None
    );
    assert_eq!(
        next_visible(&media, &order, Some(MediaId(4)), 1, &marks, false),
        None
    );
    assert_eq!(
        next_visible(&media, &order, Some(MediaId(0)), -1, &marks, false),
        None
    );
    assert_eq!(next_visible(&media, &order, None, 0, &marks, false), None);
}
#[test]
fn missing_focus_starts_at_eligible_edge_and_stars_do_not_count_as_trash() {
    let media = media();
    let order = [1, 3, 4];
    let marks = HashMap::from([
        (
            MediaId(1),
            Decision {
                rejected: true,
                ..Default::default()
            },
        ),
        (
            MediaId(4),
            Decision {
                rating: Some(5),
                ..Default::default()
            },
        ),
    ]);
    assert_eq!(
        next_visible(&media, &order, None, 1, &marks, true),
        Some(MediaId(3))
    );
    assert_eq!(
        next_visible(&media, &order, Some(MediaId(2)), -1, &marks, true),
        Some(MediaId(4))
    );
    assert_eq!(
        next_visible(&media, &order, Some(MediaId(3)), 1, &marks, true),
        Some(MediaId(4))
    );
}

#[test]
fn indexed_navigation_matches_96_cases_from_the_python_reference() {
    #[derive(serde::Deserialize)]
    struct Case {
        groups: Vec<Option<usize>>,
        ratings: Vec<Option<u8>>,
        marks: Vec<bool>,
        current: isize,
        direction: isize,
        target: Option<u8>,
        skip: bool,
        include: bool,
        expected: Option<u64>,
    }
    let cases: Vec<Case> = serde_json::from_str(include_str!("navigation_reference.json")).unwrap();
    for (n, c) in cases.iter().enumerate() {
        let index = photosort::core::navigation::Index::build(
            (0..c.groups.len()).map(|i| (MediaId(i as u64), c.groups[i], c.ratings[i], c.marks[i])),
            || false,
        )
        .unwrap();
        let preferences = photosort::core::navigation::Preferences {
            rating: c.target,
            skip_singletons: c.skip,
            cycle_similarity: false,
        };
        assert_eq!(
            index.target(
                (c.current >= 0).then_some(MediaId(c.current as u64)),
                c.direction,
                false,
                preferences,
                c.include,
                false
            ),
            c.expected.map(MediaId),
            "reference case {n}"
        );
    }
}
#[test]
fn horizontal_cycles_respect_scope_marks_and_unknown_ratings() {
    use photosort::core::navigation::{Index, Preferences};
    let index = Index::build(
        [
            (MediaId(0), Some(1), None, false),
            (MediaId(1), Some(2), Some(0), false),
            (MediaId(2), Some(1), Some(5), true),
            (MediaId(3), Some(1), Some(5), false),
        ],
        || false,
    )
    .unwrap();
    let group = Preferences {
        cycle_similarity: true,
        ..Default::default()
    };
    assert_eq!(
        index.target(Some(MediaId(3)), 1, true, group, false, false),
        Some(MediaId(0))
    );
    assert_eq!(
        index.target(Some(MediaId(0)), -1, true, group, false, false),
        Some(MediaId(3))
    );
    assert_eq!(
        index.target(Some(MediaId(0)), 1, true, group, true, false),
        Some(MediaId(2))
    );
    assert_eq!(
        index.target(
            Some(MediaId(0)),
            1,
            true,
            Preferences::default(),
            false,
            false
        ),
        Some(MediaId(1))
    );
    let rating = Preferences {
        rating: Some(0),
        ..Default::default()
    };
    assert_eq!(
        index.target(None, 1, false, rating, false, false),
        Some(MediaId(1)),
        "unknown ratings must not become unrated"
    );
    assert_eq!(
        index.target(Some(MediaId(3)), 1, false, rating, false, false),
        None
    );
    let all = Index::build(
        [
            (MediaId(0), Some(1), None, true),
            (MediaId(1), Some(1), None, true),
        ],
        || false,
    )
    .unwrap();
    assert_eq!(all.target(None, 1, true, group, false, false), None);
    assert_eq!(
        all.target(None, -1, true, group, false, true),
        Some(MediaId(1))
    );
}
#[test]
fn index_build_is_cancellable_and_large_navigation_uses_only_cached_rows() {
    use photosort::core::navigation::{Index, Preferences};
    let consumed = std::cell::Cell::new(0usize);
    let cancelled = Index::build(
        (0..100_000).map(|i| {
            consumed.set(i + 1);
            (MediaId(i as u64), Some(i / 10), Some((i % 6) as u8), false)
        }),
        || consumed.get() > 512,
    );
    assert!(cancelled.is_none());
    assert!(consumed.get() <= 769);
    let index = Index::build(
        (0..100_000).map(|i| {
            (
                MediaId(i),
                Some(i as usize / 10),
                Some((i % 6) as u8),
                false,
            )
        }),
        || false,
    )
    .unwrap();
    for _ in 0..1000 {
        assert_eq!(
            index.target(
                Some(MediaId(99995)),
                -1,
                false,
                Preferences {
                    rating: Some(5),
                    ..Default::default()
                },
                false,
                false
            ),
            Some(MediaId(99989))
        );
        assert_eq!(
            index.target(
                Some(MediaId(99999)),
                1,
                true,
                Preferences {
                    cycle_similarity: true,
                    ..Default::default()
                },
                false,
                false
            ),
            Some(MediaId(99990))
        );
    }
}

#[test]
fn file_keyboard_position_navigation_checks_only_traversed_cached_rows() {
    use photosort::core::navigation::next_position;
    let visits = std::cell::Cell::new(0);
    let next = next_position(100_000, Some(50_000), 1, |i| {
        visits.set(visits.get() + 1);
        i != 50_001
    });
    assert_eq!(next, Some(50_002));
    assert_eq!(visits.get(), 2);
    assert_eq!(next_position(3, Some(0), -1, |_| true), None);
    assert_eq!(next_position(3, Some(2), 1, |_| true), None);
    assert_eq!(next_position(3, None, -1, |_| true), Some(2));
    assert_eq!(next_position(0, None, 1, |_| true), None);
}
