import diskcache
import os
import logging # Added for startup logging
import time # Added for startup timing
from PIL import Image
from typing import Any, Optional, Tuple

# Import the settings function to get the cache size limit
from src.core.app_settings import get_preview_cache_size_bytes, DEFAULT_PREVIEW_CACHE_SIZE_GB

# Default path for the preview PIL image cache
DEFAULT_PREVIEW_CACHE_DIR = os.path.join(os.path.expanduser('~'), '.cache', 'phototagger_preview_pil_images')

class PreviewCache:
    """
    Manages a disk-based cache for preview PIL.Image objects.
    The cache size is configurable via app_settings.
    """
    def __init__(self, cache_dir: str = DEFAULT_PREVIEW_CACHE_DIR):
        init_start_time = time.perf_counter()
        logging.info(f"PreviewCache.__init__ - Start, dir: {cache_dir}")
        """
        Initializes the preview PIL image cache.
        The size limit is read from app_settings.

        Args:
            cache_dir (str): The directory where the cache will be stored.
        """
        os.makedirs(cache_dir, exist_ok=True)
        self._cache_dir = cache_dir
        self._size_limit_bytes = get_preview_cache_size_bytes()
        # Settings for general PIL images, can be adjusted.
        # Using a relatively small disk_min_file_size to ensure even smaller previews are disk-backed if desired.
        self._cache = diskcache.Cache(directory=cache_dir, size_limit=self._size_limit_bytes, disk_min_file_size=256*1024) # 256KB
        log_msg = f"[PreviewCache] Initialized at {cache_dir} with size limit {self._size_limit_bytes / (1024*1024*1024):.2f} GB"
        # print(log_msg) # Replaced by logging
        logging.info(f"PreviewCache.__init__ - DiskCache instantiated. {log_msg}")
        logging.info(f"PreviewCache.__init__ - End: {time.perf_counter() - init_start_time:.4f}s")

    def get(self, key: Tuple[str, Tuple[int, int], bool]) -> Optional[Image.Image]:
        """
        Retrieves an item from the cache.
        Key is typically (normalized_path, resolution_tuple, apply_auto_edits_bool).

        Args:
            key: The cache key.

        Returns:
            Optional[Image.Image]: The cached PIL Image, or None if not found or not an Image.
        """
        try:
            cached_item = self._cache.get(key)
            if isinstance(cached_item, Image.Image):
                return cached_item
            elif cached_item is not None:
                print(f"Warning: Unexpected item type in preview_cache for key {key}. Type: {type(cached_item)}")
                # self.delete(key)
            return None
        except Exception as e:
            print(f"Error accessing preview_cache for key {key}: {e}.")
            return None

    def set(self, key: Tuple[str, Tuple[int, int], bool], value: Image.Image) -> None:
        """
        Adds or updates an item in the cache.
        Key is typically (normalized_path, resolution_tuple, apply_auto_edits_bool).

        Args:
            key: The cache key.
            value (Image.Image): The PIL Image to cache.
        """
        if not isinstance(value, Image.Image):
            print(f"Error: Attempted to cache non-Image object for key {key}. Type: {type(value)}")
            return
        try:
            self._cache.set(key, value)
        except Exception as e:
            print(f"Error setting item in preview_cache for key {key}: {e}")
            
    def delete(self, key: Tuple[str, Tuple[int, int], bool]) -> None:
        """
        Deletes an item from the cache.

        Args:
            key: The cache key to delete.
        """
        try:
            if key in self._cache:
                del self._cache[key]
        except Exception as e:
            print(f"Error deleting item from preview_cache for key {key}: {e}")

    def clear(self) -> None:
        """Clears all items from the cache."""
        try:
            count = len(self._cache)
            self._cache.clear()
            print(f"[PreviewCache] Cleared {count} items.")
        except Exception as e:
            print(f"Error clearing preview_cache: {e}")

    def volume(self) -> int:
        """
        Returns the current disk usage of the cache in bytes.
        """
        try:
            return self._cache.volume()
        except Exception as e:
            print(f"Error getting preview_cache volume: {e}")
            return 0
            
    def get_current_size_limit_gb(self) -> float:
        """Returns the current configured size limit in GB."""
        return self._size_limit_bytes / (1024 * 1024 * 1024)

    def reinitialize_from_settings(self) -> None:
        """
        Closes and reinitializes the cache with the current size limit from app_settings.
        """
        print("[PreviewCache] Reinitializing preview PIL cache...")
        self.close() # Close the existing cache
        
        self._size_limit_bytes = get_preview_cache_size_bytes()
        self._cache = diskcache.Cache(directory=self._cache_dir, size_limit=self._size_limit_bytes, disk_min_file_size=256*1024)
        print(f"[PreviewCache] Reinitialized. New size limit: {self._size_limit_bytes / (1024*1024*1024):.2f} GB.")

    def close(self) -> None:
        """Closes the cache."""
        try:
            self._cache.close()
            print("[PreviewCache] Cache closed.")
        except Exception as e:
            print(f"Error closing preview_cache: {e}")

    def __contains__(self, key: Tuple[str, Tuple[int, int], bool]) -> bool:
        return key in self._cache

    def __del__(self):
        self.close()
