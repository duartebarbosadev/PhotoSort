import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from unittest.mock import Mock

import pytest

from ui.worker_manager import WorkerManager


@pytest.mark.parametrize(
    "thread_attribute",
    [
        "scanner_thread",
        "similarity_thread",
        "blur_detection_thread",
        "rating_loader_thread",
        "rotation_detection_thread",
        "cuda_detection_thread",
        "update_check_thread",
        "rating_writer_thread",
        "rotation_application_thread",
        "thumbnail_preload_thread",
        "preview_warm_thread",
        "best_shot_thread",
        "ai_rating_thread",
        "grouping_preview_thread",
        "grouping_workflow_thread",
        "pick_best_thread",
        "easy_delete_thread",
        "fix_rotation_detect_thread",
    ],
)
def test_any_worker_running_covers_every_managed_workflow(thread_attribute):
    manager = WorkerManager(Mock())
    thread = Mock()
    thread.isRunning.return_value = True
    setattr(manager, thread_attribute, thread)

    assert manager.is_any_worker_running() is True


def test_stop_update_check_uses_shared_cancellation_path():
    manager = WorkerManager(Mock())
    manager.update_check_thread = thread = Mock()
    manager.update_check_worker = Mock(spec=[])
    thread.isRunning.return_value = True
    thread.wait.return_value = True

    manager.stop_update_check()

    thread.quit.assert_called_once_with()
    thread.wait.assert_called_once_with(5000)
    assert manager.update_check_thread is None
    assert manager.update_check_worker is None
