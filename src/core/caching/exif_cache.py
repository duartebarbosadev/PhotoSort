import diskcache
import os
import logging # Added for startup logging
import time # Added for startup timing
from typing import Optional, Dict, Any

# Default path for the EXIF metadata cache
DEFAULT_EXIF_CACHE_DIR = os.path.join(os.path.expanduser('~'), '.cache', 'phototagger_exif_data')
DEFAULT_EXIF_CACHE_SIZE_LIMIT_MB = 256  # Default 256MB limit

class ExifCache:
    """
    Manages a disk-based cache for image EXIF metadata (dictionaries).
    """
    def __init__(self, cache_dir: str = DEFAULT_EXIF_CACHE_DIR, size_limit_mb: int = DEFAULT_EXIF_CACHE_SIZE_LIMIT_MB):
        init_start_time = time.perf_counter()
        logging.info(f"ExifCache.__init__ - Start, dir: {cache_dir}, size_limit: {size_limit_mb:.2f} MB")
        """
        Initializes the EXIF metadata cache.

        Args:
            cache_dir (str): The directory where the cache will be stored.
            size_limit_mb (int): The maximum size of the cache in megabytes.
        """
        os.makedirs(cache_dir, exist_ok=True)
        self._cache_dir = cache_dir
        size_limit_bytes = size_limit_mb * 1024 * 1024
        
        # disk_min_file_size=0 means all entries go to disk files immediately.
        # For potentially larger dicts, this might be reasonable.
        self._cache = diskcache.Cache(directory=cache_dir, size_limit=size_limit_bytes, disk_min_file_size=4096) # Store larger items on disk
        log_msg = f"[ExifCache] Initialized at {cache_dir} with size limit {size_limit_mb:.2f} MB"
        # print(log_msg) # Replaced by logging
        logging.info(f"ExifCache.__init__ - DiskCache instantiated. {log_msg}")
        logging.info(f"ExifCache.__init__ - End: {time.perf_counter() - init_start_time:.4f}s")

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
                print(f"Warning: Unexpected item type in exif_cache for key {key}. Type: {type(cached_item)}")
                # self.delete(key) # Optionally delete malformed entry
            return None
        except Exception as e:
            print(f"Error accessing exif_cache for key {key}: {e}.")
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
            print(f"Error: Attempted to cache non-dictionary object for key {key}. Type: {type(value)}")
            return
        try:
            self._cache.set(key, value)
        except Exception as e:
            print(f"Error setting item in exif_cache for key {key}: {e}")

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
            print(f"Error deleting item from exif_cache for key {key}: {e}")
    
    def clear(self) -> None:
        """Clears all items from the cache."""
        try:
            count = len(self._cache)
            self._cache.clear()
            print(f"[ExifCache] Cleared {count} items.")
        except Exception as e:
            print(f"Error clearing exif_cache: {e}")

    def volume(self) -> int:
        """
        Returns the current disk usage of the cache in bytes.
        """
        try:
            return self._cache.volume()
        except Exception as e:
            print(f"Error getting exif_cache volume: {e}")
            return 0

    def close(self) -> None:
        """Closes the cache."""
        try:
            self._cache.close()
            print("[ExifCache] Cache closed.")
        except Exception as e:
            print(f"Error closing exif_cache: {e}")

    def __contains__(self, key: str) -> bool:
        return key in self._cache

    def __del__(self):
        self.close()

if __name__ == '__main__':
    # Example Usage
    test_cache_dir_exif = os.path.join(os.path.expanduser('~'), '.cache', 'test_phototagger_exif_data')
    exif_c = ExifCache(cache_dir=test_cache_dir_exif, size_limit_mb=1) # 1MB limit for test
    print(f"Initial EXIF cache volume: {exif_c.volume() / 1024:.2f} KB")

    test_image_path_exif = "/test/image_exif.jpg"
    test_metadata_exif: Dict[str, Any] = {
        "EXIF:Make": "CameraBrand",
        "EXIF:Model": "CameraModel",
        "XMP:Rating": 3,
        "XMP:Label": "Green"
    }

    try:
        # Test set and get
        exif_c.set(test_image_path_exif, test_metadata_exif)
        retrieved_metadata = exif_c.get(test_image_path_exif)

        if retrieved_metadata is not None:
            print(f"Retrieved metadata for key {test_image_path_exif}. Metadata: {retrieved_metadata}")
            assert retrieved_metadata == test_metadata_exif
            assert retrieved_metadata.get("XMP:Rating") == 3
        else:
            print(f"Failed to retrieve metadata for key {test_image_path_exif}.")

        # Test contains
        if test_image_path_exif in exif_c:
            print(f"Key {test_image_path_exif} exists in EXIF cache.")
        else:
            print(f"Key {test_image_path_exif} does NOT exist in EXIF cache (ERROR).")

        print(f"EXIF Cache volume after adding item: {exif_c.volume() / 1024:.2f} KB")

        # Test update
        updated_metadata: Dict[str, Any] = {**test_metadata_exif, "XMP:Rating": 5}
        exif_c.set(test_image_path_exif, updated_metadata)
        retrieved_metadata_updated = exif_c.get(test_image_path_exif)
        print(f"Updated metadata: {retrieved_metadata_updated}")
        assert retrieved_metadata_updated == updated_metadata
        assert retrieved_metadata_updated.get("XMP:Rating") == 5
        
        # Test delete
        exif_c.delete(test_image_path_exif)
        if test_image_path_exif not in exif_c:
            print(f"Key {test_image_path_exif} successfully deleted from EXIF cache.")
        else:
            print(f"Key {test_image_path_exif} still exists after delete in EXIF cache (ERROR).")
        
        print(f"EXIF Cache volume after deleting item: {exif_c.volume() / 1024:.2f} KB")

        # Test non-dict set
        print("Testing EXIF cache set with non-dict value (should print error):")
        exif_c.set("/test/another_exif.jpg", "not_a_dictionary") # type: ignore

        # Test getting non-existent key
        print(f"Getting non-existent key from EXIF cache: {exif_c.get('/path/to/nonexistent_exif.jpg')}")

    except Exception as e_exif:
        print(f"Error during ExifCache test: {e_exif}")
    finally:
        # Clean up test cache
        exif_c.clear()
        exif_c.close()
        import shutil
        if os.path.exists(test_cache_dir_exif):
             shutil.rmtree(test_cache_dir_exif)
        print("Test ExifCache cleaned up.")