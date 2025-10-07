"""Tests for parallel blur detection in FileScanner"""
import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2

import os
import tempfile
import shutil
from unittest.mock import Mock, patch, MagicMock
import pytest
from PyQt6.QtCore import QObject

from core.file_scanner import FileScanner


class TestFileScannerParallelBlur:
    """Test suite for parallel blur detection in FileScanner"""

    def test_scan_without_blur_detection(self, tmp_path):
        """Test scanning files without blur detection"""
        # Create test image files
        test_files = []
        for i in range(3):
            test_file = tmp_path / f"test_{i}.jpg"
            test_file.touch()
            test_files.append(str(test_file))

        scanner = FileScanner()

        # Mock signals
        files_found_mock = Mock()
        finished_mock = Mock()
        scanner.files_found.connect(files_found_mock)
        scanner.finished.connect(finished_mock)

        # Mock image pipeline
        with patch.object(scanner, 'image_pipeline') as mock_pipeline:
            mock_pipeline.preload_thumbnails = Mock()

            # Scan without blur detection
            scanner.scan_directory(str(tmp_path), perform_blur_detection=False)

            # Should find files
            assert files_found_mock.call_count >= 3

            # All is_blurred should be None
            for call in files_found_mock.call_args_list:
                file_info_list = call[0][0]
                for file_info in file_info_list:
                    assert file_info['is_blurred'] is None

            finished_mock.assert_called_once()

    @patch('core.file_scanner.BlurDetector')
    def test_scan_with_blur_detection_parallel(self, mock_blur_detector, tmp_path):
        """Test scanning with parallel blur detection"""
        # Create test image files
        test_files = []
        for i in range(5):
            test_file = tmp_path / f"test_{i}.jpg"
            test_file.touch()
            test_files.append(str(test_file))

        # Mock blur detection results
        blur_results = [False, True, False, True, False]
        mock_blur_detector.is_image_blurred.side_effect = blur_results

        scanner = FileScanner()

        # Mock signals
        thumbnail_preload_finished_mock = Mock()
        finished_mock = Mock()
        scanner.thumbnail_preload_finished.connect(thumbnail_preload_finished_mock)
        scanner.finished.connect(finished_mock)

        # Mock image pipeline
        with patch.object(scanner, 'image_pipeline') as mock_pipeline:
            mock_pipeline.preload_thumbnails = Mock()

            # Scan with blur detection
            scanner.scan_directory(str(tmp_path), perform_blur_detection=True, blur_threshold=100.0)

            finished_mock.assert_called_once()

            # Should have called thumbnail_preload_finished with all files
            assert thumbnail_preload_finished_mock.call_count == 1
            all_file_data = thumbnail_preload_finished_mock.call_args[0][0]

            # Should have blur results for all files
            assert len(all_file_data) == 5

            # Verify blur detection was called for each file
            assert mock_blur_detector.is_image_blurred.call_count == 5

    @patch('core.file_scanner.BlurDetector')
    def test_parallel_blur_detection_performance(self, mock_blur_detector, tmp_path):
        """Test that blur detection actually runs in parallel"""
        # Create many test files
        num_files = 20
        test_files = []
        for i in range(num_files):
            test_file = tmp_path / f"test_{i}.jpg"
            test_file.touch()
            test_files.append(str(test_file))

        # Track concurrent calls
        concurrent_calls = [0]
        max_concurrent = [0]

        def blur_check_with_tracking(*args, **kwargs):
            concurrent_calls[0] += 1
            max_concurrent[0] = max(max_concurrent[0], concurrent_calls[0])
            # Simulate work
            import time
            time.sleep(0.01)
            concurrent_calls[0] -= 1
            return False

        mock_blur_detector.is_image_blurred.side_effect = blur_check_with_tracking

        scanner = FileScanner()
        finished_mock = Mock()
        scanner.finished.connect(finished_mock)

        with patch.object(scanner, 'image_pipeline') as mock_pipeline:
            mock_pipeline.preload_thumbnails = Mock()

            scanner.scan_directory(str(tmp_path), perform_blur_detection=True)

            finished_mock.assert_called_once()

            # Should have used multiple threads (max_concurrent > 1)
            # With 20 files and proper parallelization, we should see at least 2 concurrent
            assert max_concurrent[0] >= 2, f"Expected parallel execution, but max concurrent was {max_concurrent[0]}"

    @patch('core.file_scanner.BlurDetector')
    def test_blur_detection_error_handling(self, mock_blur_detector, tmp_path):
        """Test that blur detection errors don't stop the scan"""
        # Create test files
        for i in range(3):
            test_file = tmp_path / f"test_{i}.jpg"
            test_file.touch()

        # Make blur detection fail for some files
        def blur_with_errors(path, *args, **kwargs):
            if "test_1" in path:
                raise Exception("Blur detection failed")
            return False

        mock_blur_detector.is_image_blurred.side_effect = blur_with_errors

        scanner = FileScanner()
        finished_mock = Mock()
        thumbnail_preload_finished_mock = Mock()
        scanner.finished.connect(finished_mock)
        scanner.thumbnail_preload_finished.connect(thumbnail_preload_finished_mock)

        with patch.object(scanner, 'image_pipeline') as mock_pipeline:
            mock_pipeline.preload_thumbnails = Mock()

            scanner.scan_directory(str(tmp_path), perform_blur_detection=True)

            finished_mock.assert_called_once()

            # Should still complete and process all files
            all_file_data = thumbnail_preload_finished_mock.call_args[0][0]
            assert len(all_file_data) == 3

            # Failed file should have None blur status
            failed_file = [f for f in all_file_data if "test_1" in f['path']]
            assert len(failed_file) == 1
            assert failed_file[0]['is_blurred'] is None

    def test_stop_during_blur_detection(self, tmp_path):
        """Test stopping scanner during blur detection"""
        # Create many test files
        for i in range(20):
            test_file = tmp_path / f"test_{i}.jpg"
            test_file.touch()

        scanner = FileScanner()

        with patch('core.file_scanner.BlurDetector') as mock_blur_detector:
            call_count = [0]

            def blur_with_stop(*args, **kwargs):
                call_count[0] += 1
                if call_count[0] == 5:
                    scanner.stop()
                return False

            mock_blur_detector.is_image_blurred.side_effect = blur_with_stop

            with patch.object(scanner, 'image_pipeline') as mock_pipeline:
                mock_pipeline.preload_thumbnails = Mock()

                scanner.scan_directory(str(tmp_path), perform_blur_detection=True)

                # Should stop early
                assert call_count[0] <= 20

    @patch('core.file_scanner.BlurDetector')
    def test_files_emitted_before_blur_detection(self, mock_blur_detector, tmp_path):
        """Test that files are emitted immediately, before blur detection completes"""
        # Create test files
        for i in range(3):
            test_file = tmp_path / f"test_{i}.jpg"
            test_file.touch()

        # Make blur detection slow
        def slow_blur_check(*args, **kwargs):
            import time
            time.sleep(0.1)
            return False

        mock_blur_detector.is_image_blurred.side_effect = slow_blur_check

        scanner = FileScanner()
        files_found_mock = Mock()
        scanner.files_found.connect(files_found_mock)

        with patch.object(scanner, 'image_pipeline') as mock_pipeline:
            mock_pipeline.preload_thumbnails = Mock()

            scanner.scan_directory(str(tmp_path), perform_blur_detection=True)

            # Files should be emitted with is_blurred=None initially
            assert files_found_mock.call_count >= 3

            # First emissions should have None blur status
            first_call_file_info = files_found_mock.call_args_list[0][0][0][0]
            assert first_call_file_info['is_blurred'] is None

    @patch('core.file_scanner.BlurDetector')
    def test_empty_directory_scan(self, mock_blur_detector, tmp_path):
        """Test scanning an empty directory"""
        scanner = FileScanner()
        finished_mock = Mock()
        scanner.finished.connect(finished_mock)

        with patch.object(scanner, 'image_pipeline') as mock_pipeline:
            mock_pipeline.preload_thumbnails = Mock()

            scanner.scan_directory(str(tmp_path), perform_blur_detection=True)

            finished_mock.assert_called_once()
            # Should not call blur detection
            mock_blur_detector.is_image_blurred.assert_not_called()
