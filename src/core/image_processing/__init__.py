# This file makes Python treat the directory 'image_processing' as a package.
try:
    import pyexiv2
    pyexiv2.set_log_level(0)
except ImportError:
    pass