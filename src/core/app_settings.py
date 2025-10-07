"""
Application Settings Module
Manages persistent application settings using QSettings.
"""

import os
from PyQt6.QtCore import QSettings

# --- Settings Constants ---

# Settings organization and application name
SETTINGS_ORGANIZATION = "PhotoSort"
SETTINGS_APPLICATION = "PhotoSort"

# Settings keys
PREVIEW_CACHE_SIZE_GB_KEY = "Cache/PreviewCacheSizeGB"
EXIF_CACHE_SIZE_MB_KEY = "Cache/ExifCacheSizeMB"  # For EXIF metadata cache
ROTATION_CONFIRM_LOSSY_KEY = "UI/RotationConfirmLossy"  # Ask before lossy rotation
RECENT_FOLDERS_KEY = "UI/RecentFolders"  # Key for recent folders list
ORIENTATION_MODEL_NAME_KEY = (
    "Models/OrientationModelName"  # Key for the orientation model file name
)
UPDATE_CHECK_ENABLED_KEY = "Updates/CheckEnabled"  # Enable automatic update checks
UPDATE_LAST_CHECK_KEY = "Updates/LastCheckTime"  # Last time updates were checked

# Default values
DEFAULT_PREVIEW_CACHE_SIZE_GB = 2.0  # Default to 2 GB for preview cache
DEFAULT_EXIF_CACHE_SIZE_MB = 256  # Default to 256 MB for EXIF cache
DEFAULT_ROTATION_CONFIRM_LOSSY = True  # Default to asking before lossy rotation
MAX_RECENT_FOLDERS = 10  # Max number of recent folders to store
DEFAULT_ORIENTATION_MODEL_NAME = None  # Default to None, so we can auto-detect
DEFAULT_UPDATE_CHECK_ENABLED = True  # Default to enable automatic update checks

# --- UI Constants ---
# Grid view settings
FIXED_ICON_SIZE = 96  # Fixed icon size for grid view
FIXED_GRID_WIDTH = 128  # Fixed grid cell width
FIXED_GRID_HEIGHT = 148  # Fixed grid cell height
GRID_SPACING = 4  # Spacing between grid items

# Main window layout
LEFT_PANEL_STRETCH = 1  # Left panel stretch factor
CENTER_PANEL_STRETCH = 3  # Center panel stretch factor
RIGHT_PANEL_STRETCH = 1  # Right panel stretch factor

# --- Processing Constants ---
# Blur detection
DEFAULT_BLUR_DETECTION_THRESHOLD = 100.0  # Default threshold for blur detection

# Iteration limits for safety
DEFAULT_MAX_ITERATIONS = 5000  # Default maximum iterations for various loops
DEFAULT_SAFETY_ITERATION_MULTIPLIER = 2  # Multiplier for safety iteration limits

# Batch processing
METADATA_PROCESSING_CHUNK_SIZE = 25  # Chunk size for metadata processing
METADATA_EMIT_BATCH_SIZE = 50  # Batch size for metadata emission

# UI Population Settings (for large folders)
LARGE_FOLDER_THRESHOLD = 500  # Items above this use chunked UI population
UI_POPULATION_CHUNK_SIZE = 25  # Items to process before calling processEvents() - smaller for more frequent updates

# Thumbnail Lazy Loading Settings
THUMBNAIL_PRELOAD_ENABLED = True  # Enable background thumbnail preloading
THUMBNAIL_PRELOAD_BATCH_SIZE = 20  # Number of thumbnails to generate per batch
THUMBNAIL_PRELOAD_VISIBLE_MARGIN = (
    10  # Number of items above/below visible area to preload
)
THUMBNAIL_MAX_WORKERS = 4  # Max concurrent thumbnail generation threads

# Preview size estimation
PREVIEW_ESTIMATED_SIZE_FACTOR = 1.35  # Factor for estimating preview sizes

# --- AI/ML Constants ---
# DBSCAN clustering parameters
DBSCAN_EPS = (
    0.05  # For cosine metric: 1 - cosine_similarity. Smaller eps = higher similarity
)
DBSCAN_MIN_SAMPLES = 2  # Minimum number of images to form a dense region (cluster)
DEFAULT_SIMILARITY_BATCH_SIZE = 16  # Default batch size for similarity processing

# RAW image processing
RAW_AUTO_EDIT_BRIGHTNESS_STANDARD = (
    1.15  # Standard brightness adjustment for RAW auto-edits
)
RAW_AUTO_EDIT_BRIGHTNESS_ENHANCED = (
    1.25  # Enhanced brightness adjustment for RAW auto-edits
)

# Model settings
ROTATION_MODEL_IMAGE_SIZE = 384  # Image size for rotation detection model

# --- Cache Constants ---
# Thumbnail cache
DEFAULT_THUMBNAIL_CACHE_SIZE_BYTES = (
    2**30
)  # 1 GiB (1,073,741,824 bytes) default for thumbnail cache
THUMBNAIL_MIN_FILE_SIZE = 1024 * 1024  # 1 MB minimum file size for disk caching

# Preview cache
PREVIEW_CACHE_MIN_FILE_SIZE = 256 * 1024  # 256 KB minimum file size for disk caching

# EXIF cache
EXIF_CACHE_MIN_FILE_SIZE = 4096  # 4 KB minimum file size for disk caching

# Rating cache
DEFAULT_RATING_CACHE_SIZE_LIMIT_MB = 256  # Default 256MB limit for rating cache

# --- File Operation Constants ---

# --- Image Processing Constants ---
# Image size settings
THUMBNAIL_MAX_SIZE = (256, 256)  # Maximum size for thumbnails
PRELOAD_MAX_RESOLUTION = (1920, 1200)  # Fixed high resolution for preloading
BLUR_DETECTION_PREVIEW_SIZE = (640, 480)  # Size for image used in blur detection

# --- Model Settings ---
DEFAULT_CLIP_MODEL = (
    "sentence-transformers/clip-ViT-B-32"  # Common default, adjust if different
)

# --- Update Check Constants ---
UPDATE_CHECK_INTERVAL_HOURS = 24  # Check for updates every 24 hours
UPDATE_CHECK_TIMEOUT_SECONDS = 10  # Timeout for update check requests
GITHUB_REPO_OWNER = "duartebarbosadev"  # GitHub repository owner
GITHUB_REPO_NAME = "PhotoSort"  # GitHub repository name


def _get_settings() -> QSettings:
    """Get a QSettings instance with the application's organization and name."""
    return QSettings(SETTINGS_ORGANIZATION, SETTINGS_APPLICATION)


# --- Preview Cache Size ---
def get_preview_cache_size_gb() -> float:
    """Gets the configured preview cache size in GB from settings."""
    settings = _get_settings()
    return settings.value(
        PREVIEW_CACHE_SIZE_GB_KEY, DEFAULT_PREVIEW_CACHE_SIZE_GB, type=float
    )


def set_preview_cache_size_gb(size_gb: float):
    """Sets the preview cache size in GB in settings."""
    settings = _get_settings()
    settings.setValue(PREVIEW_CACHE_SIZE_GB_KEY, size_gb)


def get_preview_cache_size_bytes() -> int:
    """Gets the configured preview cache size in bytes."""
    return int(get_preview_cache_size_gb() * 1024 * 1024 * 1024)


# --- EXIF Cache Size ---
def get_exif_cache_size_mb() -> int:
    """Gets the configured EXIF cache size in MB from settings."""
    settings = _get_settings()
    return settings.value(EXIF_CACHE_SIZE_MB_KEY, DEFAULT_EXIF_CACHE_SIZE_MB, type=int)


def set_exif_cache_size_mb(size_mb: int):
    """Sets the EXIF cache size in MB in settings."""
    settings = _get_settings()
    settings.setValue(EXIF_CACHE_SIZE_MB_KEY, size_mb)


def get_exif_cache_size_bytes() -> int:
    """Gets the configured EXIF cache size in bytes."""
    return get_exif_cache_size_mb() * 1024 * 1024


# --- PyTorch/CUDA Information ---
# --- Rotation Settings ---
def get_rotation_confirm_lossy() -> bool:
    """Get whether to confirm lossy rotations."""
    settings = _get_settings()
    return settings.value(
        ROTATION_CONFIRM_LOSSY_KEY, DEFAULT_ROTATION_CONFIRM_LOSSY, type=bool
    )


def set_rotation_confirm_lossy(confirm: bool):
    """Set whether to confirm lossy rotations."""
    settings = _get_settings()
    settings.setValue(ROTATION_CONFIRM_LOSSY_KEY, confirm)


# --- Recent Folders ---
def get_recent_folders() -> list[str]:
    """Gets the list of recent folders from settings."""
    settings = _get_settings()
    recent_folders = settings.value(RECENT_FOLDERS_KEY, [], type=list)
    # Filter out folders that no longer exist
    return [folder for folder in recent_folders if os.path.isdir(folder)]


def add_recent_folder(path: str):
    """Adds a folder to the top of the recent folders list."""
    if not path or not os.path.isdir(path):
        return

    settings = _get_settings()
    # Get the list of current, valid recent folders
    recent_folders = get_recent_folders()

    # Use os.path.normpath to handle platform-specific path separators (e.g., / vs \)
    normalized_path = os.path.normpath(path)

    # Remove if already exists (case-insensitive on Windows)
    # and normalize existing paths for comparison
    recent_folders = [
        p
        for p in recent_folders
        if os.path.normpath(p).lower() != normalized_path.lower()
    ]

    # Add the new path to the beginning of the list
    recent_folders.insert(0, normalized_path)

    # Trim the list to the maximum size
    if len(recent_folders) > MAX_RECENT_FOLDERS:
        recent_folders = recent_folders[:MAX_RECENT_FOLDERS]

    settings.setValue(RECENT_FOLDERS_KEY, recent_folders)


def is_pytorch_cuda_available() -> bool:
    """Check if PyTorch with CUDA support is available."""
    try:
        import torch

        return torch.cuda.is_available()
    except ImportError:
        return False


# --- Orientation Model ---
def get_orientation_model_name() -> str | None:
    """Gets the configured orientation model name from settings."""
    settings = _get_settings()
    return settings.value(
        ORIENTATION_MODEL_NAME_KEY, DEFAULT_ORIENTATION_MODEL_NAME, type=str
    )


def set_orientation_model_name(model_name: str):
    """Sets the orientation model name in settings."""
    settings = _get_settings()
    settings.setValue(ORIENTATION_MODEL_NAME_KEY, model_name)


# --- Update Check Settings ---
def get_update_check_enabled() -> bool:
    """Gets whether automatic update checks are enabled."""
    settings = _get_settings()
    return settings.value(
        UPDATE_CHECK_ENABLED_KEY, DEFAULT_UPDATE_CHECK_ENABLED, type=bool
    )


def set_update_check_enabled(enabled: bool):
    """Sets whether automatic update checks are enabled."""
    settings = _get_settings()
    settings.setValue(UPDATE_CHECK_ENABLED_KEY, enabled)


def get_last_update_check_time() -> int:
    """Gets the timestamp of the last update check (seconds since epoch)."""
    settings = _get_settings()
    return settings.value(UPDATE_LAST_CHECK_KEY, 0, type=int)


def set_last_update_check_time(timestamp: int):
    """Sets the timestamp of the last update check."""
    settings = _get_settings()
    settings.setValue(UPDATE_LAST_CHECK_KEY, timestamp)
