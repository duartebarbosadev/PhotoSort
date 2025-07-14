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
AUTO_EDIT_PHOTOS_KEY = "UI/AutoEditPhotos"  # Key for auto edit photos setting
MARK_FOR_DELETION_MODE_KEY = "UI/MarkForDeletionMode"  # Key for mark for deletion mode
RECENT_FOLDERS_KEY = "UI/RecentFolders"  # Key for recent folders list
ORIENTATION_MODEL_NAME_KEY = (
    "Models/OrientationModelName"  # Key for the orientation model file name
)

# Default values
DEFAULT_PREVIEW_CACHE_SIZE_GB = 2.0  # Default to 2 GB for preview cache
DEFAULT_EXIF_CACHE_SIZE_MB = 256  # Default to 256 MB for EXIF cache
DEFAULT_ROTATION_CONFIRM_LOSSY = True  # Default to asking before lossy rotation
DEFAULT_AUTO_EDIT_PHOTOS = False  # Default auto edit photos setting
DEFAULT_MARK_FOR_DELETION_MODE = True  # Default mark for deletion mode setting
MAX_RECENT_FOLDERS = 10  # Max number of recent folders to store
DEFAULT_ORIENTATION_MODEL_NAME = None  # Default to None, so we can auto-detect

# --- Model Settings ---
DEFAULT_CLIP_MODEL = (
    "sentence-transformers/clip-ViT-B-32"  # Common default, adjust if different
)


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


# --- Auto Edit Photos Setting ---
def get_auto_edit_photos() -> bool:
    """Get whether auto edit photos is enabled."""
    settings = _get_settings()
    return settings.value(AUTO_EDIT_PHOTOS_KEY, DEFAULT_AUTO_EDIT_PHOTOS, type=bool)


def set_auto_edit_photos(enabled: bool):
    """Set whether auto edit photos is enabled."""
    settings = _get_settings()
    settings.setValue(AUTO_EDIT_PHOTOS_KEY, enabled)


# --- Mark for Deletion Mode Setting ---
def get_mark_for_deletion_mode() -> bool:
    """Get whether mark for deletion mode is enabled."""
    settings = _get_settings()
    return settings.value(
        MARK_FOR_DELETION_MODE_KEY, DEFAULT_MARK_FOR_DELETION_MODE, type=bool
    )


def set_mark_for_deletion_mode(enabled: bool):
    """Set whether mark for deletion mode is enabled."""
    settings = _get_settings()
    settings.setValue(MARK_FOR_DELETION_MODE_KEY, enabled)


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
