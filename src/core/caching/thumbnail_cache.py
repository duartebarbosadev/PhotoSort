import diskcache
import os
import logging # Added for startup logging
import time # Added for startup timing
from PIL import Image
from typing import Any, Optional, Tuple

# Default path for the thumbnail cache
DEFAULT_THUMBNAIL_CACHE_DIR = os.path.join(os.path.expanduser('~'), '.cache', 'phototagger_thumbnails')

class ThumbnailCache:
    """
    Manages a disk-based cache for image thumbnails (PIL.Image objects).
    """
    def __init__(self, cache_dir: str = DEFAULT_THUMBNAIL_CACHE_DIR, size_limit: int = 2**30): # Default 1GB limit
        init_start_time = time.perf_counter()
        logging.info(f"ThumbnailCache.__init__ - Start, dir: {cache_dir}, size_limit: {size_limit / (1024*1024):.2f} MB")
        """
        Initializes the thumbnail cache.

        Args:
            cache_dir (str): The directory where the cache will be stored.
            size_limit (int): The maximum size of the cache in bytes.
        """
        os.makedirs(cache_dir, exist_ok=True)
        self._cache_dir = cache_dir
        # Settings for general PIL images, can be adjusted
        self._cache = diskcache.Cache(directory=cache_dir, size_limit=size_limit, disk_min_file_size=1024*1024)
        log_msg = f"[ThumbnailCache] Initialized at {cache_dir} with size limit {size_limit / (1024*1024):.2f} MB"
        logging.info(f"ThumbnailCache.__init__ - DiskCache instantiated. {log_msg}")
        logging.info(f"ThumbnailCache.__init__ - End: {time.perf_counter() - init_start_time:.4f}s")

    def get(self, key: Tuple[str, bool]) -> Optional[Image.Image]:
        """
        Retrieves an item from the cache.
        The key is typically (normalized_path, apply_auto_edits_bool).

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
                # This case should ideally not happen if we consistently cache PIL.Image
                logging.warning(f"Unexpected item type in thumbnail_cache for key {key}. Type: {type(cached_item)}")
                # Optionally delete the malformed entry
                # self.delete(key) 
            return None
        except Exception as e:
            logging.error(f"Error accessing thumbnail_cache for key {key}: {e}")
            return None

    def set(self, key: Tuple[str, bool], value: Image.Image) -> None:
        """
        Adds or updates an item in the cache.
        The key is typically (normalized_path, apply_auto_edits_bool).

        Args:
            key: The cache key.
            value (Image.Image): The PIL Image to cache.
        """
        if not isinstance(value, Image.Image):
            logging.error(f"Attempted to cache non-Image object for key {key}. Type: {type(value)}")
            return
        try:
            self._cache.set(key, value)
        except Exception as e:
            logging.error(f"Error setting item in thumbnail_cache for key {key}: {e}")

    def delete(self, key: Tuple[str, bool]) -> None:
        """
        Deletes an item from the cache.

        Args:
            key: The cache key to delete.
        """
        try:
            if key in self._cache:
                del self._cache[key]
        except Exception as e:
            logging.error(f"Error deleting item from thumbnail_cache for key {key}: {e}")

    def delete_all_for_path(self, file_path: str) -> None:
        """
        Deletes all cache entries for a specific file path.
        
        Args:
            file_path: The file path to clear from cache.
        """
        try:
            import unicodedata
            import os
            normalized_path = unicodedata.normalize('NFC', os.path.normpath(file_path))
            
            # Find all keys that match this file path
            keys_to_delete = []
            for key in self._cache:
                if isinstance(key, tuple) and len(key) >= 1:
                    # Normalize both paths for comparison to handle Unicode differences
                    key_path = unicodedata.normalize('NFC', os.path.normpath(key[0]))
                    if key_path == normalized_path:
                        keys_to_delete.append(key)
            
            # Delete the matching keys
            for key in keys_to_delete:
                del self._cache[key]
                
            if keys_to_delete:
                logging.info(f"Deleted {len(keys_to_delete)} thumbnail cache entries for {os.path.basename(file_path)}")
                
        except Exception as e:
            logging.error(f"Error deleting thumbnail cache entries for path {file_path}: {e}")

    def clear(self) -> None:
        """Clears all items from the cache."""
        try:
            count = len(self._cache)
            self._cache.clear()
            logging.info(f"Cleared {count} items.")
        except Exception as e:
            logging.error(f"Error clearing thumbnail_cache: {e}")

    def volume(self) -> int:
        """
        Returns the current disk usage of the cache in bytes.
        """
        try:
            return self._cache.volume()
        except Exception as e:
            logging.error(f"Error getting thumbnail_cache volume: {e}")
            return 0

    def close(self) -> None:
        """Closes the cache."""
        try:
            self._cache.close()
            logging.info("Cache closed.")
        except Exception as e:
            logging.error(f"Error closing thumbnail_cache: {e}")

    def __contains__(self, key: Tuple[str, bool]) -> bool:
        return key in self._cache

    def __del__(self):
        self.close()