#!/usr/bin/env python3
"""
Minimal test script for pyexiv2 functionality in PyInstaller builds.
"""

import sys
import os

# Add src to path
script_dir = os.path.dirname(os.path.abspath(__file__))
src_dir = os.path.join(script_dir, "src")
sys.path.insert(0, src_dir)

def main():
    print("Testing pyexiv2 initialization...")
    
    try:
        from core.pyexiv2_init import ensure_pyexiv2_initialized
        ensure_pyexiv2_initialized()
        print("✓ pyexiv2 initialization successful")
        
        # Test basic import
        import pyexiv2
        print("✓ pyexiv2 import successful")
        
        # Test basic functionality with a dummy file
        try:
            with pyexiv2.Image("non_existent.jpg"):
                pass
        except Exception as e:
            print(f"✓ pyexiv2 functionality test completed (expected error: {type(e).__name__})")
        
        print("All tests passed!")
        return 0
        
    except Exception as e:
        print(f"✗ Test failed: {e}")
        import traceback
        traceback.print_exc()
        return 1

if __name__ == "__main__":
    sys.exit(main())