"""
PyInstaller runtime hook to ensure pyexiv2 is loaded before Qt libraries.

This hook ensures that pyexiv2 is imported before any Qt/PyQt libraries
to prevent DLL conflicts on Windows.
"""

import sys

# Import pyexiv2 first to prevent Qt conflicts
try:
    import pyexiv2  # noqa: F401

    # Force initialization of pyexiv2 library to load all DLLs
    try:
        # Try to create a dummy image to fully initialize the library
        test_path = "non_existent_file_for_init.jpg"
        try:
            with pyexiv2.Image(test_path):
                pass
        except (FileNotFoundError, OSError, RuntimeError):
            # Expected - we just want to trigger initialization
            pass
    except Exception:
        # Initialization may fail, but that's okay as long as pyexiv2 is imported
        pass

except ImportError:
    # If pyexiv2 is not available, log but don't fail
    print("Warning: pyexiv2 not available in runtime hook")

# Check if any Qt modules are already loaded
qt_modules = [
    name for name in sys.modules.keys() if name.startswith(("PyQt", "PySide", "Qt"))
]
if qt_modules:
    print(f"Warning: Qt modules detected before pyexiv2 initialization: {qt_modules}")
else:
    print("Runtime hook: pyexiv2 loaded before Qt modules")
