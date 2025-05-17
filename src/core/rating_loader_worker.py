import os
import time
import logging
import traceback # Keep for error logging in run_load
from PyQt6.QtCore import QObject, pyqtSignal
from typing import List, Dict, Any, Optional

from src.core.rating_handler import MetadataHandler
from src.core.caching.rating_cache import RatingCache
from src.ui.app_state import AppState

# DEFAULT_METADATA_WORKERS is no longer needed as batching is handled by MetadataHandler

class RatingLoaderWorker(QObject):
    """
    Worker to load ratings and other essential metadata for images in the background.
    Metadata fetching is now done in a single batch call.
    """
    progress_update = pyqtSignal(int, int, str)  # current, total, basename
    # Emit the full metadata dictionary for flexibility, though currently only rating is primary
    metadata_loaded = pyqtSignal(str, dict)      # image_path, metadata_dict {'rating': int, 'label': Optional[str], 'date': Optional[date_obj]}
    finished = pyqtSignal()
    error = pyqtSignal(str)

    def __init__(self,
                 image_data_list: List[Dict[str, Any]], # Expects list of dicts with 'path'
                 rating_disk_cache: RatingCache,
                 app_state: AppState,
                 parent: Optional[QObject] = None):
        super().__init__(parent)
        self._image_data_list = image_data_list
        self._rating_disk_cache = rating_disk_cache
        self._app_state = app_state
        self._is_running = True

    def stop(self):
        self._is_running = False
        logging.info("[RatingLoaderWorker] Stop requested.")

    def run_load(self):
        self._is_running = True
        image_paths_to_process = [
            fd['path'] for fd in self._image_data_list 
            if fd and isinstance(fd, dict) and 'path' in fd and os.path.isfile(fd['path'])
        ]
        total_files = len(image_paths_to_process)
        processed_count = 0

        if not image_paths_to_process:
            logging.info("[RatingLoaderWorker] No valid image paths to process.")
            self.finished.emit()
            return

        if not self._rating_disk_cache:
            self.error.emit("Rating disk cache is not available.")
            self.finished.emit(); return
        if not self._app_state or not self._app_state.exif_disk_cache:
            self.error.emit("Application state or EXIF disk cache is not available.")
            self.finished.emit(); return

        total_load_start_time = time.perf_counter()
        logging.info(f"[RatingLoaderWorker] Starting batch metadata load for {total_files} files.")

        try:
            # Single batch call to the refactored MetadataHandler
            batch_results = MetadataHandler.get_batch_display_metadata(
                image_paths_to_process,
                self._rating_disk_cache,
                self._app_state.exif_disk_cache
            )

            for i, image_path_norm in enumerate(image_paths_to_process):
                if not self._is_running:
                    logging.info(f"[RatingLoaderWorker] Processing stopped during result iteration at index {i}. Path: {image_path_norm}")
                    break
                
                metadata = batch_results.get(image_path_norm)
                basename = os.path.basename(image_path_norm)
                processed_count += 1

                if metadata:
                    # Update AppState's in-memory caches
                    self._app_state.rating_cache[image_path_norm] = metadata.get('rating', 0)
                    self._app_state.label_cache[image_path_norm] = metadata.get('label')
                    if metadata.get('date'):
                        self._app_state.date_cache[image_path_norm] = metadata['date']
                    else:
                        self._app_state.date_cache.pop(image_path_norm, None)
                    
                    logging.debug(f"[RatingLoaderWorker] Emitting metadata_loaded for {image_path_norm}")
                    self.metadata_loaded.emit(image_path_norm, metadata)
                else:
                    logging.warning(f"[RatingLoaderWorker] No metadata returned for {image_path_norm} from batch call.")
                    # Emit with default/empty metadata if necessary, or skip
                    self.metadata_loaded.emit(image_path_norm, {'rating': 0, 'label': None, 'date': None})


                self.progress_update.emit(processed_count, total_files, basename)
                logging.info(f"[RatingLoaderWorker] Processed {processed_count}/{total_files}: {basename}")

        except Exception as e:
            error_msg = f"Error during batch metadata loading: {e}\n{traceback.format_exc()}"
            logging.error(f"[RatingLoaderWorker] {error_msg}")
            self.error.emit(error_msg)
 
        total_load_duration = time.perf_counter() - total_load_start_time
        avg_time_per_file = total_load_duration / total_files if total_files > 0 else 0
        logging.info(f"[RatingLoaderWorker] Finished batch metadata processing for {processed_count}/{total_files} files.")
        logging.info(f"[RatingLoaderWorker] Total time: {total_load_duration:.2f}s. Average time per file: {avg_time_per_file:.4f}s.")
 
        try:
            logging.info(f"[RatingLoaderWorker] Emitting finished signal. self._is_running: {self._is_running}")
            self.finished.emit()
            logging.info("[RatingLoaderWorker] Finished signal emitted successfully.")
        except Exception as e_finish:
            logging.error(f"[RatingLoaderWorker] Exception during/after emitting finished signal: {e_finish}", exc_info=True)
            self.error.emit(f"Exception in finish sequence: {e_finish}")
        finally:
            logging.info("[RatingLoaderWorker] Exiting run_load method.")


if __name__ == '__main__':
    # Basic test structure for the simplified worker
    logging.basicConfig(level=logging.DEBUG, format='%(asctime)s - %(levelname)s - %(message)s')
    
    class MockExifCache:
        def __init__(self): self._cache = {}
        def get(self, key): return self._cache.get(key)
        def set(self, key, value): self._cache[key] = value
        def delete(self, key): self._cache.pop(key, None)

    class MockAppState:
        def __init__(self):
            self.rating_cache = {}
            self.label_cache = {}
            self.date_cache = {}
            self.exif_disk_cache = MockExifCache()

    class MockRatingDiskCache:
        def __init__(self): self._cache = {}
        def get(self, key): return self._cache.get(key)
        def set(self, key, value): self._cache[key] = value

    print("Basic RatingLoaderWorker test (simplified batch version).")
    
    # Dummy data - create some dummy files for exiftool to actually touch
    test_files_dir = "temp_test_files_for_rating_loader"
    os.makedirs(test_files_dir, exist_ok=True)
    
    mock_image_paths = []
    for i in range(3):
        path = os.path.join(test_files_dir, f"test_img_{i}.jpg")
        with open(path, "w") as f: # Create empty files
            f.write("dummy content")
        mock_image_paths.append(path)

    img_list = [{'path': p} for p in mock_image_paths]
    
    mock_app_state = MockAppState()
    mock_disk_rating_cache = MockRatingDiskCache()

    worker = RatingLoaderWorker(img_list, mock_disk_rating_cache, mock_app_state)

    # Connect signals (example)
    from PyQt6.QtCore import QCoreApplication, QTimer # For event loop simulation
    app = QCoreApplication.instance()
    if app is None: app = QCoreApplication([])

    def on_progress(curr, total, name):
        print(f"Progress: {curr}/{total} - {name}")

    def on_metadata_loaded(path, meta_dict):
        print(f"Metadata loaded: {os.path.basename(path)} -> {meta_dict}")
        assert mock_app_state.rating_cache.get(path) == meta_dict.get('rating', 0)
        assert mock_app_state.label_cache.get(path) == meta_dict.get('label')
        if meta_dict.get('date'):
            assert mock_app_state.date_cache.get(path) == meta_dict.get('date')

    finished_event = threading.Event()
    import threading

    def on_finished():
        print("Rating loading finished.")
        print("AppState in-memory ratings:", mock_app_state.rating_cache)
        print("AppState in-memory labels:", mock_app_state.label_cache)
        print("AppState in-memory dates:", mock_app_state.date_cache)
        finished_event.set()

    def on_error(err_msg):
        print(f"WORKER ERROR: {err_msg}")
        finished_event.set() # Also set event on error to stop test

    worker.progress_update.connect(on_progress)
    worker.metadata_loaded.connect(on_metadata_loaded)
    worker.finished.connect(on_finished)
    worker.error.connect(on_error)

    print("Running worker...")
    worker_thread_obj = threading.Thread(target=worker.run_load)
    worker_thread_obj.start()
    
    # Keep script alive and process Qt signals (basic simulation)
    start_q_event_time = time.time()
    while not finished_event.is_set() and (time.time() - start_q_event_time < 20): # Timeout for test
        app.processEvents()
        time.sleep(0.05)
    
    worker_thread_obj.join(timeout=5)
    print("Test script finished.")

    # Cleanup dummy files
    import shutil
    if os.path.exists(test_files_dir):
        shutil.rmtree(test_files_dir)
    print(f"Cleaned up {test_files_dir}")