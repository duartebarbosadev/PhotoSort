use photosort::core::{
    decisions::{self, Change},
    model::*,
    selection::Selection,
};
use std::collections::HashMap;
fn media(count: usize) -> Vec<Media> {
    (0..count)
        .map(|i| Media {
            id: MediaId(i as u64),
            path: format!("/unavailable/{i}.jpg").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint::default(),
        })
        .collect()
}
#[test]
fn modifier_ranges_follow_display_order_and_shrink_from_a_stable_anchor() {
    let items = media(5);
    let order = [2, 0, 4, 1, 3];
    let mut selected = Selection::default();
    selected.select(MediaId(4), &items, &order, false, false);
    selected.select(MediaId(1), &items, &order, false, true);
    assert_eq!(selected.ordered(&items, &order), [MediaId(4), MediaId(1)]);
    selected.select(MediaId(2), &items, &order, false, true);
    assert_eq!(
        selected.ordered(&items, &order),
        [MediaId(2), MediaId(0), MediaId(4)]
    );
    selected.select(MediaId(3), &items, &order, true, false);
    selected.select(MediaId(1), &items, &order, true, true);
    assert_eq!(selected.len(), 5);
    selected.select(MediaId(0), &items, &order, true, false);
    assert!(!selected.contains(MediaId(0)));
    selected.select(MediaId(99), &items, &order, false, false);
    assert_eq!(
        selected.len(),
        4,
        "missing targets must not erase selection"
    );
}
#[test]
fn selection_is_not_capped_by_comparison_and_reconciles_hidden_or_removed_rows() {
    let items = media(100_000);
    let order: Vec<_> = (0..items.len()).collect();
    let mut selected = Selection::default();
    selected.all(&items, &order);
    assert_eq!(selected.len(), 100_000);
    let index: HashMap<_, _> = items.iter().enumerate().map(|(i, m)| (m.id, i)).collect();
    let mut decisions = HashMap::new();
    let started = std::time::Instant::now();
    let changed = decisions::stage(
        &items,
        &index,
        &mut decisions,
        selected.ordered(&items, &order),
        Change::Rating(4),
    );
    eprintln!("100k selected rating stage: {:?}", started.elapsed());
    assert_eq!(changed.len(), 100_000);
    assert!(decisions.values().all(|d| d.rating == Some(4)));
    selected.reconcile(&items, &[99_999, 17]);
    assert_eq!(
        selected.ordered(&items, &[99_999, 17]),
        [MediaId(99_999), MediaId(17)]
    );
    selected.select(MediaId(17), &items, &[99_999, 17], false, true);
    assert_eq!(
        selected.ordered(&items, &[99_999, 17]),
        [MediaId(17)],
        "hidden range anchor must not leak into new scope"
    );
    selected.reconcile(&items, &[]);
    assert!(selected.is_empty());
    selected.single(MediaId(7));
    selected.clear();
    assert!(selected.is_empty());
}
#[test]
fn manual_batch_uses_same_rules_as_single_actions_without_duplicate_toggles_or_io() {
    let mut items = media(3);
    items[1].kind = MediaKind::Raw;
    items[2].kind = MediaKind::Video;
    let index: HashMap<_, _> = items.iter().enumerate().map(|(i, m)| (m.id, i)).collect();
    let mut batch = HashMap::new();
    let mut individual = HashMap::new();
    for action in [
        Change::Rating(5),
        Change::Rotate(3),
        Change::ToggleTrash,
        Change::Trash(false),
        Change::Rating(0),
    ] {
        let changed = decisions::stage(
            &items,
            &index,
            &mut batch,
            [MediaId(0), MediaId(1), MediaId(1), MediaId(2), MediaId(999)],
            action,
        );
        for id in [MediaId(0), MediaId(1), MediaId(2)] {
            decisions::stage(&items, &index, &mut individual, [id], action);
        }
        assert_eq!(batch, individual);
        assert!(changed.len() <= 3);
    }
    assert_eq!(
        batch[&MediaId(2)],
        Decision::default(),
        "video may be marked but not rated or rotated"
    );
    assert_eq!(batch[&MediaId(0)].rating, Some(0));
    assert_eq!(batch[&MediaId(1)].turns, 3);
    assert!(
        decisions::stage(
            &items,
            &index,
            &mut batch,
            [MediaId(0)],
            Change::Rating(255)
        )
        .is_empty()
    );
}

#[test]
fn bookmark_snapshot_retains_range_anchor_independently_of_focus() {
    let items = media(10);
    let order: Vec<_> = (0..10).collect();
    let mut selection = Selection::default();
    selection.restore([MediaId(7), MediaId(2)], Some(MediaId(7)));
    let (ids, anchor) = selection.snapshot();
    let mut restored = Selection::default();
    restored.restore(ids, anchor);
    restored.select(MediaId(5), &items, &order, false, true);
    assert_eq!(
        restored.ordered(&items, &order),
        vec![MediaId(5), MediaId(6), MediaId(7)],
        "Shift selection must extend from the saved anchor, not the first sorted member"
    );
}

#[test]
fn file_keyboard_supporting_file_ranges_use_the_browser_selection_rules() {
    use photosort::core::selection::select_ordered;
    let order = [MediaId(900), MediaId(20), MediaId(700), MediaId(1)];
    let mut ids = std::collections::HashSet::new();
    let mut anchor = None;
    select_ordered(
        &mut ids,
        &mut anchor,
        order[1],
        order.into_iter(),
        true,
        false,
    );
    select_ordered(
        &mut ids,
        &mut anchor,
        order[3],
        order.into_iter(),
        false,
        true,
    );
    assert_eq!(ids, order[1..].iter().copied().collect());
    select_ordered(
        &mut ids,
        &mut anchor,
        order[2],
        order.into_iter(),
        false,
        true,
    );
    assert_eq!(ids, order[1..=2].iter().copied().collect());
    select_ordered(
        &mut ids,
        &mut anchor,
        MediaId(999),
        order.into_iter(),
        false,
        false,
    );
    assert_eq!(ids.len(), 2);
    assert_eq!(anchor, Some(order[1]));
}

#[test]
fn clear_all_trash_is_linear_and_preserves_other_decisions_without_media_io() {
    let mut decisions: HashMap<_, _> = (0..100_000)
        .map(|i| {
            (
                MediaId(i),
                Decision {
                    rejected: i % 2 == 0,
                    rating: Some((i % 6) as u8),
                    turns: (i % 4) as u8,
                },
            )
        })
        .collect();
    let started = std::time::Instant::now();
    let changed = decisions::clear_trash_marks(&mut decisions);
    eprintln!("100k library clear: {:?}", started.elapsed());
    assert_eq!(changed.len(), 50_000);
    assert!(changed.iter().all(|id| id.0 % 2 == 0));
    for (id, decision) in &decisions {
        assert!(!decision.rejected);
        assert_eq!(decision.rating, Some((id.0 % 6) as u8));
        assert_eq!(decision.turns, (id.0 % 4) as u8);
    }
    assert!(decisions::clear_trash_marks(&mut decisions).is_empty());
}
