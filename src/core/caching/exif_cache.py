import diskcache
import os
import stat
import sys
import logging
import time
import threading
import unicodedata
from collections.abc import Iterable
from typing import Any
from core.runtime_paths import resolve_user_cache_dir
from core.windows_file_identity import windows_file_identity

# Import the settings functions to get the cache size limit
from core.app_settings import (
    get_exif_cache_size_bytes,
    get_exif_cache_size_mb,
    EXIF_CACHE_MIN_FILE_SIZE,
)

logger = logging.getLogger(__name__)
ARW_CACHE_LOG_INTERVAL = 250
_ENTRY_VERSION = 2
_IS_WINDOWS = sys.platform == "win32"
FileIdentity = tuple[int, int, int, int, int]


class ExifCache:
    """
    Manages a disk-based cache for image EXIF metadata (dictionaries).
    The cache size is configurable via app_settings.

    Entries use normalized absolute paths and a file-version identity. Legacy
    entries without identity information are misses and refresh on the next read.
    """

    @staticmethod
    def _path_key(path: str) -> str:
        return unicodedata.normalize("NFC", os.path.abspath(path))

    @staticmethod
    def file_identity(path: str) -> FileIdentity | None:
        """Identify a file version without reading image contents.

        Try Unicode path forms consistently with MetadataProcessor, including on
        filesystems which store decomposed names. Fail closed for inaccessible files.
        """
        path = os.path.abspath(path)
        for candidate in dict.fromkeys(
            (
                path,
                unicodedata.normalize("NFC", path),
                unicodedata.normalize("NFD", path),
            )
        ):
            if _IS_WINDOWS:
                identity = windows_file_identity(candidate)
                if identity is not None:
                    return identity
                continue
            try:
                info = os.stat(candidate)
            except FileNotFoundError:
                continue
            except OSError:
                return None
            if not stat.S_ISREG(info.st_mode):
                return None
            return (
                info.st_size,
                info.st_mtime_ns,
                info.st_ctime_ns,
                info.st_dev,
                info.st_ino,
            )
        return None

    def __init__(self, cache_dir: str | None = None):
        if cache_dir is None:
            cache_dir = resolve_user_cache_dir("exif_data")
        init_start_time = time.perf_counter()
        self._cache = None
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
        self._arw_cache_write_count = 0
        self._arw_cache_write_lock = threading.Lock()

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

    def get(self, key: str) -> dict[str, Any] | None:
        """
        Retrieves an item (metadata dictionary) from the cache.
        The path is normalized to an absolute key and its current identity checked.

        Args:
            key (str): The cache key (file path).

        Returns:
            The metadata dictionary, or None for a missing, stale or legacy entry.
        """
        try:
            cached_item = self._cache.get(self._path_key(key))
            # Legacy dictionaries have no source identity and must be extracted
            # again. Keep the envelope private so callers still receive metadata.
            if (
                isinstance(cached_item, tuple)
                and len(cached_item) == 3
                and cached_item[0] == _ENTRY_VERSION
                and isinstance(cached_item[2], dict)
            ):
                identity = self.file_identity(key)
                if identity is not None and cached_item[1] == identity:
                    return cached_item[2]
            return None
        except Exception as e:
            logger.error(
                f"Error reading from EXIF cache for key '{key}': {e}", exc_info=True
            )
            return None

    def set(
        self,
        key: str,
        value: dict[str, Any],
        *,
        source_identity: FileIdentity | None = None,
    ) -> None:
        """
        Adds or updates an item (metadata dictionary) in the cache.
        The key is typically the normalized file path.

        Args:
            key (str): The cache key (file path).
            value (Dict[str, Any]): The metadata dictionary to cache.
            source_identity: Identity captured before extraction. Reject the write
                if the file changed during extraction or while results were queued.
        """
        if not isinstance(value, dict):
            logger.error(
                f"Attempted to cache non-dictionary object for key '{os.path.basename(key)}'. Type: {type(value)}"
            )
            return
        try:
            identity = self.file_identity(key)
            if identity is None or (
                source_identity is not None and identity != source_identity
            ):
                return
            file_ext = os.path.splitext(key)[1].lower()
            if file_ext == ".arw":
                with self._arw_cache_write_lock:
                    self._arw_cache_write_count += 1
                    count = self._arw_cache_write_count
                if count == 1 or count % ARW_CACHE_LOG_INTERVAL == 0:
                    logger.debug(
                        "Caching ARW metadata #%d for %s: %d keys",
                        count,
                        os.path.basename(key),
                        len(value),
                    )
            self._cache.set(self._path_key(key), (_ENTRY_VERSION, identity, value))
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
            key = self._path_key(key)
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

    def dataset_residency(self, keys: Iterable[str]) -> tuple[int, int]:
        """Return how many unique dataset keys are still present in the cache."""
        canonical_keys = {self._path_key(key) for key in keys if key}
        try:
            resident_count = sum(key in self._cache for key in canonical_keys)
            return resident_count, len(canonical_keys)
        except Exception:
            logger.error("Error checking EXIF dataset cache residency.", exc_info=True)
            # Suppress a capacity warning when residency could not be measured.
            return len(canonical_keys), len(canonical_keys)

    def is_near_capacity(self, threshold: float = 0.95) -> bool:
        """Whether disk usage is close enough to the limit for eviction churn."""
        if self._size_limit_bytes <= 0:
            return False
        return self.volume() >= int(self._size_limit_bytes * threshold)

    def get_current_size_limit_bytes(self) -> int:
        """Return the configured cache limit in bytes."""
        return self._size_limit_bytes

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
            if self._cache is not None:
                self._cache.close()
            logger.debug("EXIF cache closed.")
        except Exception:
            logger.error("Error closing EXIF cache.", exc_info=True)

    def __contains__(self, key: str) -> bool:
        return self.get(key) is not None

    def __del__(self):
        self.close()
