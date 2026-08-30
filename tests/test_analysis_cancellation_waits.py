import threading
from concurrent.futures import Future
from unittest.mock import Mock

import pytest
from PyQt6.QtWidgets import QApplication

from core.utils.futures import completed_until_cancelled
from workers.ai_rating_worker import AiRatingWorker
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


def test_ai_transport_cleanup_does_not_wait_for_inflight_request_lock():
    from core.ai.ai_rating_pipeline import LLMAiRatingStrategy

    strategy = LLMAiRatingStrategy.__new__(LLMAiRatingStrategy)
    strategy._lock = threading.Lock()
    strategy._close_lock = threading.Lock()
    strategy._cancel_event = threading.Event()
    strategy._client_closed = False
    closed = threading.Event()
    strategy._client = Mock()
    strategy._client.close.side_effect = closed.set

    strategy._lock.acquire()  # Simulate the lock held by a stalled LLM request.
    cleanup = threading.Thread(target=strategy.shutdown)
    try:
        strategy.request_cancel()
        assert strategy._cancel_event.is_set()
        strategy._client.close.assert_not_called()  # No transport I/O on the UI thread.
        cleanup.start()
        assert closed.wait(1), "Cleanup must not wait for the stalled request's lock"
        cleanup.join(1)
    finally:
        strategy._lock.release()
        if cleanup.ident is not None:
            cleanup.join(2)
    strategy.shutdown()
    strategy._client.close.assert_called_once()


@pytest.mark.parametrize("kind", ["rotation", "ai_rating", "ai_validation"])
def test_cancelled_analysis_exits_while_current_call_is_still_blocked(kind):
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
            return 90 if kind == "rotation" else {"rating": 5}
        finally:
            task_exited.set()

    if kind == "rotation":
        worker = RotationDetectionStepWorker(
            ["first.jpg", "queued.jpg"],
            image_pipeline=Mock(),
            model_detector=Mock(predict_rotation_angle=stalled),
            num_workers=1,
        )
    else:
        strategy = Mock(rate_image=stalled)
        if kind == "ai_validation":
            strategy.validate_connection = stalled
        worker = AiRatingWorker(
            ["first.jpg", "queued.jpg"],
            max_workers=1,
            strategy=strategy,
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
    assert calls == [
        "connection validation" if kind == "ai_validation" else "first.jpg"
    ]
    assert results == errors == []
