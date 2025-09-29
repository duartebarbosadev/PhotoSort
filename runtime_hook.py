"""
PyInstaller runtime hook to ensure pyexiv2 is loaded before Qt libraries.

This hook ensures that pyexiv2 is imported before any Qt/PyQt libraries
to prevent DLL conflicts on Windows.
"""

import sys
import os

# Try to set up the path for finding modules
hook_dir = os.path.dirname(__file__)

# In PyInstaller, try multiple path setups
if getattr(sys, 'frozen', False):
    # We're in a PyInstaller frozen app
    if hasattr(sys, '_MEIPASS'):
        # Add _MEIPASS and src subdirectory to path
        sys.path.insert(0, sys._MEIPASS)
        src_path = os.path.join(sys._MEIPASS, 'src')
        if os.path.exists(src_path):
            sys.path.insert(0, src_path)
else:
    # Development mode - add src directory to path
    src_dir = os.path.join(hook_dir, "src")
    if src_dir not in sys.path and os.path.exists(src_dir):
        sys.path.insert(0, src_dir)

# Use the centralized initialization function
try:
    from core.pyexiv2_init import ensure_pyexiv2_initialized  # noqa: E402
    ensure_pyexiv2_initialized()
    print("Runtime hook: pyexiv2 initialized successfully")
except ImportError as e:
    print(f"Runtime hook: Could not import pyexiv2_init module: {e}")
    print("Runtime hook: Attempting direct pyexiv2 initialization")
    try:
        import pyexiv2  # noqa: F401
        print("Runtime hook: Direct pyexiv2 import successful")
    except Exception as e2:
        print(f"Runtime hook: Failed to initialize pyexiv2: {e2}")
        print("Runtime hook: Continuing without pyexiv2 functionality")
except Exception as e:
    print(f"Runtime hook: Failed to initialize pyexiv2: {e}")
    print("Runtime hook: Continuing without pyexiv2 functionality")
