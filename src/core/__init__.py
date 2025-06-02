# Core logic package

from .app_settings import (
    get_preview_cache_size_gb,
    set_preview_cache_size_gb,
    get_preview_cache_size_bytes,
    DEFAULT_PREVIEW_CACHE_SIZE_GB,
    PREVIEW_CACHE_SIZE_KEY,
    ORGANIZATION_NAME,
    APPLICATION_NAME
)

from .file_scanner import FileScanner, SUPPORTED_EXTENSIONS
from .metadata_processor import MetadataProcessor # New metadata processor
from .app_settings import DEFAULT_CLIP_MODEL, is_pytorch_cuda_available # Import from app_settings

# Image processing and caching sub-packages/modules
from .image_processing.raw_image_processor import RawImageProcessor, is_raw_extension
from .image_processing.standard_image_processor import StandardImageProcessor, SUPPORTED_STANDARD_EXTENSIONS as SUPPORTED_STD_IMG_EXTENSIONS # Alias to avoid name clash
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
    "DEFAULT_PREVIEW_CACHE_SIZE_GB",
    "PREVIEW_CACHE_SIZE_KEY",
    "ORGANIZATION_NAME",
    "APPLICATION_NAME",
    # file_scanner
    "FileScanner",
    "SUPPORTED_EXTENSIONS",
    # metadata_processor (was rating_handler)
    "MetadataProcessor",
    # similarity_engine
    # "SimilarityEngine", # Removed as it's no longer directly imported here
    "DEFAULT_CLIP_MODEL",
    "is_pytorch_cuda_available", # Updated to export the function
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