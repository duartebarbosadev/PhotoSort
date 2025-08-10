from src.ui.helpers.navigation_utils import navigate_group_cyclic, navigate_linear


def test_group_cyclic_basic():
    sibs = ["a", "b", "c"]
    assert navigate_group_cyclic(sibs, "b", "right", True, set()) == "c"
    assert navigate_group_cyclic(sibs, "c", "right", True, set()) == "a"
    assert navigate_group_cyclic(sibs, "a", "left", True, set()) == "c"


def test_group_cyclic_skip_deleted():
    sibs = ["a", "b", "c", "d"]
    deleted = {"b", "c"}
    # starting from d left should go to a (skip b,c)
    assert navigate_group_cyclic(sibs, "d", "left", True, deleted) == "a"


def test_linear_down():
    ordered = ["a", "b", "c"]
    assert navigate_linear(ordered, None, "down", True, set()) == "a"
    assert navigate_linear(ordered, "a", "down", True, set()) == "b"
    assert navigate_linear(ordered, "c", "down", True, set()) is None


def test_linear_up():
    ordered = ["a", "b", "c"]
    assert navigate_linear(ordered, None, "up", True, set()) == "c"
    assert navigate_linear(ordered, "c", "up", True, set()) == "b"
    assert navigate_linear(ordered, "a", "up", True, set()) is None


def test_linear_skip_deleted():
    ordered = ["a", "b", "c", "d"]
    deleted = {"b", "c"}
    assert navigate_linear(ordered, "a", "down", True, deleted) == "d"
    assert navigate_linear(ordered, "d", "up", True, deleted) == "a"
