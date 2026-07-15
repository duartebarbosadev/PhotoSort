from __future__ import annotations

import logging
from PyQt6.QtCore import QObject, pyqtSignal, QThread
from typing import Any, Callable, Dict, List, Optional, TYPE_CHECKING

# Import worker classes
from core.file_scanner import FileScanner

try:
    from PyQt6 import sip as _sip  # type: ignore[attr-defined]
except ImportError:  # pragma: no cover - environment dependent
    import sip as _sip  # type: ignore[import-not-found]

from core.image_pipeline import ImagePipeline
from core.caching.rating_cache import RatingCache
from core.caching.exif_cache import ExifCache
from ui.app_state import AppState
from core.app_settings import get_best_shot_batch_size

sip = _sip

logger = logging.getLogger(__name__)

if TYPE_CHECKING:
    from ui.ui_components import (
        BlurDetectionWorker,
        CudaDetectionWorker,
        RotationDetectionWorker,
        SimilarityWorker,
    )
    from workers.ai_rating_worker import AiRatingWorker
    from workers.best_shot_worker import BestShotWorker
    from workers.easy_delete_worker import EasyDeleteWorker
    from workers.grouping_worker import GroupingPreviewWorker, GroupingWorkflowWorker
    from workers.pick_best_worker import PickBestWorker
    from workers.rating_loader_worker import RatingLoaderWorker
    from workers.rating_writer_worker import RatingWriterWorker
    from workers.rotation_application_worker import RotationApplicationWorker
    from workers.rotation_detection_step_worker import RotationDetectionStepWorker
    from workers.thumbnail_preload_worker import ThumbnailPreloadWorker
    from workers.update_worker import UpdateCheckWorker


class WorkerManager(QObject):
    """
    Manages background workers (FileScanner, SimilarityEngine, etc.) and their QThreads.
    """

    # File Scanner Signals
    file_scan_found_files = pyqtSignal(
        list
    )  # list of dicts: [{'path': str, 'is_blurred': Optional[bool], 'media_type': str}]
    file_scan_thumbnail_preload_finished = pyqtSignal(list)  # list of dicts
    file_scan_finished = pyqtSignal()
    file_scan_error = pyqtSignal(str)

    # Similarity Engine Signals
    similarity_progress = pyqtSignal(int, str)  # percentage, message
    similarity_embeddings_generated = pyqtSignal(dict)  # {image_path: embedding_vector}
    similarity_clustering_complete = pyqtSignal(dict)  # {image_path: cluster_id}
    similarity_error = pyqtSignal(str)

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
    cuda_detection_finished = pyqtSignal(str)

    # Update Check Signals
    update_check_finished = pyqtSignal(
        bool, object, str
    )  # (update_available, update_info, error_message)

    # Rating Writer Signals
    rating_write_progress = pyqtSignal(int, int, str)  # current, total, filename
    rating_written = pyqtSignal(str, int, bool)  # path, rating, success
    rating_write_finished = pyqtSignal(int, int)  # successful_count, failed_count
    rating_write_error = pyqtSignal(str)

    # Rotation Application Signals
    rotation_application_progress = pyqtSignal(
        int, int, str
    )  # current, total, filename
    rotation_applied = pyqtSignal(
        str, str, bool, str, bool
    )  # path, direction, success, message, is_lossy
    rotation_application_finished = pyqtSignal(
        int, int
    )  # successful_count, failed_count
    rotation_application_error = pyqtSignal(str)

    # Thumbnail Preload Signals (background, not blocking scan)
    thumbnail_preload_progress = pyqtSignal(int, int, str)  # current, total, message
    thumbnail_preload_finished = pyqtSignal(object)  # completed image paths
    thumbnail_preload_error = pyqtSignal(str)

    # Best Shot Analysis Signals
    best_shot_progress = pyqtSignal(int, str)
    best_shot_complete = pyqtSignal(object)
    best_shot_error = pyqtSignal(str)

    # AI Rating Signals
    ai_rating_progress = pyqtSignal(int, str)
    ai_rating_complete = pyqtSignal(object)
    ai_rating_error = pyqtSignal(str)
    ai_rating_warning = pyqtSignal(str)

    # Grouping workflow signals
    grouping_preview_progress = pyqtSignal(int, str)
    grouping_preview_ready = pyqtSignal(object)
    grouping_preview_error = pyqtSignal(str)
    grouping_workflow_progress = pyqtSignal(int, str)
    grouping_workflow_complete = pyqtSignal(object)
    grouping_workflow_error = pyqtSignal(str)

    # Pick Best signals
    pick_best_progress = pyqtSignal(int, str)
    pick_best_complete = pyqtSignal(dict)
    pick_best_error = pyqtSignal(str)

    # Easy Delete signals
    easy_delete_progress = pyqtSignal(int, str)
    easy_delete_complete = pyqtSignal(dict)
    easy_delete_error = pyqtSignal(str)

    # Fix Rotation Detection signals
    fix_rotation_progress = pyqtSignal(int, str)
    fix_rotation_complete = pyqtSignal(dict)  # {path: angle}
    fix_rotation_model_not_found = pyqtSignal(str)
    fix_rotation_error = pyqtSignal(str)

    # Fix Rotation Apply signals (reuse rotation_application_* signals)

    def __init__(
        self, image_pipeline_instance: ImagePipeline, parent: Optional[QObject] = None
    ):
        super().__init__(parent)
        self.image_pipeline = image_pipeline_instance

        self.scanner_thread: Optional[QThread] = None
        self.file_scanner: Optional[FileScanner] = None

        self.similarity_thread: Optional[QThread] = None
        self.similarity_worker: Optional["SimilarityWorker"] = None

        self.blur_detection_thread: Optional[QThread] = None
        self.blur_detection_worker: Optional[BlurDetectionWorker] = None

        self.rating_loader_thread: Optional[QThread] = None
        self.rating_loader_worker: Optional[RatingLoaderWorker] = None

        self.rating_writer_thread: Optional[QThread] = None
        self.rating_writer_worker: Optional[RatingWriterWorker] = None

        self.rotation_detection_thread: Optional[QThread] = None
        self.rotation_detection_worker: Optional[RotationDetectionWorker] = None

        self.rotation_application_thread: Optional[QThread] = None
        self.rotation_application_worker: Optional[RotationApplicationWorker] = None

        self.thumbnail_preload_thread: Optional[QThread] = None
        self.thumbnail_preload_worker: Optional[ThumbnailPreloadWorker] = None
        self.best_shot_thread: Optional[QThread] = None
        self.best_shot_worker: Optional[BestShotWorker] = None
        self.ai_rating_thread: Optional[QThread] = None
        self.ai_rating_worker: Optional[AiRatingWorker] = None
        self.grouping_preview_thread: Optional[QThread] = None
        self.grouping_preview_worker: Optional[GroupingPreviewWorker] = None
        self.grouping_workflow_thread: Optional[QThread] = None
        self.grouping_workflow_worker: Optional[GroupingWorkflowWorker] = None

        self.pick_best_thread: Optional[QThread] = None
        self.pick_best_worker: Optional[PickBestWorker] = None

        self.easy_delete_thread: Optional[QThread] = None
        self.easy_delete_worker: Optional[EasyDeleteWorker] = None

        self.fix_rotation_detect_thread: Optional[QThread] = None
        self.fix_rotation_detect_worker: Optional[RotationDetectionStepWorker] = None

        self.cuda_detection_thread: Optional[QThread] = None
        self.cuda_detection_worker: Optional[CudaDetectionWorker] = None

        self.update_check_thread: Optional[QThread] = None
        self.update_check_worker: Optional[UpdateCheckWorker] = None

    def _terminate_thread(
        self,
        thread: Optional[QThread],
        worker_stop_method: Optional[Callable[[], Any]] = None,
        *,
        allow_terminate: bool = True,
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
                if allow_terminate:
                    logger.warning(
                        f"Thread {thread} did not quit gracefully. Terminating."
                    )
                    thread.terminate()
                    thread.wait()  # Wait for termination
                else:
                    logger.warning(
                        "Thread %s did not quit gracefully and will be left running.",
                        thread,
                    )
                    return thread, None
            logger.debug(f"Thread {thread} stopped.")
        # Even if not running, or None, ensure we return None for reassignment
        return None, None

    def _cleanup_worker_refs(
        self,
        thread_attribute: str,
        worker_attribute: str,
        label: str,
    ) -> None:
        """Release one worker/thread pair after its thread has finished."""

        worker = getattr(self, worker_attribute)
        if worker is not None:
            try:
                if not sip.isdeleted(worker):
                    worker.deleteLater()
            except Exception:
                logger.debug("%s worker was already deleted.", label, exc_info=True)
            setattr(self, worker_attribute, None)

        thread = getattr(self, thread_attribute)
        if thread is not None:
            try:
                if not sip.isdeleted(thread):
                    thread.deleteLater()
            except Exception:
                logger.debug("%s thread was already deleted.", label, exc_info=True)
            setattr(self, thread_attribute, None)
        logger.info("%s thread and worker cleaned up.", label)

    def _stop_worker(
        self,
        thread_attribute: str,
        worker_attribute: str,
        *,
        allow_terminate: bool = True,
        before_stop: Optional[Callable[[Any], None]] = None,
    ) -> None:
        """Request cooperative cancellation for one managed worker slot."""

        worker = getattr(self, worker_attribute)
        if worker is not None and before_stop is not None:
            try:
                before_stop(worker)
            except Exception:
                logger.debug("Worker pre-stop hook failed.", exc_info=True)
        stop_method = getattr(worker, "stop", None) if worker is not None else None
        remaining_thread, _ = self._terminate_thread(
            getattr(self, thread_attribute),
            stop_method,
            allow_terminate=allow_terminate,
        )
        setattr(self, thread_attribute, remaining_thread)
        if remaining_thread is None:
            setattr(self, worker_attribute, None)

    def _finish_worker_slot(
        self,
        thread_attribute: str,
        worker_attribute: str,
        label: str,
    ) -> None:
        """Quit a completed worker's event loop, then release its references."""

        thread = getattr(self, thread_attribute)
        if thread is not None and thread.isRunning():
            thread.quit()
            thread.wait()
        self._cleanup_worker_refs(thread_attribute, worker_attribute, label)

    def _cleanup_scanner_refs(self):
        self._cleanup_worker_refs("scanner_thread", "file_scanner", "File scanner")

    # --- File Scanner Management ---
    def start_file_scan(
        self,
        folder_path: str,
        perform_blur_detection: bool,
        blur_threshold: float,
    ):
        self.stop_file_scan()  # Ensure any previous scan is stopped
        self.scanner_thread = QThread()
        self.file_scanner = FileScanner(
            image_pipeline=self.image_pipeline
        )  # Inject shared pipeline instance
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
        self._stop_worker("scanner_thread", "file_scanner")

    def _cleanup_similarity_refs(self):
        self._cleanup_worker_refs(
            "similarity_thread", "similarity_worker", "Similarity analysis"
        )

    # --- Similarity Engine Management ---
    def start_similarity_analysis(
        self, file_paths: List[str], allow_model_download: bool = False
    ):
        from ui.ui_components import SimilarityWorker

        self.stop_similarity_analysis()
        self.similarity_thread = QThread()
        self.similarity_worker = SimilarityWorker(
            file_paths,
            allow_model_download=allow_model_download,
            image_pipeline=self.image_pipeline,
        )
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
        self._stop_worker("similarity_thread", "similarity_worker")

    def _cleanup_blur_detection_refs(self):
        self._cleanup_worker_refs(
            "blur_detection_thread", "blur_detection_worker", "Blur detection"
        )

    # --- Blur Detection Management ---
    def start_blur_detection(
        self,
        image_data_list: List[Dict[str, Any]],
        blur_threshold: float,
        apply_auto_edits_for_raw: bool,
    ):
        from ui.ui_components import BlurDetectionWorker

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
        self._stop_worker("blur_detection_thread", "blur_detection_worker")

    def _cleanup_rating_loader_refs(self):
        self._cleanup_worker_refs(
            "rating_loader_thread", "rating_loader_worker", "Rating loader"
        )

    # --- Rating Loader Management ---
    def start_rating_load(
        self,
        image_data_list: List[Dict[str, Any]],
        rating_disk_cache: RatingCache,
        app_state: AppState,
    ):
        from workers.rating_loader_worker import RatingLoaderWorker

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
        self._stop_worker(
            "rating_loader_thread",
            "rating_loader_worker",
            before_stop=lambda worker: worker.disable_emits(),
        )

    def _cleanup_rotation_detection_refs(self):
        self._cleanup_worker_refs(
            "rotation_detection_thread",
            "rotation_detection_worker",
            "Rotation detection",
        )

    # --- Rotation Detection Management ---
    def start_rotation_detection(self, image_paths: List[str], exif_cache: "ExifCache"):
        from ui.ui_components import RotationDetectionWorker

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
        self._stop_worker("rotation_detection_thread", "rotation_detection_worker")

    def _cleanup_cuda_detection_refs(self):
        self._cleanup_worker_refs(
            "cuda_detection_thread", "cuda_detection_worker", "CUDA detection"
        )

    # --- CUDA Detection Management ---
    def start_cuda_detection(self):
        from ui.ui_components import CudaDetectionWorker

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
        self._stop_worker("cuda_detection_thread", "cuda_detection_worker")

    def _cleanup_grouping_preview_refs(self):
        self._cleanup_worker_refs(
            "grouping_preview_thread", "grouping_preview_worker", "Grouping preview"
        )

    def start_grouping_preview(
        self,
        items: List[Dict[str, Any]],
        mode: str,
        source_root: Optional[str] = None,
        location_depth: int = 3,
    ):
        from workers.grouping_worker import GroupingPreviewWorker

        self.stop_grouping_preview()
        self.grouping_preview_thread = QThread()
        self.grouping_preview_worker = GroupingPreviewWorker(
            items,
            mode,
            source_root,
            location_depth,
            image_pipeline=self.image_pipeline,
        )
        self.grouping_preview_worker.moveToThread(self.grouping_preview_thread)

        self.grouping_preview_worker.progress_update.connect(
            self.grouping_preview_progress
        )
        self.grouping_preview_worker.preview_ready.connect(self.grouping_preview_ready)
        self.grouping_preview_worker.error.connect(self.grouping_preview_error)
        self.grouping_preview_worker.finished.connect(self.grouping_preview_thread.quit)
        self.grouping_preview_thread.started.connect(self.grouping_preview_worker.run)
        self.grouping_preview_thread.finished.connect(
            self._cleanup_grouping_preview_refs
        )
        self.grouping_preview_thread.start()
        logger.info("Grouping preview thread started.")

    def stop_grouping_preview(self):
        self._stop_worker("grouping_preview_thread", "grouping_preview_worker")

    def _cleanup_grouping_workflow_refs(self):
        self._cleanup_worker_refs(
            "grouping_workflow_thread",
            "grouping_workflow_worker",
            "Grouping workflow",
        )

    def start_grouping_workflow(
        self,
        items: List[Dict[str, Any]],
        mode: str,
        source_root: str,
        output_root: Optional[str] = None,
        group_name_overrides: Optional[Dict[str, str]] = None,
        prepared_plan=None,
        location_depth: int = 3,
        move_companions: bool = False,
    ):
        from workers.grouping_worker import GroupingWorkflowWorker

        self.stop_grouping_workflow()
        self.grouping_workflow_thread = QThread()
        self.grouping_workflow_worker = GroupingWorkflowWorker(
            items=items,
            mode=mode,
            source_root=source_root,
            output_root=output_root,
            group_name_overrides=group_name_overrides,
            prepared_plan=prepared_plan,
            location_depth=location_depth,
            move_companions=move_companions,
            image_pipeline=self.image_pipeline,
        )
        self.grouping_workflow_worker.moveToThread(self.grouping_workflow_thread)

        self.grouping_workflow_worker.progress_update.connect(
            self.grouping_workflow_progress
        )
        self.grouping_workflow_worker.completed.connect(self.grouping_workflow_complete)
        self.grouping_workflow_worker.error.connect(self.grouping_workflow_error)
        self.grouping_workflow_worker.finished.connect(
            self.grouping_workflow_thread.quit
        )
        self.grouping_workflow_thread.started.connect(self.grouping_workflow_worker.run)
        self.grouping_workflow_thread.finished.connect(
            self._cleanup_grouping_workflow_refs
        )
        self.grouping_workflow_thread.start()
        logger.info("Grouping workflow thread started.")

    def stop_grouping_workflow(self):
        self._stop_worker(
            "grouping_workflow_thread",
            "grouping_workflow_worker",
            allow_terminate=False,
        )

    def stop_all_workers(self):
        logger.info("Stopping all workers...")
        self.stop_file_scan()
        self.stop_similarity_analysis()
        self.stop_blur_detection()
        self.stop_rating_load()
        self.stop_rotation_detection()
        self.stop_rating_writer()
        self.stop_rotation_application()
        self.stop_thumbnail_preload()
        self.stop_cuda_detection()
        self.stop_update_check()
        self.stop_best_shot_analysis()
        self.stop_ai_rating()
        self.stop_grouping_preview()
        self.stop_grouping_workflow()
        self.stop_pick_best_analysis()
        self.stop_easy_delete_analysis()
        self.stop_fix_rotation_detection()
        logger.info("All workers stop requested.")

    def is_file_scanner_running(self) -> bool:
        return self.scanner_thread is not None and self.scanner_thread.isRunning()

    def is_similarity_worker_running(self) -> bool:
        return self.similarity_thread is not None and self.similarity_thread.isRunning()

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

    def is_best_shot_worker_running(self) -> bool:
        return self.best_shot_thread is not None and self.best_shot_thread.isRunning()

    def is_ai_rating_running(self) -> bool:
        return self.ai_rating_thread is not None and self.ai_rating_thread.isRunning()

    def is_grouping_preview_running(self) -> bool:
        return (
            self.grouping_preview_thread is not None
            and self.grouping_preview_thread.isRunning()
        )

    def is_grouping_workflow_running(self) -> bool:
        return (
            self.grouping_workflow_thread is not None
            and self.grouping_workflow_thread.isRunning()
        )

    def is_pick_best_running(self) -> bool:
        return self.pick_best_thread is not None and self.pick_best_thread.isRunning()

    def start_update_check(self, current_version: str):
        """Start checking for updates in a background thread."""
        from workers.update_worker import UpdateCheckWorker

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
        self._finish_worker_slot(
            "update_check_thread", "update_check_worker", "Update check"
        )

    def is_update_check_running(self) -> bool:
        return (
            self.update_check_thread is not None
            and self.update_check_thread.isRunning()
        )

    def stop_update_check(self) -> None:
        """Stop an in-flight update check during application shutdown."""

        self._stop_worker("update_check_thread", "update_check_worker")

    def is_any_worker_running(self) -> bool:
        return (
            self.is_file_scanner_running()
            or self.is_similarity_worker_running()
            or self.is_blur_detection_running()
            or self.is_rating_loader_running()
            or self.is_rotation_detection_running()
            or self.is_cuda_detection_running()
            or self.is_update_check_running()
            or self.is_rating_writer_running()
            or self.is_rotation_application_running()
            or self.is_thumbnail_preload_running()
            or self.is_grouping_preview_running()
            or self.is_grouping_workflow_running()
            or self.is_best_shot_worker_running()
            or self.is_ai_rating_running()
            or self.is_pick_best_running()
            or self.is_easy_delete_running()
            or self.is_fix_rotation_running()
        )

    # --- Rating Writer Management ---
    def start_rating_writer(
        self,
        rating_operations: List,
        rating_disk_cache: Optional[RatingCache] = None,
        exif_disk_cache: Optional[ExifCache] = None,
    ):
        """Start writing ratings in a background thread."""
        from workers.rating_writer_worker import RatingWriterWorker

        if self.is_rating_writer_running():
            logger.warning("Rating writer is already running")
            return

        logger.info(
            f"Starting rating writer for {len(rating_operations)} operations..."
        )

        self.rating_writer_thread = QThread()
        self.rating_writer_worker = RatingWriterWorker(
            rating_disk_cache=rating_disk_cache, exif_disk_cache=exif_disk_cache
        )
        self.rating_writer_worker.moveToThread(self.rating_writer_thread)

        # Connect signals
        self.rating_writer_worker.progress.connect(self.rating_write_progress.emit)
        self.rating_writer_worker.rating_written.connect(self.rating_written.emit)
        self.rating_writer_worker.finished.connect(self.rating_write_finished.emit)
        self.rating_writer_worker.error.connect(self.rating_write_error.emit)
        self.rating_writer_worker.finished.connect(self._cleanup_rating_writer_worker)

        # Connect start signal
        self.rating_writer_thread.started.connect(
            lambda: self.rating_writer_worker.write_ratings(rating_operations)
        )

        # Start the thread
        self.rating_writer_thread.start()

    def _cleanup_rating_writer_worker(self):
        """Clean up the rating writer worker and thread."""
        self._finish_worker_slot(
            "rating_writer_thread", "rating_writer_worker", "Rating writer"
        )

    def is_rating_writer_running(self) -> bool:
        return (
            self.rating_writer_thread is not None
            and self.rating_writer_thread.isRunning()
        )

    def stop_rating_writer(self):
        """Stop the rating writer thread."""
        self._stop_worker("rating_writer_thread", "rating_writer_worker")

    # --- Rotation Application Management ---
    def start_rotation_application(
        self,
        approved_rotations: Dict[str, int],
        exif_disk_cache: Optional[ExifCache] = None,
    ):
        """Start applying rotations in a background thread."""
        from workers.rotation_application_worker import RotationApplicationWorker

        if self.is_rotation_application_running():
            logger.warning("Rotation application is already running")
            return

        logger.info(
            f"Starting rotation application for {len(approved_rotations)} rotations..."
        )

        self.rotation_application_thread = QThread()
        self.rotation_application_worker = RotationApplicationWorker(
            exif_disk_cache=exif_disk_cache
        )
        self.rotation_application_worker.moveToThread(self.rotation_application_thread)

        # Connect signals
        self.rotation_application_worker.progress.connect(
            self.rotation_application_progress.emit
        )
        self.rotation_application_worker.rotation_applied.connect(
            self.rotation_applied.emit
        )
        self.rotation_application_worker.finished.connect(
            self.rotation_application_finished.emit
        )
        self.rotation_application_worker.error.connect(
            self.rotation_application_error.emit
        )
        self.rotation_application_worker.finished.connect(
            self._cleanup_rotation_application_worker
        )

        # Connect start signal
        self.rotation_application_thread.started.connect(
            lambda: self.rotation_application_worker.apply_rotations(approved_rotations)
        )

        # Start the thread
        self.rotation_application_thread.start()

    def _cleanup_rotation_application_worker(self):
        """Clean up the rotation application worker and thread."""
        self._finish_worker_slot(
            "rotation_application_thread",
            "rotation_application_worker",
            "Rotation application",
        )

    def is_rotation_application_running(self) -> bool:
        return (
            self.rotation_application_thread is not None
            and self.rotation_application_thread.isRunning()
        )

    def stop_rotation_application(self):
        """Stop the rotation application thread."""
        self._stop_worker("rotation_application_thread", "rotation_application_worker")

    # --- Thumbnail Preload Management ---
    def start_thumbnail_preload(self, image_paths: List[str]):
        """Start preloading thumbnails in a background thread (non-blocking)."""
        from workers.thumbnail_preload_worker import ThumbnailPreloadWorker

        if self.is_thumbnail_preload_running():
            logger.warning("Thumbnail preload is already running")
            return

        logger.info(f"Starting thumbnail preload for {len(image_paths)} images...")

        self.thumbnail_preload_thread = QThread()
        self.thumbnail_preload_worker = ThumbnailPreloadWorker(
            image_pipeline=self.image_pipeline
        )
        self.thumbnail_preload_worker.moveToThread(self.thumbnail_preload_thread)

        # Connect signals
        self.thumbnail_preload_worker.progress.connect(
            self.thumbnail_preload_progress.emit
        )
        self.thumbnail_preload_worker.finished.connect(
            self.thumbnail_preload_finished.emit
        )
        self.thumbnail_preload_worker.error.connect(self.thumbnail_preload_error.emit)
        self.thumbnail_preload_worker.finished.connect(
            self._cleanup_thumbnail_preload_worker
        )

        # Connect start signal
        self.thumbnail_preload_thread.started.connect(
            lambda: self.thumbnail_preload_worker.preload_thumbnails(image_paths)
        )

        # Start the thread
        self.thumbnail_preload_thread.start()

    def _cleanup_thumbnail_preload_worker(self, _image_paths=None):
        """Clean up the thumbnail preload worker and thread."""
        self._finish_worker_slot(
            "thumbnail_preload_thread",
            "thumbnail_preload_worker",
            "Thumbnail preload",
        )

    def is_thumbnail_preload_running(self) -> bool:
        return (
            self.thumbnail_preload_thread is not None
            and self.thumbnail_preload_thread.isRunning()
        )

    def stop_thumbnail_preload(self):
        """Stop the thumbnail preload thread."""
        self._stop_worker("thumbnail_preload_thread", "thumbnail_preload_worker")

    def _cleanup_best_shot_worker(self):
        self._cleanup_worker_refs(
            "best_shot_thread", "best_shot_worker", "Best shot analysis"
        )

    def _cleanup_ai_rating_worker(self):
        self._cleanup_worker_refs("ai_rating_thread", "ai_rating_worker", "AI rating")

    def start_pick_best_analysis(self, cluster_map: Dict[int, List[str]]) -> None:
        """Start the pick-best scoring worker."""
        from workers.pick_best_worker import PickBestWorker

        self.stop_pick_best_analysis()
        if not cluster_map:
            self.pick_best_complete.emit({})
            return

        self.pick_best_thread = QThread()
        self.pick_best_worker = PickBestWorker(
            cluster_map=cluster_map,
            image_pipeline=self.image_pipeline,
        )
        self.pick_best_worker.moveToThread(self.pick_best_thread)

        self.pick_best_worker.progress_update.connect(self.pick_best_progress.emit)
        self.pick_best_worker.completed.connect(self.pick_best_complete.emit)
        self.pick_best_worker.error.connect(self.pick_best_error.emit)
        self.pick_best_worker.finished.connect(self.pick_best_thread.quit)
        self.pick_best_worker.finished.connect(self.pick_best_worker.deleteLater)
        self.pick_best_thread.finished.connect(self._cleanup_pick_best_worker)
        self.pick_best_thread.started.connect(self.pick_best_worker.run)

        self.pick_best_thread.start()
        logger.info("Pick best analysis thread started.")

    def stop_pick_best_analysis(self) -> None:
        self._stop_worker("pick_best_thread", "pick_best_worker")

    def _cleanup_pick_best_worker(self) -> None:
        self._cleanup_worker_refs(
            "pick_best_thread", "pick_best_worker", "Pick best analysis"
        )

    def start_easy_delete_analysis(
        self,
        image_paths: List[str],
        cluster_map: Optional[Dict[int, List[str]]] = None,
        embeddings_cache: Optional[Dict] = None,
        exif_disk_cache=None,
    ) -> None:
        from workers.easy_delete_worker import EasyDeleteWorker

        self.stop_easy_delete_analysis()
        if not image_paths:
            self.easy_delete_complete.emit({})
            return

        self.easy_delete_thread = QThread()
        self.easy_delete_worker = EasyDeleteWorker(
            image_paths=image_paths,
            cluster_map=cluster_map,
            embeddings_cache=embeddings_cache,
            exif_disk_cache=exif_disk_cache,
            image_pipeline=self.image_pipeline,
        )
        self.easy_delete_worker.moveToThread(self.easy_delete_thread)

        self.easy_delete_worker.progress_update.connect(self.easy_delete_progress.emit)
        self.easy_delete_worker.completed.connect(self.easy_delete_complete.emit)
        self.easy_delete_worker.error.connect(self.easy_delete_error.emit)
        self.easy_delete_worker.finished.connect(self.easy_delete_thread.quit)
        self.easy_delete_worker.finished.connect(self.easy_delete_worker.deleteLater)
        self.easy_delete_thread.finished.connect(self._cleanup_easy_delete_worker)
        self.easy_delete_thread.started.connect(self.easy_delete_worker.run)

        self.easy_delete_thread.start()
        logger.info("Easy delete analysis thread started.")

    def stop_easy_delete_analysis(self) -> None:
        self._stop_worker("easy_delete_thread", "easy_delete_worker")

    def is_easy_delete_running(self) -> bool:
        return (
            self.easy_delete_thread is not None and self.easy_delete_thread.isRunning()
        )

    def _cleanup_easy_delete_worker(self) -> None:
        self._cleanup_worker_refs(
            "easy_delete_thread", "easy_delete_worker", "Easy delete analysis"
        )

    # ------------------------------------------------------------------
    # Fix Rotation Detection
    # ------------------------------------------------------------------

    def start_fix_rotation_detection(self, image_paths: List[str]) -> None:
        from workers.rotation_detection_step_worker import RotationDetectionStepWorker

        self.stop_fix_rotation_detection()
        if not image_paths:
            self.fix_rotation_complete.emit({})
            return

        from core.image_features.rotation_detector import RotationDetector
        from core.caching.exif_cache import ExifCache

        rotation_detector = RotationDetector(
            image_pipeline=self.image_pipeline,
            exif_cache=ExifCache(),
        )

        self.fix_rotation_detect_thread = QThread()
        self.fix_rotation_detect_worker = RotationDetectionStepWorker(
            image_paths=image_paths,
            rotation_detector=rotation_detector,
        )
        self.fix_rotation_detect_worker.moveToThread(self.fix_rotation_detect_thread)

        self.fix_rotation_detect_worker.progress_update.connect(
            self.fix_rotation_progress.emit
        )
        self.fix_rotation_detect_worker.completed.connect(
            self.fix_rotation_complete.emit
        )
        self.fix_rotation_detect_worker.model_not_found.connect(
            self.fix_rotation_model_not_found.emit
        )
        self.fix_rotation_detect_worker.error.connect(self.fix_rotation_error.emit)
        self.fix_rotation_detect_worker.finished.connect(
            self.fix_rotation_detect_thread.quit
        )
        self.fix_rotation_detect_worker.finished.connect(
            self.fix_rotation_detect_worker.deleteLater
        )
        self.fix_rotation_detect_thread.finished.connect(
            self._cleanup_fix_rotation_detect_worker
        )
        self.fix_rotation_detect_thread.started.connect(
            self.fix_rotation_detect_worker.run
        )

        self.fix_rotation_detect_thread.start()
        logger.info("Fix rotation detection thread started.")

    def stop_fix_rotation_detection(self) -> None:
        self._stop_worker("fix_rotation_detect_thread", "fix_rotation_detect_worker")

    def is_fix_rotation_running(self) -> bool:
        return (
            self.fix_rotation_detect_thread is not None
            and self.fix_rotation_detect_thread.isRunning()
        )

    def _cleanup_fix_rotation_detect_worker(self) -> None:
        self._cleanup_worker_refs(
            "fix_rotation_detect_thread",
            "fix_rotation_detect_worker",
            "Fix rotation detection",
        )

    def start_best_shot_analysis(
        self,
        cluster_map: Dict[int, List[str]],
        *,
        folder_path: Optional[str] = None,
        analysis_cache=None,
    ):
        """Start the best-shot ranking worker."""
        from workers.best_shot_worker import BestShotWorker

        self.stop_best_shot_analysis()
        if not cluster_map:
            self.best_shot_complete.emit({})
            return

        self.best_shot_thread = QThread()
        self.best_shot_worker = BestShotWorker(
            cluster_map=cluster_map,
            image_pipeline=self.image_pipeline,
            folder_path=folder_path,
            analysis_cache=analysis_cache,
            best_shot_batch_size=get_best_shot_batch_size(),
        )
        self.best_shot_worker.moveToThread(self.best_shot_thread)

        self.best_shot_worker.progress_update.connect(self.best_shot_progress.emit)
        self.best_shot_worker.completed.connect(self.best_shot_complete.emit)
        self.best_shot_worker.error.connect(self.best_shot_error.emit)
        self.best_shot_worker.finished.connect(self.best_shot_thread.quit)
        self.best_shot_worker.finished.connect(self.best_shot_worker.deleteLater)
        self.best_shot_thread.finished.connect(self._cleanup_best_shot_worker)
        self.best_shot_thread.started.connect(self.best_shot_worker.run)

        self.best_shot_thread.start()
        logger.info("Best shot analysis thread started.")

    def stop_best_shot_analysis(self):
        self._stop_worker("best_shot_thread", "best_shot_worker")

    def start_ai_rating(
        self,
        image_paths: List[str],
    ) -> None:
        """Start AI-driven rating for the provided images."""
        from workers.ai_rating_worker import AiRatingWorker

        self.stop_ai_rating()
        if not image_paths:
            self.ai_rating_complete.emit({})
            return

        self.ai_rating_thread = QThread()
        self.ai_rating_worker = AiRatingWorker(
            image_paths=image_paths,
            image_pipeline=self.image_pipeline,
        )
        self.ai_rating_worker.moveToThread(self.ai_rating_thread)

        self.ai_rating_worker.progress_update.connect(self.ai_rating_progress.emit)
        self.ai_rating_worker.completed.connect(self.ai_rating_complete.emit)
        self.ai_rating_worker.error.connect(self.ai_rating_error.emit)
        self.ai_rating_worker.warning.connect(self.ai_rating_warning.emit)
        self.ai_rating_worker.finished.connect(self.ai_rating_thread.quit)
        self.ai_rating_worker.finished.connect(self.ai_rating_worker.deleteLater)
        self.ai_rating_thread.finished.connect(self._cleanup_ai_rating_worker)
        self.ai_rating_thread.started.connect(self.ai_rating_worker.run)

        self.ai_rating_thread.start()
        logger.info("AI rating thread started.")

    def stop_ai_rating(self) -> None:
        self._stop_worker("ai_rating_thread", "ai_rating_worker")
