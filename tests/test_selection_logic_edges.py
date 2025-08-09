import pytest
from src.ui.selection_utils import select_next_surviving_path


class TestSelectionEdgeCases:
    """Additional edge-case coverage for select_next_surviving_path."""

    def test_anchor_survives_prefers_anchor(self):
        before = ["a", "b", "c", "d"]
        removed = ["b", "d"]
        anchor = "c"  # survives
        after = ["a", "c"]
        assert select_next_surviving_path(before, removed, anchor, after) == "c"

    def test_no_before_list_fallback_last_after(self):
        before = []
        removed = []
        anchor = None
        after = ["x", "y", "z"]
        # With no context, function returns last visible item
        assert select_next_surviving_path(before, removed, anchor, after) == "z"

    def test_anchor_invalid_removed_empty_heuristic(self):
        before = ["img_001.jpg", "img_010.jpg", "img_020.jpg", "img_030.jpg"]
        removed: list[str] = []  # nothing removed, but anchor invalid
        anchor = "img_015.jpg"  # not in before
        after = before.copy()
        # Heuristic LCP will likely pick img_010 or img_020 as forward search candidate.
        candidate = select_next_surviving_path(before, removed, anchor, after)
        assert candidate in {"img_010.jpg", "img_020.jpg"}

    def test_removed_contains_non_existent_paths(self):
        before = ["a", "b", "c"]
        removed = ["ghost", "b"]  # one real, one fake
        anchor = "b"
        after = ["a", "c"]
        # Anchor removed; should advance forward first (to c) if possible
        assert select_next_surviving_path(before, removed, anchor, after) == "c"

    def test_all_after_items_not_in_before_returns_last(self):
        # Artificial edge: after contains items unknown to before (e.g., drastic refresh)
        before = ["a", "b"]
        removed = ["a", "b"]
        anchor = "b"
        after = ["x", "y"]  # not realistic, but exercise fallback path
        assert select_next_surviving_path(before, removed, anchor, after) == "y"


@pytest.mark.parametrize(
    "before,removed,anchor,after,expected",
    [
        # Forward preference when anchor removed in middle
        (["a", "b", "c", "d"], ["b"], "b", ["a", "c", "d"], "c"),
        # Backward fallback when anchor was last
        (["a", "b", "c"], ["c"], "c", ["a", "b"], "b"),
    ],
)
def test_parametrized_regressions(before, removed, anchor, after, expected):
    assert select_next_surviving_path(before, removed, anchor, after) == expected
