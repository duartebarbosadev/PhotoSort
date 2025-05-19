from PyQt6.QtCore import QObject, pyqtSignal, QThread
from typing import List, Dict, Any, Optional, TYPE_CHECKING

# Import worker classes
from src.core.file_scanner import FileScanner
# from src.core.similarity_engine import SimilarityEngine # Commented out for lazy loading
from src.ui.ui_components import PreviewPreloaderWorker, BlurDetectionWorker
from src.core.image_pipeline import ImagePipeline # Needed for PreviewPreloaderWorker
from src.core.rating_loader_worker import RatingLoaderWorker # Import RatingLoaderWorker
from src.core.caching.rating_cache import RatingCache # For type hinting
from src.ui.app_state import AppState # For type hinting

if TYPE_CHECKING: # Allow type hinting for SimilarityEngine without circular import/eager load
    from src.core.similarity_engine import SimilarityEngine

class WorkerManager(QObject):
    """
    Manages background workers (FileScanner, SimilarityEngine, etc.) and their QThreads.
    """
    # File Scanner Signals
    file_scan_found_files = pyqtSignal(list)  # list of dicts: [{'path': str, 'is_blurred': Optional[bool]}]
    file_scan_thumbnail_preload_finished = pyqtSignal(list) # list of dicts
    file_scan_finished = pyqtSignal()
    file_scan_error = pyqtSignal(str)

    # Similarity Engine Signals
    similarity_progress = pyqtSignal(int, str) # percentage, message
    similarity_embeddings_generated = pyqtSignal(dict) # {image_path: embedding_vector}
    similarity_clustering_complete = pyqtSignal(dict) # {image_path: cluster_id}
    similarity_error = pyqtSignal(str)

    # Preview Preloader Signals
    preview_preload_progress = pyqtSignal(int, str) # percentage, message
    preview_preload_finished = pyqtSignal()
    preview_preload_error = pyqtSignal(str)

    # Blur Detection Signals
    blur_detection_progress = pyqtSignal(int, int, str) # current, total, basename
    blur_detection_status_updated = pyqtSignal(str, bool) # image_path, is_blurred
    blur_detection_finished = pyqtSignal()
    blur_detection_error = pyqtSignal(str)

    # Rating Loader Signals
    rating_load_progress = pyqtSignal(int, int, str)  # current, total, basename
    rating_load_metadata_batch_loaded = pyqtSignal(list)  # List of tuples: [(image_path, metadata_dict), ...]
    rating_load_finished = pyqtSignal()
    rating_load_error = pyqtSignal(str)

    def __init__(self, image_pipeline_instance: ImagePipeline, parent: Optional[QObject] = None):
        super().__init__(parent)
        self.image_pipeline = image_pipeline_instance

        self.scanner_thread: Optional[QThread] = None
        self.file_scanner: Optional[FileScanner] = None

        self.similarity_thread: Optional[QThread] = None
        self.similarity_engine: Optional["SimilarityEngine"] = None # Use string literal for type hint

        self.preview_preloader_thread: Optional[QThread] = None
        self.preview_preloader_worker: Optional[PreviewPreloaderWorker] = None

        self.blur_detection_thread: Optional[QThread] = None
        self.blur_detection_worker: Optional[BlurDetectionWorker] = None

        self.rating_loader_thread: Optional[QThread] = None
        self.rating_loader_worker: Optional[RatingLoaderWorker] = None

    def _terminate_thread(self, thread: Optional[QThread], worker_stop_method: Optional[callable] = None):
        if thread is not None and thread.isRunning(): # Explicitly check for None before calling isRunning
            if worker_stop_method:
                try:
                    worker_stop_method()
                except Exception as e:
                    print(f"[WorkerManager] Error calling worker stop method: {e}")
            thread.quit()
            if not thread.wait(5000):  # Wait 5 seconds
                print(f"[WorkerManager] Thread {thread} did not quit gracefully, terminating.")
                thread.terminate()
                thread.wait()  # Wait for termination
            print(f"[WorkerManager] Thread {thread} stopped.")
        # Even if not running, or None, ensure we return None for reassignment
        return None, None

    def _cleanup_scanner_refs(self):
        if self.file_scanner:
            self.file_scanner.deleteLater()
            self.file_scanner = None
        if self.scanner_thread:
            self.scanner_thread.deleteLater()
            self.scanner_thread = None
        print("[WorkerManager] Scanner thread and worker references cleaned up.")

    # --- File Scanner Management ---
    def start_file_scan(self, folder_path: str, apply_auto_edits: bool, perform_blur_detection: bool, blur_threshold: float):
        self.stop_file_scan() # Ensure any previous scan is stopped
        self.scanner_thread = QThread()
        self.file_scanner = FileScanner() # ImagePipeline is now part of FileScanner itself
        self.file_scanner.moveToThread(self.scanner_thread)

        # Connect signals from FileScanner to WorkerManager's signals
        self.file_scanner.files_found.connect(self.file_scan_found_files)
        self.file_scanner.thumbnail_preload_finished.connect(self.file_scan_thumbnail_preload_finished)
        self.file_scanner.finished.connect(self.file_scan_finished)
        self.file_scanner.error.connect(self.file_scan_error)

        self.scanner_thread.started.connect(
            lambda: self.file_scanner.scan_directory(
                folder_path,
                apply_auto_edits=apply_auto_edits,
                perform_blur_detection=perform_blur_detection, # This is passed to scanner
                blur_threshold=blur_threshold
            )
        )
        self.file_scan_finished.connect(self.scanner_thread.quit)
        self.file_scan_error.connect(self.scanner_thread.quit)
        
        # Connect to our cleanup method instead of direct deleteLater from here
        self.scanner_thread.finished.connect(self._cleanup_scanner_refs)
        
        self.scanner_thread.start()
        print("[WorkerManager] File scanner thread started.")

    def stop_file_scan(self):
        worker_stop = self.file_scanner.stop if self.file_scanner else None
        # _terminate_thread will set self.scanner_thread and self.file_scanner to None if they were cleaned up
        # However, the explicit cleanup is better.
        temp_thread, _ = self._terminate_thread(self.scanner_thread, worker_stop)
        if temp_thread is None: # if _terminate_thread returned None, it means it handled it or was already None
            self.scanner_thread = None
            self.file_scanner = None # Worker should also be considered gone
        else: # This case should ideally not be hit if cleanup is proper
            self.scanner_thread = temp_thread


    def _cleanup_similarity_refs(self):
        if self.similarity_engine:
            self.similarity_engine.deleteLater()
            self.similarity_engine = None
        if self.similarity_thread:
            self.similarity_thread.deleteLater()
            self.similarity_thread = None
        print("[WorkerManager] Similarity engine thread and worker references cleaned up.")

    # --- Similarity Engine Management ---
    def start_similarity_analysis(self, file_paths: List[str], apply_auto_edits: bool):
        self.stop_similarity_analysis()
        self.similarity_thread = QThread()
        from src.core.similarity_engine import SimilarityEngine # Lazy import
        self.similarity_engine = SimilarityEngine()
        self.similarity_engine.moveToThread(self.similarity_thread)

        self.similarity_engine.progress_update.connect(self.similarity_progress)
        self.similarity_engine.embeddings_generated.connect(self.similarity_embeddings_generated)
        self.similarity_engine.clustering_complete.connect(self.similarity_clustering_complete)
        self.similarity_engine.error.connect(self.similarity_error)

        self.similarity_thread.started.connect(
            lambda: self.similarity_engine.generate_embeddings_for_files(file_paths, apply_auto_edits)
        )
        # Similarity engine's process has multiple stages, quit on clustering_complete or error
        self.similarity_clustering_complete.connect(self.similarity_thread.quit)
        self.similarity_error.connect(self.similarity_thread.quit)

        self.similarity_thread.finished.connect(self._cleanup_similarity_refs)

        self.similarity_thread.start()
        print("[WorkerManager] Similarity engine thread started.")

    def stop_similarity_analysis(self):
        worker_stop = self.similarity_engine.stop if self.similarity_engine else None
        temp_thread, _ = self._terminate_thread(self.similarity_thread, worker_stop)
        if temp_thread is None:
            self.similarity_thread = None
            self.similarity_engine = None
        else:
            self.similarity_thread = temp_thread

    def _cleanup_preview_preloader_refs(self):
        if self.preview_preloader_worker:
            self.preview_preloader_worker.deleteLater()
            self.preview_preloader_worker = None
        if self.preview_preloader_thread:
            self.preview_preloader_thread.deleteLater()
            self.preview_preloader_thread = None
        print("[WorkerManager] Preview preloader thread and worker references cleaned up.")

    # --- Preview Preloader Management ---
    def start_preview_preload(self, image_paths: List[str], apply_auto_edits: bool):
        self.stop_preview_preload()
        self.preview_preloader_thread = QThread()
        self.preview_preloader_worker = PreviewPreloaderWorker(
            image_paths, None, apply_auto_edits, self.image_pipeline
        )
        self.preview_preloader_worker.moveToThread(self.preview_preloader_thread)

        self.preview_preloader_worker.progress_update.connect(self.preview_preload_progress)
        self.preview_preloader_worker.finished.connect(self.preview_preload_finished)
        self.preview_preloader_worker.error.connect(self.preview_preload_error)

        self.preview_preloader_thread.started.connect(self.preview_preloader_worker.run_preload)
        self.preview_preload_finished.connect(self.preview_preloader_thread.quit)
        self.preview_preload_error.connect(self.preview_preloader_thread.quit)
        
        self.preview_preloader_thread.finished.connect(self._cleanup_preview_preloader_refs)

        self.preview_preloader_thread.start()
        print("[WorkerManager] Preview preloader thread started.")

    def stop_preview_preload(self):
        worker_stop = self.preview_preloader_worker.stop if self.preview_preloader_worker else None
        temp_thread, _ = self._terminate_thread(self.preview_preloader_thread, worker_stop)
        if temp_thread is None:
            self.preview_preloader_thread = None
            self.preview_preloader_worker = None
        else:
            self.preview_preloader_thread = temp_thread


    def _cleanup_blur_detection_refs(self):
        if self.blur_detection_worker:
            self.blur_detection_worker.deleteLater()
            self.blur_detection_worker = None
        if self.blur_detection_thread:
            self.blur_detection_thread.deleteLater()
            self.blur_detection_thread = None
        print("[WorkerManager] Blur detection thread and worker references cleaned up.")

    # --- Blur Detection Management ---
    def start_blur_detection(self, image_data_list: List[Dict[str, Any]], blur_threshold: float, apply_auto_edits_for_raw: bool):
        self.stop_blur_detection()
        self.blur_detection_thread = QThread()
        # Ensure image_paths is a list of strings, not list of dicts
        image_paths = [data['path'] for data in image_data_list if isinstance(data, dict) and 'path' in data]
        self.blur_detection_worker = BlurDetectionWorker(
            image_paths, blur_threshold, apply_auto_edits_for_raw
        )
        self.blur_detection_worker.moveToThread(self.blur_detection_thread)

        self.blur_detection_worker.progress_update.connect(self.blur_detection_progress)
        self.blur_detection_worker.blur_status_updated.connect(self.blur_detection_status_updated)
        self.blur_detection_worker.finished.connect(self.blur_detection_finished)
        self.blur_detection_worker.error.connect(self.blur_detection_error)

        self.blur_detection_thread.started.connect(self.blur_detection_worker.run_detection)
        self.blur_detection_finished.connect(self.blur_detection_thread.quit)
        self.blur_detection_error.connect(self.blur_detection_thread.quit)
        
        self.blur_detection_thread.finished.connect(self._cleanup_blur_detection_refs)

        self.blur_detection_thread.start()
        print("[WorkerManager] Blur detection thread started.")

    def stop_blur_detection(self):
        worker_stop = self.blur_detection_worker.stop if self.blur_detection_worker else None
        temp_thread, _ = self._terminate_thread(self.blur_detection_thread, worker_stop)
        if temp_thread is None:
            self.blur_detection_thread = None
            self.blur_detection_worker = None
        else:
            self.blur_detection_thread = temp_thread

    def _cleanup_rating_loader_refs(self):
        if self.rating_loader_worker:
            self.rating_loader_worker.deleteLater()
            self.rating_loader_worker = None
        if self.rating_loader_thread:
            self.rating_loader_thread.deleteLater()
            self.rating_loader_thread = None
        print("[WorkerManager] Rating loader thread and worker references cleaned up.")

    # --- Rating Loader Management ---
    def start_rating_load(self, image_data_list: List[Dict[str, Any]], rating_disk_cache: RatingCache, app_state: AppState):
        self.stop_rating_load()
        self.rating_loader_thread = QThread()
        self.rating_loader_worker = RatingLoaderWorker(
            image_data_list,
            rating_disk_cache,
            app_state # Pass AppState instance
        )
        self.rating_loader_worker.moveToThread(self.rating_loader_thread)

        self.rating_loader_worker.progress_update.connect(self.rating_load_progress)
        self.rating_loader_worker.metadata_batch_loaded.connect(self.rating_load_metadata_batch_loaded) # Connect to the new batched signal
        self.rating_loader_worker.finished.connect(self.rating_load_finished)
        self.rating_loader_worker.error.connect(self.rating_load_error)

        self.rating_loader_thread.started.connect(self.rating_loader_worker.run_load)
        self.rating_load_finished.connect(self.rating_loader_thread.quit)
        self.rating_load_error.connect(self.rating_loader_thread.quit)
        
        self.rating_loader_thread.finished.connect(self._cleanup_rating_loader_refs)

        self.rating_loader_thread.start()
        print("[WorkerManager] Rating loader thread started.")

    def stop_rating_load(self):
        worker_stop = self.rating_loader_worker.stop if self.rating_loader_worker else None
        temp_thread, _ = self._terminate_thread(self.rating_loader_thread, worker_stop)
        if temp_thread is None:
            self.rating_loader_thread = None
            self.rating_loader_worker = None
        else:
            self.rating_loader_thread = temp_thread

    def stop_all_workers(self):
        print("[WorkerManager] Stopping all workers...")
        self.stop_file_scan()
        self.stop_similarity_analysis()
        self.stop_preview_preload()
        self.stop_blur_detection()
        self.stop_rating_load() # Add rating loader stop
        print("[WorkerManager] All workers stop requested.")

    def is_file_scanner_running(self) -> bool:
        return self.scanner_thread is not None and self.scanner_thread.isRunning()

    def is_similarity_worker_running(self) -> bool:
        return self.similarity_thread is not None and self.similarity_thread.isRunning()

    def is_preview_preloader_running(self) -> bool:
        return self.preview_preloader_thread is not None and self.preview_preloader_thread.isRunning()

    def is_blur_detection_running(self) -> bool:
        return self.blur_detection_thread is not None and self.blur_detection_thread.isRunning()

    def is_rating_loader_running(self) -> bool:
        return self.rating_loader_thread is not None and self.rating_loader_thread.isRunning()

    def is_any_worker_running(self) -> bool:
        return (
            self.is_file_scanner_running() or
            self.is_similarity_worker_running() or
            self.is_preview_preloader_running() or
            self.is_blur_detection_running() or
            self.is_rating_loader_running() # Add rating loader status
        )