from collections.abc import Callable

import diskcache
from diskcache.core import MODE_BINARY, MODE_RAW
import os
import logging
import threading
import time
import unicodedata
from PIL import Image
from core.runtime_paths import resolve_user_cache_dir
from core.caching.image_codec import decode_cached_image, encode_cached_image

# Import the settings function to get the cache size limit
from core.app_settings import (
    get_preview_cache_size_bytes,
    PREVIEW_CACHE_MIN_FILE_SIZE,
)

logger = logging.getLogger(__name__)


class PreviewCacheCapacityError(RuntimeError):
    def __init__(self, required_bytes: int):
        super().__init__("Preview cache capacity is insufficient")
        self.required_bytes = int(required_bytes)


class PreviewCache:
    """
    Manages a disk-based cache for preview PIL.Image objects.
    The cache size is configurable via app_settings.
    """

    def __init__(self, cache_dir: str | None = None):
        if cache_dir is None:
            cache_dir = resolve_user_cache_dir("previews")
        init_start_time = time.perf_counter()
        self._cache = None
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
        self._protected_paths: set[str] = set()
        self._capacity_lock = threading.RLock()
        # Settings for general PIL images, can be adjusted.
        # Using a relatively small disk_min_file_size to ensure even smaller previews are disk-backed if desired.
        self._cache = diskcache.Cache(
            directory=cache_dir,
            size_limit=self._size_limit_bytes,
            disk_min_file_size=PREVIEW_CACHE_MIN_FILE_SIZE,
            eviction_policy="none",
        )  # 256KB
        self._payload_bytes_by_key: dict[tuple, int] = {}
        self._rebuild_payload_accounting()
        log_msg = f"Preview cache initialized at {cache_dir} with size limit {self._size_limit_bytes / (1024 * 1024 * 1024):.2f} GB"
        logger.info(log_msg)
        logger.debug(
            f"Initialization complete in {time.perf_counter() - init_start_time:.4f}s"
        )

    def get(self, key: tuple[str, tuple[int, int], bool]) -> Image.Image | None:
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
            decoded = decode_cached_image(cached_item)
            if decoded is not None:
                return decoded
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

    def set(self, key: tuple[str, tuple[int, int], bool], value: Image.Image) -> int:
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
            return 0
        try:
            file_path = key[0]
            index_key = f"index_{file_path}"
            encoded = encode_cached_image(value, quality=92)
            with self._capacity_lock:
                self._reserve_space(len(encoded), incoming_key=key)

                with self._cache.transact():
                    # Get current index list or create new one
                    key_list = self._cache.get(index_key, default=[])
                    if key not in key_list:
                        key_list.append(key)
                        self._cache.set(index_key, key_list)
                    # Set the actual data
                    self._cache.set(key, encoded)
                    self._payload_bytes_by_key[key] = len(encoded)
            return len(encoded)
        except PreviewCacheCapacityError:
            raise
        except Exception as e:
            logger.error(
                f"Error writing to Preview cache for key '{key}': {e}", exc_info=True
            )
            return 0

    @staticmethod
    def _normalized_path(path: str) -> str:
        return unicodedata.normalize("NFC", os.path.normpath(path))

    def begin_working_set(self, file_paths) -> None:
        """Protect the active folder while keeping it within the approved limit."""
        with self._capacity_lock:
            self._protected_paths = {
                self._normalized_path(path) for path in file_paths if path
            }

    def end_working_set(self) -> None:
        with self._capacity_lock:
            self._protected_paths.clear()

    def trim_to_limit(self) -> None:
        with self._capacity_lock:
            self._reserve_space(0, incoming_key=("",))

    def payload_size(self, key: tuple) -> int:
        with self._capacity_lock:
            return self._payload_bytes_by_key.get(key, 0)

    def protected_payload_bytes(self) -> int:
        with self._capacity_lock:
            return sum(
                size
                for key, size in self._payload_bytes_by_key.items()
                if self._key_is_protected(key)
            )

    def logical_payload_bytes(self) -> int:
        """Return encoded review bytes, excluding database allocation overhead."""
        with self._capacity_lock:
            return sum(self._payload_bytes_by_key.values())

    def _rebuild_payload_accounting(self) -> None:
        """Read DiskCache's persisted sizes without materializing image payloads.

        DiskCache records file-backed byte lengths in `size`; SQLite can report
        inline BLOB lengths without returning those BLOBs. Keep this schema
        dependency here so old cache entries need no payload-reading migration.
        """
        with self._capacity_lock:
            rows = self._cache._sql(
                "SELECT key, raw, CASE WHEN mode = ? THEN size ELSE length(value) END "
                "FROM Cache WHERE mode = ? OR (mode = ? AND typeof(value) = 'blob')",
                (MODE_BINARY, MODE_BINARY, MODE_RAW),
            )
            sizes = {}
            for stored_key, raw, size in rows:
                key = self._cache.disk.get(stored_key, raw)
                if isinstance(key, tuple):
                    sizes[key] = int(size)
            self._payload_bytes_by_key = sizes

    def _key_is_protected(self, key: object) -> bool:
        if isinstance(key, str) and key.startswith("index_"):
            return (
                self._normalized_path(key.removeprefix("index_"))
                in self._protected_paths
            )
        return (
            isinstance(key, tuple)
            and bool(key)
            and isinstance(key[0], str)
            and self._normalized_path(key[0]) in self._protected_paths
        )

    def _delete_cache_key(self, key: object) -> None:
        if isinstance(key, tuple) and key and isinstance(key[0], str):
            self.delete(key)
        elif isinstance(key, str):
            self._cache.delete(key)

    def _reserve_space(self, additional_bytes: int, *, incoming_key: tuple) -> None:
        """Cull unprotected entries before a write; protected entries never evict."""
        additional_bytes = max(0, int(additional_bytes))
        previous_size = self._payload_bytes_by_key.get(incoming_key, 0)
        required = self.logical_payload_bytes() - previous_size + additional_bytes
        if required <= self._size_limit_bytes:
            return

        for existing_key in list(self._cache.iterkeys()):
            if existing_key == incoming_key or self._key_is_protected(existing_key):
                continue
            self._delete_cache_key(existing_key)
            required = self.logical_payload_bytes() - previous_size + additional_bytes
            if required <= self._size_limit_bytes:
                return

        protected_volume = self.protected_payload_bytes()
        raise PreviewCacheCapacityError(protected_volume + additional_bytes)

    def delete(self, key: tuple[str, tuple[int, int], bool]) -> None:
        """
        Deletes an item from the cache and updates the path index.

        Args:
            key: The cache key to delete.
        """
        try:
            file_path = key[0]
            index_key = f"index_{file_path}"

            with self._capacity_lock, self._cache.transact():
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
                self._payload_bytes_by_key.pop(key, None)

        except Exception as e:
            logger.error(
                f"Error deleting item from Preview cache for key '{key}': {e}",
                exc_info=True,
            )

    def delete_all_for_path(self, file_path: str) -> None:
        """
        Deletes all cache entries for a specific file path using its index.

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
                with self._capacity_lock, self._cache.transact():
                    # The index exists, use it to delete entries
                    for key in key_list:
                        self._cache.pop(key, default=None)
                        self._payload_bytes_by_key.pop(key, None)
                    self._cache.pop(index_key, default=None)

                if key_list:
                    logger.info(
                        f"Deleted {len(key_list)} indexed preview cache entries for {os.path.basename(file_path)}."
                    )
                return

        except Exception as e:
            logger.error(
                f"Error deleting preview cache entries for path '{file_path}': {e}",
                exc_info=True,
            )

    def run_when_inactive(self, operation: Callable[[], None]) -> bool:
        """Serialize cache maintenance with working-set registration and writes."""
        with self._capacity_lock:
            if self._protected_paths:
                return False
            operation()
            return True

    def clear(self) -> bool:
        """Clear only when no folder owns a working set, including unwritten sets."""

        def clear_entries() -> None:
            count = len(self._cache)
            self._cache.clear()
            self._payload_bytes_by_key.clear()
            logger.info(f"Cleared {count} items from Preview cache.")

        try:
            return self.run_when_inactive(clear_entries)
        except Exception as e:
            logger.error(f"Error clearing Preview cache: {e}", exc_info=True)
            return False

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

    @property
    def size_limit_bytes(self) -> int:
        with self._capacity_lock:
            return self._size_limit_bytes

    def reinitialize_from_settings(self) -> bool:
        """Apply settings in place; never replace a cache used by worker threads."""
        return self.set_size_limit(get_preview_cache_size_bytes())

    def set_size_limit(self, size_limit_bytes: int) -> bool:
        """Allow live increases, but defer reductions until the folder is released."""
        with self._capacity_lock:
            new_limit = max(1, int(size_limit_bytes))
            if new_limit < self._size_limit_bytes and self._protected_paths:
                return False
            self._cache.reset("size_limit", new_limit)
            self._size_limit_bytes = new_limit
            return True

    def increase_size_limit(self, size_limit_bytes: int) -> None:
        """Raise the live limit without closing a cache used by preparation threads."""
        with self._capacity_lock:
            new_limit = max(self._size_limit_bytes, int(size_limit_bytes))
            self._cache.reset("size_limit", new_limit)
            self._size_limit_bytes = new_limit

    def close(self) -> None:
        """Closes the cache."""
        try:
            if self._cache is not None:
                self._cache.close()
            logger.debug("Preview cache closed.")
        except Exception:
            logger.error("Error closing Preview cache.", exc_info=True)

    def __contains__(self, key: tuple[str, tuple[int, int], bool]) -> bool:
        return key in self._cache

    def __del__(self):
        self.close()
