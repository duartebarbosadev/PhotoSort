import os
import re
import time
import logging
import math
import shutil
from datetime import datetime as datetime_obj
from typing import Any

from PyQt6.QtCore import QObject, QTimer
from core.best_photo_finder.payloads import PickBestResults
from core.app_settings import (
    DBSCAN_MIN_SAMPLES,
    add_recent_folder,
    get_similarity_clustering_eps,
    get_cull_grouping_strictness,
    get_companion_files_preference,
    set_preview_cache_size_gb,
)
from core.similarity_embedding_model import SimilarityEmbeddingModel
from core.similarity_cache import (
    SimilarityClusteringResult,
    build_similarity_signature,
    normalize_fingerprints,
    normalize_cluster_results,
)
from core.media_utils import is_image_extension
from core.subject_grouping import CullClusteringResult
from core.model_provisioning import AESTHETIC_MODEL, EMBEDDING_MODEL, MODEL_REGISTRY
from ui.controllers.model_prerequisites import (
    DeferredModelStarts,
    ModelConsentState,
    PrerequisiteDecline,
    PrerequisiteOutcome,
    confirm_model_prerequisites,
)
from core.image_file_ops import ImageFileOperations
from core.grouping import GroupingMode, build_grouping_output_root

logger = logging.getLogger(__name__)

# Grouping modes whose plan is produced by the DINO embedding model, so they
# need the same download/acceleration consent as Similarity, Cull and Pick Best.
MODEL_BACKED_GROUPING_MODES = {GroupingMode.SIMILARITY.value, GroupingMode.MIXED.value}


def _grouping_mode_needs_model(mode: str | None) -> bool:
    return str(mode or "") in MODEL_BACKED_GROUPING_MODES


ROTATION_LOADING_OVERLAY_DELAY_MS = 2000


def _workflow_is_cancelled(controller: object, workflow: str) -> bool:
    return workflow in getattr(controller, "_cancelled_workflows", set())


def _reactivate_workflow(controller: object, workflow: str) -> None:
    getattr(controller, "_cancelled_workflows", set()).discard(workflow)


# Forward declarations for type hinting to avoid circular imports.
class MainWindow:
    pass


class AppState:
    pass


class WorkerManager:
    pass


class AppController(QObject):
    def _supports_grouping_workflow_ui(self) -> bool:
        return all(
            hasattr(self.main_window, attr)
            for attr in (
                "show_grouping_step",
                "show_cull_step",
                "grouping_step_widget",
                "update_grouping_preview",
            )
        ) and hasattr(self.worker_manager, "start_grouping_preview")

    @staticmethod
    def clear_application_caches():
        """Clears all disk-backed caches without instantiating heavy pipelines."""
        start_time = time.perf_counter()
        logger.info("Clearing all application caches.")

        # Import lazily to avoid cost outside of this maintenance task
        from core.caching.thumbnail_cache import ThumbnailCache
        from core.caching.preview_cache import PreviewCache
        from core.caching.exif_cache import ExifCache
        from core.caching.rating_cache import RatingCache
        from core.caching.analysis_cache import AnalysisCache

        cache_classes = (
            ("thumbnail", ThumbnailCache),
            ("preview", PreviewCache),
            ("EXIF", ExifCache),
            ("rating", RatingCache),
        )

        for cache_name, cache_cls in cache_classes:
            cache_instance = None
            try:
                cache_instance = cache_cls()
                cache_instance.clear()
            except Exception:
                logger.error(
                    f"Error clearing {cache_name} cache.",
                    exc_info=True,
                )
            finally:
                if cache_instance is not None:
                    try:
                        cache_instance.close()
                    except Exception:
                        logger.error(
                            f"Error closing {cache_name} cache after clearing.",
                            exc_info=True,
                        )

        try:
            from core.similarity_engine import SimilarityEngine

            SimilarityEngine.clear_embedding_cache()
        except Exception:
            logger.error("Error clearing similarity cache.", exc_info=True)

        analysis_cache_instance = None
        try:
            analysis_cache_instance = AnalysisCache()
            analysis_cache_instance.clear_all()
        except Exception:
            logger.error("Error clearing analysis cache.", exc_info=True)
        finally:
            if analysis_cache_instance is not None:
                try:
                    analysis_cache_instance.close()
                except Exception:
                    logger.error(
                        "Error closing analysis cache after clearing.",
                        exc_info=True,
                    )

        logger.info(
            f"Application caches cleared in {time.perf_counter() - start_time:.2f}s."
        )

    """
    Manages interactions between the WorkerManager, AppState, and the UI (MainWindow).
    This class handles the logic for loading data, running analyses,
    and responding to worker signals, keeping the MainWindow class cleaner
    and focused on UI presentation.
    """

    def __init__(
        self,
        main_window: MainWindow,
        app_state: AppState,
        worker_manager: WorkerManager,
        parent=None,
    ):
        super().__init__(parent)
        self.main_window = main_window
        self.app_state = app_state
        self.worker_manager = worker_manager
        # Track pending rotations for batch preview regeneration
        self._pending_rotated_paths: list[str] = []
        self._rotation_loading_text = "Applying rotations..."
        self._rotation_loading_overlay_visible = False
        self._rotation_loading_overlay_timer = QTimer(self)
        self._rotation_loading_overlay_timer.setSingleShot(True)
        self._rotation_loading_overlay_timer.timeout.connect(
            self._show_delayed_rotation_loading_overlay
        )
        # Cache volume at preview-preload start, used for per-run diagnostics.
        self._ai_rating_warning_messages: list[str] = []
        self._pick_best_pending_after_subject_grouping: bool = False
        self._pick_best_owns_subject_grouping: bool = False
        self._easy_delete_pending_after_similarity: bool = False
        self._pending_grouping_preview: tuple[object, str] | None = None
        self._pending_grouping_preview_start: (
            tuple[list[dict[str, Any]], str, str | None, int] | None
        ) = None
        self._cancelled_workflows: set[str] = set()
        self._ignore_similarity_results = False
        self._pending_exif_cache_capacity_warning: tuple[int, int, int] | None = None
        self._pending_folder_load: tuple[str, dict[str, bool]] | None = None
        self._pending_folder_load_after_workers: tuple[str, dict[str, bool]] | None = (
            None
        )
        self._folder_asset_session_id: str | None = None
        self._rating_load_complete = False
        self._cull_prerequisites_declined = False
        self._model_consent = ModelConsentState()
        self._cull_grouping_fingerprints: dict[str, tuple[int, int]] | None = None
        # (missing model keys, torch device) resolved once per process off the UI thread.
        self._model_environment: tuple[tuple[str, ...], str] | None = None
        self._consent_prompt_active = False
        # Starts waiting on the probe above, kept together so every reset path
        # clears all of them.
        self._deferred_starts = DeferredModelStarts()

    def is_workflow_analysis_running(self, workflow: str) -> bool:
        if workflow == "organize":
            return bool(
                self.worker_manager.is_grouping_preview_running()
                or self._deferred_starts.is_armed("grouping_preview")
            )
        if workflow == "easy_delete":
            return bool(
                self.worker_manager.is_easy_delete_running()
                or self._easy_delete_pending_after_similarity
            )
        if workflow == "pick_best":
            return bool(
                self.worker_manager.is_pick_best_running()
                or self._pick_best_pending_after_subject_grouping
                or self._deferred_starts.is_armed("pick_best_scoring")
            )
        if workflow == "fix_rotation":
            return self.worker_manager.is_fix_rotation_running()
        return False

    def cancel_workflow_analysis(self, workflow: str) -> None:
        """Cancel only work owned by the departing workflow."""
        self._cancelled_workflows.add(workflow)
        if workflow == "organize":
            self._deferred_starts.disarm("grouping_preview")
            self.worker_manager.request_stop_grouping_preview()
            self._pending_grouping_preview = None
            self._pending_grouping_preview_start = None
        elif workflow == "easy_delete":
            depended_on_similarity = self._easy_delete_pending_after_similarity
            self._easy_delete_pending_after_similarity = False
            self.worker_manager.request_stop_easy_delete_analysis()
            if depended_on_similarity:
                # Also drop a start still waiting on the model environment probe,
                # otherwise the probe callback would silently undo this cancel.
                self._deferred_starts.disarm("similarity")
                self._ignore_similarity_results = True
                self.worker_manager.request_stop_similarity_analysis()
        elif workflow == "pick_best":
            depended_on_grouping = self._pick_best_pending_after_subject_grouping
            owned_grouping = self._pick_best_owns_subject_grouping
            self._pick_best_pending_after_subject_grouping = False
            self._pick_best_owns_subject_grouping = False
            self._deferred_starts.disarm("pick_best_scoring")
            self.worker_manager.request_stop_pick_best_analysis()
            if depended_on_grouping and owned_grouping:
                self._deferred_starts.disarm("cull_grouping")
                self.worker_manager.request_stop_cull_subject_grouping()
                self.main_window.cancel_cull_grouping_progress(
                    "Pick Best same-subject preparation cancelled."
                )
        elif workflow == "fix_rotation":
            self.worker_manager.request_stop_fix_rotation_detection()

    def cancel_pending_background_starts(self) -> None:
        """Discard queued follow-up work before a folder change or shutdown."""
        self._deferred_starts.clear()
        self._pending_grouping_preview_start = None
        self._pick_best_pending_after_subject_grouping = False
        self._pick_best_owns_subject_grouping = False
        self._easy_delete_pending_after_similarity = False

    def _sync_active_image(self, workflow_step: str) -> None:
        controller = getattr(self.main_window, "active_image_controller", None)
        if controller is not None:
            controller.sync_workflow(workflow_step)

    def connect_signals(self):
        """Connects signals from the WorkerManager to the controller's slots."""
        # File Scan Worker
        self.worker_manager.file_scan_found_files.connect(self.handle_files_found)
        self.worker_manager.file_scan_finished.connect(self.handle_scan_finished)
        self.worker_manager.file_scan_error.connect(self.handle_scan_error)
        self.worker_manager.thumbnail_session_progress.connect(
            self.handle_review_asset_progress
        )
        self.worker_manager.thumbnail_session_finished.connect(
            self.handle_review_asset_finished
        )
        self.worker_manager.thumbnail_session_error.connect(
            self.handle_review_asset_error
        )
        self.worker_manager.thumbnail_session_capacity_required.connect(
            self.handle_review_asset_capacity_required
        )

        # Similarity Worker
        self.worker_manager.similarity_progress.connect(self.handle_similarity_progress)
        self.worker_manager.similarity_embeddings_generated.connect(
            self.handle_embeddings_generated
        )
        self.worker_manager.similarity_regional_embeddings_generated.connect(
            self.handle_regional_embeddings_generated
        )
        self.worker_manager.similarity_clustering_complete.connect(
            self.handle_clustering_complete
        )
        self.worker_manager.similarity_error.connect(self.handle_similarity_error)
        self.worker_manager.cull_grouping_progress.connect(
            self.handle_cull_grouping_progress
        )
        self.worker_manager.cull_grouping_complete.connect(
            self.handle_cull_grouping_complete
        )
        self.worker_manager.cull_grouping_error.connect(self.handle_cull_grouping_error)
        self.worker_manager.cull_grouping_finished.connect(
            self._schedule_similarity_resume_after_cull
        )
        self.worker_manager.model_environment_ready.connect(
            self.handle_model_environment_ready
        )

        # Rating Loader Worker
        self.worker_manager.rating_load_progress.connect(
            self.handle_rating_load_progress
        )
        self.worker_manager.rating_load_metadata_batch_loaded.connect(
            self.handle_metadata_batch_loaded
        )
        self.worker_manager.rating_load_finished.connect(
            self.handle_rating_load_finished
        )
        self.worker_manager.rating_load_error.connect(self.handle_rating_load_error)
        self.worker_manager.rating_load_cache_capacity_warning.connect(
            self.handle_exif_cache_capacity_warning
        )

        # Update Check Worker
        self.worker_manager.update_check_finished.connect(
            self.handle_update_check_finished
        )

        # Rating Writer Worker
        self.worker_manager.rating_write_progress.connect(
            self.handle_rating_write_progress
        )
        self.worker_manager.rating_written.connect(self.handle_rating_written)
        self.worker_manager.rating_write_finished.connect(
            self.handle_rating_write_finished
        )
        self.worker_manager.rating_write_error.connect(self.handle_rating_write_error)

        # Rotation Application Worker
        self.worker_manager.rotation_application_progress.connect(
            self.handle_rotation_application_progress
        )
        self.worker_manager.rotation_applied.connect(self.handle_rotation_applied)
        self.worker_manager.rotation_application_finished.connect(
            self.handle_rotation_application_finished
        )
        self.worker_manager.rotation_application_error.connect(
            self.handle_rotation_application_error
        )

        # AI Rating Worker
        self.worker_manager.ai_rating_progress.connect(self.handle_ai_rating_progress)
        self.worker_manager.ai_rating_complete.connect(self.handle_ai_rating_complete)
        self.worker_manager.ai_rating_error.connect(self.handle_ai_rating_error)
        self.worker_manager.ai_rating_warning.connect(self.handle_ai_rating_warning)
        self.worker_manager.grouping_preview_progress.connect(
            self.handle_grouping_preview_progress
        )
        self.worker_manager.grouping_preview_ready.connect(
            self.handle_grouping_preview_ready
        )
        self.worker_manager.grouping_preview_error.connect(
            self.handle_grouping_preview_error
        )
        self.worker_manager.grouping_workflow_progress.connect(
            self.handle_grouping_workflow_progress
        )
        self.worker_manager.grouping_workflow_complete.connect(
            self.handle_grouping_workflow_complete
        )
        self.worker_manager.grouping_workflow_error.connect(
            self.handle_grouping_workflow_error
        )

        # Pick Best Worker
        self.worker_manager.pick_best_progress.connect(self.handle_pick_best_progress)
        self.worker_manager.pick_best_complete.connect(self.handle_pick_best_complete)
        self.worker_manager.pick_best_error.connect(self.handle_pick_best_error)

        # Easy Delete Worker
        self.worker_manager.easy_delete_progress.connect(
            self.handle_easy_delete_progress
        )
        self.worker_manager.easy_delete_complete.connect(
            self.handle_easy_delete_complete
        )
        self.worker_manager.easy_delete_assessments_ready.connect(
            self.handle_easy_delete_assessments_ready
        )
        self.worker_manager.easy_delete_error.connect(self.handle_easy_delete_error)

        # Fix Rotation Worker
        self.worker_manager.fix_rotation_progress.connect(
            self.handle_fix_rotation_progress
        )
        self.worker_manager.fix_rotation_complete.connect(
            self.handle_fix_rotation_complete
        )
        self.worker_manager.fix_rotation_model_not_found.connect(
            self.handle_fix_rotation_model_not_found
        )
        self.worker_manager.fix_rotation_error.connect(self.handle_fix_rotation_error)
        # Rotation application signals are already wired in the cull-step path; reuse them for apply feedback
        self.worker_manager.rotation_application_progress.connect(
            self._on_fix_rotation_apply_progress
        )
        self.worker_manager.rotation_application_finished.connect(
            self._on_fix_rotation_apply_finished
        )

    # --- Public Methods (called from MainWindow) ---

    def load_folder(
        self,
        folder_path: str,
        *,
        skip_grouping_step: bool = False,
        record_as_source: bool = True,
        preserve_deletion_marks: bool = False,
        _interrupt_confirmed: bool = False,
    ):
        load_folder_start_time = time.perf_counter()
        logger.info("Loading folder: %s", folder_path)
        if self.worker_manager.is_grouping_workflow_running():
            logger.info("Folder load blocked while grouping workflow is still running.")
            self.main_window.statusBar().showMessage(
                "Grouping is still moving files. Wait for it to finish before loading another folder.",
                4000,
            )
            return
        if getattr(self.worker_manager, "is_file_deletion_running", lambda: False)():
            self.main_window.statusBar().showMessage(
                "Files are still moving to Trash. Wait before loading another folder.",
                4000,
            )
            return
        if self.worker_manager.is_rotation_application_running():
            self.main_window.statusBar().showMessage(
                "Rotations are still being written. Wait before loading another folder.",
                4000,
            )
            return
        if self.worker_manager.is_rating_writer_running():
            self.main_window.statusBar().showMessage(
                "Ratings are still being written. Wait before loading another folder.",
                4000,
            )
            return

        if not _interrupt_confirmed and self.worker_manager.is_any_worker_running():
            if not self.main_window.dialog_manager.confirm_interrupt_for_folder_change(
                folder_path
            ):
                logger.info(
                    "Folder change cancelled; current background work retained."
                )
                return
        # Keep approval with deferred requests so resuming after deletion or
        # worker shutdown does not ask the same question again.
        _interrupt_confirmed = True

        marked_files = self.app_state.get_marked_files()
        preserved_marks = set(marked_files) if preserve_deletion_marks else set()
        if marked_files and not preserve_deletion_marks:
            choice = (
                self.main_window.dialog_manager.show_folder_change_confirmation_dialog(
                    marked_files
                )
            )
            if choice == "commit":
                logger.info(
                    "User chose to commit deletions before switching folders (%d files).",
                    len(marked_files),
                )
                self._pending_folder_load = (
                    folder_path,
                    {
                        "skip_grouping_step": skip_grouping_step,
                        "record_as_source": record_as_source,
                        "preserve_deletion_marks": preserve_deletion_marks,
                        "_interrupt_confirmed": _interrupt_confirmed,
                    },
                )
                started = (
                    self.main_window._commit_marked_deletions_without_confirmation()
                )
                if started is False:
                    self._pending_folder_load = None
                return
            elif choice == "ignore":
                logger.info(
                    "User chose to ignore %d marked deletions before switching folders.",
                    len(marked_files),
                )
                self.main_window._clear_all_deletion_marks()
            else:
                logger.info("Folder load cancelled due to pending deletions.")
                self.main_window.statusBar().showMessage("Folder load cancelled.", 3000)
                return

        # Do not change consent, queued starts or folder state until all dialogs
        # have been accepted. Choosing to stay must leave the current run intact.
        self._cull_prerequisites_declined = False
        self._model_consent = ModelConsentState()
        self._cull_grouping_fingerprints = None
        AppController.cancel_pending_background_starts(self)
        self.main_window.show_loading_overlay("Preparing to scan folder...")

        self.worker_manager.request_stop_all_workers()
        is_any_worker_active = getattr(
            self.worker_manager,
            "is_any_worker_active",
            self.worker_manager.is_any_worker_running,
        )
        if is_any_worker_active():
            self._pending_folder_load_after_workers = (
                folder_path,
                {
                    "skip_grouping_step": skip_grouping_step,
                    "record_as_source": record_as_source,
                    "preserve_deletion_marks": preserve_deletion_marks,
                    "_interrupt_confirmed": _interrupt_confirmed,
                },
            )
            self.main_window.update_loading_text(
                "Stopping background work before scanning the new folder…"
            )
            QTimer.singleShot(25, self._finish_folder_load_after_workers)
            return

        add_recent_folder(folder_path)
        self.main_window.menu_manager.update_recent_folders_menu()

        image_pipeline = getattr(self.main_window, "image_pipeline", None)
        if image_pipeline is not None:
            image_pipeline.end_active_review_working_set()
        self.app_state.clear_all_file_specific_data()
        self._folder_asset_session_id = None
        self._rating_load_complete = False
        if preserved_marks:
            self.app_state.marked_for_deletion.update(preserved_marks)
        self.main_window.reset_thumbnail_requests()
        self.main_window.reset_preview_requests()
        self.main_window.hide_exif_progress()
        self.main_window.invalidate_last_displayed_preview()
        self._pending_grouping_preview = None
        self._pending_exif_cache_capacity_warning = None
        self.app_state.current_folder_path = folder_path
        self.app_state.skip_grouping_step_once = skip_grouping_step
        if record_as_source:
            self.app_state.grouping_source_root = folder_path
        if skip_grouping_step:
            self.app_state.grouping_output_root = folder_path
        folder_display_name = (
            os.path.basename(folder_path) if folder_path else "Selected Folder"
        )
        self.main_window._update_image_info_label(
            status_message_override=f"Folder: {folder_display_name} | Preparing scan..."
        )

        # Reset UI elements related to analysis
        self.main_window.cluster_filter_combo.clear()
        self.main_window.cluster_filter_combo.addItems(["All Clusters"])
        self.main_window.cluster_filter_combo.setEnabled(False)
        self.main_window.menu_manager.update_cluster_filter_menu([])
        self.main_window.menu_manager.set_cluster_sort_menu_visible(False)
        self.main_window.cluster_sort_combo.setEnabled(False)
        self.main_window.menu_manager.set_cluster_sort_menu_enabled(False)
        self.main_window.cluster_sort_combo.setCurrentIndex(0)
        self.main_window.menu_manager.group_by_similarity_action.setEnabled(False)
        self.main_window.menu_manager.group_by_similarity_action.setChecked(False)
        self.main_window.refresh_navigation_shortcut_actions()

        self.main_window.file_system_model.clear()
        self.main_window.file_system_model.setColumnCount(1)
        if hasattr(self.main_window, "mark_cull_model_dirty"):
            self.main_window.mark_cull_model_dirty()
        self.main_window.update_grouping_preview("Preparing grouping preview...")
        self.main_window.show_grouping_step()

        self.main_window.update_loading_text(
            f"Scanning folder: {os.path.basename(folder_path)}..."
        )
        self.main_window.menu_manager.open_folder_action.setEnabled(False)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(False)
        self.main_window.menu_manager.ai_rate_images_action.setEnabled(False)

        logger.debug(
            f"Folder prep complete in {time.perf_counter() - load_folder_start_time:.2f}s. Starting file scan."
        )
        self.worker_manager.start_file_scan(folder_path)

    def _finish_folder_load_after_workers(self) -> None:
        """Resume a folder change after cancellable workers have exited."""

        if getattr(self.main_window, "_shutdown_in_progress", False):
            self._pending_folder_load_after_workers = None
            return
        is_any_worker_active = getattr(
            self.worker_manager,
            "is_any_worker_active",
            self.worker_manager.is_any_worker_running,
        )
        if is_any_worker_active():
            QTimer.singleShot(25, self._finish_folder_load_after_workers)
            return
        pending = self._pending_folder_load_after_workers
        self._pending_folder_load_after_workers = None
        if pending is None:
            return
        folder_path, options = pending
        self.load_folder(folder_path, **options)

    def resume_folder_load_after_deletion(self, successful: bool) -> None:
        pending = self._pending_folder_load
        if pending is None:
            return
        if not successful:
            self._pending_folder_load = None
            self.main_window.statusBar().showMessage(
                "Folder change cancelled because some items could not be moved to Trash.",
                5000,
            )
            return
        self._finish_pending_folder_load()

    def _finish_pending_folder_load(self) -> None:
        """Resume a deferred folder load after the deletion thread has exited."""
        if getattr(self.worker_manager, "is_file_deletion_running", lambda: False)():
            QTimer.singleShot(25, self._finish_pending_folder_load)
            return

        pending = self._pending_folder_load
        self._pending_folder_load = None
        if pending is None:
            return
        folder_path, options = pending
        self.load_folder(folder_path, **options)

    def start_active_similarity_grouping(self) -> None:
        """Start the cluster analysis owned by the active workflow.

        Cull and Pick Best use the high-precision same-subject namespace. The
        remaining workflows use the coarse visual-similarity namespace shared by
        Organize and Easy Delete.
        """

        if getattr(self.app_state, "workflow_step", None) in {"cull", "pick_best"}:
            self.start_cull_similarity_workflow()
            return
        self.start_similarity_analysis()

    def start_similarity_analysis(self):
        self._ignore_similarity_results = False
        logger.info("Starting similarity analysis.")
        if self.worker_manager.is_similarity_worker_running():
            self.main_window.statusBar().showMessage(
                "Similarity analysis is already in progress.", 3000
            )
            return

        if not self.app_state.image_files_data:
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage(
                "No images loaded to analyze similarity.", 3000
            )
            return

        paths_for_similarity = self._get_image_paths()
        if not paths_for_similarity:
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage(
                "No valid image paths for similarity analysis.", 3000
            )
            return
        skipped_videos = len(self._get_media_paths()) - len(paths_for_similarity)
        if skipped_videos > 0:
            self.main_window.statusBar().showMessage(
                f"Analyzing images only. Skipping {skipped_videos} video(s).",
                4000,
            )

        if self.worker_manager.is_cull_grouping_running():
            # Both analyses use the shared DINO execution pipeline. Keep the
            # coarse request queued until Cull has released its model worker,
            # rather than silently terminating Cull and orphaning dependants.
            self._deferred_starts.arm("similarity")
            message = "Waiting for same-subject grouping to finish…"
            if self._easy_delete_pending_after_similarity:
                self.main_window.easy_delete_step_widget.show_loading(
                    f"Step 1/2: {message}", -1
                )
            else:
                self.main_window.statusBar().showMessage(message, 3000)
            return

        if self._model_environment is None:
            # Importing torch and resolving model snapshots takes seconds, so the
            # answer is produced by a worker and this start resumes on its signal.
            self._deferred_starts.arm("similarity")
            self.main_window.statusBar().showMessage(
                "Checking the local similarity model…", 3000
            )
            self._start_model_environment_probe()
            return

        outcome = self._confirm_model_prerequisites(
            [EMBEDDING_MODEL.key],
            feature="visual similarity grouping",
        )
        if not outcome.approved:
            if outcome.declined is PrerequisiteDecline.BUSY:
                return
            if outcome.declined is PrerequisiteDecline.DOWNLOAD:
                logger.info("Similarity model download declined by user.")
                message = (
                    "Similarity analysis canceled. Model download was not approved."
                )
            else:
                message = (
                    "Similarity analysis canceled; hardware acceleration "
                    "is unavailable."
                )
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage(message, 5000)
            if self._easy_delete_pending_after_similarity:
                self._easy_delete_pending_after_similarity = False
                self.main_window.easy_delete_step_widget.show_error(message)
            return
        allow_model_download = outcome.allow_download

        if self._easy_delete_pending_after_similarity:
            self.main_window.hide_loading_overlay()
            self.main_window.easy_delete_step_widget.show_loading(
                "Step 1/2: Starting similarity analysis...", 0
            )
        else:
            self.main_window.show_loading_overlay("Starting similarity analysis...")
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(False)
        self.worker_manager.start_similarity_analysis(
            paths_for_similarity,
            allow_model_download=allow_model_download,
            folder_path=getattr(self.app_state, "current_folder_path", None),
            analysis_cache=getattr(self.app_state, "analysis_cache", None),
            fingerprints=self._similarity_fingerprints(paths_for_similarity),
        )

    def _schedule_similarity_resume_after_cull(self) -> None:
        """Defer arbitration until any replacement Cull worker has been installed."""

        QTimer.singleShot(0, self._resume_similarity_after_cull)

    def _resume_similarity_after_cull(self) -> None:
        """Resume the one coarse request queued behind Cull, if it still exists."""

        if not self._deferred_starts.is_armed("similarity"):
            return
        if self.worker_manager.is_cull_grouping_running():
            QTimer.singleShot(25, self._resume_similarity_after_cull)
            return
        if self._deferred_starts.take("similarity") is not None:
            self.start_similarity_analysis()

    def _similarity_fingerprints(
        self, paths: list[str] | None = None
    ) -> dict[str, tuple[int, int]]:
        requested = set(paths or self._get_image_paths())
        supplied: dict[str, tuple[int, int]] = {}
        for item in self._get_image_file_data():
            path = item.get("path")
            size = item.get("file_size")
            mtime_ns = item.get("mtime_ns")
            if (
                path in requested
                and isinstance(size, int)
                and isinstance(mtime_ns, int)
            ):
                supplied[path] = (size, mtime_ns)
        return normalize_fingerprints(list(requested), supplied)

    def _current_similarity_signature(self, paths: list[str]) -> str:
        model = SimilarityEmbeddingModel()
        return build_similarity_signature(
            paths,
            self._similarity_fingerprints(paths),
            model_cache_key=model.cache_key,
            regional_cache_key=model.region_cache_key,
            clustering_eps=get_similarity_clustering_eps(),
            min_samples=DBSCAN_MIN_SAMPLES,
        )

    def refresh_grouping_preview(self):
        _reactivate_workflow(self, "organize")
        self._pending_grouping_preview = None
        if not self.app_state.image_files_data:
            self.main_window.update_grouping_preview("No files loaded for grouping.")
            return
        mode = self.app_state.selected_grouping_mode or "current"
        source_root = (
            self.app_state.grouping_source_root or self.app_state.current_folder_path
        )
        if source_root:
            self.main_window.grouping_step_widget.set_output_root_text(
                "Output root: " + source_root
            )
        allow_model_download = False
        if _grouping_mode_needs_model(mode):
            if self._model_environment is None:
                # Importing torch and resolving model snapshots takes seconds, so
                # the answer comes from a worker and this start resumes on its
                # signal instead of blocking the UI thread.
                self._deferred_starts.arm("grouping_preview")
                self._show_grouping_preview_status(
                    "Checking the local similarity model…", busy=True
                )
                self._start_model_environment_probe()
                return
            outcome = self._confirm_model_prerequisites(
                [EMBEDDING_MODEL.key],
                feature=f"{mode} grouping",
                fallback=(
                    "If you cancel, you can still group by folder, date or location."
                ),
            )
            if not outcome.approved:
                if outcome.declined is PrerequisiteDecline.BUSY:
                    return
                if outcome.declined is PrerequisiteDecline.DOWNLOAD:
                    logger.info("Grouping model download declined by user.")
                    message = (
                        f"{mode.title()} grouping needs the local similarity model. "
                        "Download it to continue."
                    )
                else:
                    message = (
                        f"{mode.title()} grouping was cancelled; hardware "
                        "acceleration is unavailable."
                    )
                self._show_grouping_preview_status(message, busy=False)
                return
            allow_model_download = outcome.allow_download

        self.main_window.update_grouping_preview("Preparing grouping preview...")
        self.main_window.grouping_step_widget.set_loading_state(
            f"Generating {mode} preview...",
            True,
            None,
        )
        request = (
            list(self.app_state.image_files_data),
            mode,
            source_root,
            self.main_window.grouping_step_widget.get_location_depth(),
            allow_model_download,
        )
        is_grouping_preview_active = getattr(
            self.worker_manager,
            "is_grouping_preview_active",
            self.worker_manager.is_grouping_preview_running,
        )
        if is_grouping_preview_active():
            self._pending_grouping_preview_start = request
            self.worker_manager.request_stop_grouping_preview()
            QTimer.singleShot(25, self._start_pending_grouping_preview)
            return
        self._pending_grouping_preview_start = None
        self.worker_manager.start_grouping_preview(
            request[0],
            request[1],
            request[2],
            location_depth=request[3],
            analysis_cache=getattr(self.app_state, "analysis_cache", None),
            folder_path=getattr(self.app_state, "current_folder_path", None),
            allow_model_download=request[4],
        )

    def _show_grouping_preview_status(self, message: str, *, busy: bool) -> None:
        """Mirror a preview status message in both Organize surfaces."""

        self.main_window.update_grouping_preview(message)
        self.main_window.grouping_step_widget.set_loading_state(message, busy, None)

    def _start_pending_grouping_preview(self) -> None:
        """Start the latest preview request after the replaced worker exits."""

        if self._pending_grouping_preview_start is None:
            return
        is_grouping_preview_active = getattr(
            self.worker_manager,
            "is_grouping_preview_active",
            self.worker_manager.is_grouping_preview_running,
        )
        if is_grouping_preview_active():
            QTimer.singleShot(25, self._start_pending_grouping_preview)
            return
        request = self._pending_grouping_preview_start
        self._pending_grouping_preview_start = None
        if request is None or _workflow_is_cancelled(self, "organize"):
            return
        self.worker_manager.start_grouping_preview(
            request[0],
            request[1],
            request[2],
            location_depth=request[3],
            analysis_cache=getattr(self.app_state, "analysis_cache", None),
            folder_path=getattr(self.app_state, "current_folder_path", None),
            allow_model_download=request[4],
        )

    def activate_grouping_preview(self) -> None:
        pending = self._pending_grouping_preview
        self._pending_grouping_preview = None
        if pending is None:
            self.refresh_grouping_preview()
            return
        plan, output_root = pending
        self.main_window.grouping_step_widget.set_preview_plan(plan, output_root)
        self.main_window.grouping_step_widget.set_loading_state("", False)
        self.main_window.notify_thumbnail_items_rebuilt()
        AppController._sync_active_image(self, "organize")

    def start_grouping_workflow(
        self,
        mode: str,
        group_name_overrides: dict[str, str] | None = None,
        prepared_plan=None,
    ):
        if self.worker_manager.is_grouping_workflow_running():
            self.main_window.statusBar().showMessage(
                "Grouping is already running.", 3000
            )
            return
        source_root = (
            self.app_state.grouping_source_root or self.app_state.current_folder_path
        )
        if not source_root:
            self.main_window.statusBar().showMessage(
                "No source folder available for grouping.", 3000
            )
            return
        output_root = build_grouping_output_root(source_root, mode)
        allow_model_download = False
        if prepared_plan is None and _grouping_mode_needs_model(mode):
            if self._model_environment is None:
                self._deferred_starts.arm(
                    "grouping_workflow",
                    (mode, group_name_overrides, prepared_plan),
                )
                self.main_window.grouping_step_widget.set_loading_state(
                    "Checking the local similarity model…", True, None
                )
                self._start_model_environment_probe()
                return
            outcome = self._confirm_model_prerequisites(
                [EMBEDDING_MODEL.key],
                feature=f"{mode} grouping",
                fallback=(
                    "If you cancel, you can still group by folder, date or location."
                ),
            )
            if not outcome.approved:
                if outcome.declined is PrerequisiteDecline.BUSY:
                    return
                message = (
                    f"{mode.title()} grouping needs the local similarity model. "
                    "Download it to continue."
                    if outcome.declined is PrerequisiteDecline.DOWNLOAD
                    else (
                        f"{mode.title()} grouping was cancelled; hardware "
                        "acceleration is unavailable."
                    )
                )
                self._show_grouping_preview_status(message, busy=False)
                self.main_window.statusBar().showMessage(message, 6000)
                return
            allow_model_download = outcome.allow_download
        self.app_state.selected_grouping_mode = mode
        self.app_state.grouping_output_root = output_root
        self.main_window.set_grouping_busy(True)
        self.main_window.grouping_step_widget.set_loading_state(
            f"Creating {mode} groups...",
            True,
            0,
        )
        self.main_window.show_loading_overlay("Grouping photos...")
        self.worker_manager.start_grouping_workflow(
            list(self.app_state.image_files_data),
            mode,
            source_root,
            output_root,
            group_name_overrides=group_name_overrides,
            prepared_plan=prepared_plan,
            location_depth=self.main_window.grouping_step_widget.get_location_depth(),
            move_companions=get_companion_files_preference() == "always",
            rating_cache=self.app_state.rating_disk_cache,
            exif_cache=self.app_state.exif_disk_cache,
            analysis_cache=self.app_state.analysis_cache,
            allow_model_download=allow_model_download,
        )

    def _build_cluster_path_map(self) -> dict[int, list[str]]:
        valid_image_paths = set(self._get_image_paths())
        cluster_map: dict[int, list[str]] = {}
        for path, cluster_id in self.app_state.cluster_results.items():
            if cluster_id is None or path not in valid_image_paths:
                continue
            cluster_map.setdefault(cluster_id, []).append(path)
        return cluster_map

    def _build_pick_best_cluster_map(self) -> dict[int, list[str]]:
        valid_paths = set(self._get_image_paths())
        marked = self.app_state.marked_for_deletion
        cluster_map: dict[int, list[str]] = {}
        for path, cluster_id in self.app_state.cull_cluster_results.items():
            if cluster_id is None or path not in valid_paths or path in marked:
                continue
            cluster_map.setdefault(cluster_id, []).append(path)
        return cluster_map

    def _restore_analysis_state(self):
        folder_path = self.app_state.current_folder_path
        if not folder_path:
            return
        available_paths = {
            item.get("path") for item in self._get_image_file_data() if item.get("path")
        }
        image_paths = self._get_image_paths()
        valid_clusters = self.app_state.analysis_cache.load_valid_cluster_results(
            folder_path,
            signature=self._current_similarity_signature(image_paths),
            expected_paths=set(image_paths),
        )
        if valid_clusters is None:
            return

        def _parse_cluster_key(key) -> int | None:
            if isinstance(key, int):
                return key
            if isinstance(key, str):
                stripped = key.strip()
                match = re.match(r"(-?\d+)", stripped)
                if match:
                    try:
                        return int(match.group(1))
                    except ValueError:
                        return None
            return None

        restored_anything = False

        saved_clusters = valid_clusters
        if isinstance(saved_clusters, dict):
            filtered_clusters = {
                path: _parse_cluster_key(cluster_id)
                for path, cluster_id in saved_clusters.items()
                if path in available_paths
                and _parse_cluster_key(cluster_id) is not None
            }
            if filtered_clusters:
                self.app_state.cluster_results = {
                    path: int(cluster_id)
                    for path, cluster_id in filtered_clusters.items()
                }
                cluster_ids = sorted(set(filtered_clusters.values()))
                self.main_window.populate_cluster_filter(cluster_ids)
                self.main_window.menu_manager.group_by_similarity_action.setEnabled(
                    True
                )
                if cluster_ids:
                    self.main_window.menu_manager.set_cluster_sort_menu_visible(True)
                    self.main_window.menu_manager.set_cluster_sort_menu_enabled(True)
                    self.main_window.cluster_sort_combo.setEnabled(True)
                restored_anything = True
        clustered_paths = set(self.app_state.cluster_results.keys())
        missing_paths = available_paths - clustered_paths

        if restored_anything:
            self.main_window.statusBar().showMessage(
                "Restored previous analysis results.", 4000
            )

        if missing_paths:
            self.main_window.menu_manager.analyze_similarity_action.setEnabled(True)
            self.main_window.menu_manager.group_by_similarity_action.setChecked(False)
            self.main_window.menu_manager.group_by_similarity_action.setEnabled(True)
            self.main_window.statusBar().showMessage(
                "New images detected. Run Analyze Similarity to include them.", 5000
            )

    def start_ai_rating_all(self):
        """Kick off AI-driven rating for every loaded image."""
        logger.info("Starting AI rating for all images.")

        if self.worker_manager.is_ai_rating_running():
            self.main_window.statusBar().showMessage(
                "AI rating is already running.", 3000
            )
            return
        if self.worker_manager.is_rating_loader_running():
            self.main_window.statusBar().showMessage(
                "Metadata is still loading. AI rating will be available when it finishes.",
                4000,
            )
            return

        if not self.app_state.image_files_data:
            self.main_window.statusBar().showMessage("No images loaded to rate.", 3000)
            return

        image_paths = self._get_image_paths()
        if not image_paths:
            self.main_window.statusBar().showMessage(
                "No valid image paths available for AI rating.", 3000
            )
            return
        skipped_videos = len(self._get_media_paths()) - len(image_paths)
        if skipped_videos > 0:
            self.main_window.statusBar().showMessage(
                f"AI rating is image-only. Skipping {skipped_videos} video(s).",
                4000,
            )

        image_paths_to_rate, already_rated_count = self._partition_unrated_images(
            image_paths
        )
        if not image_paths_to_rate:
            self.main_window.statusBar().showMessage(
                "All images already have ratings.", 4000
            )
            return

        self.main_window.show_loading_overlay("Requesting AI ratings...")
        self.main_window.menu_manager.ai_rate_images_action.setEnabled(False)
        status_message = f"AI rating started for {len(image_paths_to_rate)} image(s)..."
        if already_rated_count:
            status_message += f" ({already_rated_count} already-rated image(s) skipped)"
        self.main_window.statusBar().showMessage(status_message, 4000)

        self._ai_rating_warning_messages = []

        self.worker_manager.start_ai_rating(image_paths=image_paths_to_rate)

    def reload_current_folder(self):
        if self.app_state.image_files_data:
            if (
                self.app_state.image_files_data[0]
                and "path" in self.app_state.image_files_data[0]
            ):
                current_dir = os.path.dirname(
                    self.app_state.image_files_data[0]["path"]
                )
                if os.path.isdir(current_dir):
                    self.load_folder(current_dir)
                    return
        self.main_window.statusBar().showMessage("No folder context to reload.", 3000)

    def rename_image(self, old_path: str, new_path: str):
        """Renames an image file."""
        success, message = ImageFileOperations.rename_image(old_path, new_path)
        if not success:
            self.main_window.statusBar().showMessage(message, 5000)

    # --- Private Helper Methods ---

    def _get_image_file_data(
        self, file_data_list: list[dict[str, Any]] | None = None
    ) -> list[dict[str, Any]]:
        source = file_data_list if file_data_list is not None else []
        if file_data_list is None:
            source = getattr(self.app_state, "image_files_data", []) or []
        return [
            fd
            for fd in source
            if isinstance(fd, dict) and fd.get("media_type", "image") == "image"
        ]

    def _get_media_file_data(
        self, file_data_list: list[dict[str, Any]] | None = None
    ) -> list[dict[str, Any]]:
        source = file_data_list if file_data_list is not None else []
        if file_data_list is None:
            source = getattr(self.app_state, "image_files_data", []) or []
        return [fd for fd in source if isinstance(fd, dict) and fd.get("path")]

    def _get_image_paths(
        self, file_data_list: list[dict[str, Any]] | None = None
    ) -> list[str]:
        return [
            fd.get("path")
            for fd in self._get_image_file_data(file_data_list)
            if fd.get("path")
        ]

    def _get_media_paths(
        self, file_data_list: list[dict[str, Any]] | None = None
    ) -> list[str]:
        return [
            fd.get("path")
            for fd in self._get_media_file_data(file_data_list)
            if fd.get("path")
        ]

    def _filter_image_paths(self, paths: list[str]) -> list[str]:
        return [path for path in paths if path and is_image_extension(path)]

    def _partition_unrated_images(
        self, image_paths: list[str]
    ) -> tuple[list[str], int]:
        """Partition from the worker-populated memory cache only."""

        unrated: list[str] = []
        already_rated_count = 0
        for path in image_paths:
            existing_rating = self.app_state.rating_cache.get(os.path.normpath(path), 0)
            if existing_rating is not None and existing_rating != 0:
                already_rated_count += 1
                continue
            unrated.append(path)
        return unrated, already_rated_count

    # --- Slots for WorkerManager Signals ---

    def handle_files_found(self, batch_of_file_data: list[dict[str, Any]]):
        self.app_state.extend_file_data(batch_of_file_data)
        self.main_window.update_loading_text(
            f"Scanning... {len(self.app_state.image_files_data)} files found"
        )
        self.main_window._update_image_info_label()

    def handle_scan_finished(self):
        self.main_window.update_loading_text(
            "Scan finished. Preparing review images..."
        )
        self.main_window.menu_manager.open_folder_action.setEnabled(False)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(False)
        self.main_window.menu_manager.group_by_similarity_action.setEnabled(False)
        self.main_window.menu_manager.ai_rate_images_action.setEnabled(False)

        media_file_data = self._get_media_file_data()
        if media_file_data:
            self.worker_manager.start_rating_load(
                media_file_data.copy(),
                self.app_state.rating_disk_cache,
                self.app_state,
            )
            paths = [item["path"] for item in media_file_data if item.get("path")]
            session_id = self.main_window.start_thumbnail_warming(paths)
            self._folder_asset_session_id = str(session_id) if session_id else None
            if self._folder_asset_session_id is not None:
                return

            logger.error("Could not start review-asset preparation")
            self._activate_loaded_folder(asset_failures=len(paths))
        else:
            self._activate_loaded_folder(asset_failures=0)

    def _cancel_folder_for_review_capacity(self) -> None:
        self.worker_manager.request_stop_rating_load()
        self._folder_asset_session_id = None
        self._rating_load_complete = False
        self._pending_exif_cache_capacity_warning = None
        self.main_window.hide_exif_progress()
        pipeline = getattr(self.main_window, "image_pipeline", None)
        if pipeline is not None:
            pipeline.end_active_review_working_set()
        self.app_state.clear_all_file_specific_data()
        self.app_state.current_folder_path = None
        self.main_window.reset_thumbnail_requests()
        self.main_window.hide_loading_overlay()
        self.main_window.menu_manager.open_folder_action.setEnabled(True)
        self.main_window.statusBar().showMessage(
            "Folder load canceled because its review images do not fit in the approved cache.",
            7000,
        )

    def _activate_loaded_folder(self, *, asset_failures: int) -> None:
        """Expose workflows only after every review asset has been attempted."""
        has_images = bool(self._get_image_file_data())
        self.main_window.menu_manager.open_folder_action.setEnabled(True)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(has_images)
        self.main_window.menu_manager.group_by_similarity_action.setEnabled(has_images)
        self.main_window.menu_manager.ai_rate_images_action.setEnabled(
            has_images and self._rating_load_complete
        )

        self._restore_analysis_state()
        if self._supports_grouping_workflow_ui():
            if hasattr(self.main_window, "mark_cull_model_dirty"):
                self.main_window.mark_cull_model_dirty()
            skip_grouping_step_once = getattr(
                self.app_state, "skip_grouping_step_once", False
            )
            if skip_grouping_step_once:
                self.app_state.skip_grouping_step_once = False
                self.main_window.show_cull_step()
            else:
                self.main_window.show_grouping_step()
                self.refresh_grouping_preview()
        else:
            self.main_window._rebuild_model_view()

        self.main_window.hide_loading_overlay()
        if asset_failures:
            self.main_window.statusBar().showMessage(
                f"Review preparation finished with {asset_failures} file(s) that could not be displayed.",
                7000,
            )

        self.main_window._update_image_info_label()
        resume_transition = getattr(
            self.main_window, "resume_workflow_transition_after_reload", None
        )
        if callable(resume_transition):
            resume_transition()

    def handle_review_asset_progress(
        self,
        session_id: str,
        attempted: int,
        total: int,
        failures: int,
        _paused: bool,
    ) -> None:
        if session_id != self._folder_asset_session_id or total <= 0:
            return
        percent = min(100, round((attempted / total) * 100))
        failure_suffix = f" — {failures} failed" if failures else ""
        self.main_window.update_loading_text(
            f"Preparing review images {attempted:,} / {total:,} ({percent}%){failure_suffix}"
        )

    def handle_review_asset_finished(
        self, session_id: str, _attempted: int, failures: int
    ) -> None:
        if session_id != self._folder_asset_session_id:
            return
        self._folder_asset_session_id = None
        self._activate_loaded_folder(asset_failures=failures)

    def handle_review_asset_error(self, session_id: str, message: str) -> None:
        if session_id != self._folder_asset_session_id:
            return
        logger.error("Review-asset preparation error: %s", message)
        self.main_window.update_loading_text(
            "Review preparation encountered an error; finishing available files..."
        )

    def handle_review_asset_capacity_required(
        self, session_id: str, required_bytes: int
    ) -> None:
        """Pause preparation until the user approves enough additional storage."""
        if session_id != self._folder_asset_session_id:
            self.worker_manager.resolve_thumbnail_capacity_request(session_id, False)
            return

        pipeline = self.main_window.image_pipeline
        current_limit = pipeline.preview_cache.size_limit_bytes
        required_bytes = max(int(required_bytes), current_limit + 1)
        cache_dir = pipeline.preview_cache._cache_dir
        try:
            free_bytes = shutil.disk_usage(cache_dir).free
        except OSError:
            free_bytes = 0
        available_bytes = free_bytes + pipeline.preview_cache.volume()
        if required_bytes > available_bytes:
            self.main_window.dialog_manager.show_preview_cache_disk_space_error(
                required_bytes, available_bytes
            )
            self.worker_manager.resolve_thumbnail_capacity_request(session_id, False)
            self._folder_asset_session_id = None
            self._cancel_folder_for_review_capacity()
            return

        approved = (
            self.main_window.dialog_manager.confirm_preview_cache_capacity_increase(
                required_bytes, current_limit
            )
        )
        if not approved:
            self.worker_manager.resolve_thumbnail_capacity_request(session_id, False)
            self._folder_asset_session_id = None
            self._cancel_folder_for_review_capacity()
            return

        approved_gb = math.ceil((required_bytes / (1024**3)) * 4) / 4
        set_preview_cache_size_gb(approved_gb)
        approved_bytes = int(approved_gb * (1024**3))
        pipeline.preview_cache.increase_size_limit(approved_bytes)
        self.worker_manager.resolve_thumbnail_capacity_request(session_id, True)

    def handle_scan_error(self, message: str):
        logger.error(f"File scan error: {message}")
        self.main_window.statusBar().showMessage(f"Scan Error: {message}")
        self.main_window.menu_manager.open_folder_action.setEnabled(True)

        error_folder_display = "N/A"
        if self.app_state.current_folder_path:
            error_folder_display = os.path.basename(self.app_state.current_folder_path)
            if not error_folder_display:
                error_folder_display = self.app_state.current_folder_path
        self.main_window._update_image_info_label(
            status_message_override=f"Folder: {error_folder_display} | Scan error."
        )

        self.main_window.hide_loading_overlay()
        self.main_window.menu_manager.ai_rate_images_action.setEnabled(False)

    def handle_grouping_preview_progress(self, _progress: int, message: str):
        if _workflow_is_cancelled(self, "organize"):
            return
        self.main_window.update_grouping_preview(message)
        self.main_window.grouping_step_widget.set_loading_state(message, True, None)

    def handle_grouping_preview_ready(self, plan):
        if _workflow_is_cancelled(self, "organize"):
            return
        if _grouping_mode_needs_model(getattr(plan, "mode", None)):
            self._mark_models_installed([EMBEDDING_MODEL.key])
        mode_label = str(getattr(plan, "mode", "grouping")).title()
        source_root = (
            self.app_state.grouping_source_root or self.app_state.current_folder_path
        )
        output_root = source_root or ""
        plan_source_root = getattr(plan, "source_root", "") or ""
        if (
            plan_source_root
            and source_root
            and os.path.normcase(os.path.normpath(plan_source_root))
            != os.path.normcase(os.path.normpath(source_root))
        ):
            logger.info(
                "Discarding stale grouping preview for %s; active source is %s",
                plan_source_root,
                source_root,
            )
            return
        self.main_window.update_grouping_preview(
            f"{mode_label}: {len(plan.groups)} folders ready."
        )
        if getattr(self.app_state, "workflow_step", "organize") != "organize":
            logger.info(
                "Grouping preview is ready but Organize is not visible; "
                "deferring tree construction until Organize is opened."
            )
            self._pending_grouping_preview = (plan, output_root)
            return
        self._pending_grouping_preview = None
        self.main_window.grouping_step_widget.set_preview_plan(plan, output_root)
        self.main_window.grouping_step_widget.set_loading_state("", False)
        self.main_window.notify_thumbnail_items_rebuilt()
        AppController._sync_active_image(self, "organize")

    def handle_grouping_preview_error(self, message: str):
        if _workflow_is_cancelled(self, "organize"):
            return
        self.main_window.update_grouping_preview(f"Preview unavailable: {message}")
        self.main_window.grouping_step_widget.set_loading_state(
            f"Preview unavailable: {message}",
            False,
        )

    def handle_grouping_workflow_progress(self, progress: int, message: str):
        self.main_window.update_loading_text(message)
        self.main_window.grouping_step_widget.set_loading_state(
            message,
            True,
            progress,
        )

    def handle_grouping_workflow_complete(self, summary):
        path_updates = {
            entry.original_path: entry.new_path
            for entry in getattr(summary, "entries", [])
            if getattr(entry, "new_path", None)
        }
        try:
            self.app_state.update_paths(
                path_updates,
                migrate_disk_caches=False,
            )
        except Exception:
            logger.debug(
                "Failed to update in-memory path caches after grouping.",
                exc_info=True,
            )
        else:
            sync_workflows = getattr(
                self.main_window,
                "_sync_workflow_results_after_file_mutation",
                None,
            )
            if callable(sync_workflows):
                sync_workflows()
        self.app_state.grouping_run_summary = {
            "mode": summary.mode,
            "output_root": summary.output_root,
            "moved_count": summary.moved_count,
            "deleted_count": getattr(summary, "deleted_count", 0),
            "unassigned_count": summary.unassigned_count,
            "skipped_count": summary.skipped_count,
        }
        self.app_state.grouping_output_root = summary.output_root
        self.main_window.set_grouping_busy(False)
        self.main_window.hide_loading_overlay()
        self.main_window.grouping_step_widget.set_loading_state(
            "Grouping complete", False
        )
        self.main_window.statusBar().showMessage(
            f"Grouping complete in {summary.output_root}.",
            5000,
        )
        self._finalize_grouping_workflow_completion(summary)

    def _finalize_grouping_workflow_completion(self, summary):
        if self.worker_manager.is_grouping_workflow_running():
            QTimer.singleShot(
                25, lambda: self._finalize_grouping_workflow_completion(summary)
            )
            return
        self.load_folder(
            summary.output_root,
            skip_grouping_step=True,
            record_as_source=False,
            preserve_deletion_marks=True,
        )
        self.main_window.finish_pending_close_after_grouping()

    def handle_grouping_workflow_error(self, message: str):
        self.main_window.set_grouping_busy(False)
        self.main_window.hide_loading_overlay()
        self.main_window.grouping_step_widget.set_loading_state(
            f"Grouping failed: {message}",
            False,
        )
        self.main_window.statusBar().showMessage(
            f"Grouping failed: {message}",
            5000,
        )
        self.main_window.cancel_pending_close_after_grouping()
        cancel_transition = getattr(
            self.main_window, "cancel_pending_workflow_transition", None
        )
        if callable(cancel_transition):
            cancel_transition()

    def start_pick_best_workflow(self) -> None:
        """Start Pick Best after preparing shared same-subject groups if needed."""
        _reactivate_workflow(self, "pick_best")
        if not self.app_state.image_files_data:
            self.main_window.statusBar().showMessage("No images loaded.", 3000)
            return

        if (
            self.worker_manager.is_pick_best_running()
            or self._pick_best_pending_after_subject_grouping
        ):
            return

        if self.app_state.pick_best_results:
            return

        widget = self.main_window.pick_best_step_widget

        image_paths = set(self._get_image_paths())
        grouped_paths = set(self.app_state.cull_cluster_results)
        same_subject_groups_ready = bool(image_paths) and image_paths.issubset(
            grouped_paths
        )

        if same_subject_groups_ready:
            self._start_pick_best_scoring()
        else:
            logger.info(
                "Pick Best: same-subject groups are incomplete; "
                "running the shared DINO grouping analysis first."
            )
            grouping_already_running = self.worker_manager.is_cull_grouping_running()
            self._pick_best_pending_after_subject_grouping = True
            self._pick_best_owns_subject_grouping = False
            widget.show_loading("Step 1/2: Preparing same-subject groups…", 0)
            self.start_cull_similarity_workflow()
            # The grouping run may still be deferred behind the model environment
            # probe, so a synchronous "is running" check alone would under-claim it.
            self._pick_best_owns_subject_grouping = bool(
                not grouping_already_running
                and (
                    self.worker_manager.is_cull_grouping_running()
                    or self._deferred_starts.is_armed("cull_grouping")
                )
            )

    def _start_pick_best_scoring(self) -> None:
        cluster_map = self._build_pick_best_cluster_map()
        widget = self.main_window.pick_best_step_widget
        total_clusters = len(cluster_map)
        scorable = sum(1 for paths in cluster_map.values() if len(paths) >= 2)
        if scorable == 0:
            logger.info("Pick Best: no clusters with 2+ images.")
            widget.show_loading(
                f"No comparable clusters found ({total_clusters} singleton cluster(s)).\n"
                "Click 'Done' to continue to Cull."
            )
            return
        widget.show_loading(f"Step 2/2: Scoring {scorable} cluster(s)…", 0)
        if self._model_environment is None:
            self._deferred_starts.arm("pick_best_scoring")
            self._start_model_environment_probe()
            return
        outcome = self._confirm_model_prerequisites(
            [AESTHETIC_MODEL.key], feature="Pick Best scoring"
        )
        if not outcome.approved:
            if outcome.declined is PrerequisiteDecline.BUSY:
                return
            message = (
                "Pick Best needs the local scoring model. Download it to continue."
                if outcome.declined is PrerequisiteDecline.DOWNLOAD
                else "Pick Best was cancelled; hardware acceleration is unavailable."
            )
            widget.show_error(message)
            return
        self.worker_manager.start_pick_best_analysis(
            cluster_map, allow_model_download=outcome.allow_download
        )

    def handle_pick_best_progress(self, percent: int, message: str) -> None:
        if _workflow_is_cancelled(self, "pick_best"):
            return
        self.main_window.pick_best_step_widget.show_loading(
            f"Scoring images… {message}", percent
        )

    def handle_pick_best_complete(self, results: PickBestResults) -> None:
        if _workflow_is_cancelled(self, "pick_best"):
            return
        logger.info(f"Pick Best complete: {len(results)} clusters scored.")
        self._mark_models_installed([AESTHETIC_MODEL.key])
        self.app_state.pick_best_results = results
        # Build quick path→is_winner lookup
        self.app_state.pick_best_winners_by_path.clear()
        for cluster_data in results.values():
            winner = cluster_data.get("winner_path")
            if winner:
                self.app_state.pick_best_winners_by_path[winner] = True
        self.main_window.pick_best_step_widget.show_results(results)
        AppController._sync_active_image(self, "pick_best")

    def handle_pick_best_error(self, message: str) -> None:
        if _workflow_is_cancelled(self, "pick_best"):
            return
        logger.error(f"Pick Best error: {message}", exc_info=False)
        self.main_window.pick_best_step_widget.show_error(message)
        self.main_window.statusBar().showMessage(f"Pick Best error: {message}", 6000)

    # ------------------------------------------------------------------
    # Easy Delete workflow
    # ------------------------------------------------------------------

    def start_easy_delete_workflow(self) -> None:
        _reactivate_workflow(self, "easy_delete")
        if not self.app_state.image_files_data:
            self.main_window.statusBar().showMessage("No images loaded.", 3000)
            return

        if (
            self.worker_manager.is_easy_delete_running()
            or self._easy_delete_pending_after_similarity
        ):
            return

        if self.app_state.easy_delete_results is not None:
            self.main_window.easy_delete_step_widget.show_results(
                self.app_state.easy_delete_results
            )
            AppController._sync_active_image(self, "easy_delete")
            return

        image_paths = set(self._get_image_paths())
        clustered_paths = set(self.app_state.cluster_results)
        embedded_paths = set(getattr(self.app_state, "embeddings_cache", {}) or {})
        duplicate_inputs_ready = bool(image_paths) and image_paths.issubset(
            clustered_paths & embedded_paths
        )

        if duplicate_inputs_ready:
            self._start_easy_delete_detection()
        else:
            logger.info(
                "Easy Delete: similarity inputs are missing or incomplete; "
                "running similarity first."
            )
            self._easy_delete_pending_after_similarity = True
            self.main_window.easy_delete_step_widget.show_loading(
                "Step 1/2: Computing similarity embeddings and clusters…", 0
            )
            self.start_similarity_analysis()

    def _start_easy_delete_detection(self) -> None:
        image_paths = self._get_image_paths()
        cluster_map = self._build_cluster_path_map()
        embeddings = getattr(self.app_state, "embeddings_cache", {}) or {}
        exif_cache = self.app_state.exif_disk_cache
        getattr(self.app_state, "easy_delete_pair_assessments", {}).clear()

        self.main_window.easy_delete_step_widget.show_loading(
            "Step 2/2: Detecting blurry, dark, overexposed, and duplicate images…", 0
        )
        self.worker_manager.start_easy_delete_analysis(
            image_paths=image_paths,
            cluster_map=cluster_map,
            embeddings_cache=embeddings,
            exif_disk_cache=exif_cache,
            analysis_cache=getattr(self.app_state, "analysis_cache", None),
            folder_path=getattr(self.app_state, "current_folder_path", None),
            fingerprints=self._similarity_fingerprints(image_paths),
        )

    def handle_easy_delete_progress(self, percent: int, message: str) -> None:
        if _workflow_is_cancelled(self, "easy_delete"):
            return
        self.main_window.easy_delete_step_widget.show_loading(message, percent)

    def handle_easy_delete_complete(self, results: dict) -> None:
        if _workflow_is_cancelled(self, "easy_delete"):
            return
        logger.info(f"Easy Delete complete: {len(results)} images flagged.")
        self.app_state.easy_delete_results = results
        self.main_window.easy_delete_step_widget.show_results(results)
        AppController._sync_active_image(self, "easy_delete")

    def handle_easy_delete_assessments_ready(self, assessments: dict) -> None:
        if _workflow_is_cancelled(self, "easy_delete"):
            return
        self.app_state.easy_delete_pair_assessments = dict(assessments)

    def handle_easy_delete_error(self, message: str) -> None:
        if _workflow_is_cancelled(self, "easy_delete"):
            return
        logger.error(f"Easy Delete error: {message}", exc_info=False)
        self.main_window.easy_delete_step_widget.show_error(message)
        self.main_window.statusBar().showMessage(f"Easy Delete error: {message}", 6000)

    # ------------------------------------------------------------------
    # Fix Rotation workflow
    # ------------------------------------------------------------------

    def start_fix_rotation_workflow(self) -> None:
        _reactivate_workflow(self, "fix_rotation")
        if not self.app_state.image_files_data:
            self.main_window.statusBar().showMessage("No images loaded.", 3000)
            return

        if self.worker_manager.is_fix_rotation_running():
            return

        if self.app_state.fix_rotation_results is not None:
            self.main_window.fix_rotation_step_widget.show_results(
                self.app_state.fix_rotation_results
            )
            AppController._sync_active_image(self, "fix_rotation")
            return

        image_paths = self._get_image_paths()
        if not image_paths:
            self.main_window.fix_rotation_step_widget.show_results({})
            return

        self.main_window.fix_rotation_step_widget.show_loading(
            "Starting rotation analysis…", 0
        )
        self.worker_manager.start_fix_rotation_detection(image_paths)

    def handle_fix_rotation_progress(self, percent: int, message: str) -> None:
        if _workflow_is_cancelled(self, "fix_rotation"):
            return
        self.main_window.fix_rotation_step_widget.show_loading(message, percent)

    def handle_fix_rotation_complete(self, results: dict) -> None:
        if _workflow_is_cancelled(self, "fix_rotation"):
            return
        logger.info(
            f"Fix Rotation detection complete: {len(results)} images need rotation."
        )
        self.app_state.fix_rotation_results = results
        self.main_window.fix_rotation_step_widget.show_results(results)
        AppController._sync_active_image(self, "fix_rotation")

    def handle_fix_rotation_model_not_found(self, message: str) -> None:
        if _workflow_is_cancelled(self, "fix_rotation"):
            return
        logger.warning(f"Fix Rotation model not found: {message}")
        self.main_window.fix_rotation_step_widget.show_model_not_found(message)

    def handle_fix_rotation_error(self, message: str) -> None:
        if _workflow_is_cancelled(self, "fix_rotation"):
            return
        logger.error(f"Fix Rotation error: {message}", exc_info=False)
        self.main_window.fix_rotation_step_widget.show_error(message)
        self.main_window.statusBar().showMessage(f"Fix Rotation error: {message}", 6000)

    def start_fix_rotation_apply(self, rotations: dict) -> None:
        """Apply the approved rotations dict {path: angle} using the rotation application worker."""
        if not rotations:
            return
        self.worker_manager.start_rotation_application(rotations)

    def _on_fix_rotation_apply_progress(
        self, current: int, total: int, filename: str
    ) -> None:
        if self.app_state.workflow_step == "fix_rotation":
            self.main_window.fix_rotation_step_widget.show_applying(
                current, total, filename
            )

    def _on_fix_rotation_apply_finished(self, successful: int, failed: int) -> None:
        if self.app_state.workflow_step == "fix_rotation":
            self.main_window.fix_rotation_step_widget.show_apply_complete(
                successful, failed
            )
            self.main_window.statusBar().showMessage(
                f"Rotation applied: {successful} OK, {failed} failed.", 5000
            )
        finish_transition = getattr(
            self.main_window, "finish_workflow_transition_after_rotations", None
        )
        if callable(finish_transition):
            finish_transition(successful, failed)

    def handle_rating_load_progress(self, current: int, total: int, basename: str):
        self.main_window.set_exif_progress(current, total)

    def handle_metadata_batch_loaded(
        self, metadata_batch: list[tuple[str, dict[str, Any]]]
    ):
        currently_selected_paths = self.main_window._get_selected_file_paths_from_view()
        needs_active_selection_refresh = False

        for image_path, metadata in metadata_batch:
            if not metadata:
                continue

            for viewer in self.main_window.advanced_image_viewer.image_viewers:
                if viewer.isVisible() and viewer._file_path == image_path:
                    viewer.update_rating_display(metadata.get("rating", 0))

            if image_path in currently_selected_paths:
                needs_active_selection_refresh = True

        if needs_active_selection_refresh:
            self.main_window._handle_file_selection_changed()

        self.main_window._apply_filter()

    def handle_rating_load_finished(self):
        logger.info("Background rating loading finished.")
        self._rating_load_complete = True
        self.main_window.statusBar().showMessage(
            "Background rating loading finished.", 3000
        )

        if self._folder_asset_session_id is None:
            self.main_window.hide_loading_overlay()
        self.main_window.hide_exif_progress()
        menu_manager = getattr(self.main_window, "menu_manager", None)
        ai_rating_action = getattr(menu_manager, "ai_rate_images_action", None)
        if ai_rating_action is not None:
            ai_rating_action.setEnabled(
                self._folder_asset_session_id is None
                and bool(self._get_image_file_data())
            )

        warning = self._pending_exif_cache_capacity_warning
        self._pending_exif_cache_capacity_warning = None
        if warning is not None:
            dataset_entries, resident_entries, cache_limit_bytes = warning
            QTimer.singleShot(
                0,
                lambda: (
                    self.main_window.dialog_manager.show_exif_cache_capacity_warning(
                        dataset_entries,
                        resident_entries,
                        cache_limit_bytes,
                    )
                ),
            )

    def handle_exif_cache_capacity_warning(
        self,
        dataset_entries: int,
        resident_entries: int,
        cache_limit_bytes: int,
    ) -> None:
        """Defer the modal warning until background metadata loading finishes."""
        self._pending_exif_cache_capacity_warning = (
            dataset_entries,
            resident_entries,
            cache_limit_bytes,
        )

    def handle_rating_load_error(self, message: str):
        logger.error(f"Rating load failed: {message}", exc_info=True)
        self.main_window.statusBar().showMessage(f"Rating Load Error: {message}", 5000)
        if self._folder_asset_session_id is None:
            self.main_window.hide_loading_overlay()
        self.main_window.hide_exif_progress()

    def handle_similarity_progress(self, percentage, message):
        if getattr(self, "_ignore_similarity_results", False):
            return
        suffix = (
            f" ({percentage}%)" if percentage is not None and percentage >= 0 else ""
        )
        if self._easy_delete_pending_after_similarity:
            self.main_window.easy_delete_step_widget.show_loading(
                f"Step 1/2: {message}", percentage
            )
            return
        self.main_window.update_loading_text(f"Similarity: {message}{suffix}")

    def handle_embeddings_generated(self, embeddings_dict):
        if getattr(self, "_ignore_similarity_results", False):
            return
        self.app_state.embeddings_cache = embeddings_dict
        if self._easy_delete_pending_after_similarity:
            self.main_window.easy_delete_step_widget.show_loading(
                "Step 1/2: Embeddings generated. Clustering...", -1
            )
            return
        self.main_window.update_loading_text("Embeddings generated. Clustering...")

    def handle_regional_embeddings_generated(self, embeddings_dict):
        if getattr(self, "_ignore_similarity_results", False):
            return
        self.app_state.regional_embeddings_cache = embeddings_dict

    def start_cull_similarity_workflow(self) -> None:
        """Start Cull as one consented, continuous, cancellable task."""
        if self.worker_manager.is_cull_grouping_running():
            return
        if not self._get_image_paths() or not self.app_state.current_folder_path:
            return

        # An explicit start always re-offers previously declined prerequisites.
        self._cull_prerequisites_declined = False
        self.app_state.cull_grouping_error = None
        self.main_window.show_cull_grouping_progress(
            "Starting fast DINO same-subject analysis…", 0
        )
        self._start_cull_subject_grouping_background()

    def is_cull_grouping_declined(self) -> bool:
        """Report whether the user refused the prerequisites for Cull grouping.

        Callers that start grouping implicitly (such as opening the Cull page) use
        this to avoid re-prompting for a download the user already refused.
        """

        return self._cull_prerequisites_declined

    def cancel_cull_similarity_workflow(self) -> None:
        """Cancel Cull analysis without modifying or moving source media."""
        self.worker_manager.request_stop_cull_subject_grouping()
        self._cull_grouping_fingerprints = None
        self._deferred_starts.disarm("cull_grouping")
        self.app_state.cull_grouping_error = "Same-subject grouping was cancelled."
        self.main_window.mark_cull_model_dirty()
        self.main_window.cancel_cull_grouping_progress(
            "Same-subject grouping cancelled."
        )
        if self.app_state.workflow_step == "cull":
            self.main_window._ensure_cull_model_ready()

    def _start_model_environment_probe(self) -> None:
        """Probe every managed model once, off the GUI thread."""

        self.worker_manager.start_model_environment_probe(sorted(MODEL_REGISTRY))

    def _reset_model_environment(self) -> None:
        """Force the next model-backed start to re-probe what is on disk.

        Used when the user explicitly retries: the cached probe result may be
        stale (a download was cancelled, or a snapshot was removed), and we want
        the consent prompt to be offered again rather than silently reusing it.
        """

        self._model_environment = None
        self._model_consent.forget_downloads()

    def retry_pick_best_workflow(self) -> None:
        """Re-run Pick Best, re-offering any model download the user declined."""

        self._reset_model_environment()
        # A failed or cancelled attempt can leave the "waiting for grouping" latch
        # set, which would make the restart a silent no-op.
        self._pick_best_pending_after_subject_grouping = False
        self.start_pick_best_workflow()

    def retry_easy_delete_workflow(self) -> None:
        """Re-run Easy Delete, re-offering any model download the user declined."""

        self._reset_model_environment()
        self._easy_delete_pending_after_similarity = False
        self.start_easy_delete_workflow()

    def _confirm_model_prerequisites(self, model_keys, *, feature, fallback=""):
        """Ask for any outstanding consent for ``model_keys``.

        The prompt is modal, and ``QDialog.exec`` runs a nested event loop, so a
        queued signal can re-enter a workflow start method while the user is still
        looking at the dialog. Refusing the re-entrant call keeps a single prompt
        on screen and stops the same workflow being started twice.
        """

        if self._consent_prompt_active:
            logger.debug(
                "Ignoring a re-entrant consent request for %s while a prompt is open.",
                feature,
            )
            return PrerequisiteOutcome(declined=PrerequisiteDecline.BUSY)

        missing_keys, torch_device = self._model_environment or ((), "cpu")
        self._consent_prompt_active = True
        try:
            return confirm_model_prerequisites(
                self.main_window.dialog_manager,
                self._model_consent,
                required_keys=model_keys,
                missing_keys=missing_keys,
                torch_device=torch_device,
                feature=feature,
                fallback=fallback,
            )
        finally:
            self._consent_prompt_active = False

    def _mark_models_installed(self, model_keys) -> None:
        """Record that a successful run proved these models are on disk."""

        self._model_consent.reset_downloads(model_keys)
        if self._model_environment is None:
            return
        missing, device = self._model_environment
        self._model_environment = (
            tuple(key for key in missing if key not in set(model_keys)),
            device,
        )

    def handle_model_environment_ready(
        self, missing_model_keys: tuple[str, ...], torch_device: str
    ) -> None:
        """Resume deferred workflow starts once the background probe has answered."""
        self._model_environment = (tuple(missing_model_keys), torch_device)
        if self._deferred_starts.take("cull_grouping") is not None:
            self._start_cull_subject_grouping_background()
        if self._deferred_starts.take("similarity") is not None:
            self.start_similarity_analysis()
        if self._deferred_starts.take("pick_best_scoring") is not None:
            self._start_pick_best_scoring()
        if self._deferred_starts.take("grouping_preview") is not None:
            if not _workflow_is_cancelled(self, "organize"):
                self.refresh_grouping_preview()
        pending_grouping_workflow = self._deferred_starts.take("grouping_workflow")
        if pending_grouping_workflow is not None:
            mode, overrides, prepared_plan = pending_grouping_workflow
            self.start_grouping_workflow(
                mode,
                group_name_overrides=overrides,
                prepared_plan=prepared_plan,
            )

    def _abort_cull_grouping(
        self, *, reason: str, status_message: str, pick_best_message: str
    ) -> None:
        """Leave Cull usable without groups after declined prerequisites."""
        self._cull_prerequisites_declined = True
        self._deferred_starts.disarm("cull_grouping")
        self.app_state.cull_cluster_results.clear()
        self.app_state.cull_grouping_error = reason
        self.main_window.mark_cull_model_dirty()
        self.main_window.revert_group_by_similarity()
        if self.app_state.workflow_step == "cull":
            self.main_window._ensure_cull_model_ready()
        self.main_window.fail_cull_grouping_progress(reason)
        self.main_window.statusBar().showMessage(status_message, 6000)
        if self._pick_best_pending_after_subject_grouping:
            self._pick_best_pending_after_subject_grouping = False
            self._pick_best_owns_subject_grouping = False
            self.main_window.pick_best_step_widget.show_error(pick_best_message)

    def _confirm_cull_prerequisites(self) -> bool | None:
        """Return whether model downloads are allowed, or ``None`` if declined."""
        outcome = self._confirm_model_prerequisites(
            [EMBEDDING_MODEL.key],
            feature="same-subject grouping",
            fallback=(
                "If you cancel, Cull remains available without similarity groups."
            ),
        )
        if outcome.declined is PrerequisiteDecline.BUSY:
            # The prompt already on screen owns this decision; do not abort Cull
            # on behalf of a duplicate start.
            return None
        if outcome.declined is PrerequisiteDecline.DOWNLOAD:
            self._abort_cull_grouping(
                reason="Same-subject grouping requires the local Cull model.",
                status_message=(
                    "Download cancelled. Cull remains available without "
                    "similarity groups."
                ),
                pick_best_message=(
                    "Pick Best needs the local same-subject model. "
                    "Download it to continue."
                ),
            )
            return None
        if outcome.declined is PrerequisiteDecline.ACCELERATION:
            self._abort_cull_grouping(
                reason=(
                    "Same-subject grouping was cancelled because acceleration "
                    "is unavailable."
                ),
                status_message=(
                    "Same-subject grouping was cancelled; "
                    "hardware acceleration is unavailable."
                ),
                pick_best_message="Same-subject preparation was cancelled.",
            )
            return None
        return outcome.allow_download

    def _start_cull_subject_grouping_background(self) -> None:
        if self.worker_manager.is_cull_grouping_running():
            return
        paths = self._get_image_paths()
        folder_path = self.app_state.current_folder_path
        if not paths or not folder_path:
            return

        if self._model_environment is None:
            # Importing torch and resolving model snapshots takes seconds, so the
            # answer is produced by a worker and this start resumes on its signal.
            self._deferred_starts.arm("cull_grouping")
            self.main_window.show_cull_grouping_progress(
                "Checking the local same-subject model…", -1
            )
            self._start_model_environment_probe()
            return

        allow_download = self._confirm_cull_prerequisites()
        if allow_download is None:
            return

        timestamps = {}
        for record in self._get_image_file_data():
            path = record.get("path")
            if not path:
                continue
            value = self.app_state.date_cache.get(path)
            if value is None:
                try:
                    value = datetime_obj.fromtimestamp(os.path.getmtime(path))
                except OSError:
                    value = None
            timestamps[path] = value

        self.app_state.cull_grouping_error = None
        fingerprints = self._similarity_fingerprints(paths)
        self._cull_grouping_fingerprints = fingerprints
        self.worker_manager.start_cull_subject_grouping(
            paths=paths,
            fingerprints=fingerprints,
            timestamps=timestamps,
            strictness=get_cull_grouping_strictness(),
            analysis_cache=self.app_state.analysis_cache,
            folder_path=folder_path,
            allow_model_download=allow_download,
        )

    def handle_cull_grouping_progress(self, percent: int, message: str) -> None:
        stage_match = re.search(r"\((\d+)\s*/\s*(\d+)\)", message)
        if stage_match and int(stage_match.group(2)) > 0:
            displayed = int(100 * int(stage_match.group(1)) / int(stage_match.group(2)))
        else:
            displayed = percent
        self.main_window.show_cull_grouping_progress(message, displayed)
        if getattr(self, "_pick_best_pending_after_subject_grouping", False):
            self.main_window.pick_best_step_widget.show_loading(
                f"Step 1/2: {message}", displayed
            )

    def handle_cull_grouping_complete(self, result: CullClusteringResult) -> None:
        current_paths = set(self._get_image_paths())
        current_fingerprints = self._similarity_fingerprints(sorted(current_paths))
        if (
            set(result.clusters) != current_paths
            or self._cull_grouping_fingerprints != current_fingerprints
        ):
            logger.info("Discarding stale Cull grouping result.")
            self._cull_grouping_fingerprints = None
            self.main_window.finish_cull_grouping_progress()
            if self._pick_best_pending_after_subject_grouping:
                self._pick_best_pending_after_subject_grouping = False
                self._pick_best_owns_subject_grouping = False
                self.main_window.pick_best_step_widget.show_error(
                    "Photos changed while same-subject groups were being prepared. "
                    "Try Pick Best again."
                )
            return
        self._cull_grouping_fingerprints = None
        # A completed run proves the models are now installed locally.
        self._mark_models_installed([EMBEDDING_MODEL.key])
        normalized = normalize_cluster_results(result.clusters)
        if normalized != self.app_state.cull_cluster_results:
            self.app_state.clear_pick_best_results()
        self.app_state.cull_cluster_results = normalized
        self.app_state.cull_grouping_error = None
        cluster_ids = sorted(set(self.app_state.cull_cluster_results.values()))
        self.main_window.cluster_filter_combo.clear()
        self.main_window.cluster_filter_combo.addItems(
            ["All Clusters"] + [f"Cluster {cluster_id}" for cluster_id in cluster_ids]
        )
        self.main_window.cluster_filter_combo.setEnabled(bool(cluster_ids))
        self.main_window.menu_manager.update_cluster_filter_menu(cluster_ids)
        self.main_window.mark_cull_model_dirty()
        if self.app_state.workflow_step == "cull":
            self.main_window._ensure_cull_model_ready()
        self.main_window.finish_cull_grouping_progress()
        self.main_window.statusBar().showMessage(
            f"Cull same-subject grouping ready: "
            f"{len(set(self.app_state.cull_cluster_results.values()))} groups.",
            5000,
        )
        if self._pick_best_pending_after_subject_grouping:
            self._pick_best_pending_after_subject_grouping = False
            self._pick_best_owns_subject_grouping = False
            self._start_pick_best_scoring()

    def handle_cull_grouping_error(self, message: str) -> None:
        self._cull_grouping_fingerprints = None
        self.app_state.cull_cluster_results.clear()
        self.app_state.cull_grouping_error = message
        self._model_consent.approved_downloads.discard(EMBEDDING_MODEL.key)
        self.main_window.mark_cull_model_dirty()
        if self.app_state.workflow_step == "cull":
            self.main_window._ensure_cull_model_ready()
        self.main_window.fail_cull_grouping_progress(message)
        self.main_window.statusBar().showMessage(
            f"Cull same-subject grouping unavailable: {message}", 8000
        )
        if self._pick_best_pending_after_subject_grouping:
            self._pick_best_pending_after_subject_grouping = False
            self._pick_best_owns_subject_grouping = False
            self.main_window.pick_best_step_widget.show_error(
                f"Same-subject preparation failed: {message}"
            )

    def handle_clustering_complete(
        self,
        result: SimilarityClusteringResult | dict[str, object],
    ):
        if getattr(self, "_ignore_similarity_results", False):
            return
        if isinstance(result, SimilarityClusteringResult):
            cluster_results_dict = result.clusters
        else:
            cluster_results_dict = result
        self.app_state.cluster_results = normalize_cluster_results(cluster_results_dict)

        self.main_window.menu_manager.analyze_similarity_action.setEnabled(
            bool(self._get_image_file_data())
        )

        if not self.app_state.cluster_results:
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage(
                "Clustering did not produce results.", 3000
            )
            if self._easy_delete_pending_after_similarity:
                self._easy_delete_pending_after_similarity = False
                self.main_window.easy_delete_step_widget.show_error(
                    "Clustering did not produce results."
                )
            return

        if self._easy_delete_pending_after_similarity:
            self.main_window.easy_delete_step_widget.show_loading(
                "Step 1/2: Clustering complete. Updating view...", -1
            )
        else:
            self.main_window.update_loading_text(
                "Clustering complete. Updating view..."
            )
        cluster_ids = sorted(set(self.app_state.cluster_results.values()))
        self.main_window.cluster_filter_combo.clear()
        self.main_window.cluster_filter_combo.addItems(
            ["All Clusters"] + [f"Cluster {cid}" for cid in cluster_ids]
        )
        self.main_window.cluster_filter_combo.setEnabled(True)
        self.main_window.menu_manager.update_cluster_filter_menu(cluster_ids)
        self.main_window.menu_manager.group_by_similarity_action.setChecked(True)
        if (
            self.main_window.menu_manager.group_by_similarity_action.isChecked()
            and self.app_state.cluster_results
        ):
            self.main_window.menu_manager.set_cluster_sort_menu_visible(True)
            self.main_window.cluster_sort_combo.setEnabled(True)
            self.main_window.menu_manager.set_cluster_sort_menu_enabled(True)
        self.main_window.refresh_navigation_shortcut_actions()
        if self.main_window.group_by_similarity_mode:
            self.main_window._rebuild_model_view()

        if self._easy_delete_pending_after_similarity:
            self._easy_delete_pending_after_similarity = False
            self._start_easy_delete_detection()
        else:
            self.main_window.hide_loading_overlay()

    def handle_similarity_error(self, message):
        if getattr(self, "_ignore_similarity_results", False):
            return
        logger.error(f"Similarity analysis failed: {message}", exc_info=True)
        if self._easy_delete_pending_after_similarity:
            self._easy_delete_pending_after_similarity = False
            self.main_window.easy_delete_step_widget.show_error(message)
        self.main_window.statusBar().showMessage(f"Similarity Error: {message}", 8000)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(
            bool(self._get_image_file_data())
        )
        self.main_window.hide_loading_overlay()

    def handle_ai_rating_progress(self, percentage: int, message: str):
        suffix = (
            f" ({percentage}%)" if percentage is not None and percentage >= 0 else ""
        )
        self.main_window.update_loading_text(f"AI rating: {message}{suffix}")

    def handle_ai_rating_warning(self, message: str):
        logger.warning("AI rating warning: %s", message)
        self.main_window.statusBar().showMessage(message, 6000)
        lowered = message.lower()
        if "failed" in lowered or "skipped" in lowered:
            self._ai_rating_warning_messages.append(message)

    def handle_ai_rating_complete(self, results: dict[str, dict[str, Any]]):
        self.main_window.hide_loading_overlay()
        self.main_window.menu_manager.ai_rate_images_action.setEnabled(
            bool(self._get_image_file_data())
        )

        normalized_results = results or {}
        self.app_state.ai_rating_results = dict(normalized_results)

        ratings_applied = 0
        rating_operations: list[tuple[str, int]] = []
        for image_path, payload in normalized_results.items():
            if not isinstance(payload, dict):
                continue
            rating_value = payload.get("rating")
            if rating_value is None:
                continue
            try:
                rating_int = int(round(float(rating_value)))
            except TypeError, ValueError:
                logger.debug("Skipping non-numeric AI rating for %s", image_path)
                continue

            ratings_applied += 1
            self.app_state.rating_cache[image_path] = rating_int

            for viewer in self.main_window.advanced_image_viewer.image_viewers:
                if viewer.isVisible() and viewer._file_path == image_path:
                    viewer.update_rating_display(rating_int)

            rating_operations.append((image_path, rating_int))

        if ratings_applied:
            self.main_window._apply_filter()
            self.main_window.statusBar().showMessage(
                f"AI rating complete for {ratings_applied} image(s).", 4000
            )
            if rating_operations:
                if self.worker_manager.is_rating_writer_running():
                    logger.info(
                        "Rating writer already running; skipping automatic metadata write"
                    )
                else:
                    self.main_window.statusBar().showMessage(
                        f"Saving AI ratings to image metadata ({len(rating_operations)} files)...",
                        5000,
                    )
                    self.worker_manager.start_rating_writer(
                        rating_operations=rating_operations,
                        rating_disk_cache=self.app_state.rating_disk_cache,
                        exif_disk_cache=self.app_state.exif_disk_cache,
                    )
        else:
            self.main_window.statusBar().showMessage(
                "AI rating finished but no ratings were applied.", 5000
            )

        if self._ai_rating_warning_messages:
            summary_message = self._ai_rating_warning_messages[-1]
            self.main_window.statusBar().showMessage(summary_message, 7000)
            self._ai_rating_warning_messages = []

    def handle_ai_rating_error(self, message: str):
        logger.error(f"AI rating failed: {message}", exc_info=True)
        self.main_window.hide_loading_overlay()
        self.main_window.statusBar().showMessage(f"AI rating error: {message}", 8000)
        self.main_window.menu_manager.ai_rate_images_action.setEnabled(
            bool(self._get_image_file_data())
        )
        self._ai_rating_warning_messages = []

    def _apply_approved_rotations(self, approved_rotations: dict[str, int]):
        """Apply the approved rotations to the images using background worker."""
        logger.info(
            f"Starting rotation application for {len(approved_rotations)} images."
        )

        if self.worker_manager.is_rotation_application_running():
            logger.warning(
                "Rotation application already in progress; ignoring new request."
            )
            self.main_window.statusBar().showMessage(
                "Rotation is already being applied...", 4000
            )
            return

        AppController._begin_rotation_loading_feedback(self)
        try:
            self.worker_manager.start_rotation_application(
                approved_rotations=approved_rotations,
                exif_disk_cache=self.app_state.exif_disk_cache,
            )
        except Exception:
            AppController._finish_rotation_loading_feedback(self)
            raise

    def _begin_rotation_loading_feedback(self) -> None:
        """Delay disruptive rotation feedback so fast operations stay seamless."""

        self._rotation_loading_overlay_timer.stop()
        self._rotation_loading_text = "Applying rotations..."
        self._rotation_loading_overlay_visible = False
        self._rotation_loading_overlay_timer.start(ROTATION_LOADING_OVERLAY_DELAY_MS)

    def _show_delayed_rotation_loading_overlay(self) -> None:
        """Show the latest rotation progress only if work is still running."""

        if not self.worker_manager.is_rotation_application_running():
            return
        self._rotation_loading_overlay_visible = True
        self.main_window.show_loading_overlay(self._rotation_loading_text)

    def _finish_rotation_loading_feedback(self) -> None:
        """Cancel delayed feedback and hide it only when it was actually shown."""

        self._rotation_loading_overlay_timer.stop()
        if self._rotation_loading_overlay_visible:
            self.main_window.hide_loading_overlay()
        self._rotation_loading_overlay_visible = False

    # --- Update Check Handlers ---

    def manual_check_for_updates(self):
        """Manually check for updates (called from menu)."""
        from core.build_info import VERSION
        from ui.update_dialog import UpdateCheckDialog

        # Show the update check dialog
        self.update_check_dialog = UpdateCheckDialog(self.main_window)
        self.update_check_dialog.show()

        # Start the update check
        current_version = VERSION or "dev"
        self.worker_manager.start_update_check(current_version)

    def automatic_check_for_updates(self):
        """Automatically check for updates on startup if enabled."""
        from core.update_checker import UpdateChecker
        from core.build_info import VERSION

        update_checker = UpdateChecker()
        if update_checker.should_check_for_updates():
            logger.info("Starting automatic update check...")
            current_version = VERSION or "dev"
            self.worker_manager.start_update_check(current_version)

    def handle_update_check_finished(
        self, update_available: bool, update_info, error_message: str
    ):
        """Handle the completion of an update check."""
        from ui.update_dialog import UpdateNotificationDialog
        from core.build_info import VERSION

        # Handle manual check dialog if it exists
        is_manual_check = (
            hasattr(self, "update_check_dialog")
            and self.update_check_dialog is not None
        )

        if is_manual_check:
            if update_available:
                # Close the manual check dialog since we'll show the update notification
                self.update_check_dialog.reject()
                self.update_check_dialog = None
            elif error_message:
                self.update_check_dialog.set_status(f"Error: {error_message}", True)
            else:
                self.update_check_dialog.set_status("No updates available.", True)

        # Show update notification dialog only if an update is available
        if update_available and update_info:
            current_version = VERSION or "dev"
            dialog = UpdateNotificationDialog(
                update_info, current_version, self.main_window
            )
            dialog.exec()
        elif error_message:
            logger.warning(f"Update check failed: {error_message}")
        else:
            logger.info("No updates available")

    # --- Rating Writer Handlers ---

    def apply_rating_to_selection(self, rating: int, selected_paths: list[str]):
        """Apply rating to multiple images using background worker."""
        if not selected_paths:
            return

        image_paths = self._filter_image_paths(selected_paths)
        skipped_videos = len(selected_paths) - len(image_paths)
        if not image_paths:
            self.main_window.statusBar().showMessage(
                "Ratings are currently supported for images only.", 3000
            )
            return

        # Create list of (path, rating) tuples for the worker
        rating_operations = [(path, rating) for path in image_paths]

        # Start the worker
        self.worker_manager.start_rating_writer(
            rating_operations=rating_operations,
            rating_disk_cache=self.app_state.rating_disk_cache,
            exif_disk_cache=self.app_state.exif_disk_cache,
        )

        # Show a status message
        if len(selected_paths) == 1:
            self.main_window.statusBar().showMessage(
                f"Setting rating to {rating}...", 2000
            )
        elif skipped_videos > 0:
            self.main_window.statusBar().showMessage(
                f"Setting rating to {rating} for {len(image_paths)} image(s). "
                f"Skipping {skipped_videos} video(s).",
                3000,
            )
        else:
            self.main_window.statusBar().showMessage(
                f"Setting rating to {rating} for {len(image_paths)} images...", 2000
            )

    def handle_rating_write_progress(self, current: int, total: int, filename: str):
        """Handle progress updates from rating writer."""
        # Only show progress for multi-image operations
        if total > 1:
            self.main_window.statusBar().showMessage(
                f"Setting rating {current}/{total}: {filename}", 500
            )

    def handle_rating_written(self, file_path: str, rating: int, success: bool):
        """Handle completion of a single rating write."""
        if success:
            # Update in-memory cache
            self.app_state.rating_cache[file_path] = rating
            # Update the display for any visible viewers
            for viewer in self.main_window.advanced_image_viewer.image_viewers:
                if viewer.isVisible() and viewer._file_path == file_path:
                    viewer.update_rating_display(rating)
        else:
            logger.warning(f"Failed to write rating for {os.path.basename(file_path)}")

    def handle_rating_write_finished(self, successful_count: int, failed_count: int):
        """Handle completion of all rating writes."""
        # Refresh the filter to show/hide items based on new ratings
        self.main_window._apply_filter()

        # Show summary message
        if failed_count == 0:
            if successful_count > 1:
                self.main_window.statusBar().showMessage(
                    f"Successfully updated ratings for {successful_count} images", 3000
                )
        else:
            self.main_window.statusBar().showMessage(
                f"Updated {successful_count} ratings, {failed_count} failed", 5000
            )

    def handle_rating_write_error(self, error_message: str):
        """Handle errors from rating writer."""
        logger.error(f"Rating writer error: {error_message}")
        self.main_window.statusBar().showMessage(
            f"Error writing ratings: {error_message}", 5000
        )

    # --- Rotation Application Handlers ---

    def handle_rotation_application_progress(
        self, current: int, total: int, filename: str
    ):
        """Handle progress updates from rotation application worker."""
        progress_text = f"Rotating {current}/{total}: {filename}"
        self._rotation_loading_text = progress_text
        if self._rotation_loading_overlay_visible:
            self.main_window.update_loading_text(progress_text)

    def handle_rotation_applied(
        self,
        file_path: str,
        direction: str,
        success: bool,
        message: str,
        is_lossy: bool,
    ):
        """Handle completion of a single rotation.

        Defers UI refresh until batch completion and invalidates shared caches.
        """
        fix_rotation_widget = getattr(
            self.main_window, "fix_rotation_step_widget", None
        )
        if fix_rotation_widget is not None:
            fix_rotation_widget.record_apply_result(file_path, success)

        if success:
            if self.app_state.fix_rotation_results is not None:
                self.app_state.fix_rotation_results.pop(file_path, None)
            # Track for batch processing
            self._pending_rotated_paths.append(file_path)

            # Perform only lightweight cache invalidation (no preview regeneration yet)
            filename = os.path.basename(file_path)
            logger.info(f"Rotation completed for '{filename}' (Lossy: {is_lossy})")
            self.main_window.image_pipeline.invalidate_path(file_path)

    def handle_rotation_application_finished(
        self, successful_count: int, failed_count: int
    ):
        """Handle completion of all rotation applications.

        Refreshes visible UI lazily; previews are generated only when requested.
        """
        logger.info(
            f"Rotation batch finished: {successful_count} successful, {failed_count} failed. "
            f"Processing {len(self._pending_rotated_paths)} rotated images..."
        )
        AppController._finish_rotation_loading_feedback(self)

        try:
            if self._pending_rotated_paths:
                rotated_paths = list(self._pending_rotated_paths)
                self.app_state.invalidate_similarity_for_paths(rotated_paths)
                sync_workflows = getattr(
                    self.main_window,
                    "_sync_workflow_results_after_file_mutation",
                    None,
                )
                if callable(sync_workflows):
                    # Fix Rotation owns the active apply-result lifecycle and
                    # removes successful rows in show_apply_complete().
                    sync_workflows(exclude={"fix_rotation"})
                self.main_window._batch_update_rotated_thumbnails(rotated_paths)

                selected_paths = self.main_window._get_selected_file_paths_from_view()
                if any(path in selected_paths for path in rotated_paths):
                    self.main_window.image_inspection_controller.refresh_paths(
                        rotated_paths
                    )
                    self.main_window.invalidate_last_displayed_preview()
                    self.main_window._handle_file_selection_changed()

        except Exception as e:
            logger.error(
                f"Error during batch post-rotation processing: {e}", exc_info=True
            )
        finally:
            # Clear pending list
            self._pending_rotated_paths.clear()

        # Show summary message
        if successful_count > 0 and failed_count == 0:
            self.main_window.statusBar().showMessage(
                f"Successfully applied {successful_count} rotations.", 5000
            )
        elif successful_count > 0 and failed_count > 0:
            self.main_window.statusBar().showMessage(
                f"Applied {successful_count} rotations successfully, {failed_count} failed.",
                5000,
            )
        elif failed_count > 0:
            self.main_window.statusBar().showMessage(
                f"Failed to apply {failed_count} rotations.", 5000
            )

    def handle_rotation_application_error(self, error_message: str):
        """Handle errors from rotation application worker."""
        logger.error(f"Rotation application error: {error_message}")
        AppController._finish_rotation_loading_feedback(self)
        self.main_window.statusBar().showMessage(
            f"Error applying rotations: {error_message}", 5000
        )
        cancel_transition = getattr(
            self.main_window, "cancel_pending_workflow_transition", None
        )
        if callable(cancel_transition):
            cancel_transition()
