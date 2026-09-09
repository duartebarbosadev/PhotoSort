import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

import threading
from concurrent.futures import Future
from unittest.mock import Mock

from PyQt6.QtWidgets import QApplication

from core.utils.futures import completed_until_cancelled
from workers.rotation_detection_step_worker import RotationDetectionStepWorker

_app = QApplication.instance() or QApplication([])


def test_shared_wait_exits_without_any_future_completing():
    future = Future()
    future.set_running_or_notify_cancel()
    cancel = threading.Event()
    yielded = []
    thread = threading.Thread(
        target=lambda: yielded.extend(
            completed_until_cancelled([future], cancel.is_set)
        )
    )
    thread.start()
    try:
        cancel.set()
        thread.join(1)
        assert not thread.is_alive()
        assert not future.done()
        assert yielded == []
    finally:
        future.set_result(None)
        thread.join(2)


def test_shared_wait_skips_executor_cancelled_futures():
    cancelled, ready = Future(), Future()
    cancelled.cancel()
    ready.set_result("ready")
    assert list(completed_until_cancelled([cancelled, ready], lambda: False)) == [ready]


def test_cancelled_analysis_exits_while_current_call_is_still_blocked():
    started, release, task_exited = (
        threading.Event(),
        threading.Event(),
        threading.Event(),
    )
    calls = []

    def stalled(path="connection validation", **kwargs):
        calls.append(path)
        started.set()
        try:
            release.wait(5)
            return 90
        finally:
            task_exited.set()

    worker = RotationDetectionStepWorker(
        ["first.jpg", "queued.jpg"],
        image_pipeline=Mock(),
        model_detector=Mock(predict_rotation_angle=stalled),
        num_workers=1,
    )
    results, errors = [], []
    worker.completed.connect(results.append)
    worker.error.connect(errors.append)
    thread = threading.Thread(target=worker.run)
    thread.start()
    try:
        assert started.wait(2)
        worker.stop()
        thread.join(1)
        assert not thread.is_alive(), (
            "Cancellation must not wait for a native/network result"
        )
        assert not task_exited.is_set()
    finally:
        release.set()
        thread.join(3)
        assert task_exited.wait(2)
        _app.processEvents()
    assert calls == ["first.jpg"]
    assert results == errors == []
