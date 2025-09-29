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


def _setup_pyexiv2_library_path():
    """Set up library paths for pyexiv2 in frozen applications."""
    if not getattr(sys, 'frozen', False):
        return  # Not frozen, normal import should work
    
    # We're in a PyInstaller frozen app
    import os
    
    # Try to find the bundled pyexiv2 directory
    if hasattr(sys, '_MEIPASS'):
        # PyInstaller temporary directory
        base_path = sys._MEIPASS
        pyexiv2_lib_paths = [
            os.path.join(base_path, 'pyexiv2', 'lib'),
            os.path.join(base_path, 'pyexiv2'),
        ]
    else:
        # Fallback for other frozen environments
        base_path = os.path.dirname(sys.executable)
        pyexiv2_lib_paths = [
            os.path.join(base_path, 'pyexiv2', 'lib'),
            os.path.join(base_path, 'Frameworks', 'pyexiv2', 'lib'),  # macOS app bundle structure
            os.path.join(base_path, '..', 'Frameworks', 'pyexiv2', 'lib'),  # Alternative macOS path
        ]
    
    # Add any existing pyexiv2 lib directories to the library path
    for lib_path in pyexiv2_lib_paths:
        if os.path.isdir(lib_path):
            logger.debug(f"Adding pyexiv2 lib path: {lib_path}")
            
            # Add to system library path
            if sys.platform.startswith('darwin'):  # macOS
                dyld_path = os.environ.get('DYLD_LIBRARY_PATH', '')
                if lib_path not in dyld_path:
                    os.environ['DYLD_LIBRARY_PATH'] = f"{lib_path}:{dyld_path}" if dyld_path else lib_path
            elif sys.platform.startswith('linux'):  # Linux
                ld_path = os.environ.get('LD_LIBRARY_PATH', '')
                if lib_path not in ld_path:
                    os.environ['LD_LIBRARY_PATH'] = f"{lib_path}:{ld_path}" if ld_path else lib_path
            elif sys.platform.startswith('win'):  # Windows
                # Windows uses PATH for DLL loading
                path = os.environ.get('PATH', '')
                if lib_path not in path:
                    os.environ['PATH'] = f"{lib_path};{path}"


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
            # Set up library paths for frozen applications
            _setup_pyexiv2_library_path()

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
            # Instead of raising, log and continue - allow app to start even if pyexiv2 fails
            logger.warning("Continuing without pyexiv2 functionality")
            _PYEXIV2_INITIALIZED = True  # Mark as "initialized" to prevent repeated attempts


# Initialize immediately when this module is imported
ensure_pyexiv2_initialized()
