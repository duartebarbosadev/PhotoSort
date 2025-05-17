import os
import time # Add time import
from PyQt6.QtCore import QObject, pyqtSignal
from typing import List, Dict, Any, Optional

from src.core.rating_handler import MetadataHandler
from src.core.caching.rating_cache import RatingCache
from src.ui.app_state import AppState # To update AppState's in-memory cache

class RatingLoaderWorker(QObject):
    """
    Worker to load ratings for images in the background and populate caches.
    """
    progress_update = pyqtSignal(int, int, str)  # current, total, basename
    rating_loaded = pyqtSignal(str, int)         # image_path, rating
    finished = pyqtSignal()
    error = pyqtSignal(str)

    def __init__(self,
                 image_data_list: List[Dict[str, Any]],
                 rating_disk_cache: RatingCache,
                 app_state: AppState, # Pass AppState to update its in-memory rating_cache
                 parent: Optional[QObject] = None):
        super().__init__(parent)
        self._image_data_list = image_data_list
        self._rating_disk_cache = rating_disk_cache
        self._app_state = app_state
        self._is_running = True

    def stop(self):
        self._is_running = False

    def run_load(self):
        self._is_running = True
        total_files = len(self._image_data_list)
        processed_count = 0
        total_load_start_time = time.perf_counter()
        print(f"[PERF][RatingLoaderWorker] Starting to load metadata for {total_files} files.")

        if not self._rating_disk_cache:
            self.error.emit("Rating disk cache is not available.")
            self.finished.emit()
            return
            
        if not self._app_state:
            self.error.emit("Application state is not available.")
            self.finished.emit()
            return
        
        if not self._app_state.exif_disk_cache: # Ensure ExifCache is also available
            self.error.emit("EXIF disk cache is not available in AppState.")
            self.finished.emit()
            return

        for i, file_data in enumerate(self._image_data_list):
            if not self._is_running:
                # self.error.emit("Rating loading cancelled.") # Not really an error
                break
            
            path = file_data['path']
            basename = os.path.basename(path)
            self.progress_update.emit(processed_count + 1, total_files, basename)
            
            single_file_load_start_time = time.perf_counter()
            try:
                # get_display_metadata now handles fetching from ExifCache, then RatingCache, then EXIF
                # It will also update the disk caches if it fetches from EXIF
                metadata = MetadataHandler.get_display_metadata(
                    path,
                    self._rating_disk_cache,
                    self._app_state.exif_disk_cache # Pass the ExifCache from AppState
                )
                rating = metadata.get('rating', 0) # Default to 0 if not found

                # Update AppState's in-memory cache
                self._app_state.rating_cache[path] = rating
                # Note: AppState's label_cache and date_cache are typically updated by MainWindow
                # when an image is selected and _fetch_and_update_metadata_for_selection is called.
                # If we want RatingLoaderWorker to populate these too, we'd add:
                # self._app_state.label_cache[path] = metadata.get('label')
                # self._app_state.date_cache[path] = metadata.get('date')

                self.rating_loaded.emit(path, rating)
                
            except Exception as e:
                # Log error for individual file but continue processing others
                print(f"[RatingLoaderWorker] Error loading rating for {basename}: {e}")
            finally:
                single_file_load_duration = time.perf_counter() - single_file_load_start_time
                print(f"[PERF][RatingLoaderWorker] Metadata load for {basename} took {single_file_load_duration:.4f}s")
            
            processed_count += 1
        
        total_load_duration = time.perf_counter() - total_load_start_time
        avg_time_per_file = total_load_duration / processed_count if processed_count > 0 else 0
        print(f"[PERF][RatingLoaderWorker] Finished loading metadata for {processed_count}/{total_files} files.")
        print(f"[PERF][RatingLoaderWorker] Total time: {total_load_duration:.2f}s. Average time per file: {avg_time_per_file:.4f}s.")

        if not self._is_running:
            print("[RatingLoaderWorker] Rating loading was stopped.")
        
        self.finished.emit()

if __name__ == '__main__':
    # This is a placeholder for potential direct testing of the worker.
    # For full testing, it needs a mock Qt event loop or integration.
    class MockAppState:
        def __init__(self):
            self.rating_cache = {} # In-memory

    class MockRatingCache:
        def __init__(self):
            self.cache_data = {}
        def get(self, key):
            return self.cache_data.get(key)
        def set(self, key, value):
            self.cache_data[key] = value
        def __contains__(self, key):
            return key in self.cache_data

    print("Basic RatingLoaderWorker test structure.")
    
    # Dummy data
    img_list = [{'path': '/fake/image1.jpg'}, {'path': '/fake/image2.png'}]
    
    # Mock objects
    mock_app_state = MockAppState()
    mock_disk_cache = MockRatingCache()

    # Create worker
    worker = RatingLoaderWorker(img_list, mock_disk_cache, mock_app_state)

    # Connect signals (example)
    def on_progress(curr, total, name):
        print(f"Progress: {curr}/{total} - {name}")

    def on_rating_loaded(path, rating):
        print(f"Rating loaded: {path} -> {rating}")
        # Check if AppState in-memory cache is updated
        assert mock_app_state.rating_cache.get(path) == rating
        # Check if disk cache was (potentially) updated by MetadataHandler
        # This part is tricky as MetadataHandler updates it directly.
        # Here, we'd expect MetadataHandler.get_display_metadata to have called mock_disk_cache.set
        # if the rating wasn't already in mock_disk_cache.
        # For this simple test, we can pre-populate the mock_disk_cache or assume it was populated.
        # Example: if we assume MetadataHandler found a rating of 3 for image1.jpg and stored it:
        if path == '/fake/image1.jpg':
            assert mock_disk_cache.cache_data.get(path) == 3 # Assuming MetadataHandler put 3 there
        elif path == '/fake/image2.png':
             assert mock_disk_cache.cache_data.get(path) == 0 # Assuming MetadataHandler put 0 there


    def on_finished():
        print("Rating loading finished.")
        print("Final AppState in-memory ratings:", mock_app_state.rating_cache)
        print("Final Disk Cache ratings:", mock_disk_cache.cache_data)

    worker.progress_update.connect(on_progress)
    worker.rating_loaded.connect(on_rating_loaded)
    worker.finished.connect(on_finished)

    # Simulate MetadataHandler behavior for the test (it would normally call exiftool)
    # For this test, we manually populate what MetadataHandler would put into the disk cache.
    # Let's say image1.jpg has a rating of 3 from EXIF, image2.png has 0.
    mock_disk_cache.set('/fake/image1.jpg', 3) # Simulate that this was read from EXIF and cached
    # image2.png might not have a rating in EXIF, so it defaults to 0 and gets cached as 0.
    # MetadataHandler's get_display_metadata would call cache.set(path, 0) if rating was 0.
    # So, for image2, we expect it to become 0 in the cache *during* the worker's run if not already there.


    print("Running worker...")
    # In a real app, this runs in a QThread. Here, we call it directly for simplicity.
    # Note: This direct call will block.
    # worker.run_load()

    print("Test setup complete. In a real scenario, worker.run_load() would be started in a QThread.")
    print("To run this test effectively, you'd need to mock os.path.basename, os.path.isfile, and MetadataHandler.get_display_metadata")
    print("or run it within a Qt event loop.")