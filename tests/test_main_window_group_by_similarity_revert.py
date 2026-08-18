import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from types import SimpleNamespace

from src.ui.main_window import MainWindow


class _Action:
    def __init__(self, checked):
        self._checked = checked
        self.set_calls = []

    def isChecked(self):
        return self._checked

    def setChecked(self, value):
        self._checked = value
        self.set_calls.append(value)


def _window(checked):
    window = SimpleNamespace(
        menu_manager=SimpleNamespace(group_by_similarity_action=_Action(checked)),
    )
    window._uncheck_group_by_similarity = lambda: (
        MainWindow._uncheck_group_by_similarity(window)
    )
    return window


def test_revert_defers_the_uncheck_until_the_toggle_handler_returns(monkeypatch):
    """Unchecking inline would be undone by the rest of _toggle_group_by_similarity."""
    scheduled = []
    monkeypatch.setattr(
        "src.ui.main_window.QTimer",
        SimpleNamespace(singleShot=lambda ms, fn: scheduled.append((ms, fn))),
    )
    window = _window(checked=True)

    MainWindow.revert_group_by_similarity(window)

    action = window.menu_manager.group_by_similarity_action
    assert action.isChecked() is True, "must not uncheck inline"
    assert [ms for ms, _ in scheduled] == [0]

    scheduled[0][1]()
    assert action.isChecked() is False


def test_revert_is_a_no_op_when_grouping_was_never_switched_on(monkeypatch):
    scheduled = []
    monkeypatch.setattr(
        "src.ui.main_window.QTimer",
        SimpleNamespace(singleShot=lambda ms, fn: scheduled.append((ms, fn))),
    )
    window = _window(checked=False)

    MainWindow.revert_group_by_similarity(window)

    assert scheduled == []
    assert window.menu_manager.group_by_similarity_action.set_calls == []


def test_uncheck_goes_through_the_action_so_the_view_rebuilds():
    """setChecked emits toggled(False), which clears the mode and rebuilds."""
    window = _window(checked=True)

    MainWindow._uncheck_group_by_similarity(window)

    assert window.menu_manager.group_by_similarity_action.set_calls == [False]
