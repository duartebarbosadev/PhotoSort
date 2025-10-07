"""Tests for RatingWriterWorker"""

import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2

import os
import tempfile
from unittest.mock import Mock, patch
from PyQt6.QtCore import QObject

from workers.rating_writer_worker import RatingWriterWorker


class TestRatingWriterWorker:
    """Test suite for RatingWriterWorker"""

    def test_worker_initialization(self):
        """Test worker initializes correctly"""
        worker = RatingWriterWorker()
        assert worker is not None
        assert isinstance(worker, QObject)
        assert worker._is_running is True

    def test_worker_stop(self):
        """Test worker stop method"""
        worker = RatingWriterWorker()
        assert worker._is_running is True
        worker.stop()
        assert worker._is_running is False

    @patch("workers.rating_writer_worker.MetadataProcessor")
    def test_write_single_rating_success(self, mock_metadata_processor):
        """Test writing a single rating successfully"""
        mock_metadata_processor.set_rating.return_value = True

        worker = RatingWriterWorker()

        # Set up signal mocks
        progress_mock = Mock()
        rating_written_mock = Mock()
        finished_mock = Mock()

        worker.progress.connect(progress_mock)
        worker.rating_written.connect(rating_written_mock)
        worker.finished.connect(finished_mock)

        # Create temp file
        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            # Write rating
            rating_operations = [(tmp_path, 5)]
            worker.write_ratings(rating_operations)

            # Verify signals
            progress_mock.assert_called_once_with(1, 1, os.path.basename(tmp_path))
            rating_written_mock.assert_called_once_with(tmp_path, 5, True)
            finished_mock.assert_called_once_with(1, 0)  # 1 success, 0 failures

            # Verify MetadataProcessor was called
            mock_metadata_processor.set_rating.assert_called_once()
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    @patch("workers.rating_writer_worker.MetadataProcessor")
    def test_write_multiple_ratings(self, mock_metadata_processor):
        """Test writing multiple ratings"""
        mock_metadata_processor.set_rating.return_value = True

        worker = RatingWriterWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        # Create temp files
        temp_files = []
        for i in range(3):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            rating_operations = [(path, 3) for path in temp_files]
            worker.write_ratings(rating_operations)

            # Should finish with 3 successes, 0 failures
            finished_mock.assert_called_once_with(3, 0)
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rating_writer_worker.MetadataProcessor")
    def test_write_rating_failure(self, mock_metadata_processor):
        """Test handling rating write failures"""
        mock_metadata_processor.set_rating.return_value = False

        worker = RatingWriterWorker()
        rating_written_mock = Mock()
        finished_mock = Mock()

        worker.rating_written.connect(rating_written_mock)
        worker.finished.connect(finished_mock)

        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            rating_operations = [(tmp_path, 5)]
            worker.write_ratings(rating_operations)

            # Should report failure
            rating_written_mock.assert_called_once_with(tmp_path, 5, False)
            finished_mock.assert_called_once_with(0, 1)  # 0 success, 1 failure
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)

    def test_write_rating_missing_file(self):
        """Test handling missing files"""
        worker = RatingWriterWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        # Non-existent file
        rating_operations = [("/nonexistent/file.jpg", 5)]
        worker.write_ratings(rating_operations)

        # Should complete with failure
        finished_mock.assert_called_once_with(0, 1)

    @patch("workers.rating_writer_worker.MetadataProcessor")
    def test_stop_during_processing(self, mock_metadata_processor):
        """Test stopping worker during processing"""
        mock_metadata_processor.set_rating.return_value = True

        worker = RatingWriterWorker()

        # Create many temp files
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
                return True

            mock_metadata_processor.set_rating.side_effect = stop_after_first

            rating_operations = [(path, 3) for path in temp_files]
            worker.write_ratings(rating_operations)

            # Should stop early, not process all files
            assert mock_metadata_processor.set_rating.call_count < 10
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rating_writer_worker.MetadataProcessor")
    def test_mixed_success_and_failure(self, mock_metadata_processor):
        """Test handling mix of successes and failures"""
        # Alternate success/failure
        mock_metadata_processor.set_rating.side_effect = [True, False, True, False]

        worker = RatingWriterWorker()
        finished_mock = Mock()
        worker.finished.connect(finished_mock)

        temp_files = []
        for i in range(4):
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
                temp_files.append(tmp.name)

        try:
            rating_operations = [(path, 3) for path in temp_files]
            worker.write_ratings(rating_operations)

            # Should have 2 successes, 2 failures
            finished_mock.assert_called_once_with(2, 2)
        finally:
            for path in temp_files:
                if os.path.exists(path):
                    os.unlink(path)

    @patch("workers.rating_writer_worker.MetadataProcessor")
    def test_exception_during_write(self, mock_metadata_processor):
        """Test handling exceptions during write"""
        mock_metadata_processor.set_rating.side_effect = Exception("Test error")

        worker = RatingWriterWorker()
        rating_written_mock = Mock()
        finished_mock = Mock()

        worker.rating_written.connect(rating_written_mock)
        worker.finished.connect(finished_mock)

        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as tmp:
            tmp_path = tmp.name

        try:
            rating_operations = [(tmp_path, 5)]
            worker.write_ratings(rating_operations)

            # Should report failure
            rating_written_mock.assert_called_once_with(tmp_path, 5, False)
            finished_mock.assert_called_once_with(0, 1)
        finally:
            if os.path.exists(tmp_path):
                os.unlink(tmp_path)
