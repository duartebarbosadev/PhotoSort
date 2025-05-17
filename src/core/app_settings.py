import json
from PyQt6.QtCore import QSettings
import os

ORGANIZATION_NAME = "PhotoRanker"
APPLICATION_NAME = "PhotoRanker"

# Default cache size in GB
DEFAULT_PREVIEW_CACHE_SIZE_GB = 1.0
PREVIEW_CACHE_SIZE_KEY = "preview_cache_size_gb"

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

if __name__ == '__main__':
    # Test functions
    print(f"Initial preview cache size: {get_preview_cache_size_gb()} GB ({get_preview_cache_size_bytes()} bytes)")
    set_preview_cache_size_gb(2.5)
    print(f"Set preview cache size to: {get_preview_cache_size_gb()} GB")
    # QSettings stores it as double if float is passed, or string if string is passed.
    # Let's test reading it back after setting to ensure type consistency.
    settings = QSettings(ORGANIZATION_NAME, APPLICATION_NAME)
    print(f"Raw value from QSettings: {settings.value(PREVIEW_CACHE_SIZE_KEY)}, type: {type(settings.value(PREVIEW_CACHE_SIZE_KEY))}")

    set_preview_cache_size_gb(DEFAULT_PREVIEW_CACHE_SIZE_GB) # Reset to default
    print(f"Reset preview cache size to: {get_preview_cache_size_gb()} GB")
    print(f"Raw value after reset: {settings.value(PREVIEW_CACHE_SIZE_KEY)}, type: {type(settings.value(PREVIEW_CACHE_SIZE_KEY))}")

    print(f"Default CLIP Model: {DEFAULT_CLIP_MODEL}")
    print(f"PyTorch CUDA Available: {is_pytorch_cuda_available()}")
    # Call it again to test caching
    print(f"PyTorch CUDA Available (cached): {is_pytorch_cuda_available()}")