import json
from PyQt6.QtCore import QSettings
import os

ORGANIZATION_NAME = "PhotoRanker"
APPLICATION_NAME = "PhotoRanker"

# Default cache size in GB
DEFAULT_PREVIEW_CACHE_SIZE_GB = 1.0
PREVIEW_CACHE_SIZE_KEY = "preview_cache_size_gb"

# --- EXIF Cache Settings ---
DEFAULT_EXIF_CACHE_SIZE_MB = 256  # Default EXIF cache size in Megabytes (MB)
EXIF_CACHE_SIZE_MB_KEY = "exif_cache_size_mb"

# --- Model Settings ---
DEFAULT_CLIP_MODEL = "sentence-transformers/clip-ViT-B-32" # Common default, adjust if different

# --- PyTorch CUDA Availability ---
_pytorch_cuda_available_cache = None

def is_pytorch_cuda_available() -> bool:
    """Checks if PyTorch CUDA is available, with caching."""
    global _pytorch_cuda_available_cache
    if _pytorch_cuda_available_cache is None:
        try:
            import torch # Local import
            _pytorch_cuda_available_cache = torch.cuda.is_available()
        except ImportError:
            _pytorch_cuda_available_cache = False # PyTorch not installed
        except Exception: # Broad exception for other torch/cuda related issues
            _pytorch_cuda_available_cache = False # Assume not available on other errors
    return _pytorch_cuda_available_cache

# --- Preview Cache Size ---
def get_preview_cache_size_gb() -> float:
    """Gets the configured preview cache size in Gigabytes (GB)."""
    settings = QSettings(ORGANIZATION_NAME, APPLICATION_NAME)
    # Ensure a string is passed as default to settings.value() if key not found
    size_gb_str = settings.value(PREVIEW_CACHE_SIZE_KEY, str(DEFAULT_PREVIEW_CACHE_SIZE_GB))
    try:
        # QSettings might return int/float directly if stored as such, handle that
        if isinstance(size_gb_str, (int, float)):
            return float(size_gb_str)
        return float(str(size_gb_str)) # Convert to str first for safety
    except ValueError:
        return DEFAULT_PREVIEW_CACHE_SIZE_GB

def set_preview_cache_size_gb(size_gb: float):
    """Sets the preview cache size in Gigabytes (GB)."""
    settings = QSettings(ORGANIZATION_NAME, APPLICATION_NAME)
    settings.setValue(PREVIEW_CACHE_SIZE_KEY, float(size_gb)) # Store as float
    print(f"[AppSettings] Preview cache size set to: {size_gb} GB")

def get_preview_cache_size_bytes() -> int:
    """Gets the configured preview cache size in bytes."""
    size_gb = get_preview_cache_size_gb()
    return int(size_gb * 1024 * 1024 * 1024)

# --- EXIF Cache Size Functions ---
def get_exif_cache_size_mb() -> int:
    """Gets the configured EXIF cache size in Megabytes (MB)."""
    settings = QSettings(ORGANIZATION_NAME, APPLICATION_NAME)
    size_mb_str = settings.value(EXIF_CACHE_SIZE_MB_KEY, str(DEFAULT_EXIF_CACHE_SIZE_MB))
    try:
        if isinstance(size_mb_str, (int, float)): # QSettings might return int/float
            return int(float(size_mb_str))
        return int(str(size_mb_str)) # Convert to str first for safety then int
    except ValueError:
        return DEFAULT_EXIF_CACHE_SIZE_MB

def set_exif_cache_size_mb(size_mb: int):
    """Sets the EXIF cache size in Megabytes (MB)."""
    settings = QSettings(ORGANIZATION_NAME, APPLICATION_NAME)
    settings.setValue(EXIF_CACHE_SIZE_MB_KEY, int(size_mb)) # Store as int
    print(f"[AppSettings] EXIF cache size set to: {size_mb} MB")

def get_exif_cache_size_bytes() -> int:
    """Gets the configured EXIF cache size in bytes."""
    size_mb = get_exif_cache_size_mb()
    return int(size_mb * 1024 * 1024)