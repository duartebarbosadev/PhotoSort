import diskcache
import os
import logging
import time
from typing import Optional
from core.app_settings import DEFAULT_RATING_CACHE_SIZE_LIMIT_MB

logger = logging.getLogger(__name__)

# Default path for the rating cache
DEFAULT_RATING_CACHE_DIR = os.path.join(
    os.path.expanduser("~"), ".cache", "photosort_ratings"
)


class RatingCache:
    """
    Manages a disk-based cache for image ratings (integers).
    """

    def __init__(
        self,
        cache_dir: str = DEFAULT_RATING_CACHE_DIR,
        size_limit_mb: int = DEFAULT_RATING_CACHE_SIZE_LIMIT_MB,
    ):
        init_start_time = time.perf_counter()
        logger.info(
            f"Initializing Rating cache: {cache_dir} (Size Limit: {size_limit_mb:.2f} MB)"
        )
        """
        Initializes the rating cache.

        Args:
            cache_dir (str): The directory where the cache will be stored.
            size_limit_mb (int): The maximum size of the cache in megabytes.
        """
        os.makedirs(cache_dir, exist_ok=True)
        self._cache_dir = cache_dir
        size_limit_bytes = size_limit_mb * 1024 * 1024
        # Ratings are very small, so disk_min_file_size can be small or default.
        # For integers, diskcache might store them efficiently in memory before flushing.
        self._cache = diskcache.Cache(
            directory=cache_dir, size_limit=size_limit_bytes, disk_min_file_size=0
        )  # Store even small entries on disk
        log_msg = f"Rating cache initialized at {cache_dir} with size limit {size_limit_mb:.2f} MB"
        logger.info(log_msg)
        logger.debug(
            f"Initialization complete in {time.perf_counter() - init_start_time:.4f}s"
        )

    def get(self, key: str) -> Optional[int]:
        """
        Retrieves an item (rating) from the cache.
        The key is typically the normalized file path.

        Args:
            key (str): The cache key (file path).

        Returns:
            Optional[int]: The cached rating, or None if not found or not an integer.
        """
        try:
            cached_item = self._cache.get(key)
            if isinstance(cached_item, int):
                return cached_item
            elif cached_item is not None:
                logger.warning(
                    f"Invalid item type in Rating cache for key '{key}': {type(cached_item)}"
                )
                # self.delete(key) # Optionally delete malformed entry
            return None
        except Exception as e:
            logger.error(
                f"Error reading from Rating cache for key '{key}': {e}", exc_info=True
            )
            return None

    def set(self, key: str, value: int) -> None:
        """
        Adds or updates an item (rating) in the cache.
        The key is typically the normalized file path.

        Args:
            key (str): The cache key (file path).
            value (int): The rating to cache.
        """
        if not isinstance(value, int):
            logger.error(
                f"Attempted to cache non-integer object for key '{key}'. Type: {type(value)}"
            )
            return
        try:
            self._cache.set(key, value)
        except Exception as e:
            logger.error(
                f"Error writing to Rating cache for key '{key}': {e}", exc_info=True
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
                f"Error deleting item from Rating cache for key '{key}': {e}",
                exc_info=True,
            )

    def clear(self) -> None:
        """Clears all items from the cache."""
        try:
            count = len(self._cache)
            self._cache.clear()
            logger.info(f"Cleared {count} items from Rating cache.")
        except Exception as e:
            logger.error(f"Error clearing Rating cache: {e}", exc_info=True)

    def volume(self) -> int:
        """
        Returns the current disk usage of the cache in bytes.
        """
        try:
            return self._cache.volume()
        except Exception as e:
            logger.error(f"Error getting Rating cache volume: {e}", exc_info=True)
            return 0

    def close(self) -> None:
        """Closes the cache."""
        try:
            self._cache.close()
            logger.debug("Rating cache closed.")
        except Exception:
            logger.error("Error closing Rating cache.", exc_info=True)

    def __contains__(self, key: str) -> bool:
        return key in self._cache

    def __del__(self):
        self.close()
