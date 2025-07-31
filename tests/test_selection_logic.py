import os
import sys
from src.ui.selection_utils import find_next_visible_path_after_deletions

# Add project root to Python path
project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if project_root not in sys.path:
    sys.path.insert(0, project_root)


class TestDeletionSelection:
    """Tests the image selection logic after deletions."""

    def test_delete_middle_item_selects_next(self):
        """When deleting an item, the next item should be selected."""
        visible_before = ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
        deleted = ["b.jpg"]
        anchor = "b.jpg"
        visible_after = ["a.jpg", "c.jpg", "d.jpg"]

        next_path = find_next_visible_path_after_deletions(
            visible_before, deleted, anchor, visible_after
        )
        assert next_path == "c.jpg"

    def test_delete_last_item_selects_previous(self):
        """When deleting the last item, the new last item should be selected."""
        visible_before = ["a.jpg", "b.jpg", "c.jpg"]
        deleted = ["c.jpg"]
        anchor = "c.jpg"
        visible_after = ["a.jpg", "b.jpg"]

        next_path = find_next_visible_path_after_deletions(
            visible_before, deleted, anchor, visible_after
        )
        assert next_path == "b.jpg"

    def test_delete_first_item_selects_next(self):
        """When deleting the first item, the new first item should be selected."""
        visible_before = ["a.jpg", "b.jpg", "c.jpg"]
        deleted = ["a.jpg"]
        anchor = "a.jpg"
        visible_after = ["b.jpg", "c.jpg"]

        next_path = find_next_visible_path_after_deletions(
            visible_before, deleted, anchor, visible_after
        )
        assert next_path == "b.jpg"

    def test_delete_block_selects_item_after_block(self):
        """When deleting a block of images, select the one after the block."""
        visible_before = ["a", "b", "c", "d", "e", "f"]
        deleted = ["c", "d"]
        anchor = "c"
        visible_after = ["a", "b", "e", "f"]

        next_path = find_next_visible_path_after_deletions(
            visible_before, deleted, anchor, visible_after
        )
        assert next_path == "e"

    def test_delete_block_at_end_selects_item_before_block(self):
        """When deleting a block at the end, select the item before the block."""
        visible_before = ["a", "b", "c", "d", "e", "f"]
        deleted = ["e", "f"]
        anchor = "e"
        visible_after = ["a", "b", "c", "d"]

        next_path = find_next_visible_path_after_deletions(
            visible_before, deleted, anchor, visible_after
        )
        assert next_path == "d"

    def test_delete_all_returns_none(self):
        """When all images are deleted, the result should be None."""
        visible_before = ["a.jpg", "b.jpg"]
        deleted = ["a.jpg", "b.jpg"]
        anchor = "a.jpg"
        visible_after = []

        next_path = find_next_visible_path_after_deletions(
            visible_before, deleted, anchor, visible_after
        )
        assert next_path is None

    def test_delete_everything_but_one_selects_that_one(self):
        """If only one image remains, it should be selected."""
        visible_before = ["a", "b", "c", "d"]
        deleted = ["a", "c", "d"]
        anchor = "c"
        visible_after = ["b"]

        next_path = find_next_visible_path_after_deletions(
            visible_before, deleted, anchor, visible_after
        )
        assert next_path == "b"

    def test_no_anchor_deleted_first_selects_next(self):
        """If anchor is gone but first deleted item is first, selects next valid."""
        visible_before = ["a", "b", "c", "d"]
        deleted = ["b", "c"]
        anchor = None
        visible_after = ["a", "d"]

        next_path = find_next_visible_path_after_deletions(
            visible_before, deleted, anchor, visible_after
        )
        assert next_path == "d"

    def test_delete_b_and_d_with_b_selected_moves_to_c(self):
        """When deleting B and D while B is selected, selection should move to C."""
        visible_before = ["A", "B", "C", "D", "E"]
        deleted = ["B", "D"]
        anchor = "B"
        visible_after = ["A", "C", "E"]

        next_path = find_next_visible_path_after_deletions(
            visible_before, deleted, anchor, visible_after
        )
        assert next_path == "C"
