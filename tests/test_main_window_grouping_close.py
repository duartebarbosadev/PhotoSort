from types import SimpleNamespace
from unittest.mock import Mock
import pytest

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
        is_file_deletion_running=lambda: False,
        is_any_worker_running=lambda: False,
        request_stop_all_workers=Mock(),
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
    worker_manager.request_stop_all_workers.assert_called_once()
    assert event.accepted


def test_close_requests_worker_stop_without_waiting(monkeypatch):
    callbacks = []
    status_bar = _DummyStatusBar()
    preview_controller = SimpleNamespace(shutdown=Mock())
    worker_manager = SimpleNamespace(
        is_grouping_workflow_running=lambda: False,
        is_file_deletion_running=lambda: False,
        is_any_worker_running=lambda: True,
        request_stop_all_workers=Mock(),
    )
    window = SimpleNamespace(
        worker_manager=worker_manager,
        dialog_manager=SimpleNamespace(
            confirm_interrupt_for_close=Mock(return_value=True)
        ),
        grouping_step_widget=SimpleNamespace(
            pending_grouping_action_lines=lambda: [],
            has_unsaved_grouping_edits=lambda: False,
        ),
        app_state=SimpleNamespace(get_marked_files=lambda: []),
        preview_load_controller=preview_controller,
        statusBar=lambda: status_bar,
        _close_after_grouping_save=False,
        _shutdown_in_progress=False,
        _finish_close_after_workers=Mock(),
    )
    event = _DummyEvent()
    monkeypatch.setattr(
        "src.ui.main_window.QTimer.singleShot",
        lambda _delay, callback: callbacks.append(callback),
    )

    MainWindow.closeEvent(window, event)

    worker_manager.request_stop_all_workers.assert_called_once_with()
    assert event.ignored
    assert not event.accepted
    assert window._shutdown_in_progress is True
    assert callbacks == [window._finish_close_after_workers]
    assert status_bar.messages[-1][0] == "Stopping background work…"


def test_declining_close_interrupt_preserves_pending_changes_and_workers():
    window = SimpleNamespace(
        worker_manager=SimpleNamespace(
            is_grouping_workflow_running=lambda: False,
            is_any_worker_running=lambda: True,
            request_stop_all_workers=Mock(),
        ),
        dialog_manager=SimpleNamespace(
            confirm_interrupt_for_close=Mock(return_value=False)
        ),
        preview_load_controller=SimpleNamespace(shutdown=Mock()),
        grouping_step_widget=Mock(),
    )
    event = _DummyEvent()
    MainWindow.closeEvent(window, event)
    assert event.ignored and not event.accepted
    window.worker_manager.request_stop_all_workers.assert_not_called()
    window.preview_load_controller.shutdown.assert_not_called()
    window.grouping_step_widget.has_unsaved_grouping_edits.assert_not_called()


@pytest.mark.parametrize(
    "writer", ["is_rotation_application_running", "is_rating_writer_running"]
)
def test_close_waits_for_file_writes_without_cancelling_them(writer):
    manager = SimpleNamespace(
        is_grouping_workflow_running=lambda: False,
        request_stop_all_workers=Mock(),
    )
    setattr(manager, writer, lambda: True)
    window = SimpleNamespace(
        worker_manager=manager, statusBar=lambda: _DummyStatusBar()
    )
    event = _DummyEvent()
    MainWindow.closeEvent(window, event)
    assert event.ignored and not event.accepted
    manager.request_stop_all_workers.assert_not_called()


def test_close_keeps_preview_owner_alive_until_pools_drain(monkeypatch):
    active = [True]
    window = SimpleNamespace(
        _shutdown_in_progress=True,
        worker_manager=SimpleNamespace(is_any_worker_running=lambda: False),
        preview_load_controller=SimpleNamespace(is_active=lambda: active[0]),
        close=Mock(),
        _finish_close_after_workers=Mock(),
    )
    callbacks = []
    monkeypatch.setattr(
        "src.ui.main_window.QTimer.singleShot", lambda _ms, fn: callbacks.append(fn)
    )
    event = _DummyEvent()
    MainWindow.closeEvent(window, event)
    assert event.ignored and not event.accepted
    MainWindow._finish_close_after_workers(window)
    window.close.assert_not_called()
    assert callbacks == [window._finish_close_after_workers]
    active[0] = False
    MainWindow._finish_close_after_workers(window)
    window.close.assert_called_once()
