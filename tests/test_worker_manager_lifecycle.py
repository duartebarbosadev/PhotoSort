import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from unittest.mock import Mock

import pytest

from ui.worker_manager import WorkerManager


@pytest.mark.parametrize(
    "thread_attribute",
    [
        "scanner_thread",
        "similarity_thread",
        "rating_loader_thread",
        "cuda_detection_thread",
        "update_check_thread",
        "rating_writer_thread",
        "rotation_application_thread",
        "thumbnail_preload_thread",
        "preview_warm_thread",
        "ai_rating_thread",
        "grouping_preview_thread",
        "grouping_workflow_thread",
        "file_deletion_thread",
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
    assert manager.is_any_worker_active() is True


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


def test_request_stop_all_workers_never_waits_on_worker_threads():
    manager = WorkerManager(Mock())
    manager.similarity_thread = thread = Mock()
    manager.similarity_worker = worker = Mock()
    thread.isRunning.return_value = True

    manager.request_stop_all_workers()

    worker.stop.assert_called_once_with()
    thread.requestInterruption.assert_called_once_with()
    thread.quit.assert_called_once_with()
    thread.wait.assert_not_called()


@pytest.mark.parametrize(
    ("method_name", "thread_attribute", "worker_attribute"),
    [
        (
            "request_stop_similarity_analysis",
            "similarity_thread",
            "similarity_worker",
        ),
        (
            "request_stop_grouping_preview",
            "grouping_preview_thread",
            "grouping_preview_worker",
        ),
        (
            "request_stop_thumbnail_preload",
            "thumbnail_preload_thread",
            "thumbnail_preload_worker",
        ),
        (
            "request_stop_pick_best_analysis",
            "pick_best_thread",
            "pick_best_worker",
        ),
        (
            "request_stop_easy_delete_analysis",
            "easy_delete_thread",
            "easy_delete_worker",
        ),
        (
            "request_stop_fix_rotation_detection",
            "fix_rotation_detect_thread",
            "fix_rotation_detect_worker",
        ),
    ],
)
def test_ui_cancellation_methods_never_wait(
    method_name, thread_attribute, worker_attribute
):
    manager = WorkerManager(Mock())
    thread = Mock()
    worker = Mock()
    thread.isRunning.return_value = True
    setattr(manager, thread_attribute, thread)
    setattr(manager, worker_attribute, worker)

    getattr(manager, method_name)()

    worker.stop.assert_called_once_with()
    thread.quit.assert_called_once_with()
    thread.wait.assert_not_called()
