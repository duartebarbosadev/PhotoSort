from types import SimpleNamespace
from unittest.mock import Mock

from ui.app_controller import AppController


def test_folder_load_resume_waits_for_deletion_thread_shutdown(monkeypatch):
    callbacks = []
    worker_manager = SimpleNamespace(is_file_deletion_running=lambda: True)
    controller = AppController(
        SimpleNamespace(),
        SimpleNamespace(),
        worker_manager,
    )
    controller._pending_folder_load = (
        "/next",
        {
            "skip_grouping_step": False,
            "record_as_source": True,
            "preserve_deletion_marks": False,
        },
    )
    controller.load_folder = Mock()
    monkeypatch.setattr(
        "ui.app_controller.QTimer.singleShot",
        lambda _delay, callback: callbacks.append(callback),
    )

    controller.resume_folder_load_after_deletion(True)

    assert controller._pending_folder_load is not None
    controller.load_folder.assert_not_called()
    worker_manager.is_file_deletion_running = lambda: False
    callbacks.pop()()
    controller.load_folder.assert_called_once_with(
        "/next",
        skip_grouping_step=False,
        record_as_source=True,
        preserve_deletion_marks=False,
    )
