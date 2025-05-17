import diskcache
import os
import logging # Added for startup logging
import time # Added for startup timing
from typing import Optional

# Default path for the rating cache
DEFAULT_RATING_CACHE_DIR = os.path.join(os.path.expanduser('~'), '.cache', 'phototagger_ratings')
DEFAULT_RATING_CACHE_SIZE_LIMIT_MB = 256 # Default 256MB limit, ratings are small

class RatingCache:
    """
    Manages a disk-based cache for image ratings (integers).
    """
    def __init__(self, cache_dir: str = DEFAULT_RATING_CACHE_DIR, size_limit_mb: int = DEFAULT_RATING_CACHE_SIZE_LIMIT_MB):
        init_start_time = time.perf_counter()
        logging.info(f"RatingCache.__init__ - Start, dir: {cache_dir}, size_limit: {size_limit_mb:.2f} MB")
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
        self._cache = diskcache.Cache(directory=cache_dir, size_limit=size_limit_bytes, disk_min_file_size=0) # Store even small entries on disk
        log_msg = f"[RatingCache] Initialized at {cache_dir} with size limit {size_limit_mb:.2f} MB"
        # print(log_msg) # Replaced by logging
        logging.info(f"RatingCache.__init__ - DiskCache instantiated. {log_msg}")
        logging.info(f"RatingCache.__init__ - End: {time.perf_counter() - init_start_time:.4f}s")

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
                print(f"Warning: Unexpected item type in rating_cache for key {key}. Type: {type(cached_item)}")
                # self.delete(key) # Optionally delete malformed entry
            return None
        except Exception as e:
            print(f"Error accessing rating_cache for key {key}: {e}.")
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
            print(f"Error: Attempted to cache non-integer object for key {key}. Type: {type(value)}")
            return
        try:
            self._cache.set(key, value)
        except Exception as e:
            print(f"Error setting item in rating_cache for key {key}: {e}")

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
            print(f"Error deleting item from rating_cache for key {key}: {e}")
    
    def clear(self) -> None:
        """Clears all items from the cache."""
        try:
            count = len(self._cache)
            self._cache.clear()
            print(f"[RatingCache] Cleared {count} items.")
        except Exception as e:
            print(f"Error clearing rating_cache: {e}")

    def volume(self) -> int:
        """
        Returns the current disk usage of the cache in bytes.
        """
        try:
            return self._cache.volume()
        except Exception as e:
            print(f"Error getting rating_cache volume: {e}")
            return 0

    def close(self) -> None:
        """Closes the cache."""
        try:
            self._cache.close()
            print("[RatingCache] Cache closed.")
        except Exception as e:
            print(f"Error closing rating_cache: {e}")

    def __contains__(self, key: str) -> bool:
        return key in self._cache

    def __del__(self):
        self.close()

if __name__ == '__main__':
    # Example Usage
    test_cache_dir = os.path.join(os.path.expanduser('~'), '.cache', 'test_phototagger_ratings')
    cache = RatingCache(cache_dir=test_cache_dir, size_limit_mb=1) # 1MB limit for test
    print(f"Initial cache volume: {cache.volume() / 1024:.2f} KB")

    test_image_path = "/test/image_rating.jpg"
    test_rating = 4

    try:
        # Test set and get
        cache.set(test_image_path, test_rating)
        retrieved_rating = cache.get(test_image_path)

        if retrieved_rating is not None:
            print(f"Retrieved rating for key {test_image_path}. Rating: {retrieved_rating}")
            assert retrieved_rating == test_rating
        else:
            print(f"Failed to retrieve rating for key {test_image_path}.")

        # Test contains
        if test_image_path in cache:
            print(f"Key {test_image_path} exists in cache.")
        else:
            print(f"Key {test_image_path} does NOT exist in cache (ERROR).")

        print(f"Cache volume after adding item: {cache.volume() / 1024:.2f} KB")

        # Test update
        cache.set(test_image_path, 5)
        retrieved_rating_updated = cache.get(test_image_path)
        print(f"Updated rating: {retrieved_rating_updated}")
        assert retrieved_rating_updated == 5
        
        # Test delete
        cache.delete(test_image_path)
        if test_image_path not in cache:
            print(f"Key {test_image_path} successfully deleted.")
        else:
            print(f"Key {test_image_path} still exists after delete (ERROR).")
        
        print(f"Cache volume after deleting item: {cache.volume() / 1024:.2f} KB")

        # Test non-integer set
        print("Testing set with non-integer value (should print error):")
        cache.set("/test/another.jpg", "not_an_integer") # type: ignore

        # Test getting non-existent key
        print(f"Getting non-existent key: {cache.get('/path/to/nonexistent.jpg')}")

    except Exception as e:
        print(f"Error during RatingCache test: {e}")
    finally:
        # Clean up test cache
        cache.clear()
        cache.close()
        import shutil
        if os.path.exists(test_cache_dir):
             shutil.rmtree(test_cache_dir)
        print("Test RatingCache cleaned up.")