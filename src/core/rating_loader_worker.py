import os
import time
import concurrent.futures
from PyQt6.QtCore import QObject, pyqtSignal
from typing import List, Dict, Any, Optional, Tuple

from src.core.rating_handler import MetadataHandler
from src.core.caching.rating_cache import RatingCache
from src.ui.app_state import AppState

# Define default number of workers for metadata loading
DEFAULT_METADATA_WORKERS = min(os.cpu_count() or 1, 4) # At least 1, max 4. Default to 1 if cpu_count is None.

class RatingLoaderWorker(QObject):
    """
    Worker to load ratings and other essential metadata for images in the background
    using a thread pool for parallel processing.
    """
    progress_update = pyqtSignal(int, int, str)  # current, total, basename
    rating_loaded = pyqtSignal(str, int)         # image_path, rating (could be expanded to full metadata dict)
    finished = pyqtSignal()
    error = pyqtSignal(str)

    def __init__(self,
                 image_data_list: List[Dict[str, Any]],
                 rating_disk_cache: RatingCache,
                 app_state: AppState,
                 num_workers: int = DEFAULT_METADATA_WORKERS,
                 parent: Optional[QObject] = None):
        super().__init__(parent)
        self._image_data_list = image_data_list
        self._rating_disk_cache = rating_disk_cache # RatingCache instance
        self._app_state = app_state # AppState instance (contains ExifCache)
        self._num_workers = num_workers
        self._is_running = True
        self._executor: Optional[concurrent.futures.ThreadPoolExecutor] = None
        self._futures: List[concurrent.futures.Future] = []


    def stop(self):
        self._is_running = False
        if self._executor:
            # Cancel pending futures. This doesn't stop already running tasks.
            for future in self._futures:
                if not future.done():
                    future.cancel()
            # Shutdown will wait for running tasks to complete by default.
            # We could use `cancel_futures=True` in Python 3.9+ for shutdown,
            # or just rely on the _is_running flag inside tasks.
            self._executor.shutdown(wait=False) # Don't wait here, let run_load finish up
        print("[RatingLoaderWorker] Stop requested.")

    def _process_file_metadata(self, file_data: Dict[str, Any]) -> Optional[Tuple[str, int]]:
        """
        Processes a single file to fetch metadata. Executed by a thread pool worker.
        Returns (path, rating) or None on error/skip.
        """
        if not self._is_running:
            return None

        path = file_data['path']
        basename = os.path.basename(path)
        
        single_file_load_start_time = time.perf_counter()
        try:
            metadata = MetadataHandler.get_display_metadata(
                path,
                self._rating_disk_cache,
                self._app_state.exif_disk_cache # Pass the ExifCache from AppState
            )
            rating = metadata.get('rating', 0)

            # Update AppState's in-memory caches (dict operations are thread-safe for this use case)
            self._app_state.rating_cache[path] = rating
            # Optionally update other metadata in AppState if needed immediately
            # self._app_state.label_cache[path] = metadata.get('label')
            # self._app_state.date_cache[path] = metadata.get('date')
            
            single_file_load_duration = time.perf_counter() - single_file_load_start_time
            print(f"[PERF][RatingLoaderWorker] Metadata load for {basename} (in thread) took {single_file_load_duration:.4f}s")
            return path, rating
            
        except Exception as e:
            print(f"[RatingLoaderWorker] Error loading metadata for {basename} (in thread): {e}")
            # Optionally emit an error signal per file, or just log
            return None # Indicate error for this file

    def run_load(self):
        self._is_running = True
        total_files = len(self._image_data_list)
        processed_count = 0
        
        if not self._rating_disk_cache:
            self.error.emit("Rating disk cache is not available.")
            self.finished.emit(); return
        if not self._app_state:
            self.error.emit("Application state is not available.")
            self.finished.emit(); return
        if not self._app_state.exif_disk_cache:
            self.error.emit("EXIF disk cache is not available in AppState.")
            self.finished.emit(); return

        total_load_start_time = time.perf_counter()
        print(f"[PERF][RatingLoaderWorker] Starting parallel metadata load for {total_files} files with {self._num_workers} workers.")

        self._futures = []
        with concurrent.futures.ThreadPoolExecutor(max_workers=self._num_workers) as executor:
            self._executor = executor
            for file_data in self._image_data_list:
                if not self._is_running: break
                future = executor.submit(self._process_file_metadata, file_data)
                self._futures.append(future)

            for future in concurrent.futures.as_completed(self._futures):
                if not self._is_running and not future.done(): # If stop was called and future hasn't run/finished
                    future.cancel() # Attempt to cancel if not already running/done
                    continue

                processed_count += 1
                path_for_progress = "Unknown" # Fallback if file_data not easily accessible from future
                
                try:
                    result = future.result() # Get result or raise exception from worker task
                    if result:
                        path, rating = result
                        path_for_progress = os.path.basename(path)
                        self.rating_loaded.emit(path, rating)
                except concurrent.futures.CancelledError:
                     print(f"[RatingLoaderWorker] A metadata task was cancelled.")
                except Exception as e:
                    # This catches errors from _process_file_metadata if not caught inside
                    print(f"[RatingLoaderWorker] Exception from metadata task: {e}")
                
                if self._is_running : # Only emit progress if still running
                     self.progress_update.emit(processed_count, total_files, path_for_progress)

        self._executor = None # Clear executor reference
        self._futures = []

        total_load_duration = time.perf_counter() - total_load_start_time
        avg_time_per_file = total_load_duration / processed_count if processed_count > 0 else 0
        print(f"[PERF][RatingLoaderWorker] Finished parallel metadata load for {processed_count}/{total_files} files.")
        print(f"[PERF][RatingLoaderWorker] Total time: {total_load_duration:.2f}s. Average time per file: {avg_time_per_file:.4f}s.")

        if not self._is_running:
            print("[RatingLoaderWorker] Rating loading was stopped/cancelled.")
        
        self.finished.emit()


if __name__ == '__main__':
    # This is a placeholder for potential direct testing of the worker.
    # For full testing, it needs a mock Qt event loop or integration.
    class MockExifCache: # Basic mock for AppState
        def get(self, key): return None
        def set(self, key, value): pass

    class MockAppState:
        def __init__(self):
            self.rating_cache = {} # In-memory
            self.exif_disk_cache = MockExifCache() # Add mock exif cache

    class MockRatingCache: # Disk cache for ratings
        def __init__(self):
            self.cache_data = {}
        def get(self, key):
            return self.cache_data.get(key)
        def set(self, key, value):
            self.cache_data[key] = value
        def __contains__(self, key):
            return key in self.cache_data

    print("Basic RatingLoaderWorker test structure (parallel version).")
    
    # Dummy data
    # Create more files for a better parallel test
    num_test_files = 10
    img_list = [{'path': f'/fake/image{i}.jpg'} for i in range(num_test_files)]
    
    # Mock objects
    mock_app_state = MockAppState()
    mock_disk_rating_cache = MockRatingCache()

    # --- Mock MetadataHandler.get_display_metadata ---
    # Store original for restoration if needed, though for simple test, direct mock is fine
    original_get_display_metadata = MetadataHandler.get_display_metadata
    
    processed_files_count_in_mock = 0
    
    def mock_get_display_metadata(image_path, rating_disk_cache, exif_disk_cache):
        global processed_files_count_in_mock
        processed_files_count_in_mock +=1
        # Simulate some work and potential rating
        basename = os.path.basename(image_path)
        # print(f"Mock get_display_metadata called for: {basename}")
        time.sleep(0.1) # Simulate work
        
        # Simulate finding a rating, e.g., based on filename
        mock_rating = 0
        if "image3" in basename: mock_rating = 3
        elif "image7" in basename: mock_rating = 5
        
        # Simulate caching behavior (simplified)
        if rating_disk_cache and image_path not in rating_disk_cache:
            rating_disk_cache.set(image_path, mock_rating)
        if exif_disk_cache and exif_disk_cache.get(image_path) is None: # Simulate EXIF cache miss/set
            exif_disk_cache.set(image_path, {"XMP:Rating": mock_rating, "EXIF:Make": "MockCamera"})

        return {'rating': mock_rating, 'label': None, 'date': None}

    MetadataHandler.get_display_metadata = mock_get_display_metadata
    # --- End Mock ---

    # Create worker
    worker = RatingLoaderWorker(img_list, mock_disk_rating_cache, mock_app_state, num_workers=2)

    # Connect signals (example)
    def on_progress(curr, total, name):
        print(f"Progress: {curr}/{total} - {name}")

    def on_rating_loaded(path, rating):
        print(f"Rating loaded: {path} -> {rating} (Thread: {QThread.currentThreadId()})") # type: ignore
        assert mock_app_state.rating_cache.get(path) == rating
        # Check if disk cache was populated by the mock
        if path == '/fake/image3.jpg': assert mock_disk_rating_cache.get(path) == 3
        elif path == '/fake/image7.jpg': assert mock_disk_rating_cache.get(path) == 5
        elif mock_disk_rating_cache.get(path) is not None : assert mock_disk_rating_cache.get(path) == 0


    finished_event = threading.Event() # Using threading.Event for simple sync in test script
    import threading

    def on_finished():
        print("Rating loading finished.")
        print(f"Processed files by mock: {processed_files_count_in_mock}")
        print("Final AppState in-memory ratings:", mock_app_state.rating_cache)
        # print("Final Disk Cache ratings:", mock_disk_rating_cache.cache_data)
        MetadataHandler.get_display_metadata = original_get_display_metadata # Restore
        finished_event.set()


    worker.progress_update.connect(on_progress)
    worker.rating_loaded.connect(on_rating_loaded)
    worker.finished.connect(on_finished)

    print("Running worker...")
    # To run this test effectively, it should be in a Qt event loop.
    # For a simple script, we'll call run_load directly. It will block.
    # In a real app, QThread manages this.
    
    # If running outside Qt app, QThread might not be ideal for direct call.
    # For this script, let's assume we are simulating the call that QThread.started would make.
    
    # Create a QCoreApplication for the event loop if running standalone test with signals
    from PyQt6.QtCore import QCoreApplication
    app = QCoreApplication.instance()
    if app is None:
        app = QCoreApplication([])

    # For testing directly without a full Qt app, running in a separate Python thread
    # and using QCoreApplication.processEvents() is complex.
    # The worker itself using ThreadPoolExecutor is Python threads.
    # Emitting Qt signals from these threads is fine if a Q[Core]Application event loop is running.
    
    # Simple direct call for this test script:
    worker_thread = threading.Thread(target=worker.run_load)
    worker_thread.start()
    
    # Keep script alive while worker runs, allow signals to process (basic simulation)
    # In a real app, app.exec() handles this.
    while not finished_event.is_set():
        app.processEvents() # Process Qt signals
        time.sleep(0.05)
    
    worker_thread.join() # Ensure thread finishes
    print("Test script finished.")