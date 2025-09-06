import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2

from unittest.mock import patch
from core.pyexiv2_init import ensure_pyexiv2_initialized, _PYEXIV2_INITIALIZED


class TestPyExiv2Init:
    """Test cases for PyExiv2 initialization module."""

    def test_ensure_pyexiv2_initialized_idempotent(self):
        """Test that ensure_pyexiv2_initialized can be called multiple times safely."""
        # Should not raise an exception
        ensure_pyexiv2_initialized()
        ensure_pyexiv2_initialized()
        ensure_pyexiv2_initialized()

        # Should set the global flag
        assert _PYEXIV2_INITIALIZED is True

    def test_ensure_pyexiv2_initialized_thread_safety(self):
        """Test that initialization is thread-safe."""
        import threading

        results = []
        errors = []

        def init_worker():
            try:
                ensure_pyexiv2_initialized()
                results.append(True)
            except Exception as e:
                errors.append(e)

        # Create multiple threads
        threads = []
        for _ in range(5):
            thread = threading.Thread(target=init_worker)
            threads.append(thread)
            thread.start()

        # Wait for all threads to complete
        for thread in threads:
            thread.join()

        # All threads should succeed
        assert len(results) == 5
        assert len(errors) == 0
        assert _PYEXIV2_INITIALIZED is True

    def test_qt_modules_warning(self, caplog):
        """Test that warning is logged if Qt modules are imported before pyexiv2."""
        # Reset the initialization flag for this test
        import core.pyexiv2_init

        original_flag = core.pyexiv2_init._PYEXIV2_INITIALIZED
        core.pyexiv2_init._PYEXIV2_INITIALIZED = False

        try:
            with patch("core.pyexiv2_init.sys.modules") as mock_modules:
                # Mock sys.modules to include Qt modules
                mock_modules.keys.return_value = [
                    "PyQt6.QtCore",
                    "PyQt6.QtWidgets",
                    "other_module",
                ]

                ensure_pyexiv2_initialized()

                # Should log a warning about Qt modules
                assert "Qt modules already imported" in caplog.text
                assert "PyQt6.QtCore" in caplog.text
        finally:
            # Restore the original flag
            core.pyexiv2_init._PYEXIV2_INITIALIZED = original_flag

    def test_initialization_error_handling(self):
        """Test that initialization errors are handled gracefully."""
        # Reset the global flag for this test
        import core.pyexiv2_init

        original_flag = core.pyexiv2_init._PYEXIV2_INITIALIZED
        core.pyexiv2_init._PYEXIV2_INITIALIZED = False

        try:
            with patch("core.pyexiv2_init.logger") as mock_logger:
                # Mock a failure in the test initialization part
                with patch("pyexiv2.Image") as mock_image:
                    mock_image.side_effect = RuntimeError("Test error")

                    # Should not raise, but should log the error
                    ensure_pyexiv2_initialized()

                    # Should log debug message about test failure (which is expected)
                    mock_logger.debug.assert_called()
        finally:
            # Restore the original flag
            core.pyexiv2_init._PYEXIV2_INITIALIZED = original_flag
