"""Tests for RotationApplicationWorker"""
import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2

import os
import tempfile
from unittest.mock import Mock, patch
import pytest
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

    @patch('workers.rotation_application_worker.MetadataProcessor')
    def test_apply_single_rotation_clockwise_success(self, mock_metadata_processor):
        """Test applying a single clockwise rotation"""
        # Mock metadata-first rotation success
        mock_metadata_processor.try_metadata_rotation_first.return_value = (True, False, "Success")

        worker = RotationApplicationWorker()

        # Set up signal mocks
        progress_mock = Mock()
        rotation_applied_mock = Mock()
        finished_mock = Mock()

        worker.progress.connect(progress_mock)
        worker.rotation_applied.connect(rotation_applied_mock)
        worker.finished.connect(finished_mock)

        # Create temp file
        with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as tmp:
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

    @patch('workers.rotation_application_worker.MetadataProcessor')
    def test_apply_rotation_counterclockwise(self, mock_metadata_processor):
        """Test applying counterclockwise rotation"""
        mock_metadata_processor.try_metadata_rotation_first.return_value = (True, False, "Success")

        worker = RotationApplicationWorker()
        rotation_applied_mock = Mock()
        worker.rotation_applied.connect(rotation_applied_mock)

        with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as tmp:
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

    @patch('workers.rotation_application_worker.MetadataProcessor')
    def test_apply_rotation_180(self, mock_metadata_processor):
        """Test applying 180 degree rotation"""
        mock_metadata_processor.try_metadata_rotation_first.return_value = (True, False, "Success")

        worker = RotationApplicationWorker()
        rotation_applied_mock = Mock()
        worker.rotation_applied.connect(rotation_applied_mock)

        with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as tmp:
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

    @patch('workers.rotation_application_worker.MetadataProcessor')
    def test_apply_rotation_lossy_fallback(self, mock_metadata_processor):
        """Test lossy rotation fallback when metadata rotation fails"""
        # Metadata rotation indicates lossy needed
        mock_metadata_processor.try_metadata_rotation_first.return_value = (False, True, "Needs lossy")
        mock_metadata_processor.rotate_image.return_value = True

        worker = RotationApplicationWorker()
        rotation_applied_mock = Mock()
        worker.rotation_applied.connect(rotation_applied_mock)

        with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as tmp:
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

    @patch('workers.rotation_application_worker.MetadataProcessor')
    def test_apply_multiple_rotations(self, mock_metadata_processor):
        """Test applying rotations to multiple files"""
        mock_metadata_processor.try_metadata_rotation_first.return_value = (True, False, "Success")

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        temp_files = []
        for i in range(3):
            with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as tmp:
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

    @patch('workers.rotation_application_worker.MetadataProcessor')
    def test_unsupported_rotation_angle(self, mock_metadata_processor):
        """Test handling unsupported rotation angles"""
        worker = RotationApplicationWorker()
        rotation_applied_mock = Mock()
        finished_mock = Mock()

        worker.rotation_applied.connect(rotation_applied_mock)
        worker.finished.connect(finished_mock)

        with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as tmp:
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

    @patch('workers.rotation_application_worker.MetadataProcessor')
    def test_rotation_failure(self, mock_metadata_processor):
        """Test handling rotation failures"""
        # Both metadata and lossy fail
        mock_metadata_processor.try_metadata_rotation_first.return_value = (False, True, "Needs lossy")
        mock_metadata_processor.rotate_image.return_value = False

        worker = RotationApplicationWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as tmp:
            tmp_path = tmp.name

        try:
            approved_rotations = {tmp_path: 90}
            worker.apply_rotations(approved_rotations)

            # Should report failure
            finished_mock.assert_called_once_with(0, 1)
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    @patch('workers.rotation_application_worker.MetadataProcessor')
    def test_stop_during_processing(self, mock_metadata_processor):
        """Test stopping worker during batch rotation"""
        mock_metadata_processor.try_metadata_rotation_first.return_value = (True, False, "Success")

        worker = RotationApplicationWorker()

        temp_files = []
        for i in range(10):
            with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            # Stop after first call
            call_count = [0]

            def stop_after_first(*args, **kwargs):
                call_count[0] += 1
                if call_count[0] == 1:
                    worker.stop()
                return (True, False, "Success")

            mock_metadata_processor.try_metadata_rotation_first.side_effect = stop_after_first

            approved_rotations = {path: 90 for path in temp_files}
            worker.apply_rotations(approved_rotations)

            # Should stop early
            assert mock_metadata_processor.try_metadata_rotation_first.call_count < 10
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch('workers.rotation_application_worker.MetadataProcessor')
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
            with tempfile.NamedTemporaryFile(suffix='.jpg', delete=False) as tmp:
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
