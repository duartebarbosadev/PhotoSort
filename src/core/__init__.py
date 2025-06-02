# Core logic package

from .app_settings import (
    get_preview_cache_size_gb,
    set_preview_cache_size_gb,
    get_preview_cache_size_bytes,
    get_exif_cache_size_mb,
    set_exif_cache_size_mb,
    get_exif_cache_size_bytes,
    DEFAULT_PREVIEW_CACHE_SIZE_GB,
    DEFAULT_EXIF_CACHE_SIZE_MB,
    PREVIEW_CACHE_SIZE_GB_KEY,
    EXIF_CACHE_SIZE_MB_KEY,
    SETTINGS_ORGANIZATION,
    SETTINGS_APPLICATION,
    is_pytorch_cuda_available
)

from .file_scanner import FileScanner, SUPPORTED_EXTENSIONS
from .metadata_processor import MetadataProcessor

# Image processing and caching sub-packages/modules
from .image_processing.raw_image_processor import RawImageProcessor, is_raw_extension
from .image_processing.standard_image_processor import StandardImageProcessor, SUPPORTED_STANDARD_EXTENSIONS as SUPPORTED_STD_IMG_EXTENSIONS
from .image_processing.image_orientation_handler import ImageOrientationHandler

from .image_features.blur_detector import BlurDetector, BLUR_DETECTION_PREVIEW_SIZE

from .caching.thumbnail_cache import ThumbnailCache
from .caching.preview_cache import PreviewCache

from .image_file_ops import ImageFileOperations
from .image_pipeline import ImagePipeline, THUMBNAIL_MAX_SIZE, PRELOAD_MAX_RESOLUTION

__all__ = [
    # app_settings
    "get_preview_cache_size_gb",
    "set_preview_cache_size_gb", 
    "get_preview_cache_size_bytes",
    "get_exif_cache_size_mb",
    "set_exif_cache_size_mb",
    "get_exif_cache_size_bytes",
    "DEFAULT_PREVIEW_CACHE_SIZE_GB",
    "DEFAULT_EXIF_CACHE_SIZE_MB",
    "PREVIEW_CACHE_SIZE_GB_KEY",
    "EXIF_CACHE_SIZE_MB_KEY",
    "SETTINGS_ORGANIZATION", 
    "SETTINGS_APPLICATION",
    "is_pytorch_cuda_available",
    # file_scanner
    "FileScanner",
    "SUPPORTED_EXTENSIONS",
    # metadata_processor
    "MetadataProcessor",
    # image_processing
    "RawImageProcessor",
    "is_raw_extension",
    "StandardImageProcessor",
    "SUPPORTED_STD_IMG_EXTENSIONS",
    "ImageOrientationHandler",
    # image_features
    "BlurDetector",
    "BLUR_DETECTION_PREVIEW_SIZE",
    # caching
    "ThumbnailCache",
    "PreviewCache",
    # image_file_ops
    "ImageFileOperations",
    # image_pipeline
    "ImagePipeline",
    "THUMBNAIL_MAX_SIZE",
    "PRELOAD_MAX_RESOLUTION",
]