import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2
from src.ui.controllers.rotation_controller import RotationController


class DummyApplier:
    def __init__(self):
        self.applied = []

    def __call__(self, mapping):
        self.applied.append(dict(mapping))


def make_controller(suggestions):
    applier = DummyApplier()
    ctrl = RotationController(suggestions, applier)
    return ctrl, applier


def test_accept_paths_and_next_basic():
    suggestions = {"a.jpg": 90, "b.jpg": 180, "c.jpg": 270}
    ctrl, applier = make_controller(suggestions)
    before = ctrl.get_visible_order()
    accepted = ctrl.accept_paths(["b.jpg"])
    assert accepted == ["b.jpg"]
    assert "b.jpg" not in ctrl.rotation_suggestions
    next_path = ctrl.compute_next_after_accept(before, accepted, "b.jpg")
    # After removing b, remaining order preserves sequence of a, c; anchor b picks next c
    assert next_path == "c.jpg"
    assert applier.applied[0] == {"b.jpg": 180}


def test_accept_all_clears():
    suggestions = {"a.jpg": 90, "b.jpg": 180}
    ctrl, applier = make_controller(suggestions)
    accepted = ctrl.accept_all()
    assert accepted == ["a.jpg", "b.jpg"]
    assert not ctrl.has_suggestions()
    assert applier.applied[0] == {"a.jpg": 90, "b.jpg": 180}


def test_refuse_paths():
    suggestions = {"a.jpg": 90, "b.jpg": 180, "c.jpg": 270}
    ctrl, applier = make_controller(suggestions)
    refused = ctrl.refuse_paths(["a.jpg", "x.jpg"])  # x ignored
    assert refused == ["a.jpg"]
    assert "a.jpg" not in ctrl.rotation_suggestions
    assert ctrl.has_suggestions()
    # No application when refusing
    assert applier.applied == []


def test_compute_next_after_accept_end_boundary():
    suggestions = {"a.jpg": 90, "b.jpg": 180, "c.jpg": 270}
    ctrl, applier = make_controller(suggestions)
    before = ctrl.get_visible_order()
    accepted = ctrl.accept_paths(["c.jpg"])  # last element
    next_path = ctrl.compute_next_after_accept(before, accepted, "c.jpg")
    # Wraps to previous remaining (b.jpg) given anchor at end
    assert next_path == "b.jpg"


def test_accept_paths_empty_input():
    suggestions = {"a.jpg": 90}
    ctrl, applier = make_controller(suggestions)
    accepted = ctrl.accept_paths([])
    assert accepted == []
    # Still present
    assert ctrl.has_suggestions()
