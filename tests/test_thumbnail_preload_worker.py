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

    def test_preload_thumbnails_success(self):
        """Test successful thumbnail preloading."""
        mock_pipeline = Mock()
        worker = ThumbnailPreloadWorker(image_pipeline=mock_pipeline)

        # Mock preload_thumbnails to simulate successful operation
        mock_pipeline.preload_thumbnails.return_value = None

        # Collect emitted signals
        progress_signals = []
        finished_emitted = []

        worker.progress.connect(lambda c, t, m: progress_signals.append((c, t, m)))
        worker.finished.connect(lambda paths: finished_emitted.append(paths))

        test_paths = ["/test/image1.jpg", "/test/image2.jpg"]

        # Run the preload
        worker.preload_thumbnails(test_paths)

        # Verify pipeline method was called with correct args
        mock_pipeline.preload_thumbnails.assert_called_once()
        call_args = mock_pipeline.preload_thumbnails.call_args

        # Check that paths were passed
        assert call_args[0][0] == test_paths

        # Check that callbacks were provided
        assert "progress_callback" in call_args[1]
        assert "should_continue_callback" in call_args[1]

        # Verify finished signal was emitted
        assert finished_emitted == [test_paths]

    def test_progress_callback_emits_signal(self):
        """Test that progress callback emits progress signal."""
        mock_pipeline = Mock()
        worker = ThumbnailPreloadWorker(image_pipeline=mock_pipeline)

        # Capture progress callback
        progress_callback_ref = None

        def capture_callback(*args, **kwargs):
            nonlocal progress_callback_ref
            progress_callback_ref = kwargs.get("progress_callback")

        mock_pipeline.preload_thumbnails.side_effect = capture_callback

        # Collect progress signals
        progress_signals = []
        worker.progress.connect(lambda c, t, m: progress_signals.append((c, t, m)))

        # Run preload
        worker.preload_thumbnails(["/test/img.jpg"])

        # Simulate progress callback being called
        assert progress_callback_ref is not None
        progress_callback_ref(5, 10)

        # Verify signal was emitted with correct data
        assert len(progress_signals) == 1
        current, total, message = progress_signals[0]
        assert current == 5
        assert total == 10
        assert "5/10" in message

    def test_should_continue_callback_respects_stop(self):
        """Test that should_continue callback returns False when stopped."""
        mock_pipeline = Mock()
        worker = ThumbnailPreloadWorker(image_pipeline=mock_pipeline)

        # Capture should_continue callback
        should_continue_callback_ref = None

        def capture_callback(*args, **kwargs):
            nonlocal should_continue_callback_ref
            should_continue_callback_ref = kwargs.get("should_continue_callback")

        mock_pipeline.preload_thumbnails.side_effect = capture_callback

        # Run preload
        worker.preload_thumbnails(["/test/img.jpg"])

        # Initially should continue
        assert should_continue_callback_ref is not None
        assert should_continue_callback_ref() is True

        # After stop, should not continue
        worker.stop()
        assert should_continue_callback_ref() is False

    def test_exception_in_preload_emits_error(self):
        """Test that exceptions during preload emit error signal."""
        mock_pipeline = Mock()
        worker = ThumbnailPreloadWorker(image_pipeline=mock_pipeline)

        # Make pipeline raise exception
        mock_pipeline.preload_thumbnails.side_effect = Exception("Test error")

        # Collect error signals
        error_signals = []
        worker.error.connect(lambda msg: error_signals.append(msg))

        # Run preload
        worker.preload_thumbnails(["/test/img.jpg"])

        # Verify error was emitted
        assert len(error_signals) == 1
        assert "Test error" in error_signals[0]

    def test_empty_path_list(self):
        """Test preloading with empty path list."""
        mock_pipeline = Mock()
        worker = ThumbnailPreloadWorker(image_pipeline=mock_pipeline)

        finished_emitted = []
        worker.finished.connect(lambda paths: finished_emitted.append(paths))

        # Run with empty list
        worker.preload_thumbnails([])

        # Should NOT call pipeline with empty list (early return optimization)
        mock_pipeline.preload_thumbnails.assert_not_called()

        # Should still emit finished signal
        assert finished_emitted == [[]]

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
        background_started = threading.Event()
        release_background = threading.Event()
        calls = []

        def ensure(path, *, promote_to_memory):
            calls.append((path, promote_to_memory))
            if path == "background-1":
                background_started.set()
                release_background.wait(timeout=2)
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
        assert background_started.wait(timeout=2)

        worker.prioritize(["jump-target"])
        release_background.set()
        thread.join(timeout=2)

        assert not thread.is_alive()
        assert calls[0] == ("background-1", True)
        assert calls.index(("jump-target", True)) <= 4

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
