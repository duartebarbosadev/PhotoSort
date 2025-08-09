import os
import sys
from src.ui.selection_utils import select_next_surviving_path

# Ensure project root on path (in case tests run differently)
project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if project_root not in sys.path:  # pragma: no cover - defensive
    sys.path.insert(0, project_root)


class TestDeletionSelection:
    """Tests the image selection logic after removals (deletions, accepts)."""

    def test_delete_middle_item_selects_next(self):
        visible_before = ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
        removed = ["b.jpg"]
        anchor = "b.jpg"
        visible_after = ["a.jpg", "c.jpg", "d.jpg"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path == "c.jpg"

    def test_delete_last_item_selects_previous(self):
        visible_before = ["a.jpg", "b.jpg", "c.jpg"]
        removed = ["c.jpg"]
        anchor = "c.jpg"
        visible_after = ["a.jpg", "b.jpg"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path == "b.jpg"

    def test_delete_first_item_selects_next(self):
        visible_before = ["a.jpg", "b.jpg", "c.jpg"]
        removed = ["a.jpg"]
        anchor = "a.jpg"
        visible_after = ["b.jpg", "c.jpg"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path == "b.jpg"

    def test_delete_block_selects_item_after_block(self):
        visible_before = ["a", "b", "c", "d", "e", "f"]
        removed = ["c", "d"]
        anchor = "c"
        visible_after = ["a", "b", "e", "f"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path == "e"

    def test_delete_block_at_end_selects_item_before_block(self):
        visible_before = ["a", "b", "c", "d", "e", "f"]
        removed = ["e", "f"]
        anchor = "e"
        visible_after = ["a", "b", "c", "d"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path == "d"

    def test_delete_all_returns_none(self):
        visible_before = ["a.jpg", "b.jpg"]
        removed = ["a.jpg", "b.jpg"]
        anchor = "a.jpg"
        visible_after = []
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path is None

    def test_delete_everything_but_one_selects_that_one(self):
        visible_before = ["a", "b", "c", "d"]
        removed = ["a", "c", "d"]
        anchor = "c"
        visible_after = ["b"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path == "b"

    def test_no_anchor_deleted_first_selects_next(self):
        visible_before = ["a", "b", "c", "d"]
        removed = ["b", "c"]
        anchor = None
        visible_after = ["a", "d"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path == "d"

    def test_delete_b_and_d_with_b_selected_moves_to_c(self):
        visible_before = ["A", "B", "C", "D", "E"]
        removed = ["B", "D"]
        anchor = "B"
        visible_after = ["A", "C", "E"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path == "C"

    def test_mark_b_and_e_commit_while_c_selected_keeps_c(self):
        # Current logic keeps surviving anchor; if UX needs advance, call with anchor=None.
        visible_before = ["a", "b", "c", "d", "e", "f"]
        removed = ["b", "e"]
        anchor = "c"
        visible_after = ["a", "c", "d", "f"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path == "c"

    def test_anchor_not_in_before_prefers_nearest_neighbor(self):
        visible_before = ["img_001.jpg", "img_010.jpg", "img_020.jpg", "img_030.jpg"]
        removed = ["ghost.jpg"]  # not in visible_before
        anchor = "img_015.jpg"  # not in visible_before
        visible_after = ["img_001.jpg", "img_010.jpg", "img_020.jpg", "img_030.jpg"]
        next_path = select_next_surviving_path(visible_before, removed, anchor, visible_after)
        assert next_path in {"img_010.jpg", "img_020.jpg"}
