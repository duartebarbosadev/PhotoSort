from types import SimpleNamespace
from unittest.mock import Mock

from src.ui.main_window import MainWindow


class _DummyEvent:
    def __init__(self):
        self.accepted = False
        self.ignored = False

    def accept(self):
        self.accepted = True

    def ignore(self):
        self.ignored = True


class _DummyStatusBar:
    def __init__(self):
        self.messages = []

    def showMessage(self, message: str, timeout: int):
        self.messages.append((message, timeout))


def test_close_event_blocks_while_grouping_workflow_is_running():
    status_bar = _DummyStatusBar()
    window = SimpleNamespace(
        worker_manager=SimpleNamespace(
            is_grouping_workflow_running=lambda: True,
        ),
        grouping_step_widget=SimpleNamespace(
            pending_grouping_action_lines=lambda: [],
            has_unsaved_grouping_edits=lambda: False,
        ),
        statusBar=lambda: status_bar,
    )
    event = _DummyEvent()

    MainWindow.closeEvent(window, event)

    assert event.ignored
    assert not event.accepted
    assert len(status_bar.messages) == 1
    message, timeout = status_bar.messages[0]
    assert timeout == 4000
    assert "Grouping is still moving files" in message
    assert "closing" in message


def test_close_without_grouping_edits_skips_expensive_action_preview():
    pending_actions = Mock(
        side_effect=AssertionError("unchanged plans must not walk the filesystem")
    )
    preview_controller = SimpleNamespace(shutdown=Mock())
    worker_manager = SimpleNamespace(
        is_grouping_workflow_running=lambda: False,
        stop_all_workers=Mock(),
    )
    window = SimpleNamespace(
        worker_manager=worker_manager,
        grouping_step_widget=SimpleNamespace(
            pending_grouping_action_lines=pending_actions,
            has_unsaved_grouping_edits=lambda: False,
        ),
        app_state=SimpleNamespace(get_marked_files=lambda: []),
        preview_load_controller=preview_controller,
        _close_after_grouping_save=False,
    )
    event = _DummyEvent()

    MainWindow.closeEvent(window, event)

    pending_actions.assert_not_called()
    preview_controller.shutdown.assert_called_once()
    worker_manager.stop_all_workers.assert_called_once()
    assert event.accepted
