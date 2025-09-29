"""
PyInstaller runtime hook to ensure pyexiv2 is loaded before Qt libraries.

This hook ensures that pyexiv2 is imported before any Qt/PyQt libraries
to prevent DLL conflicts on Windows.
"""

import sys
import os

# Add src directory to path for PyInstaller
hook_dir = os.path.dirname(__file__)
src_dir = os.path.join(hook_dir, "src")
if src_dir not in sys.path:
    sys.path.insert(0, src_dir)

# Use the centralized initialization function
from core.pyexiv2_init import ensure_pyexiv2_initialized  # noqa: E402

ensure_pyexiv2_initialized()
print("Runtime hook: pyexiv2 initialized successfully")
