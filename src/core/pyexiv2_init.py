"""
PyExiv2 initialization module.

This module must be imported before any Qt/PyQt imports to avoid DLL conflicts
on Windows. PyExiv2 has a known issue where it must be imported before Qt
libraries to prevent access violations.

This module should be imported at the very beginning of the application.
"""

import sys
import logging
import threading

logger = logging.getLogger(__name__)

# Global flag to track if pyexiv2 has been safely initialized
_PYEXIV2_INITIALIZED = False
_INIT_LOCK = threading.Lock()


def ensure_pyexiv2_initialized():
    """
    Ensure pyexiv2 is safely initialized before any Qt imports.
    This function is idempotent and safe to call multiple times.
    """
    global _PYEXIV2_INITIALIZED

    with _INIT_LOCK:
        if _PYEXIV2_INITIALIZED:
            return

        try:
            # Check if any Qt modules have already been imported
            qt_modules = [
                name
                for name in sys.modules.keys()
                if name.startswith(("PyQt", "PySide", "Qt"))
            ]

            if qt_modules:
                logger.warning(
                    f"Qt modules already imported before pyexiv2 initialization: {qt_modules}. "
                    "This may cause DLL conflicts on Windows."
                )

            # Import pyexiv2 to ensure it's loaded first
            import pyexiv2  # noqa: F401

            # Try to create a dummy image to fully initialize the library
            # This ensures all DLLs are loaded before any Qt operations
            try:
                # Test with a non-existent file to trigger initialization without side effects
                test_path = "non_existent_file_for_init.jpg"
                try:
                    with pyexiv2.Image(test_path):
                        pass
                except (FileNotFoundError, OSError, RuntimeError):
                    # Expected - we just want to trigger initialization
                    pass
            except Exception as init_error:
                logger.debug(
                    f"Pyexiv2 initialization test failed (this is expected): {init_error}"
                )

            _PYEXIV2_INITIALIZED = True
            logger.debug("pyexiv2 successfully initialized before Qt imports")

        except Exception as e:
            logger.error(f"Failed to initialize pyexiv2: {e}")
            raise


# Initialize immediately when this module is imported
ensure_pyexiv2_initialized()
