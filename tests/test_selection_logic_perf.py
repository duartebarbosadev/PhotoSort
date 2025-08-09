import time
from src.ui.selection_utils import select_next_surviving_path


def test_backward_when_no_forward_candidate():
    # Anchor (or its replacement) at end; forward scan yields nothing so we go backward.
    before = ["a", "b", "c"]
    removed = ["c"]
    anchor = "c"  # removed at end
    after = ["a", "b"]
    assert select_next_surviving_path(before, removed, anchor, after) == "b"


def test_prefers_forward_over_backward():
    before = ["a", "b", "c", "d"]
    removed = ["b"]
    anchor = "b"
    after = ["a", "c", "d"]
    # Forward candidate is c; backward would be a; expect c.
    assert select_next_surviving_path(before, removed, anchor, after) == "c"


def test_keeps_anchor_if_still_visible():
    before = ["a", "b", "c"]
    removed = ["a"]
    anchor = "c"  # survives
    after = ["b", "c"]
    assert select_next_surviving_path(before, removed, anchor, after) == "c"


def test_large_list_performance_and_correctness():
    # Construct large ordered list with numeric names to emulate many images.
    before = [f"img_{i:05d}.jpg" for i in range(10_000)]
    # Remove one in the middle; anchor matches removed path.
    removed_index = 5432
    removed_item = before[removed_index]
    removed = [removed_item]
    anchor = removed_item
    after = before[:removed_index] + before[removed_index + 1 :]

    start = time.perf_counter()
    next_path = select_next_surviving_path(before, removed, anchor, after)
    duration = time.perf_counter() - start

    # Expect the element immediately after the removed one.
    expected = before[removed_index + 1]
    assert next_path == expected
    # Soft performance assertion: should complete very quickly (< 5 ms typical; allow generous 0.1s)
    assert duration < 0.1, f"Selection took too long: {duration:.4f}s"
