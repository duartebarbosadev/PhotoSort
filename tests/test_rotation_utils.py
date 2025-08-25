from src.ui.helpers.rotation_utils import compute_next_after_rotation


def test_compute_next_after_rotation_basic_forward():
    before = ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
    accepted = ["b.jpg"]
    after = ["a.jpg", "c.jpg", "d.jpg"]
    assert compute_next_after_rotation(before, accepted, after) == "c.jpg"


def test_compute_next_after_rotation_multi_delete():
    before = ["a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg"]
    accepted = ["b.jpg", "c.jpg"]
    after = ["a.jpg", "d.jpg", "e.jpg"]
    # Expect to advance to first surviving after earliest accepted (c's successor -> d)
    assert compute_next_after_rotation(before, accepted, after) == "d.jpg"


def test_compute_next_after_rotation_end_boundary():
    before = ["a.jpg", "b.jpg"]
    accepted = ["b.jpg"]
    after = ["a.jpg"]
    assert compute_next_after_rotation(before, accepted, after) == "a.jpg"


def test_compute_next_after_rotation_all_removed():
    before = ["a.jpg"]
    accepted = ["a.jpg"]
    after = []
    assert compute_next_after_rotation(before, accepted, after) is None
