"""
Application Settings Module
Manages persistent application settings using QSettings.
"""

import os
from PyQt6.QtCore import QSettings
from typing import Optional

# --- Settings Constants ---

# Settings organization and application name
SETTINGS_ORGANIZATION = "PhotoRanker"
SETTINGS_APPLICATION = "PhotoRanker"

# Settings keys
PREVIEW_CACHE_SIZE_GB_KEY = "Cache/PreviewCacheSizeGB"
EXIF_CACHE_SIZE_MB_KEY = "Cache/ExifCacheSizeMB" # For EXIF metadata cache
ROTATION_CONFIRM_LOSSY_KEY = "UI/RotationConfirmLossy" # Ask before lossy rotation

# Default values
DEFAULT_PREVIEW_CACHE_SIZE_GB = 2.0 # Default to 2 GB for preview cache
DEFAULT_EXIF_CACHE_SIZE_MB = 256 # Default to 256 MB for EXIF cache
DEFAULT_ROTATION_CONFIRM_LOSSY = True # Default to asking before lossy rotation

# --- Model Settings ---
DEFAULT_CLIP_MODEL = "sentence-transformers/clip-ViT-B-32" # Common default, adjust if different

def _get_settings() -> QSettings:
    """Get a QSettings instance with the application's organization and name."""
    return QSettings(SETTINGS_ORGANIZATION, SETTINGS_APPLICATION)

# --- Preview Cache Size ---
def get_preview_cache_size_gb() -> float:
    """Gets the configured preview cache size in GB from settings."""
    settings = _get_settings()
    return settings.value(PREVIEW_CACHE_SIZE_GB_KEY, DEFAULT_PREVIEW_CACHE_SIZE_GB, type=float)

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
    return settings.value(ROTATION_CONFIRM_LOSSY_KEY, DEFAULT_ROTATION_CONFIRM_LOSSY, type=bool)

def set_rotation_confirm_lossy(confirm: bool):
    """Set whether to confirm lossy rotations."""
    settings = _get_settings()
    settings.setValue(ROTATION_CONFIRM_LOSSY_KEY, confirm)

def is_pytorch_cuda_available() -> bool:
    """Check if PyTorch with CUDA support is available."""
    try:
        import torch
        return torch.cuda.is_available()
    except ImportError:
        return False