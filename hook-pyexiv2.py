"""
PyInstaller hook for pyexiv2 package.

This hook ensures that pyexiv2's native libraries are properly collected
and bundled with the frozen application.
"""

from PyInstaller.utils.hooks import collect_data_files, collect_dynamic_libs
from PyInstaller.compat import is_darwin, is_win
import os
import glob

# Collect all data files from pyexiv2 package (includes lib directory)
datas = collect_data_files('pyexiv2')

# Collect dynamic libraries specifically
binaries = collect_dynamic_libs('pyexiv2')

# Additional hidden imports for pyexiv2 dependencies
hiddenimports = []

# Ensure the lib directory is included with all its files
try:
    import pyexiv2
    pyexiv2_path = os.path.dirname(pyexiv2.__file__)
    lib_path = os.path.join(pyexiv2_path, 'lib')
    
    if os.path.exists(lib_path):
        print(f"PyInstaller hook: Found pyexiv2 lib directory at {lib_path}")
        
        # Add all files in the lib directory
        for root, dirs, files in os.walk(lib_path):
            for file in files:
                src_path = os.path.join(root, file)
                rel_path = os.path.relpath(src_path, pyexiv2_path)
                dst_path = os.path.join('pyexiv2', rel_path)
                
                # Add as data file
                datas.append((src_path, os.path.dirname(dst_path)))
                
                # If it's a dynamic library, also add to binaries for proper loading
                if file.endswith(('.so', '.dll', '.dylib')):
                    # For binaries, put them in the root or lib subdirectory
                    if is_darwin:
                        binaries.append((src_path, 'pyexiv2/lib'))
                    elif is_win:
                        binaries.append((src_path, '.'))
                    else:  # Linux
                        binaries.append((src_path, 'pyexiv2/lib'))
                    print(f"PyInstaller hook: Added binary {file} -> {dst_path}")
                    
                    # Special handling for libexiv2.so - create a versioned symlink
                    if file == 'libexiv2.so':
                        # Also add it as libexiv2.so.28 which is what exiv2api.so expects
                        # Format: (source_path, dest_dir)
                        versioned_dst = os.path.join('pyexiv2', 'lib') if not is_win else '.'
                        # We'll handle this in the runtime hook instead since PyInstaller binaries format
                        # doesn't support renaming easily
                        pass

except ImportError:
    print("PyInstaller hook: pyexiv2 not available during hook execution")
    pass

print(f"PyInstaller hook: Collected {len(datas)} data files and {len(binaries)} binaries for pyexiv2")