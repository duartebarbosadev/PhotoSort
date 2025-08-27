"""
Tests for worker manager automatic RAW processing functionality.

This module tests that the WorkerManager correctly handles the updated
file scanning interface without apply_auto_edits parameters.
"""

import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2
import inspect
from unittest.mock import Mock, patch
from src.ui.worker_manager import WorkerManager


class TestWorkerManagerAutomaticRaw:
    """Tests for WorkerManager automatic RAW processing integration."""

    def test_start_file_scan_method_signature(self):
        """Test that start_file_scan no longer has apply_auto_edits parameter."""
        mock_image_pipeline = Mock()
        worker_manager = WorkerManager(mock_image_pipeline)

        # Check method signature
        sig = inspect.signature(worker_manager.start_file_scan)
        params = list(sig.parameters.keys())

        expected_params = ["folder_path", "perform_blur_detection", "blur_threshold"]

        # Check that apply_auto_edits is not in parameters
        assert "apply_auto_edits" not in params, (
            f"start_file_scan should not have 'apply_auto_edits' parameter. "
            f"Found parameters: {params}"
        )

        # Check that expected parameters are present
        for expected_param in expected_params:
            assert expected_param in params, (
                f"start_file_scan should have '{expected_param}' parameter. "
                f"Found parameters: {params}"
            )

    @patch("src.ui.worker_manager.QThread")
    @patch("src.ui.worker_manager.FileScanner")
    def test_start_file_scan_calls_scanner_without_apply_auto_edits(
        self, mock_file_scanner_class, mock_qthread
    ):
        """Test that file scanner is called without apply_auto_edits parameter."""
        # Setup mocks
        mock_scanner = Mock()
        mock_file_scanner_class.return_value = mock_scanner
        mock_thread = Mock()
        mock_qthread.return_value = mock_thread

        worker_manager = WorkerManager(Mock())

        # Call start_file_scan
        worker_manager.start_file_scan(
            folder_path="/test/path", perform_blur_detection=True, blur_threshold=85.0
        )

        # Verify QThread was created and started
        mock_qthread.assert_called_once()
        mock_thread.start.assert_called_once()

        # Verify that the scanner's scan_directory method would be called with correct parameters
        # The actual call happens in a lambda connected to thread.started signal
        # We can verify the connection was made correctly
        assert mock_thread.started.connect.called

        # Get the lambda function that was connected
        connected_lambda = mock_thread.started.connect.call_args[0][0]

        # Execute the lambda to see what it calls
        connected_lambda()

        # Verify scanner.scan_directory was called with the right parameters
        mock_scanner.scan_directory.assert_called_once_with(
            "/test/path", perform_blur_detection=True, blur_threshold=85.0
        )

        # Verify apply_auto_edits was NOT passed
        call_args = mock_scanner.scan_directory.call_args
        args, kwargs = call_args
        assert "apply_auto_edits" not in kwargs

    @patch("src.ui.worker_manager.QThread")
    @patch("src.ui.worker_manager.FileScanner")
    def test_file_scan_workflow_integration(
        self, mock_file_scanner_class, mock_qthread
    ):
        """Test complete file scan workflow without apply_auto_edits."""
        # Setup mocks
        mock_scanner = Mock()
        mock_file_scanner_class.return_value = mock_scanner
        mock_thread = Mock()
        mock_qthread.return_value = mock_thread

        # Mock signal connections
        mock_scanner.files_found = Mock()
        mock_scanner.thumbnail_preload_progress = Mock()
        mock_scanner.thumbnail_preload_finished = Mock()
        mock_scanner.finished = Mock()
        mock_scanner.error = Mock()

        worker_manager = WorkerManager(Mock())

        # Start file scan
        worker_manager.start_file_scan(
            folder_path="/test/images",
            perform_blur_detection=False,
            blur_threshold=100.0,
        )

        # Verify all necessary signal connections were made
        mock_scanner.files_found.connect.assert_called()
        mock_scanner.thumbnail_preload_finished.connect.assert_called()
        mock_scanner.finished.connect.assert_called()
        mock_scanner.error.connect.assert_called()

        # Verify thread management
        mock_thread.started.connect.assert_called()
        # The actual signal connections are internal and difficult to test without complex mocking
        # We mainly care that the scanner is called correctly without apply_auto_edits
        mock_thread.finished.connect.assert_called()
        mock_thread.start.assert_called_once()

    def test_stop_file_scan_cleanup(self):
        """Test that stopping file scan properly cleans up resources."""
        worker_manager = WorkerManager(Mock())

        # Mock existing scanner and thread
        mock_scanner = Mock()
        mock_thread = Mock()

        worker_manager.file_scanner = mock_scanner
        worker_manager.scanner_thread = mock_thread

        # Call stop_file_scan
        worker_manager.stop_file_scan()

        # Verify cleanup
        mock_scanner.stop.assert_called_once()
        mock_thread.quit.assert_called_once()
        mock_thread.wait.assert_called_once_with(5000)  # 5 second timeout

    @patch("src.ui.worker_manager.QThread")
    @patch("src.ui.worker_manager.FileScanner")
    def test_multiple_scan_requests_stop_previous(
        self, mock_file_scanner_class, mock_qthread
    ):
        """Test that starting a new scan stops any previous scan."""
        # Setup mocks
        mock_scanner1 = Mock()
        mock_scanner2 = Mock()
        mock_file_scanner_class.side_effect = [mock_scanner1, mock_scanner2]

        mock_thread1 = Mock()
        mock_thread2 = Mock()
        mock_qthread.side_effect = [mock_thread1, mock_thread2]

        worker_manager = WorkerManager(Mock())

        # Start first scan
        worker_manager.start_file_scan(
            folder_path="/test/path1", perform_blur_detection=True, blur_threshold=90.0
        )

        # Verify first scan setup
        assert worker_manager.file_scanner == mock_scanner1
        assert worker_manager.scanner_thread == mock_thread1

        # Start second scan
        worker_manager.start_file_scan(
            folder_path="/test/path2", perform_blur_detection=False, blur_threshold=80.0
        )

        # Verify first scan was stopped
        mock_scanner1.stop.assert_called_once()
        mock_thread1.quit.assert_called_once()
        mock_thread1.wait.assert_called_once()

        # Verify second scan was started
        assert worker_manager.file_scanner == mock_scanner2
        assert worker_manager.scanner_thread == mock_thread2
        mock_thread2.start.assert_called_once()


class TestWorkerManagerBackwardCompatibility:
    """Tests ensuring backward compatibility while removing apply_auto_edits."""

    def test_method_still_accepts_expected_parameters(self):
        """Test that start_file_scan still accepts all expected parameters."""
        worker_manager = WorkerManager(Mock())

        # Mock dependencies
        with (
            patch("src.ui.worker_manager.QThread") as mock_qthread,
            patch("src.ui.worker_manager.FileScanner") as mock_scanner_class,
        ):
            mock_thread = Mock()
            mock_scanner = Mock()
            mock_qthread.return_value = mock_thread
            mock_scanner_class.return_value = mock_scanner

            # This should work without raising any TypeError
            worker_manager.start_file_scan(
                folder_path="/test", perform_blur_detection=True, blur_threshold=95.0
            )

            # Verify it worked
            mock_qthread.assert_called_once()
            mock_scanner_class.assert_called_once()

    def test_no_deprecated_parameters_in_any_method(self):
        """Test that no WorkerManager methods have apply_auto_edits parameters."""
        worker_manager = WorkerManager(Mock())

        # Get all public methods
        methods = [
            method
            for method in dir(worker_manager)
            if callable(getattr(worker_manager, method)) and not method.startswith("_")
        ]

        for method_name in methods:
            if method_name.startswith("start_") or method_name in ["scan", "process"]:
                method = getattr(worker_manager, method_name)
                sig = inspect.signature(method)
                params = list(sig.parameters.keys())

                assert "apply_auto_edits" not in params, (
                    f"Method {method_name} should not have 'apply_auto_edits' parameter. "
                    f"Found parameters: {params}"
                )
