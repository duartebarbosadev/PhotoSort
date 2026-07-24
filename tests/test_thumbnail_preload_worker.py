"""
Tests for thumbnail preload worker.

This worker runs in the background after file scan completes,
preloading thumbnails without blocking the UI.
"""

import pyexiv2  # noqa: F401 - Must be first import to prevent Windows DLL issues

import threading
import time
from unittest.mock import Mock
from PyQt6.QtCore import QObject
from src.workers.thumbnail_preload_worker import ThumbnailPreloadWorker


class TestThumbnailPreloadWorker:
    """Tests for ThumbnailPreloadWorker."""

    def test_worker_initialization(self):
        """Test that worker can be initialized with image pipeline."""
        mock_pipeline = Mock()
        worker = ThumbnailPreloadWorker(image_pipeline=mock_pipeline)

        assert worker.image_pipeline == mock_pipeline
        assert worker._is_running is True  # Starts as True, set to False by stop()
        assert isinstance(worker, QObject)

    def test_worker_stop(self):
        """Test that worker stop flag is set correctly."""
        mock_pipeline = Mock()
        worker = ThumbnailPreloadWorker(image_pipeline=mock_pipeline)

        worker.stop()
        assert worker._is_running is False

    def test_session_processes_foreground_first_and_materializes_background(self):
        pipeline = Mock()
        calls = []
        pipeline.ensure_thumbnail_cached.side_effect = (
            lambda path, *, promote_to_memory: (
                calls.append((path, promote_to_memory)) or True
            )
        )
        worker = ThumbnailPreloadWorker(
            pipeline,
            session_id="folder",
            all_paths=["background-1", "visible", "background-2"],
            foreground_paths=["visible"],
        )

        worker.run_session()

        assert calls[0] == ("visible", True)
        assert set(calls[1:]) == {
            ("background-1", True),
            ("background-2", True),
        }

    def test_session_reprioritizes_scroll_request_during_background_work(self):
        pipeline = Mock()
        initial_batch_started = threading.Event()
        release_background = threading.Event()
        jump_started = threading.Event()
        release_jump = threading.Event()
        state_lock = threading.Lock()
        calls = []
        initial_background_count = 0

        def ensure(path, *, promote_to_memory):
            nonlocal initial_background_count
            with state_lock:
                calls.append((path, promote_to_memory))
                if path in {
                    "background-1",
                    "background-2",
                    "background-3",
                    "background-4",
                }:
                    initial_background_count += 1
                    if initial_background_count == 4:
                        initial_batch_started.set()
            if path.startswith("background-") and path != "background-5":
                release_background.wait(timeout=2)
            elif path == "jump-target":
                jump_started.set()
                release_jump.wait(timeout=2)
            return True

        pipeline.ensure_thumbnail_cached.side_effect = ensure
        worker = ThumbnailPreloadWorker(
            pipeline,
            session_id="folder",
            all_paths=[
                "background-1",
                "background-2",
                "background-3",
                "background-4",
                "background-5",
                "jump-target",
            ],
        )
        thread = threading.Thread(target=worker.run_session)
        thread.start()
        assert initial_batch_started.wait(timeout=2)

        worker.prioritize(["jump-target"])
        release_background.set()
        assert jump_started.wait(timeout=2)

        with state_lock:
            assert ("background-5", True) not in calls

        release_jump.set()
        thread.join(timeout=2)

        assert not thread.is_alive()
        assert set(calls[:4]) == {
            ("background-1", True),
            ("background-2", True),
            ("background-3", True),
            ("background-4", True),
        }
        assert calls[4] == ("jump-target", True)

    def test_session_warms_background_with_four_workers(self):
        pipeline = Mock()
        release = threading.Event()
        four_started = threading.Event()
        state_lock = threading.Lock()
        active = 0
        max_active = 0

        def ensure(_path, *, promote_to_memory):
            nonlocal active, max_active
            assert promote_to_memory is True
            with state_lock:
                active += 1
                max_active = max(max_active, active)
                if active == 4:
                    four_started.set()
            release.wait(timeout=2)
            with state_lock:
                active -= 1
            return True

        pipeline.ensure_thumbnail_cached.side_effect = ensure
        worker = ThumbnailPreloadWorker(
            pipeline,
            session_id="folder",
            all_paths=[f"background-{index}" for index in range(8)],
        )
        thread = threading.Thread(target=worker.run_session)
        thread.start()

        assert four_started.wait(timeout=2)
        release.set()
        thread.join(timeout=2)

        assert not thread.is_alive()
        assert max_active == 4

    def test_session_emits_background_results_without_waiting_for_scroll(self):
        pipeline = Mock()
        pipeline.ensure_thumbnail_cached.return_value = True
        worker = ThumbnailPreloadWorker(
            pipeline,
            session_id="folder",
            all_paths=[f"image-{index}.jpg" for index in range(25)],
        )
        ready_batches = []
        worker.session_batch_ready.connect(
            lambda _session, paths: ready_batches.append(list(paths))
        )

        worker.run_session()

        assert [len(batch) for batch in ready_batches] == [20, 5]
        assert set(path for batch in ready_batches for path in batch) == {
            f"image-{index}.jpg" for index in range(25)
        }

    def test_foreground_work_runs_while_background_is_paused(self):
        pipeline = Mock()
        pipeline.ensure_thumbnail_cached.return_value = True
        worker = ThumbnailPreloadWorker(
            pipeline,
            session_id="folder",
            all_paths=["background", "visible"],
            should_pause_background=lambda: True,
        )
        thread = threading.Thread(target=worker.run_session)
        thread.start()
        time.sleep(0.05)

        worker.prioritize(["visible"])
        deadline = time.time() + 2
        while time.time() < deadline and not pipeline.ensure_thumbnail_cached.called:
            time.sleep(0.01)
        worker.stop()
        thread.join(timeout=2)

        pipeline.ensure_thumbnail_cached.assert_called_with(
            "visible", promote_to_memory=True
        )
