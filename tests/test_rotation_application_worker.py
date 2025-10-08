"""Tests for RotationApplicationWorker"""

import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2

import os
import tempfile
from unittest.mock import Mock, patch
from PyQt6.QtCore import QObject

from workers.rotation_application_worker import RotationApplicationWorker


class TestRotationApplicationWorker:
    """Test suite for RotationApplicationWorker"""

    def test_worker_initialization(self):
        """Test worker initializes correctly"""
        worker = RotationApplicationWorker()
        assert worker is not None
        assert isinstance(worker, QObject)
        assert worker._is_running is True

    def test_worker_stop(self):
        """Test worker stop method"""
        worker = RotationApplicationWorker()
        assert worker._is_running is True
        worker.stop()
        assert worker._is_running is False

    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_apply_single_rotation_clockwise_success(self, mock_metadata_processor):
        """Test applying a single clockwise rotation"""
        # Mock metadata-first rotation success
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            True,
            False,
            "Success",
        )

        worker = RotationApplicationWorker()

        # Set up signal mocks
        progress_mock = Mock()
        rotation_applied_mock = Mock()
        finished_mock = Mock()

        worker.progress.connect(progress_mock)
        worker.rotation_applied.connect(rotation_applied_mock)
        worker.finished.connect(finished_mock)

        # Create temp file
        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            # Apply 90 degree rotation
            approved_rotations = {tmp_path: 90}
            worker.apply_rotations(approved_rotations)

            # Verify signals
            progress_mock.assert_called_once_with(1, 1, os.path.basename(tmp_path))
            rotation_applied_mock.assert_called_once()
            args = rotation_applied_mock.call_args[0]
            assert args[0] == tmp_path  # path
            assert args[1] == "clockwise"  # direction
            assert args[2] is True  # success
            assert args[4] is False  # is_lossy

            finished_mock.assert_called_once_with(1, 0)  # 1 success, 0 failures
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_apply_rotation_counterclockwise(self, mock_metadata_processor):
        """Test applying counterclockwise rotation"""
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            True,
            False,
            "Success",
        )

        worker = RotationApplicationWorker()
        rotation_applied_mock = Mock()
        worker.rotation_applied.connect(rotation_applied_mock)

        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            # Apply -90 degree rotation
            approved_rotations = {tmp_path: -90}
            worker.apply_rotations(approved_rotations)

            # Verify direction
            args = rotation_applied_mock.call_args[0]
            assert args[1] == "counterclockwise"
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_apply_rotation_180(self, mock_metadata_processor):
        """Test applying 180 degree rotation"""
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            True,
            False,
            "Success",
        )

        worker = RotationApplicationWorker()
        rotation_applied_mock = Mock()
        worker.rotation_applied.connect(rotation_applied_mock)

        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            # Apply 180 degree rotation
            approved_rotations = {tmp_path: 180}
            worker.apply_rotations(approved_rotations)

            # Verify direction
            args = rotation_applied_mock.call_args[0]
            assert args[1] == "180"
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_apply_rotation_lossy_fallback(self, mock_metadata_processor):
        """Test lossy rotation fallback when metadata rotation fails"""
        # Metadata rotation indicates lossy needed
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            False,
            True,
            "Needs lossy",
        )
        mock_metadata_processor.rotate_image.return_value = True

        worker = RotationApplicationWorker()
        rotation_applied_mock = Mock()
        worker.rotation_applied.connect(rotation_applied_mock)

        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            approved_rotations = {tmp_path: 90}
            worker.apply_rotations(approved_rotations)

            # Verify lossy rotation was called
            mock_metadata_processor.rotate_image.assert_called_once()

            # Verify is_lossy flag is True
            args = rotation_applied_mock.call_args[0]
            assert args[4] is True  # is_lossy
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_apply_multiple_rotations(self, mock_metadata_processor):
        """Test applying rotations to multiple files"""
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            True,
            False,
            "Success",
        )

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        temp_files = []
        for i in range(3):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            approved_rotations = {path: 90 for path in temp_files}
            worker.apply_rotations(approved_rotations)

            # Should finish with 3 successes, 0 failures
            finished_mock.assert_called_once_with(3, 0)
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_unsupported_rotation_angle(self, mock_metadata_processor):
        """Test handling unsupported rotation angles"""
        worker = RotationApplicationWorker()
        rotation_applied_mock = Mock()
        finished_mock = Mock()

        worker.rotation_applied.connect(rotation_applied_mock)
        worker.finished.connect(finished_mock)

        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            # Invalid rotation angle
            approved_rotations = {tmp_path: 45}
            worker.apply_rotations(approved_rotations)

            # Should report failure
            args = rotation_applied_mock.call_args[0]
            assert args[2] is False  # success = False

            finished_mock.assert_called_once_with(0, 1)  # 0 success, 1 failure
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_rotation_failure(self, mock_metadata_processor):
        """Test handling rotation failures"""
        # Both metadata and lossy fail
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            False,
            True,
            "Needs lossy",
        )
        mock_metadata_processor.rotate_image.return_value = False

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            approved_rotations = {tmp_path: 90}
            worker.apply_rotations(approved_rotations)

            # Should report failure
            finished_mock.assert_called_once_with(0, 1)
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    @patch("workers.rotation_application_worker.calculate_max_workers")
    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_stop_during_processing(
        self, mock_metadata_processor, mock_calculate_max_workers
    ):
        """Test stopping worker during batch rotation"""
        # Force sequential mode for this test (easier to control timing)
        mock_calculate_max_workers.return_value = 1
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            True,
            False,
            "Success",
        )

        worker = RotationApplicationWorker()

        temp_files = []
        for i in range(10):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            # Stop after first call
            call_count = [0]

            def stop_after_first(*args, **kwargs):
                call_count[0] += 1
                if call_count[0] == 1:
                    worker.stop()
                return (True, False, "Success")

            mock_metadata_processor.try_metadata_rotation_first.side_effect = (
                stop_after_first
            )

            approved_rotations = {path: 90 for path in temp_files}
            worker.apply_rotations(approved_rotations)

            # Should stop early in sequential mode
            assert mock_metadata_processor.try_metadata_rotation_first.call_count < 10
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_mixed_success_and_failure(self, mock_metadata_processor):
        """Test handling mix of successes and failures"""
        # Alternate success/failure
        results = [
            (True, False, "Success"),
            (False, False, "Not supported"),
            (True, False, "Success"),
            (False, False, "Not supported"),
        ]
        mock_metadata_processor.try_metadata_rotation_first.side_effect = results

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        temp_files = []
        for i in range(4):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            approved_rotations = {path: 90 for path in temp_files}
            worker.apply_rotations(approved_rotations)

            # Should have 2 successes, 2 failures
            finished_mock.assert_called_once_with(2, 2)
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rotation_application_worker.calculate_max_workers")
    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_parallel_rotation_multiple_images(
        self, mock_metadata_processor, mock_calculate_max_workers
    ):
        """Test parallel rotation with multiple images"""
        # Configure for parallel mode (max_workers > 1)
        mock_calculate_max_workers.return_value = 4
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            True,
            False,
            "Success",
        )

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        rotation_applied_mock = Mock()
        worker.finished.connect(finished_mock)
        worker.rotation_applied.connect(rotation_applied_mock)

        temp_files = []
        for i in range(8):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            approved_rotations = {path: 90 for path in temp_files}
            worker.apply_rotations(approved_rotations)

            # All 8 should succeed
            finished_mock.assert_called_once_with(8, 0)

            # All files should have rotation_applied signal
            assert rotation_applied_mock.call_count == 8
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rotation_application_worker.calculate_max_workers")
    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_sequential_mode_with_max_workers_one(
        self, mock_metadata_processor, mock_calculate_max_workers
    ):
        """Test that single worker forces sequential processing"""
        # Force sequential mode (max_workers = 1)
        mock_calculate_max_workers.return_value = 1
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            True,
            False,
            "Success",
        )

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        temp_files = []
        for i in range(5):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            approved_rotations = {path: 90 for path in temp_files}
            worker.apply_rotations(approved_rotations)

            # Should complete successfully in sequential mode
            finished_mock.assert_called_once_with(5, 0)
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rotation_application_worker.calculate_max_workers")
    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_thread_safe_progress_tracking(
        self, mock_metadata_processor, mock_calculate_max_workers
    ):
        """Test that progress tracking is thread-safe in parallel mode"""
        from PyQt6.QtCore import Qt

        # Enable parallel mode
        mock_calculate_max_workers.return_value = 4
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            True,
            False,
            "Success",
        )

        worker = RotationApplicationWorker()
        progress_mock = Mock()
        # Use DirectConnection to ensure synchronous signal delivery in tests
        worker.progress.connect(progress_mock, Qt.ConnectionType.DirectConnection)

        temp_files = []
        for i in range(10):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            approved_rotations = {path: 90 for path in temp_files}
            worker.apply_rotations(approved_rotations)

            # Progress should be called 10 times (once per file)
            assert progress_mock.call_count == 10

            # Verify all progress values from 1 to 10 are present
            progress_values = set()
            for call in progress_mock.call_args_list:
                current, total, filename = call[0]
                assert total == 10
                progress_values.add(current)

            # Should have all values from 1 to 10
            assert progress_values == set(range(1, 11))
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rotation_application_worker.calculate_max_workers")
    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_parallel_cancellation(
        self, mock_metadata_processor, mock_calculate_max_workers
    ):
        """Test cancellation during parallel rotation"""
        import time as time_module

        # Enable parallel mode
        mock_calculate_max_workers.return_value = 4

        # Make rotation slow so we can cancel mid-execution
        def slow_rotation(*args, **kwargs):
            time_module.sleep(0.1)
            return (True, False, "Success")

        mock_metadata_processor.try_metadata_rotation_first.side_effect = slow_rotation

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        temp_files = []
        for i in range(20):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            approved_rotations = {path: 90 for path in temp_files}

            # Start rotation in a separate thread to allow cancellation
            import threading as thread_module

            rotation_thread = thread_module.Thread(
                target=worker.apply_rotations, args=(approved_rotations,)
            )
            rotation_thread.start()

            # Let some work happen
            time_module.sleep(0.2)

            # Stop the worker
            worker.stop()

            # Wait for thread to finish
            rotation_thread.join(timeout=5)

            # Should have been cancelled before processing all 20
            if finished_mock.called:
                args = finished_mock.call_args[0]
                total_processed = args[0] + args[1]
                assert total_processed < 20, (
                    "Worker should have stopped before processing all files"
                )
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rotation_application_worker.calculate_max_workers")
    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_parallel_mixed_success_failure(
        self, mock_metadata_processor, mock_calculate_max_workers
    ):
        """Test parallel processing with mixed success/failure results"""
        # Enable parallel mode
        mock_calculate_max_workers.return_value = 4

        # Create a function that returns different results based on file path
        def varied_results(file_path, *args, **kwargs):
            # Make every other file fail
            if hash(file_path) % 2 == 0:
                return (True, False, "Success")
            else:
                return (False, False, "Not supported")

        mock_metadata_processor.try_metadata_rotation_first.side_effect = varied_results

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        temp_files = []
        for i in range(10):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            approved_rotations = {path: 90 for path in temp_files}
            worker.apply_rotations(approved_rotations)

            # Should have some successes and some failures
            args = finished_mock.call_args[0]
            successes = args[0]
            failures = args[1]
            assert successes + failures == 10
            assert successes > 0
            assert failures > 0
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rotation_application_worker.calculate_max_workers")
    @patch("workers.rotation_application_worker.MetadataProcessor")
    def test_single_image_uses_sequential(
        self, mock_metadata_processor, mock_calculate_max_workers
    ):
        """Test that single image uses sequential processing even with multiple workers"""
        # Enable parallel mode capability
        mock_calculate_max_workers.return_value = 8
        mock_metadata_processor.try_metadata_rotation_first.return_value = (
            True,
            False,
            "Success",
        )

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            # Single image should use sequential path
            approved_rotations = {tmp_path: 90}
            worker.apply_rotations(approved_rotations)

            # Should succeed
            finished_mock.assert_called_once_with(1, 0)
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)
