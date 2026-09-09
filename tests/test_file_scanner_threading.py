import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

import threading
from unittest.mock import Mock

from PyQt6.QtCore import QEventLoop, QThread, QTimer
from PyQt6.QtWidgets import QApplication

from core.file_scanner import FileScanner
from ui.worker_manager import WorkerManager

_app = QApplication.instance() or QApplication([])


def test_scanner_runs_off_ui_thread_and_cancelled_results_are_discarded(monkeypatch):
    started, release = threading.Event(), threading.Event()
    thread_ids, paths = [], []

    def scan(scanner, path):
        thread_ids.append(QThread.currentThread() == _app.thread())
        paths.append(path)
        started.set()
        release.wait(3)
        # Simulate callbacks queued concurrently with cancellation.
        scanner.files_found.emit([{"path": "old.jpg"}])
        scanner.thumbnail_preload_finished.emit([{"path": "old.jpg"}])
        scanner.error.emit("Old scan cancelled")
        scanner.finished.emit()

    monkeypatch.setattr(FileScanner, "scan_directory", scan)
    manager = WorkerManager(Mock())
    results, scans, errors = [], [], []
    manager.file_scan_found_files.connect(results.append)
    manager.file_scan_thumbnail_preload_finished.connect(results.append)
    manager.file_scan_finished.connect(lambda: scans.append(True))
    manager.file_scan_error.connect(errors.append)
    manager.start_file_scan("/old-folder")
    try:
        # Do not process GUI events: the scanner must start independently.
        assert started.wait(1), "Scan startup is incorrectly queued onto the UI thread"
        assert thread_ids == [False]
        manager.request_stop_all_workers()
        release.set()
        loop, timer, deadline = QEventLoop(), QTimer(), QTimer()
        timer.timeout.connect(
            lambda: loop.quit() if manager.scanner_thread is None else None
        )
        deadline.setSingleShot(True)
        deadline.timeout.connect(loop.quit)
        timer.start(10)
        deadline.start(2000)
        loop.exec()
        timer.stop()
        deadline.stop()
        assert manager.scanner_thread is None
        assert paths == ["/old-folder"]
        assert results == scans == errors == []
    finally:
        release.set()
        manager.request_stop_all_workers()
        if manager.scanner_thread is not None:
            manager.scanner_thread.wait(3000)
        _app.processEvents()


def test_prescan_cancellation_emits_finished_without_scanning(monkeypatch):
    scanner = FileScanner(Mock(), directory_path="/unused")
    walk = Mock(side_effect=AssertionError("Cancelled scans must not enumerate files"))
    monkeypatch.setattr("core.file_scanner.os.walk", walk)
    finished = []
    scanner.finished.connect(lambda: finished.append(True))
    scanner.stop()
    scanner.run()
    assert finished == [True]
    walk.assert_not_called()
