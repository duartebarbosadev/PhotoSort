import logging
from PyQt6.QtCore import QObject, pyqtSignal, QThread
from typing import List, Dict, Any, Optional, TYPE_CHECKING

# Import worker classes
from core.file_scanner import FileScanner

from ui.ui_components import (
    PreviewPreloaderWorker,
    BlurDetectionWorker,
    RotationDetectionWorker,
    SimilarityWorker,
    CudaDetectionWorker,
)
from workers.update_worker import UpdateCheckWorker
from core.image_pipeline import ImagePipeline
from workers.rating_loader_worker import (
    RatingLoaderWorker,
)
from core.caching.rating_cache import RatingCache
from core.caching.exif_cache import ExifCache
from ui.app_state import AppState

logger = logging.getLogger(__name__)

if (
    TYPE_CHECKING
):  # Allow type hinting for SimilarityEngine without circular import/eager load
    pass


class WorkerManager(QObject):
    """
    Manages background workers (FileScanner, SimilarityEngine, etc.) and their QThreads.
    """

    # File Scanner Signals
    file_scan_found_files = pyqtSignal(
        list
    )  # list of dicts: [{'path': str, 'is_blurred': Optional[bool]}]
    file_scan_thumbnail_preload_finished = pyqtSignal(list)  # list of dicts
    file_scan_finished = pyqtSignal()
    file_scan_error = pyqtSignal(str)

    # Similarity Engine Signals
    similarity_progress = pyqtSignal(int, str)  # percentage, message
    similarity_embeddings_generated = pyqtSignal(dict)  # {image_path: embedding_vector}
    similarity_clustering_complete = pyqtSignal(dict)  # {image_path: cluster_id}
    similarity_error = pyqtSignal(str)

    # Preview Preloader Signals
    preview_preload_progress = pyqtSignal(int, str)  # percentage, message
    preview_preload_finished = pyqtSignal()
    preview_preload_error = pyqtSignal(str)

    # Blur Detection Signals
    blur_detection_progress = pyqtSignal(int, int, str)  # current, total, basename
    blur_detection_status_updated = pyqtSignal(str, bool)  # image_path, is_blurred
    blur_detection_finished = pyqtSignal()
    blur_detection_error = pyqtSignal(str)

    # Rating Loader Signals
    rating_load_progress = pyqtSignal(int, int, str)  # current, total, basename
    rating_load_metadata_batch_loaded = pyqtSignal(
        list
    )  # List of tuples: [(image_path, metadata_dict), ...]
    rating_load_finished = pyqtSignal()
    rating_load_error = pyqtSignal(str)

    # Rotation Detection Signals
    rotation_detection_progress = pyqtSignal(int, int, str)  # current, total, basename
    rotation_detected = pyqtSignal(str, int)  # image_path, suggested_rotation
    rotation_detection_finished = pyqtSignal()
    rotation_detection_error = pyqtSignal(str)
    rotation_model_not_found = pyqtSignal(str)  # model_path

    # CUDA Detection Signals
    cuda_detection_finished = pyqtSignal(bool)

    # Update Check Signals
    update_check_finished = pyqtSignal(
        bool, object, str
    )  # (update_available, update_info, error_message)

    def __init__(
        self, image_pipeline_instance: ImagePipeline, parent: Optional[QObject] = None
    ):
        super().__init__(parent)
        self.image_pipeline = image_pipeline_instance

        self.scanner_thread: Optional[QThread] = None
        self.file_scanner: Optional[FileScanner] = None

        self.similarity_thread: Optional[QThread] = None
        self.similarity_worker: Optional["SimilarityWorker"] = None

        self.preview_preloader_thread: Optional[QThread] = None
        self.preview_preloader_worker: Optional[PreviewPreloaderWorker] = None

        self.blur_detection_thread: Optional[QThread] = None
        self.blur_detection_worker: Optional[BlurDetectionWorker] = None

        self.rating_loader_thread: Optional[QThread] = None
        self.rating_loader_worker: Optional[RatingLoaderWorker] = None

        self.rotation_detection_thread: Optional[QThread] = None
        self.rotation_detection_worker: Optional[RotationDetectionWorker] = None

        self.cuda_detection_thread: Optional[QThread] = None
        self.cuda_detection_worker: Optional[CudaDetectionWorker] = None

        self.update_check_thread: Optional[QThread] = None
        self.update_check_worker: Optional[UpdateCheckWorker] = None

    def _terminate_thread(
        self, thread: Optional[QThread], worker_stop_method: Optional[callable] = None
    ):
        if (
            thread is not None and thread.isRunning()
        ):  # Explicitly check for None before calling isRunning
            if worker_stop_method:
                try:
                    worker_stop_method()
                except Exception:
                    logger.error(
                        f"Error calling worker stop method for thread {thread}. "
                        f"Worker stop method: {worker_stop_method}.",
                        exc_info=True,
                    )
            thread.quit()
            if not thread.wait(5000):  # Wait 5 seconds
                logger.warning(f"Thread {thread} did not quit gracefully. Terminating.")
                thread.terminate()
                thread.wait()  # Wait for termination
            logger.debug(f"Thread {thread} stopped.")
        # Even if not running, or None, ensure we return None for reassignment
        return None, None

    def _cleanup_scanner_refs(self):
        if self.file_scanner:
            self.file_scanner.deleteLater()
            self.file_scanner = None
        if self.scanner_thread:
            self.scanner_thread.deleteLater()
            self.scanner_thread = None
        logger.debug("File scanner thread and worker cleaned up.")

    # --- File Scanner Management ---
    def start_file_scan(
        self,
        folder_path: str,
        perform_blur_detection: bool,
        blur_threshold: float,
    ):
        self.stop_file_scan()  # Ensure any previous scan is stopped
        self.scanner_thread = QThread()
        self.file_scanner = (
            FileScanner()
        )  # ImagePipeline is now part of FileScanner itself
        self.file_scanner.moveToThread(self.scanner_thread)

        # Connect signals from FileScanner to WorkerManager's signals
        self.file_scanner.files_found.connect(self.file_scan_found_files)
        self.file_scanner.thumbnail_preload_finished.connect(
            self.file_scan_thumbnail_preload_finished
        )
        self.file_scanner.finished.connect(self.file_scan_finished)
        self.file_scanner.error.connect(self.file_scan_error)

        self.scanner_thread.started.connect(
            lambda: self.file_scanner.scan_directory(
                folder_path,
                perform_blur_detection=perform_blur_detection,  # This is passed to scanner
                blur_threshold=blur_threshold,
            )
        )
        self.file_scan_finished.connect(self.scanner_thread.quit)
        self.file_scan_error.connect(self.scanner_thread.quit)

        # Connect to our cleanup method instead of direct deleteLater from here
        self.scanner_thread.finished.connect(self._cleanup_scanner_refs)

        self.scanner_thread.start()
        logger.info("File scanner thread started.")

    def stop_file_scan(self):
        worker_stop = self.file_scanner.stop if self.file_scanner else None
        # _terminate_thread will set self.scanner_thread and self.file_scanner to None if they were cleaned up
        # However, the explicit cleanup is better.
        temp_thread, _ = self._terminate_thread(self.scanner_thread, worker_stop)
        if (
            temp_thread is None
        ):  # if _terminate_thread returned None, it means it handled it or was already None
            self.scanner_thread = None
            self.file_scanner = None  # Worker should also be considered gone
        else:  # This case should ideally not be hit if cleanup is proper
            self.scanner_thread = temp_thread

    def _cleanup_similarity_refs(self):
        if self.similarity_worker:
            self.similarity_worker.deleteLater()
            self.similarity_worker = None
        if self.similarity_thread:
            self.similarity_thread.deleteLater()
            self.similarity_thread = None
        logger.info("Similarity engine thread and worker cleaned up.")

    # --- Similarity Engine Management ---
    def start_similarity_analysis(self, file_paths: List[str]):
        self.stop_similarity_analysis()
        self.similarity_thread = QThread()
        self.similarity_worker = SimilarityWorker(file_paths)
        self.similarity_worker.moveToThread(self.similarity_thread)

        # Connect signals from the new worker to the manager's signals
        self.similarity_worker.progress_update.connect(self.similarity_progress)
        self.similarity_worker.embeddings_generated.connect(
            self.similarity_embeddings_generated
        )
        self.similarity_worker.clustering_complete.connect(
            self.similarity_clustering_complete
        )
        self.similarity_worker.error.connect(self.similarity_error)
        self.similarity_worker.finished.connect(self.similarity_thread.quit)

        self.similarity_thread.started.connect(self.similarity_worker.run)
        self.similarity_thread.finished.connect(self._cleanup_similarity_refs)

        self.similarity_thread.start()
        logger.info("Similarity engine thread started.")

    def stop_similarity_analysis(self):
        worker_stop = self.similarity_worker.stop if self.similarity_worker else None
        temp_thread, _ = self._terminate_thread(self.similarity_thread, worker_stop)
        if temp_thread is None:
            self.similarity_thread = None
            self.similarity_worker = None
        else:
            self.similarity_thread = temp_thread

    def _cleanup_preview_preloader_refs(self):
        if self.preview_preloader_worker:
            self.preview_preloader_worker.deleteLater()
            self.preview_preloader_worker = None
        if self.preview_preloader_thread:
            self.preview_preloader_thread.deleteLater()
            self.preview_preloader_thread = None
        logger.info("Preview preloader thread and worker cleaned up.")

    # --- Preview Preloader Management ---
    def start_preview_preload(self, image_paths: List[str]):
        self.stop_preview_preload()
        self.preview_preloader_thread = QThread()
        self.preview_preloader_worker = PreviewPreloaderWorker(
            image_paths, None, self.image_pipeline
        )
        self.preview_preloader_worker.moveToThread(self.preview_preloader_thread)

        self.preview_preloader_worker.progress_update.connect(
            self.preview_preload_progress
        )
        self.preview_preloader_worker.finished.connect(self.preview_preload_finished)
        self.preview_preloader_worker.error.connect(self.preview_preload_error)

        self.preview_preloader_thread.started.connect(
            self.preview_preloader_worker.run_preload
        )
        self.preview_preload_finished.connect(self.preview_preloader_thread.quit)
        self.preview_preload_error.connect(self.preview_preloader_thread.quit)

        self.preview_preloader_thread.finished.connect(
            self._cleanup_preview_preloader_refs
        )

        self.preview_preloader_thread.start()
        logger.info("Preview preloader thread started.")

    def stop_preview_preload(self):
        worker_stop = (
            self.preview_preloader_worker.stop
            if self.preview_preloader_worker
            else None
        )
        temp_thread, _ = self._terminate_thread(
            self.preview_preloader_thread, worker_stop
        )
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
        logger.info("Blur detection thread and worker cleaned up.")

    # --- Blur Detection Management ---
    def start_blur_detection(
        self,
        image_data_list: List[Dict[str, Any]],
        blur_threshold: float,
        apply_auto_edits_for_raw: bool,
    ):
        self.stop_blur_detection()
        self.blur_detection_thread = QThread()
        # Ensure image_paths is a list of strings, not list of dicts
        image_paths = [
            data["path"]
            for data in image_data_list
            if isinstance(data, dict) and "path" in data
        ]
        self.blur_detection_worker = BlurDetectionWorker(
            image_paths, blur_threshold, apply_auto_edits_for_raw
        )
        self.blur_detection_worker.moveToThread(self.blur_detection_thread)

        self.blur_detection_worker.progress_update.connect(self.blur_detection_progress)
        self.blur_detection_worker.blur_status_updated.connect(
            self.blur_detection_status_updated
        )
        self.blur_detection_worker.finished.connect(self.blur_detection_finished)
        self.blur_detection_worker.error.connect(self.blur_detection_error)

        self.blur_detection_thread.started.connect(
            self.blur_detection_worker.run_detection
        )
        self.blur_detection_finished.connect(self.blur_detection_thread.quit)
        self.blur_detection_error.connect(self.blur_detection_thread.quit)

        self.blur_detection_thread.finished.connect(self._cleanup_blur_detection_refs)

        self.blur_detection_thread.start()
        logger.info("Blur detection thread started.")

    def stop_blur_detection(self):
        worker_stop = (
            self.blur_detection_worker.stop if self.blur_detection_worker else None
        )
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
        logger.info("Rating loader thread and worker cleaned up.")

    # --- Rating Loader Management ---
    def start_rating_load(
        self,
        image_data_list: List[Dict[str, Any]],
        rating_disk_cache: RatingCache,
        app_state: AppState,
    ):
        self.stop_rating_load()
        self.rating_loader_thread = QThread()
        self.rating_loader_worker = RatingLoaderWorker(
            image_data_list,
            rating_disk_cache,
            app_state,  # Pass AppState instance
        )
        self.rating_loader_worker.moveToThread(self.rating_loader_thread)

        self.rating_loader_worker.progress_update.connect(self.rating_load_progress)
        self.rating_loader_worker.metadata_batch_loaded.connect(
            self.rating_load_metadata_batch_loaded
        )  # Connect to the new batched signal
        self.rating_loader_worker.finished.connect(self.rating_load_finished)
        self.rating_loader_worker.error.connect(self.rating_load_error)

        self.rating_loader_thread.started.connect(self.rating_loader_worker.run_load)
        self.rating_load_finished.connect(self.rating_loader_thread.quit)
        self.rating_load_error.connect(self.rating_loader_thread.quit)

        self.rating_loader_thread.finished.connect(self._cleanup_rating_loader_refs)

        self.rating_loader_thread.start()
        logger.info("Rating loader thread started.")

    def stop_rating_load(self):
        if self.rating_loader_worker:
            # Prevent further signal emissions during teardown
            try:
                self.rating_loader_worker.disable_emits()
            except Exception:
                logger.debug("disable_emits not available or failed", exc_info=True)
        worker_stop = (
            self.rating_loader_worker.stop if self.rating_loader_worker else None
        )
        temp_thread, _ = self._terminate_thread(self.rating_loader_thread, worker_stop)
        if temp_thread is None:
            self.rating_loader_thread = None
            self.rating_loader_worker = None
        else:
            self.rating_loader_thread = temp_thread

    def _cleanup_rotation_detection_refs(self):
        if self.rotation_detection_worker:
            self.rotation_detection_worker.deleteLater()
            self.rotation_detection_worker = None
        if self.rotation_detection_thread:
            self.rotation_detection_thread.deleteLater()
            self.rotation_detection_thread = None
        logger.info("Rotation detection thread and worker cleaned up.")

    # --- Rotation Detection Management ---
    def start_rotation_detection(self, image_paths: List[str], exif_cache: "ExifCache"):
        self.stop_rotation_detection()
        self.rotation_detection_thread = QThread()
        self.rotation_detection_worker = RotationDetectionWorker(
            image_paths=image_paths,
            image_pipeline=self.image_pipeline,
            exif_cache=exif_cache,
        )
        self.rotation_detection_worker.moveToThread(self.rotation_detection_thread)

        self.rotation_detection_worker.progress_update.connect(
            self.rotation_detection_progress
        )
        self.rotation_detection_worker.rotation_detected.connect(self.rotation_detected)
        self.rotation_detection_worker.model_not_found.connect(
            self.rotation_model_not_found
        )
        self.rotation_detection_worker.finished.connect(
            self.rotation_detection_finished
        )
        self.rotation_detection_worker.error.connect(self.rotation_detection_error)

        self.rotation_detection_thread.started.connect(
            self.rotation_detection_worker.run
        )
        self.rotation_detection_finished.connect(self.rotation_detection_thread.quit)
        self.rotation_detection_error.connect(self.rotation_detection_thread.quit)
        self.rotation_model_not_found.connect(self.rotation_detection_thread.quit)

        self.rotation_detection_thread.finished.connect(
            self._cleanup_rotation_detection_refs
        )

        self.rotation_detection_thread.start()
        logger.info("Rotation detection thread started.")

    def stop_rotation_detection(self):
        worker_stop = (
            self.rotation_detection_worker.stop
            if self.rotation_detection_worker
            else None
        )
        temp_thread, _ = self._terminate_thread(
            self.rotation_detection_thread, worker_stop
        )
        if temp_thread is None:
            self.rotation_detection_thread = None
            self.rotation_detection_worker = None
        else:
            self.rotation_detection_thread = temp_thread

    def _cleanup_cuda_detection_refs(self):
        if self.cuda_detection_worker:
            self.cuda_detection_worker.deleteLater()
            self.cuda_detection_worker = None
        if self.cuda_detection_thread:
            self.cuda_detection_thread.deleteLater()
            self.cuda_detection_thread = None
        logger.info("CUDA detection thread and worker cleaned up.")

    # --- CUDA Detection Management ---
    def start_cuda_detection(self):
        self.stop_cuda_detection()
        self.cuda_detection_thread = QThread()
        self.cuda_detection_worker = CudaDetectionWorker()
        self.cuda_detection_worker.moveToThread(self.cuda_detection_thread)

        self.cuda_detection_worker.finished.connect(self.cuda_detection_finished)
        self.cuda_detection_worker.finished.connect(self.cuda_detection_thread.quit)
        self.cuda_detection_thread.started.connect(self.cuda_detection_worker.run)
        self.cuda_detection_thread.finished.connect(self._cleanup_cuda_detection_refs)

        self.cuda_detection_thread.start()
        logger.info("CUDA detection thread and worker started.")

    def stop_cuda_detection(self):
        temp_thread, _ = self._terminate_thread(self.cuda_detection_thread, None)
        if temp_thread is None:
            self.cuda_detection_thread = None
            self.cuda_detection_worker = None
        else:
            self.cuda_detection_thread = temp_thread

    def stop_all_workers(self):
        logger.info("Stopping all workers...")
        self.stop_file_scan()
        self.stop_similarity_analysis()
        self.stop_preview_preload()
        self.stop_blur_detection()
        self.stop_rating_load()
        self.stop_rotation_detection()
        self.stop_cuda_detection()
        logger.info("All workers stop requested.")

    def is_file_scanner_running(self) -> bool:
        return self.scanner_thread is not None and self.scanner_thread.isRunning()

    def is_similarity_worker_running(self) -> bool:
        return self.similarity_thread is not None and self.similarity_thread.isRunning()

    def is_preview_preloader_running(self) -> bool:
        return (
            self.preview_preloader_thread is not None
            and self.preview_preloader_thread.isRunning()
        )

    def is_blur_detection_running(self) -> bool:
        return (
            self.blur_detection_thread is not None
            and self.blur_detection_thread.isRunning()
        )

    def is_rating_loader_running(self) -> bool:
        return (
            self.rating_loader_thread is not None
            and self.rating_loader_thread.isRunning()
        )

    def is_rotation_detection_running(self) -> bool:
        return (
            self.rotation_detection_thread is not None
            and self.rotation_detection_thread.isRunning()
        )

    def is_cuda_detection_running(self) -> bool:
        return (
            self.cuda_detection_thread is not None
            and self.cuda_detection_thread.isRunning()
        )

    def start_update_check(self, current_version: str):
        """Start checking for updates in a background thread."""
        if self.is_update_check_running():
            logger.warning("Update check is already running")
            return

        logger.info("Starting update check...")

        self.update_check_thread = QThread()
        self.update_check_worker = UpdateCheckWorker(current_version)
        self.update_check_worker.moveToThread(self.update_check_thread)

        # Connect signals
        self.update_check_worker.update_check_finished.connect(
            self.update_check_finished.emit
        )
        self.update_check_worker.update_check_finished.connect(
            self._cleanup_update_check_worker
        )

        # Connect start signal
        self.update_check_thread.started.connect(
            self.update_check_worker.check_for_updates
        )

        # Start the thread
        self.update_check_thread.start()

    def _cleanup_update_check_worker(self):
        """Clean up the update check worker and thread."""
        if self.update_check_thread is not None:
            self.update_check_thread.quit()
            self.update_check_thread.wait()
            self.update_check_thread = None
        self.update_check_worker = None

    def is_update_check_running(self) -> bool:
        return (
            self.update_check_thread is not None
            and self.update_check_thread.isRunning()
        )

    def is_any_worker_running(self) -> bool:
        return (
            self.is_file_scanner_running()
            or self.is_similarity_worker_running()
            or self.is_preview_preloader_running()
            or self.is_blur_detection_running()
            or self.is_rating_loader_running()
            or self.is_rotation_detection_running()
            or self.is_cuda_detection_running()
            or self.is_update_check_running()
        )
