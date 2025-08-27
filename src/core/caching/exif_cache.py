import diskcache
import os
import logging
import time
from typing import Optional, Dict, Any

# Import the settings functions to get the cache size limit
from src.core.app_settings import (
    get_exif_cache_size_bytes,
    get_exif_cache_size_mb,
    EXIF_CACHE_MIN_FILE_SIZE,
)

logger = logging.getLogger(__name__)

# Default path for the EXIF metadata cache
DEFAULT_EXIF_CACHE_DIR = os.path.join(
    os.path.expanduser("~"), ".cache", "photosort_exif_data"
)
# DEFAULT_EXIF_CACHE_SIZE_LIMIT_MB is now managed by app_settings


class ExifCache:
    """
    Manages a disk-based cache for image EXIF metadata (dictionaries).
    The cache size is configurable via app_settings.
    """

    def __init__(self, cache_dir: str = DEFAULT_EXIF_CACHE_DIR):
        init_start_time = time.perf_counter()
        # size_limit_mb is now fetched from app_settings
        self._size_limit_mb = get_exif_cache_size_mb()
        logger.info(
            f"Initializing EXIF cache: {cache_dir} (Size Limit: {self._size_limit_mb} MB)"
        )
        """
        Initializes the EXIF metadata cache.
        The size limit is read from app_settings.

        Args:
            cache_dir (str): The directory where the cache will be stored.
        """
        os.makedirs(cache_dir, exist_ok=True)
        self._cache_dir = cache_dir
        self._size_limit_bytes = (
            get_exif_cache_size_bytes()
        )  # Use function from app_settings

        # disk_min_file_size=0 means all entries go to disk files immediately.
        # For potentially larger dicts, this might be reasonable.
        self._cache = diskcache.Cache(
            directory=cache_dir,
            size_limit=self._size_limit_bytes,
            disk_min_file_size=EXIF_CACHE_MIN_FILE_SIZE,
        )  # Store larger items on disk
        log_msg = f"EXIF cache initialized at {cache_dir} with size limit {self._size_limit_bytes / (1024 * 1024):.2f} MB"
        logger.info(log_msg)
        logger.debug(
            f"Initialization complete in {time.perf_counter() - init_start_time:.4f}s"
        )

    def get(self, key: str) -> Optional[Dict[str, Any]]:
        """
        Retrieves an item (metadata dictionary) from the cache.
        The key is typically the normalized file path.

        Args:
            key (str): The cache key (file path).

        Returns:
            Optional[Dict[str, Any]]: The cached metadata dictionary, or None if not found or not a dict.
        """
        try:
            cached_item = self._cache.get(key)
            if isinstance(cached_item, dict):
                return cached_item
            elif cached_item is not None:
                logger.warning(
                    f"Invalid item type in EXIF cache for key '{key}': {type(cached_item)}"
                )
                # self.delete(key) # Optionally delete malformed entry
            return None
        except Exception as e:
            logger.error(
                f"Error reading from EXIF cache for key '{key}': {e}", exc_info=True
            )
            return None

    def set(self, key: str, value: Dict[str, Any]) -> None:
        """
        Adds or updates an item (metadata dictionary) in the cache.
        The key is typically the normalized file path.

        Args:
            key (str): The cache key (file path).
            value (Dict[str, Any]): The metadata dictionary to cache.
        """
        if not isinstance(value, dict):
            logger.error(
                f"Attempted to cache non-dictionary object for key '{os.path.basename(key)}'. Type: {type(value)}"
            )
            return
        try:
            file_ext = os.path.splitext(key)[1].lower()
            if file_ext == ".arw":
                logger.debug(
                    f"Caching ARW metadata for {os.path.basename(key)}: {len(value)} keys"
                )
            self._cache.set(key, value)
        except Exception as e:
            logger.error(
                f"Error writing to EXIF cache for key '{os.path.basename(key)}': {e}",
                exc_info=True,
            )

    def delete(self, key: str) -> None:
        """
        Deletes an item from the cache.

        Args:
            key (str): The cache key to delete.
        """
        try:
            if key in self._cache:
                del self._cache[key]
        except Exception as e:
            logger.error(
                f"Error deleting item from EXIF cache for key '{key}': {e}",
                exc_info=True,
            )

    def clear(self) -> None:
        """Clears all items from the cache."""
        try:
            count = len(self._cache)
            self._cache.clear()
            logger.info(f"Cleared {count} items from EXIF cache.")
        except Exception as e:
            logger.error(f"Error clearing EXIF cache: {e}", exc_info=True)

    def volume(self) -> int:
        """
        Returns the current disk usage of the cache in bytes.
        """
        try:
            return self._cache.volume()
        except Exception as e:
            logger.error(f"Error getting EXIF cache volume: {e}", exc_info=True)
            return 0

    def get_current_size_limit_mb(self) -> int:
        """Returns the current configured size limit in MB."""
        return self._size_limit_mb

    def reinitialize_from_settings(self) -> None:
        """
        Closes and reinitializes the cache with the current size limit from app_settings.
        """
        logger.info("Reinitializing EXIF cache with new settings...")
        self.close()  # Close the existing cache

        self._size_limit_mb = get_exif_cache_size_mb()
        self._size_limit_bytes = get_exif_cache_size_bytes()
        self._cache = diskcache.Cache(
            directory=self._cache_dir,
            size_limit=self._size_limit_bytes,
            disk_min_file_size=EXIF_CACHE_MIN_FILE_SIZE,
        )
        logger.info(
            f"EXIF cache reinitialized. New size limit: {self._size_limit_mb} MB."
        )

    def close(self) -> None:
        """Closes the cache."""
        try:
            self._cache.close()
            logger.debug("EXIF cache closed.")
        except Exception:
            logger.error("Error closing EXIF cache.", exc_info=True)

    def __contains__(self, key: str) -> bool:
        return key in self._cache

    def __del__(self):
        self.close()
