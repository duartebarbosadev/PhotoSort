import logging
from datetime import datetime
from PyQt6.QtCore import QObject, pyqtSignal, QThread
from typing import Any, TYPE_CHECKING
from collections.abc import Callable, Sequence

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

sip = _sip

logger = logging.getLogger(__name__)

_WORKER_SLOTS = (
    ("scanner_thread", "file_scanner"),
    ("similarity_thread", "similarity_worker"),
    ("cull_grouping_thread", "cull_grouping_worker"),
    ("model_environment_thread", "model_environment_worker"),
    ("rating_loader_thread", "rating_loader_worker"),
    ("rating_writer_thread", "rating_writer_worker"),
    ("rotation_application_thread", "rotation_application_worker"),
    ("thumbnail_preload_thread", "thumbnail_preload_worker"),
    ("update_check_thread", "update_check_worker"),
    ("ai_rating_thread", "ai_rating_worker"),
    ("grouping_preview_thread", "grouping_preview_worker"),
    ("grouping_workflow_thread", "grouping_workflow_worker"),
    ("file_deletion_thread", "file_deletion_worker"),
    ("pick_best_thread", "pick_best_worker"),
    ("easy_delete_thread", "easy_delete_worker"),
    ("fix_rotation_detect_thread", "fix_rotation_detect_worker"),
)

if TYPE_CHECKING:
    from ui.ui_components import (
        SimilarityWorker,
    )
    from workers.ai_rating_worker import AiRatingWorker
    from workers.easy_delete_worker import EasyDeleteWorker
    from workers.cull_subject_grouping_worker import CullSubjectGroupingWorker
    from workers.model_environment_probe_worker import ModelEnvironmentProbeWorker
    from workers.file_deletion_worker import FileDeletionWorker
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
    similarity_regional_embeddings_generated = pyqtSignal(dict)
    similarity_clustering_complete = pyqtSignal(object)
    similarity_error = pyqtSignal(str)

    # Cull same-subject grouping signals
    cull_grouping_progress = pyqtSignal(int, str)
    cull_grouping_complete = pyqtSignal(object)
    cull_grouping_error = pyqtSignal(str)
    cull_grouping_finished = pyqtSignal()
    # (missing model keys, torch device)
    model_environment_ready = pyqtSignal(tuple, str)

    # Rating Loader Signals
    rating_load_progress = pyqtSignal(int, int, str)  # current, total, basename
    rating_load_metadata_batch_loaded = pyqtSignal(
        list
    )  # List of tuples: [(image_path, metadata_dict), ...]
    rating_load_finished = pyqtSignal()
    rating_load_error = pyqtSignal(str)
    rating_load_cache_capacity_warning = pyqtSignal(int, int, object)

    # CUDA Detection Signals

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

    thumbnail_session_batch_ready = pyqtSignal(str, object)
    thumbnail_session_progress = pyqtSignal(str, int, int, int, bool)
    thumbnail_session_finished = pyqtSignal(str, int, int)
    thumbnail_session_error = pyqtSignal(str, str)
    thumbnail_session_capacity_required = pyqtSignal(str, int)
    thumbnail_session_metrics = pyqtSignal(str, object)

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

    # Filesystem deletion signals
    file_deletion_progress = pyqtSignal(int, int, str)
    file_deletion_complete = pyqtSignal(object)

    # Pick Best signals
    pick_best_progress = pyqtSignal(int, str)
    pick_best_complete = pyqtSignal(dict)
    pick_best_error = pyqtSignal(str)

    # Easy Delete signals
    easy_delete_progress = pyqtSignal(int, str)
    easy_delete_complete = pyqtSignal(dict)
    easy_delete_assessments_ready = pyqtSignal(dict)
    easy_delete_error = pyqtSignal(str)

    # Fix Rotation Detection signals
    fix_rotation_progress = pyqtSignal(int, str)
    fix_rotation_complete = pyqtSignal(dict)  # {path: angle}
    fix_rotation_model_not_found = pyqtSignal(str)
    fix_rotation_error = pyqtSignal(str)

    # Fix Rotation Apply signals (reuse rotation_application_* signals)

    def __init__(
        self, image_pipeline_instance: ImagePipeline, parent: QObject | None = None
    ):
        super().__init__(parent)
        self.image_pipeline = image_pipeline_instance

        self.scanner_thread: QThread | None = None
        self.file_scanner: FileScanner | None = None

        self.similarity_thread: QThread | None = None
        self.similarity_worker: SimilarityWorker | None = None
        self.cull_grouping_thread: QThread | None = None
        self.cull_grouping_worker: CullSubjectGroupingWorker | None = None
        self.model_environment_thread: QThread | None = None
        self.model_environment_worker: ModelEnvironmentProbeWorker | None = None

        self.rating_loader_thread: QThread | None = None
        self.rating_loader_worker: RatingLoaderWorker | None = None

        self.rating_writer_thread: QThread | None = None
        self.rating_writer_worker: RatingWriterWorker | None = None

        self.rotation_application_thread: QThread | None = None
        self.rotation_application_worker: RotationApplicationWorker | None = None

        self.thumbnail_preload_thread: QThread | None = None
        self.thumbnail_preload_worker: ThumbnailPreloadWorker | None = None
        self.ai_rating_thread: QThread | None = None
        self.ai_rating_worker: AiRatingWorker | None = None
        self.grouping_preview_thread: QThread | None = None
        self.grouping_preview_worker: GroupingPreviewWorker | None = None
        self.grouping_workflow_thread: QThread | None = None
        self.grouping_workflow_worker: GroupingWorkflowWorker | None = None
        self.file_deletion_thread: QThread | None = None
        self.file_deletion_worker: FileDeletionWorker | None = None

        self.pick_best_thread: QThread | None = None
        self.pick_best_worker: PickBestWorker | None = None

        self.easy_delete_thread: QThread | None = None
        self.easy_delete_worker: EasyDeleteWorker | None = None

        self.fix_rotation_detect_thread: QThread | None = None
        self.fix_rotation_detect_worker: RotationDetectionStepWorker | None = None

        self.update_check_thread: QThread | None = None
        self.update_check_worker: UpdateCheckWorker | None = None
        self._worker_generations: dict[str, int] = {}

    def _advance_worker_generation(self, name: str) -> int:
        generation = self._worker_generations.get(name, 0) + 1
        self._worker_generations[name] = generation
        return generation

    def _emit_if_current(self, name: str, generation: int, signal, *args) -> None:
        """Drop queued callbacks belonging to a cancelled or replaced worker."""
        if self._worker_generations.get(name) == generation:
            signal.emit(*args)

    def _terminate_thread(
        self,
        thread: QThread | None,
        worker_stop_method: Callable[[], Any] | None = None,
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
        before_stop: Callable[[Any], None] | None = None,
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

    def _request_worker_stop(
        self,
        thread_attribute: str,
        worker_attribute: str,
        *,
        before_stop: Callable[[Any], None] | None = None,
    ) -> None:
        """Request cancellation without waiting on the caller's thread."""

        worker = getattr(self, worker_attribute)
        if worker is not None and before_stop is not None:
            try:
                before_stop(worker)
            except Exception:
                logger.debug("Worker pre-stop hook failed.", exc_info=True)
        stop_method = getattr(worker, "stop", None) if worker is not None else None
        if stop_method is not None:
            try:
                stop_method()
            except Exception:
                logger.error(
                    "Error requesting stop for %s.", worker_attribute, exc_info=True
                )

        thread = getattr(self, thread_attribute)
        if thread is not None and thread.isRunning():
            thread.requestInterruption()
            thread.quit()

    def _cleanup_scanner_refs(self):
        self._cleanup_worker_refs("scanner_thread", "file_scanner", "File scanner")

    # --- File Scanner Management ---
    def start_file_scan(
        self,
        folder_path: str,
    ):
        self.stop_file_scan()  # Ensure any previous scan is stopped
        generation = self._advance_worker_generation("file_scan")
        self.scanner_thread = QThread()
        self.file_scanner = FileScanner(
            image_pipeline=self.image_pipeline, directory_path=folder_path
        )  # Inject shared pipeline instance
        self.file_scanner.moveToThread(self.scanner_thread)

        # Connect signals from FileScanner to WorkerManager's signals
        self.file_scanner.files_found.connect(
            lambda files: self._emit_if_current("file_scan", generation, self.file_scan_found_files, files)
        )
        self.file_scanner.thumbnail_preload_finished.connect(
            lambda files: self._emit_if_current("file_scan", generation, self.file_scan_thumbnail_preload_finished, files)
        )
        self.file_scanner.finished.connect(
            lambda: self._emit_if_current("file_scan", generation, self.file_scan_finished)
        )
        self.file_scanner.error.connect(
            lambda message: self._emit_if_current("file_scan", generation, self.file_scan_error, message)
        )

        self.scanner_thread.started.connect(self.file_scanner.run)
        self.file_scanner.finished.connect(self.scanner_thread.quit)

        # Connect to our cleanup method instead of direct deleteLater from here
        self.scanner_thread.finished.connect(self._cleanup_scanner_refs)

        self.scanner_thread.start()
        logger.info("File scanner thread started.")

    def stop_file_scan(self):
        self._advance_worker_generation("file_scan")
        self._stop_worker("scanner_thread", "file_scanner")

    def _cleanup_similarity_refs(self):
        self._cleanup_worker_refs(
            "similarity_thread", "similarity_worker", "Similarity analysis"
        )

    # --- Similarity Engine Management ---
    def start_similarity_analysis(
        self,
        file_paths: list[str],
        allow_model_download: bool = False,
        *,
        folder_path: str | None = None,
        analysis_cache=None,
        fingerprints: dict[str, tuple[int, int]] | None = None,
    ):
        from ui.ui_components import SimilarityWorker

        self.stop_similarity_analysis()
        generation = self._advance_worker_generation("similarity")
        self.similarity_thread = QThread()
        self.similarity_worker = SimilarityWorker(
            file_paths,
            allow_model_download=allow_model_download,
            image_pipeline=self.image_pipeline,
            folder_path=folder_path,
            analysis_cache=analysis_cache,
            fingerprints=fingerprints,
        )
        self.similarity_worker.moveToThread(self.similarity_thread)

        # Connect signals from the new worker to the manager's signals
        self.similarity_worker.progress_update.connect(
            lambda percent, message: self._emit_if_current(
                "similarity", generation, self.similarity_progress, percent, message
            )
        )
        self.similarity_worker.embeddings_generated.connect(
            lambda embeddings: self._emit_if_current(
                "similarity",
                generation,
                self.similarity_embeddings_generated,
                embeddings,
            )
        )
        self.similarity_worker.regional_embeddings_generated.connect(
            lambda embeddings: self._emit_if_current(
                "similarity",
                generation,
                self.similarity_regional_embeddings_generated,
                embeddings,
            )
        )
        self.similarity_worker.clustering_complete.connect(
            lambda clusters: self._emit_if_current(
                "similarity",
                generation,
                self.similarity_clustering_complete,
                clusters,
            )
        )
        self.similarity_worker.error.connect(
            lambda message: self._emit_if_current(
                "similarity", generation, self.similarity_error, message
            )
        )
        self.similarity_worker.finished.connect(self.similarity_thread.quit)

        self.similarity_thread.started.connect(self.similarity_worker.run)
        self.similarity_thread.finished.connect(self._cleanup_similarity_refs)

        self.similarity_thread.start()
        logger.info("Similarity engine thread started.")

    def stop_similarity_analysis(self):
        self._advance_worker_generation("similarity")
        self._stop_worker("similarity_thread", "similarity_worker")

    def start_cull_subject_grouping(
        self,
        *,
        paths: list[str],
        fingerprints: dict[str, tuple[int, int]],
        timestamps: dict[str, datetime | None],
        strictness,
        analysis_cache,
        folder_path: str,
        allow_model_download: bool,
    ) -> None:
        from workers.cull_subject_grouping_worker import CullSubjectGroupingWorker

        self.stop_cull_subject_grouping()
        generation = self._advance_worker_generation("cull_grouping")
        self.cull_grouping_thread = QThread()
        self.cull_grouping_worker = CullSubjectGroupingWorker(
            paths=paths,
            fingerprints=fingerprints,
            timestamps=timestamps,
            strictness=strictness,
            image_pipeline=self.image_pipeline,
            analysis_cache=analysis_cache,
            folder_path=folder_path,
            allow_model_download=allow_model_download,
        )
        self.cull_grouping_worker.moveToThread(self.cull_grouping_thread)
        self.cull_grouping_worker.progress_update.connect(
            lambda percent, message: self._emit_if_current(
                "cull_grouping",
                generation,
                self.cull_grouping_progress,
                percent,
                message,
            )
        )
        self.cull_grouping_worker.completed.connect(
            lambda result: self._emit_if_current(
                "cull_grouping", generation, self.cull_grouping_complete, result
            )
        )
        self.cull_grouping_worker.error.connect(
            lambda message: self._emit_if_current(
                "cull_grouping", generation, self.cull_grouping_error, message
            )
        )
        self.cull_grouping_worker.finished.connect(self.cull_grouping_thread.quit)
        self.cull_grouping_thread.finished.connect(self._cleanup_cull_grouping_refs)
        self.cull_grouping_thread.started.connect(self.cull_grouping_worker.run)
        self.cull_grouping_thread.start()

    def _cleanup_cull_grouping_refs(self) -> None:
        self._cleanup_worker_refs(
            "cull_grouping_thread", "cull_grouping_worker", "Cull subject grouping"
        )
        self.cull_grouping_finished.emit()

    def start_model_environment_probe(self, model_keys: Sequence[str]) -> None:
        """Resolve model availability and the torch device off the GUI thread."""

        if self.model_environment_thread is not None:
            return
        from workers.model_environment_probe_worker import ModelEnvironmentProbeWorker

        generation = self._advance_worker_generation("model_environment")
        self.model_environment_thread = QThread()
        self.model_environment_worker = ModelEnvironmentProbeWorker(model_keys)
        self.model_environment_worker.moveToThread(self.model_environment_thread)
        self.model_environment_worker.completed.connect(
            lambda missing, device: self._emit_if_current(
                "model_environment",
                generation,
                self.model_environment_ready,
                missing,
                device,
            )
        )
        self.model_environment_worker.finished.connect(
            self.model_environment_thread.quit
        )
        self.model_environment_thread.finished.connect(
            self._cleanup_model_environment_refs
        )
        self.model_environment_thread.started.connect(self.model_environment_worker.run)
        self.model_environment_thread.start()

    def _cleanup_model_environment_refs(self) -> None:
        self._cleanup_worker_refs(
            "model_environment_thread",
            "model_environment_worker",
            "Model environment probe",
        )

    def is_model_environment_probe_running(self) -> bool:
        return self.model_environment_thread is not None

    def stop_model_environment_probe(self) -> None:
        self._advance_worker_generation("model_environment")
        self._stop_worker("model_environment_thread", "model_environment_worker")

    def stop_cull_subject_grouping(self) -> None:
        self._advance_worker_generation("cull_grouping")
        self._stop_worker("cull_grouping_thread", "cull_grouping_worker")

    def request_stop_cull_subject_grouping(self) -> None:
        self._advance_worker_generation("cull_grouping")
        self._request_worker_stop("cull_grouping_thread", "cull_grouping_worker")

    def request_stop_similarity_analysis(self) -> None:
        self._advance_worker_generation("similarity")
        self._request_worker_stop("similarity_thread", "similarity_worker")

    def _cleanup_rating_loader_refs(self):
        self._cleanup_worker_refs(
            "rating_loader_thread", "rating_loader_worker", "Rating loader"
        )

    # --- Rating Loader Management ---
    def start_rating_load(
        self,
        image_data_list: list[dict[str, Any]],
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
        self.rating_loader_worker.cache_capacity_warning.connect(
            self.rating_load_cache_capacity_warning
        )

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

    def _cleanup_grouping_preview_refs(self):
        self._cleanup_worker_refs(
            "grouping_preview_thread", "grouping_preview_worker", "Grouping preview"
        )

    def start_grouping_preview(
        self,
        items: list[dict[str, Any]],
        mode: str,
        source_root: str | None = None,
        location_depth: int = 3,
        analysis_cache=None,
        folder_path: str | None = None,
        allow_model_download: bool = False,
    ):
        from workers.grouping_worker import GroupingPreviewWorker

        if self.grouping_preview_thread is not None:
            return False
        generation = self._advance_worker_generation("grouping_preview")
        self.grouping_preview_thread = QThread()
        self.grouping_preview_worker = GroupingPreviewWorker(
            items,
            mode,
            source_root,
            location_depth,
            image_pipeline=self.image_pipeline,
            analysis_cache=analysis_cache,
            folder_path=folder_path,
            allow_model_download=allow_model_download,
        )
        self.grouping_preview_worker.moveToThread(self.grouping_preview_thread)

        self.grouping_preview_worker.progress_update.connect(
            lambda percent, message: self._emit_if_current(
                "grouping_preview",
                generation,
                self.grouping_preview_progress,
                percent,
                message,
            )
        )
        self.grouping_preview_worker.preview_ready.connect(
            lambda plan: self._emit_if_current(
                "grouping_preview", generation, self.grouping_preview_ready, plan
            )
        )
        self.grouping_preview_worker.error.connect(
            lambda message: self._emit_if_current(
                "grouping_preview", generation, self.grouping_preview_error, message
            )
        )
        self.grouping_preview_worker.finished.connect(self.grouping_preview_thread.quit)
        self.grouping_preview_thread.started.connect(self.grouping_preview_worker.run)
        self.grouping_preview_thread.finished.connect(
            self._cleanup_grouping_preview_refs
        )
        self.grouping_preview_thread.start()
        logger.info("Grouping preview thread started.")
        return True

    def stop_grouping_preview(self):
        self._advance_worker_generation("grouping_preview")
        self._stop_worker("grouping_preview_thread", "grouping_preview_worker")

    def request_stop_grouping_preview(self) -> None:
        self._advance_worker_generation("grouping_preview")
        self._request_worker_stop("grouping_preview_thread", "grouping_preview_worker")

    def _cleanup_grouping_workflow_refs(self):
        self._cleanup_worker_refs(
            "grouping_workflow_thread",
            "grouping_workflow_worker",
            "Grouping workflow",
        )

    def start_grouping_workflow(
        self,
        items: list[dict[str, Any]],
        mode: str,
        source_root: str,
        output_root: str | None = None,
        group_name_overrides: dict[str, str] | None = None,
        prepared_plan=None,
        location_depth: int = 3,
        move_companions: bool = False,
        rating_cache=None,
        exif_cache=None,
        analysis_cache=None,
        allow_model_download: bool = False,
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
            rating_cache=rating_cache,
            exif_cache=exif_cache,
            analysis_cache=analysis_cache,
            allow_model_download=allow_model_download,
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

    def _cleanup_file_deletion_refs(self) -> None:
        self._cleanup_worker_refs(
            "file_deletion_thread",
            "file_deletion_worker",
            "File deletion",
        )

    def start_file_deletion(
        self,
        targets: list[str],
        *,
        cache_paths_by_target: dict[str, list[str]] | None = None,
        rating_cache=None,
        exif_cache=None,
        analysis_cache=None,
        folder_path: str | None = None,
    ) -> bool:
        """Start one serialized Trash batch without blocking the UI thread."""

        from workers.file_deletion_worker import FileDeletionWorker

        if self.is_file_deletion_running() or not targets:
            return False
        self.file_deletion_thread = QThread()
        self.file_deletion_worker = FileDeletionWorker(
            targets,
            cache_paths_by_target=cache_paths_by_target,
            rating_cache=rating_cache,
            exif_cache=exif_cache,
            analysis_cache=analysis_cache,
            folder_path=folder_path,
        )
        self.file_deletion_worker.moveToThread(self.file_deletion_thread)
        self.file_deletion_worker.progress.connect(self.file_deletion_progress)
        self.file_deletion_worker.completed.connect(self.file_deletion_complete)
        self.file_deletion_worker.finished.connect(self.file_deletion_thread.quit)
        self.file_deletion_thread.started.connect(self.file_deletion_worker.run)
        self.file_deletion_thread.finished.connect(self._cleanup_file_deletion_refs)
        self.file_deletion_thread.start()
        logger.info("File deletion thread started for %d target(s).", len(targets))
        return True

    def stop_file_deletion(self) -> None:
        # Never terminate a thread while the platform Trash API owns a file.
        self._stop_worker(
            "file_deletion_thread",
            "file_deletion_worker",
            allow_terminate=False,
        )

    def is_file_deletion_running(self) -> bool:
        return self.file_deletion_thread is not None

    def stop_all_workers(self):
        logger.info("Stopping all workers...")
        self.stop_file_scan()
        self.stop_similarity_analysis()
        self.stop_cull_subject_grouping()
        self.stop_model_environment_probe()
        self.stop_rating_load()
        self.stop_rating_writer()
        self.stop_rotation_application()
        self.stop_thumbnail_preload()
        self.stop_update_check()
        self.stop_ai_rating()
        self.stop_grouping_preview()
        self.stop_grouping_workflow()
        self.stop_file_deletion()
        self.stop_pick_best_analysis()
        self.stop_easy_delete_analysis()
        self.stop_fix_rotation_detection()
        logger.info("All workers stop requested.")

    def request_stop_all_workers(self) -> None:
        """Request application-wide cancellation without blocking the UI thread."""

        logger.info("Requesting all workers stop without blocking...")
        for generation_name in (
            "file_scan",
            "ai_rating",
            "update_check",
            "similarity",
            "cull_grouping",
            "pick_best",
            "easy_delete",
            "fix_rotation",
            "grouping_preview",
        ):
            self._advance_worker_generation(generation_name)

        for thread_attribute, worker_attribute in _WORKER_SLOTS:
            before_stop = (
                (lambda worker: worker.disable_emits())
                if worker_attribute == "rating_loader_worker"
                else None
            )
            self._request_worker_stop(
                thread_attribute,
                worker_attribute,
                before_stop=before_stop,
            )
        logger.info("All worker cancellation requests dispatched.")

    def is_file_scanner_running(self) -> bool:
        return self.scanner_thread is not None

    def is_similarity_worker_running(self) -> bool:
        return self.similarity_thread is not None

    def is_cull_grouping_running(self) -> bool:
        return self.cull_grouping_thread is not None

    def is_rating_loader_running(self) -> bool:
        return self.rating_loader_thread is not None

    def is_ai_rating_running(self) -> bool:
        return self.ai_rating_thread is not None

    def is_grouping_preview_running(self) -> bool:
        return self.grouping_preview_thread is not None

    def is_grouping_preview_active(self) -> bool:
        return self.grouping_preview_thread is not None

    def is_grouping_workflow_running(self) -> bool:
        return self.grouping_workflow_thread is not None

    def is_pick_best_running(self) -> bool:
        return self.pick_best_thread is not None

    def start_update_check(self, current_version: str):
        """Start checking for updates in a background thread."""
        from workers.update_worker import UpdateCheckWorker

        if self.is_update_check_running():
            logger.warning("Update check is already running")
            return

        logger.info("Starting update check...")
        generation = self._advance_worker_generation("update_check")

        self.update_check_thread = QThread()
        self.update_check_worker = UpdateCheckWorker(current_version)
        self.update_check_worker.moveToThread(self.update_check_thread)

        # Connect signals
        self.update_check_worker.update_check_finished.connect(
            lambda available, info, error: self._emit_if_current(
                "update_check", generation, self.update_check_finished, available, info, error
            )
        )
        self.update_check_worker.update_check_finished.connect(
            self.update_check_thread.quit
        )
        self.update_check_thread.finished.connect(self._cleanup_update_check_worker)

        # Connect start signal
        self.update_check_thread.started.connect(
            self.update_check_worker.check_for_updates
        )

        # Start the thread
        self.update_check_thread.start()

    def _cleanup_update_check_worker(self):
        """Clean up the update check worker and thread."""
        self._cleanup_worker_refs(
            "update_check_thread", "update_check_worker", "Update check"
        )

    def is_update_check_running(self) -> bool:
        return self.update_check_thread is not None

    def stop_update_check(self) -> None:
        """Stop an in-flight update check during application shutdown."""

        self._advance_worker_generation("update_check")
        self._stop_worker("update_check_thread", "update_check_worker")

    def is_any_worker_running(self) -> bool:
        return (
            self.is_file_scanner_running()
            or self.is_similarity_worker_running()
            or self.is_cull_grouping_running()
            or self.is_rating_loader_running()
            or self.is_model_environment_probe_running()
            or self.is_update_check_running()
            or self.is_rating_writer_running()
            or self.is_rotation_application_running()
            or self.is_thumbnail_preload_running()
            or self.is_grouping_preview_running()
            or self.is_grouping_workflow_running()
            or self.is_file_deletion_running()
            or self.is_ai_rating_running()
            or self.is_pick_best_running()
            or self.is_easy_delete_running()
            or self.is_fix_rotation_running()
        )

    def is_any_worker_active(self) -> bool:
        """Whether any worker slot still owns a thread awaiting final cleanup."""

        return any(
            getattr(self, thread_attribute) is not None
            for thread_attribute, _worker_attribute in _WORKER_SLOTS
        )

    def is_resource_intensive_analysis_running(self) -> bool:
        """Whether low-priority thumbnail warming should yield compute resources."""
        return (
            self.is_similarity_worker_running()
            or self.is_cull_grouping_running()
            or self.is_rotation_application_running()
            or self.is_ai_rating_running()
            or self.is_pick_best_running()
            or self.is_easy_delete_running()
            or self.is_fix_rotation_running()
        )

    # --- Rating Writer Management ---
    def start_rating_writer(
        self,
        rating_operations: list,
        rating_disk_cache: RatingCache | None = None,
        exif_disk_cache: ExifCache | None = None,
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
        self.rating_writer_worker.finished.connect(self.rating_writer_thread.quit)
        self.rating_writer_thread.finished.connect(self._cleanup_rating_writer_worker)

        # Connect start signal
        self.rating_writer_thread.started.connect(
            lambda: self.rating_writer_worker.write_ratings(rating_operations)
        )

        # Start the thread
        self.rating_writer_thread.start()

    def _cleanup_rating_writer_worker(self):
        """Clean up the rating writer worker and thread."""
        self._cleanup_worker_refs(
            "rating_writer_thread", "rating_writer_worker", "Rating writer"
        )

    def is_rating_writer_running(self) -> bool:
        return self.rating_writer_thread is not None

    def stop_rating_writer(self):
        """Stop the rating writer thread."""
        self._stop_worker("rating_writer_thread", "rating_writer_worker")

    # --- Rotation Application Management ---
    def start_rotation_application(
        self,
        approved_rotations: dict[str, int],
        exif_disk_cache: ExifCache | None = None,
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
            self.rotation_application_thread.quit
        )
        self.rotation_application_thread.finished.connect(
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
        self._cleanup_worker_refs(
            "rotation_application_thread",
            "rotation_application_worker",
            "Rotation application",
        )

    def is_rotation_application_running(self) -> bool:
        return self.rotation_application_thread is not None

    def stop_rotation_application(self):
        """Stop the rotation application thread."""
        self._stop_worker("rotation_application_thread", "rotation_application_worker")

    # --- Thumbnail Preload Management ---
    def start_thumbnail_session(
        self,
        session_id: str,
        image_paths: list[str],
        foreground_paths: list[str] | None = None,
        *,
        prepare_folder_working_set: bool = False,
    ) -> bool:
        """Start one prioritized thumbnail session for the active folder."""
        from workers.thumbnail_preload_worker import ThumbnailPreloadWorker

        if self.thumbnail_preload_thread is not None:
            return False

        self.thumbnail_preload_thread = QThread()
        self.thumbnail_preload_worker = ThumbnailPreloadWorker(
            image_pipeline=self.image_pipeline,
            session_id=session_id,
            all_paths=image_paths,
            foreground_paths=foreground_paths or [],
            should_pause_background=self.is_resource_intensive_analysis_running,
            materialize_background=True,
            prepare_folder_working_set=prepare_folder_working_set,
        )
        self.thumbnail_preload_worker.moveToThread(self.thumbnail_preload_thread)
        self.thumbnail_preload_worker.session_batch_ready.connect(
            self.thumbnail_session_batch_ready.emit
        )
        self.thumbnail_preload_worker.session_progress.connect(
            self.thumbnail_session_progress.emit
        )
        self.thumbnail_preload_worker.session_finished.connect(
            self.thumbnail_session_finished.emit
        )
        self.thumbnail_preload_worker.session_error.connect(
            self.thumbnail_session_error.emit
        )
        self.thumbnail_preload_worker.session_capacity_required.connect(
            self.thumbnail_session_capacity_required.emit
        )
        self.thumbnail_preload_worker.session_metrics.connect(
            self.thumbnail_session_metrics.emit
        )
        self.thumbnail_preload_worker.session_finished.connect(
            self.thumbnail_preload_thread.quit
        )
        self.thumbnail_preload_worker.session_metrics.connect(
            lambda _session_id, _metrics: self.thumbnail_preload_thread.quit()
        )
        self.thumbnail_preload_thread.finished.connect(
            self._cleanup_thumbnail_preload_worker
        )
        self.thumbnail_preload_thread.started.connect(
            self.thumbnail_preload_worker.run_session
        )
        self.thumbnail_preload_thread.start()
        return True

    def resolve_thumbnail_capacity_request(
        self, session_id: str, approved: bool
    ) -> bool:
        worker = self.thumbnail_preload_worker
        if worker is None or worker.session_id != session_id:
            return False
        worker.resolve_capacity_request(approved)
        return True

    def prioritize_thumbnail_paths(
        self, session_id: str, image_paths: list[str]
    ) -> bool:
        worker = self.thumbnail_preload_worker
        if (
            worker is None
            or not self.is_thumbnail_preload_running()
            or worker.session_id != session_id
        ):
            return False
        worker.prioritize(image_paths)
        return True

    def _cleanup_thumbnail_preload_worker(self, *_args):
        """Clean up the thumbnail preload worker and thread."""
        self._cleanup_worker_refs(
            "thumbnail_preload_thread",
            "thumbnail_preload_worker",
            "Thumbnail preload",
        )

    def is_thumbnail_preload_running(self) -> bool:
        return self.thumbnail_preload_thread is not None

    def stop_thumbnail_preload(self):
        """Stop the thumbnail preload thread."""
        self._stop_worker("thumbnail_preload_thread", "thumbnail_preload_worker")

    def request_stop_thumbnail_preload(self) -> None:
        """Cancel thumbnail warming without blocking the UI thread."""

        self._request_worker_stop(
            "thumbnail_preload_thread", "thumbnail_preload_worker"
        )

    def _cleanup_ai_rating_worker(self):
        self._cleanup_worker_refs("ai_rating_thread", "ai_rating_worker", "AI rating")

    def start_pick_best_analysis(
        self,
        cluster_map: dict[int, list[str]],
        *,
        allow_model_download: bool = False,
    ) -> None:
        """Start the pick-best scoring worker."""
        from workers.pick_best_worker import PickBestWorker

        self.stop_pick_best_analysis()
        generation = self._advance_worker_generation("pick_best")
        if not cluster_map:
            self.pick_best_complete.emit({})
            return

        self.pick_best_thread = QThread()
        self.pick_best_worker = PickBestWorker(
            cluster_map=cluster_map,
            image_pipeline=self.image_pipeline,
            allow_model_download=allow_model_download,
        )
        self.pick_best_worker.moveToThread(self.pick_best_thread)

        self.pick_best_worker.progress_update.connect(
            lambda percent, message: self._emit_if_current(
                "pick_best", generation, self.pick_best_progress, percent, message
            )
        )
        self.pick_best_worker.completed.connect(
            lambda results: self._emit_if_current(
                "pick_best", generation, self.pick_best_complete, results
            )
        )
        self.pick_best_worker.error.connect(
            lambda message: self._emit_if_current(
                "pick_best", generation, self.pick_best_error, message
            )
        )
        self.pick_best_worker.finished.connect(self.pick_best_thread.quit)
        self.pick_best_worker.finished.connect(self.pick_best_worker.deleteLater)
        self.pick_best_thread.finished.connect(self._cleanup_pick_best_worker)
        self.pick_best_thread.started.connect(self.pick_best_worker.run)

        self.pick_best_thread.start()
        logger.info("Pick best analysis thread started.")

    def stop_pick_best_analysis(self) -> None:
        self._advance_worker_generation("pick_best")
        self._stop_worker("pick_best_thread", "pick_best_worker")

    def request_stop_pick_best_analysis(self) -> None:
        self._advance_worker_generation("pick_best")
        self._request_worker_stop("pick_best_thread", "pick_best_worker")

    def _cleanup_pick_best_worker(self) -> None:
        self._cleanup_worker_refs(
            "pick_best_thread", "pick_best_worker", "Pick best analysis"
        )

    def start_easy_delete_analysis(
        self,
        image_paths: list[str],
        cluster_map: dict[int, list[str]] | None = None,
        embeddings_cache: dict | None = None,
        exif_disk_cache=None,
        *,
        analysis_cache=None,
        folder_path: str | None = None,
        fingerprints: dict[str, tuple[int, int]] | None = None,
    ) -> None:
        from workers.easy_delete_worker import EasyDeleteWorker

        self.stop_easy_delete_analysis()
        generation = self._advance_worker_generation("easy_delete")
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
            analysis_cache=analysis_cache,
            folder_path=folder_path,
            fingerprints=fingerprints,
        )
        self.easy_delete_worker.moveToThread(self.easy_delete_thread)

        self.easy_delete_worker.progress_update.connect(
            lambda percent, message: self._emit_if_current(
                "easy_delete",
                generation,
                self.easy_delete_progress,
                percent,
                message,
            )
        )
        self.easy_delete_worker.assessments_ready.connect(
            lambda assessments: self._emit_if_current(
                "easy_delete",
                generation,
                self.easy_delete_assessments_ready,
                assessments,
            )
        )
        self.easy_delete_worker.completed.connect(
            lambda results: self._emit_if_current(
                "easy_delete", generation, self.easy_delete_complete, results
            )
        )
        self.easy_delete_worker.error.connect(
            lambda message: self._emit_if_current(
                "easy_delete", generation, self.easy_delete_error, message
            )
        )
        self.easy_delete_worker.finished.connect(self.easy_delete_thread.quit)
        self.easy_delete_worker.finished.connect(self.easy_delete_worker.deleteLater)
        self.easy_delete_thread.finished.connect(self._cleanup_easy_delete_worker)
        self.easy_delete_thread.started.connect(self.easy_delete_worker.run)

        self.easy_delete_thread.start()
        logger.info("Easy delete analysis thread started.")

    def stop_easy_delete_analysis(self) -> None:
        self._advance_worker_generation("easy_delete")
        self._stop_worker("easy_delete_thread", "easy_delete_worker")

    def request_stop_easy_delete_analysis(self) -> None:
        self._advance_worker_generation("easy_delete")
        self._request_worker_stop("easy_delete_thread", "easy_delete_worker")

    def is_easy_delete_running(self) -> bool:
        return self.easy_delete_thread is not None

    def _cleanup_easy_delete_worker(self) -> None:
        self._cleanup_worker_refs(
            "easy_delete_thread", "easy_delete_worker", "Easy delete analysis"
        )

    # ------------------------------------------------------------------
    # Fix Rotation Detection
    # ------------------------------------------------------------------

    def start_fix_rotation_detection(self, image_paths: list[str]) -> None:
        from workers.rotation_detection_step_worker import RotationDetectionStepWorker

        self.stop_fix_rotation_detection()
        generation = self._advance_worker_generation("fix_rotation")
        if not image_paths:
            self.fix_rotation_complete.emit({})
            return

        self.fix_rotation_detect_thread = QThread()
        self.fix_rotation_detect_worker = RotationDetectionStepWorker(
            image_paths=image_paths,
            image_pipeline=self.image_pipeline,
        )
        self.fix_rotation_detect_worker.moveToThread(self.fix_rotation_detect_thread)

        self.fix_rotation_detect_worker.progress_update.connect(
            lambda percent, message: self._emit_if_current(
                "fix_rotation",
                generation,
                self.fix_rotation_progress,
                percent,
                message,
            )
        )
        self.fix_rotation_detect_worker.completed.connect(
            lambda results: self._emit_if_current(
                "fix_rotation", generation, self.fix_rotation_complete, results
            )
        )
        self.fix_rotation_detect_worker.model_not_found.connect(
            lambda message: self._emit_if_current(
                "fix_rotation",
                generation,
                self.fix_rotation_model_not_found,
                message,
            )
        )
        self.fix_rotation_detect_worker.error.connect(
            lambda message: self._emit_if_current(
                "fix_rotation", generation, self.fix_rotation_error, message
            )
        )
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
        self._advance_worker_generation("fix_rotation")
        self._stop_worker("fix_rotation_detect_thread", "fix_rotation_detect_worker")

    def request_stop_fix_rotation_detection(self) -> None:
        self._advance_worker_generation("fix_rotation")
        self._request_worker_stop(
            "fix_rotation_detect_thread", "fix_rotation_detect_worker"
        )

    def is_fix_rotation_running(self) -> bool:
        return self.fix_rotation_detect_thread is not None

    def _cleanup_fix_rotation_detect_worker(self) -> None:
        self._cleanup_worker_refs(
            "fix_rotation_detect_thread",
            "fix_rotation_detect_worker",
            "Fix rotation detection",
        )

    def start_ai_rating(
        self,
        image_paths: list[str],
    ) -> None:
        """Start AI-driven rating for the provided images."""
        from workers.ai_rating_worker import AiRatingWorker

        self.stop_ai_rating()
        generation = self._advance_worker_generation("ai_rating")
        if not image_paths:
            self.ai_rating_complete.emit({})
            return

        self.ai_rating_thread = QThread()
        self.ai_rating_worker = AiRatingWorker(
            image_paths=image_paths,
            image_pipeline=self.image_pipeline,
        )
        self.ai_rating_worker.moveToThread(self.ai_rating_thread)

        self.ai_rating_worker.progress_update.connect(
            lambda percent, message: self._emit_if_current(
                "ai_rating", generation, self.ai_rating_progress, percent, message
            )
        )
        self.ai_rating_worker.completed.connect(
            lambda results: self._emit_if_current("ai_rating", generation, self.ai_rating_complete, results)
        )
        self.ai_rating_worker.error.connect(
            lambda message: self._emit_if_current("ai_rating", generation, self.ai_rating_error, message)
        )
        self.ai_rating_worker.warning.connect(
            lambda message: self._emit_if_current("ai_rating", generation, self.ai_rating_warning, message)
        )
        self.ai_rating_worker.finished.connect(self.ai_rating_thread.quit)
        self.ai_rating_worker.finished.connect(self.ai_rating_worker.deleteLater)
        self.ai_rating_thread.finished.connect(self._cleanup_ai_rating_worker)
        self.ai_rating_thread.started.connect(self.ai_rating_worker.run)

        self.ai_rating_thread.start()
        logger.info("AI rating thread started.")

    def stop_ai_rating(self) -> None:
        self._advance_worker_generation("ai_rating")
        self._stop_worker("ai_rating_thread", "ai_rating_worker")
