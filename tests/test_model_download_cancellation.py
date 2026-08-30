"""Exercise real spawned transfers without network access or real model weights."""

import multiprocessing
import threading
import time
from unittest.mock import Mock

import pytest

from core import model_download, model_provisioning
from core.model_download import ModelDownloadCancelled


def _stalled_transfer(connection, repo_id, options, label):
    connection.send(("progress", -1, "Transfer started"))
    threading.Event().wait(30)


def _successful_transfer(connection, repo_id, options, label):
    connection.send(("progress", 50, "Half downloaded"))
    connection.send(("result", options["snapshot"]))
    connection.close()


def _crashed_transfer(connection, repo_id, options, label):
    connection.close()


@pytest.mark.parametrize("fail", [False, True])
def test_download_child_forwards_cache_options_and_reports_result(monkeypatch, fail):
    connection = Mock()
    options = {"revision": "pinned", "cache_dir": "/cache", "allow_patterns": ["*.bin"]}

    def download(repo_id, *, tqdm_class, **kwargs):
        assert repo_id == "test/model"
        assert kwargs == {"local_files_only": False, **options}
        if fail:
            raise OSError("network unavailable")
        progress = tqdm_class(total=10, unit="B")
        progress.update(10)
        progress.close()
        return "/cache/snapshot"

    monkeypatch.setattr("huggingface_hub.snapshot_download", download)
    model_download._download_child(connection, "test/model", options, "Test model")
    connection.close.assert_called_once_with()
    if fail:
        connection.send.assert_called_once_with(("error", "network unavailable"))
    else:
        assert connection.send.call_args.args == (("result", "/cache/snapshot"),)
        assert any(
            call.args[0][0] == "progress" for call in connection.send.call_args_list
        )


def test_stalled_transfer_cancels_without_waiting_for_progress(monkeypatch):
    monkeypatch.setattr(model_download, "_download_child", _stalled_transfer)
    cancel = threading.Event()
    started_at = []

    def progress(_percent, _message):
        started_at.append(time.monotonic())
        cancel.set()

    before = {child.pid for child in multiprocessing.active_children()}
    with pytest.raises(ModelDownloadCancelled):
        model_download.download_snapshot(
            "test/model",
            options={},
            label="Test",
            progress_callback=progress,
            should_cancel=cancel.is_set,
        )
    assert time.monotonic() - started_at[0] < 3
    assert {child.pid for child in multiprocessing.active_children()} == before


def test_cancel_before_download_does_not_start_a_process(monkeypatch):
    def unexpected_context(*args):
        pytest.fail("Cancelled transfer must not start a process")

    monkeypatch.setattr(
        model_download.multiprocessing, "get_context", unexpected_context
    )
    with pytest.raises(ModelDownloadCancelled):
        model_download.download_snapshot(
            "test/model",
            options={},
            label="Test",
            should_cancel=lambda: True,
        )


def test_transfer_delivers_progress_and_snapshot(monkeypatch):
    monkeypatch.setattr(model_download, "_download_child", _successful_transfer)
    events = []
    assert (
        model_download.download_snapshot(
            "test/model",
            options={"snapshot": "/cached/model"},
            label="Test",
            progress_callback=lambda *event: events.append(event),
        )
        == "/cached/model"
    )
    assert events == [(50, "Half downloaded")]


def test_crashed_transfer_reports_failure_instead_of_waiting_forever(monkeypatch):
    monkeypatch.setattr(model_download, "_download_child", _crashed_transfer)
    with pytest.raises(OSError, match="without a result"):
        model_download.download_snapshot("test/model", options={}, label="Test")


def test_provisioning_preserves_cancellation_instead_of_reporting_network_error(
    monkeypatch,
):
    monkeypatch.setattr(
        model_provisioning, "_snapshot_download", lambda: lambda *a, **k: "missing"
    )

    def cancelled(*args, **kwargs):
        raise ModelDownloadCancelled

    monkeypatch.setattr(model_provisioning, "download_snapshot", cancelled)
    with pytest.raises(ModelDownloadCancelled):
        model_provisioning.resolve_snapshot(
            model_provisioning.AESTHETIC_MODEL,
            allow_download=True,
        )


def test_folder_switch_cancels_live_pick_best_download_and_keeps_qt_responsive(
    monkeypatch,
):
    from PyQt6.QtCore import QEventLoop, QTimer
    from PyQt6.QtWidgets import QApplication
    from ui.app_controller import AppController
    from ui.worker_manager import WorkerManager

    app = QApplication.instance() or QApplication([])
    monkeypatch.setattr(model_download, "_download_child", _stalled_transfer)
    monkeypatch.setattr(
        model_provisioning, "_snapshot_download", lambda: lambda *a, **k: "missing"
    )
    monkeypatch.setattr("ui.app_controller.add_recent_folder", lambda _path: None)
    closed = []

    class DownloadOnlySelector:
        def __init__(self, *, aesthetic_scorer, **kwargs):
            self.scorer = aesthetic_scorer

        def select(self, _paths):
            self.scorer._resolve_model_snapshot()
            raise AssertionError("The stalled transfer must be cancelled")

        def close(self):
            closed.append(True)

    monkeypatch.setattr("workers.pick_best_worker.PhotoSelector", DownloadOnlySelector)
    manager = WorkerManager(Mock())
    manager.start_file_scan = Mock()
    state = Mock()
    state.get_marked_files.return_value = []
    window = Mock()
    window._shutdown_in_progress = False
    window.dialog_manager.confirm_interrupt_for_folder_change.return_value = True
    controller = AppController(window, state, manager)
    errors, results, ticks = [], [], []
    manager.pick_best_error.connect(errors.append)
    manager.pick_best_complete.connect(results.append)

    def on_progress(_percent, message):
        if message == "Transfer started":
            # Let the UI tick while the transfer is blocked, then switch folders.
            QTimer.singleShot(100, lambda: controller.load_folder("/new-folder"))

    manager.pick_best_progress.connect(on_progress)
    loop = QEventLoop()
    heartbeat = QTimer()

    def tick():
        ticks.append(True)
        if manager.start_file_scan.called:
            loop.quit()

    heartbeat.timeout.connect(tick)
    deadline = QTimer()
    deadline.setSingleShot(True)
    deadline.timeout.connect(loop.quit)
    heartbeat.start(10)
    deadline.start(10000)
    manager.start_pick_best_analysis(
        {1: ["/a.jpg", "/b.jpg"]}, allow_model_download=True
    )
    try:
        loop.exec()
        assert manager.start_file_scan.call_count == 1
        assert not manager.is_any_worker_active()
        assert len(ticks) >= 5
        assert closed == [True]
        assert errors == results == []
        window.dialog_manager.confirm_interrupt_for_folder_change.assert_called_once()
    finally:
        heartbeat.stop()
        deadline.stop()
        manager.request_stop_all_workers()
        if manager.pick_best_thread is not None:
            manager.pick_best_thread.wait(3000)
        app.processEvents()
