import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

import threading
from types import SimpleNamespace
from unittest.mock import Mock

import pytest
from PyQt6.QtCore import QEventLoop, QThread, QTimer
from PyQt6.QtWidgets import QApplication

from ui.worker_manager import WorkerManager
from workers.rating_loader_worker import RatingLoaderWorker

_app = QApplication.instance() or QApplication([])


def _state():
    return SimpleNamespace(
        exif_disk_cache=SimpleNamespace(
            dataset_residency=lambda _paths: (1, 1),
            get_current_size_limit_bytes=lambda: 1_000_000,
        ),
        rating_cache={},
        date_cache={},
        detailed_metadata_cache={},
    )


@pytest.mark.parametrize(
    "stop_method", ["request_stop_rating_load", "request_stop_all_workers"]
)
def test_late_and_queued_metadata_cannot_repopulate_cancelled_folder(
    tmp_path, monkeypatch, stop_method
):
    source = tmp_path / "old.jpg"
    source.touch()
    path = str(source)
    started, release = threading.Event(), threading.Event()
    metadata = {"rating": 4, "date": "old-date", "raw_metadata": {"old": True}}

    def blocked_batch(*_args):
        started.set()
        assert release.wait(3)
        return {path: metadata}

    monkeypatch.setattr(
        "workers.rating_loader_worker.MetadataProcessor.get_batch_display_metadata",
        blocked_batch,
    )
    state = _state()
    manager = WorkerManager(Mock())
    batches, finishes, warnings = [], [], []
    manager.rating_load_metadata_batch_loaded.connect(batches.append)
    manager.rating_load_finished.connect(lambda: finishes.append(True))
    manager.rating_load_cache_capacity_warning.connect(
        lambda *args: warnings.append(args)
    )
    manager.start_rating_load([{"path": path}], Mock(), state)
    worker = manager.rating_loader_worker
    try:
        assert started.wait(1)

        # Emit from another thread without dispatching GUI events so these
        # callbacks are already queued when the folder is cancelled.
        def queue_results():
            worker.metadata_batch_loaded.emit([(path, metadata)])
            worker.cache_capacity_warning.emit(2, 1, 1_000_000)
            worker.finished.emit()

        sender = threading.Thread(target=queue_results)
        sender.start()
        sender.join(1)
        getattr(manager, stop_method)()
        state.rating_cache.clear()
        state.date_cache.clear()
        state.detailed_metadata_cache.clear()
        release.set()
        loop, timer, deadline = QEventLoop(), QTimer(), QTimer()
        timer.timeout.connect(
            lambda: loop.quit() if manager.rating_loader_thread is None else None
        )
        deadline.setSingleShot(True)
        deadline.timeout.connect(loop.quit)
        timer.start(10)
        deadline.start(2000)
        loop.exec()
        timer.stop()
        deadline.stop()
        assert manager.rating_loader_thread is None
        assert (
            state.rating_cache
            == state.date_cache
            == state.detailed_metadata_cache
            == {}
        )
        assert batches == finishes == warnings == []
    finally:
        release.set()
        manager.request_stop_all_workers()
        if manager.rating_loader_thread is not None:
            manager.rating_loader_thread.wait(3000)
        _app.processEvents()


def test_current_metadata_is_applied_on_ui_thread_before_notification(monkeypatch):
    monkeypatch.setattr(QThread, "start", lambda self: None)
    state = _state()
    manager = WorkerManager(Mock())
    manager.start_rating_load([], Mock(), state)
    worker = manager.rating_loader_worker
    observations = []
    manager.rating_load_metadata_batch_loaded.connect(
        lambda _batch: observations.append(
            (QThread.currentThread() == _app.thread(), dict(state.rating_cache))
        )
    )
    worker.metadata_batch_loaded.emit(
        [("current.jpg", {"rating": 3, "date": "today", "raw_metadata": {"ok": True}})]
    )
    assert observations == [(True, {"current.jpg": 3})]
    assert state.date_cache == {"current.jpg": "today"}
    assert state.detailed_metadata_cache == {"current.jpg": {"ok": True}}
    manager.stop_rating_load()


def test_worker_returns_metadata_without_writing_shared_state(tmp_path, monkeypatch):
    path = str(tmp_path / "current.jpg")
    (tmp_path / "current.jpg").touch()
    state = _state()
    monkeypatch.setattr(
        "workers.rating_loader_worker.MetadataProcessor.get_batch_display_metadata",
        lambda *_args: {path: {"rating": 3}},
    )
    worker = RatingLoaderWorker([{"path": path}], Mock(), state)
    batches = []
    worker.metadata_batch_loaded.connect(batches.append)
    worker.run_load()
    assert batches == [[(path, {"rating": 3})]]
    assert state.rating_cache == state.date_cache == state.detailed_metadata_cache == {}
