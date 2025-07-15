import diskcache
import os
import logging

logger = logging.getLogger(__name__)
import time  # Added for startup timing
from PIL import Image
from typing import Optional, Tuple

# Import the settings function to get the cache size limit
from src.core.app_settings import get_preview_cache_size_bytes

# Default path for the preview PIL image cache
DEFAULT_PREVIEW_CACHE_DIR = os.path.join(
    os.path.expanduser("~"), ".cache", "phototagger_preview_pil_images"
)


class PreviewCache:
    """
    Manages a disk-based cache for preview PIL.Image objects.
    The cache size is configurable via app_settings.
    """

    def __init__(self, cache_dir: str = DEFAULT_PREVIEW_CACHE_DIR):
        init_start_time = time.perf_counter()
        logger.info(f"Initializing Preview cache: {cache_dir}")
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
        self._cache = diskcache.Cache(
            directory=cache_dir,
            size_limit=self._size_limit_bytes,
            disk_min_file_size=256 * 1024,
        )  # 256KB
        log_msg = f"Preview cache initialized at {cache_dir} with size limit {self._size_limit_bytes / (1024 * 1024 * 1024):.2f} GB"
        logger.info(log_msg)
        logger.debug(
            f"Initialization complete in {time.perf_counter() - init_start_time:.4f}s"
        )

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
                logger.warning(
                    f"Invalid item type in Preview cache for key '{key}': {type(cached_item)}"
                )
                # self.delete(key)
            return None
        except Exception as e:
            logger.error(
                f"Error reading from Preview cache for key '{key}': {e}", exc_info=True
            )
            return None

    def set(self, key: Tuple[str, Tuple[int, int], bool], value: Image.Image) -> None:
        """
        Adds or updates an item in the cache and updates the path index.
        Key is typically (normalized_path, resolution_tuple, apply_auto_edits_bool).

        Args:
            key: The cache key.
            value (Image.Image): The PIL Image to cache.
        """
        if not isinstance(value, Image.Image):
            logger.error(
                f"Attempted to cache non-Image object for key '{key}'. Type: {type(value)}"
            )
            return
        try:
            file_path = key[0]
            index_key = f"index_{file_path}"

            with self._cache.transact():
                # Get current index list or create new one
                key_list = self._cache.get(index_key, default=[])
                if key not in key_list:
                    key_list.append(key)
                    self._cache.set(index_key, key_list)
                # Set the actual data
                self._cache.set(key, value)
        except Exception as e:
            logger.error(
                f"Error writing to Preview cache for key '{key}': {e}", exc_info=True
            )

    def delete(self, key: Tuple[str, Tuple[int, int], bool]) -> None:
        """
        Deletes an item from the cache and updates the path index.

        Args:
            key: The cache key to delete.
        """
        try:
            file_path = key[0]
            index_key = f"index_{file_path}"

            with self._cache.transact():
                # Update the index first
                key_list = self._cache.get(index_key)
                if key_list and key in key_list:
                    key_list.remove(key)
                    if key_list:
                        self._cache.set(index_key, key_list)
                    else:
                        # If list is empty, remove the index key
                        self._cache.delete(index_key)

                # Now delete the actual data. Use pop for safety.
                self._cache.pop(key, default=None)

        except Exception as e:
            logger.error(
                f"Error deleting item from Preview cache for key '{key}': {e}",
                exc_info=True,
            )

    def delete_all_for_path(self, file_path: str) -> None:
        """
        Deletes all cache entries for a specific file path using an index.
        Falls back to iterating the cache if the index is not found.

        Args:
            file_path: The file path to clear from cache.
        """
        try:
            import unicodedata
            import os

            normalized_path = unicodedata.normalize("NFC", os.path.normpath(file_path))
            index_key = f"index_{normalized_path}"

            # Try the fast, indexed deletion first
            key_list = self._cache.get(index_key)

            if key_list is not None:
                with self._cache.transact():
                    # The index exists, use it to delete entries
                    for key in key_list:
                        self._cache.pop(key, default=None)
                    self._cache.pop(index_key, default=None)

                if key_list:
                    logger.info(
                        f"Deleted {len(key_list)} indexed preview cache entries for {os.path.basename(file_path)}."
                    )
                return

            # --- Fallback for caches created before indexing was implemented ---
            logger.warning(
                f"No cache index for '{os.path.basename(normalized_path)}'. Using fallback."
            )
            keys_to_delete = []
            for key in self._cache:
                # Skip index keys
                if isinstance(key, str) and key.startswith("index_"):
                    continue

                if isinstance(key, tuple) and len(key) > 0 and isinstance(key[0], str):
                    key_path = unicodedata.normalize("NFC", os.path.normpath(key[0]))
                    if key_path == normalized_path:
                        keys_to_delete.append(key)

            if keys_to_delete:
                for key in keys_to_delete:
                    self._cache.pop(key, default=None)
                logger.info(
                    f"Deleted {len(keys_to_delete)} preview cache entries for {os.path.basename(file_path)} via fallback."
                )

        except Exception as e:
            logger.error(
                f"Error deleting preview cache entries for path '{file_path}': {e}",
                exc_info=True,
            )

    def clear(self) -> None:
        """Clears all items from the cache."""
        try:
            count = len(self._cache)
            self._cache.clear()
            logger.info(f"Cleared {count} items from Preview cache.")
        except Exception as e:
            logger.error(f"Error clearing Preview cache: {e}", exc_info=True)

    def volume(self) -> int:
        """
        Returns the current disk usage of the cache in bytes.
        """
        try:
            return self._cache.volume()
        except Exception as e:
            logger.error(f"Error getting Preview cache volume: {e}", exc_info=True)
            return 0

    def get_current_size_limit_gb(self) -> float:
        """Returns the current configured size limit in GB."""
        return self._size_limit_bytes / (1024 * 1024 * 1024)

    def reinitialize_from_settings(self) -> None:
        """
        Closes and reinitializes the cache with the current size limit from app_settings.
        """
        logger.info("Reinitializing Preview cache with new settings...")
        self.close()  # Close the existing cache

        self._size_limit_bytes = get_preview_cache_size_bytes()
        self._cache = diskcache.Cache(
            directory=self._cache_dir,
            size_limit=self._size_limit_bytes,
            disk_min_file_size=256 * 1024,
        )
        logger.info(
            f"Preview cache reinitialized. New size limit: {self.get_current_size_limit_gb():.2f} GB."
        )

    def close(self) -> None:
        """Closes the cache."""
        try:
            self._cache.close()
            logger.debug("Preview cache closed.")
        except Exception as e:
            logger.error("Error closing Preview cache.", exc_info=True)

    def __contains__(self, key: Tuple[str, Tuple[int, int], bool]) -> bool:
        return key in self._cache

    def __del__(self):
        self.close()
