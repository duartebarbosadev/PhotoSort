import glob
import os
import time
import logging
from typing import List, Dict, Any, Tuple
from PyQt6.QtCore import QObject
from core.app_settings import (
    add_recent_folder,
    get_preview_cache_size_bytes,
    PREVIEW_ESTIMATED_SIZE_FACTOR,
)
from core.file_scanner import SUPPORTED_EXTENSIONS
from core.image_file_ops import ImageFileOperations
from core.ai.best_photo_selector import DEFAULT_MODELS_ROOT


logger = logging.getLogger(__name__)


# Forward declarations for type hinting to avoid circular imports.
class MainWindow:
    pass


class AppState:
    pass


class WorkerManager:
    pass


class AppController(QObject):
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
        main_window: "MainWindow",
        app_state: "AppState",
        worker_manager: "WorkerManager",
        parent=None,
    ):
        super().__init__(parent)
        self.main_window = main_window
        self.app_state = app_state
        self.worker_manager = worker_manager

        # Track pending rotations for batch preview regeneration
        self._pending_rotated_paths: List[str] = []

    def connect_signals(self):
        """Connects signals from the WorkerManager to the controller's slots."""
        # File Scan Worker
        self.worker_manager.file_scan_found_files.connect(self.handle_files_found)
        self.worker_manager.file_scan_finished.connect(self.handle_scan_finished)
        self.worker_manager.file_scan_error.connect(self.handle_scan_error)

        # Similarity Worker
        self.worker_manager.similarity_progress.connect(self.handle_similarity_progress)
        self.worker_manager.similarity_embeddings_generated.connect(
            self.handle_embeddings_generated
        )
        self.worker_manager.similarity_clustering_complete.connect(
            self.handle_clustering_complete
        )
        self.worker_manager.similarity_error.connect(self.handle_similarity_error)

        # Preview Preloader Worker
        self.worker_manager.preview_preload_progress.connect(
            self.handle_preview_progress
        )
        self.worker_manager.preview_preload_finished.connect(
            self.handle_preview_finished
        )
        self.worker_manager.preview_preload_error.connect(self.handle_preview_error)

        # Blur Detection Worker
        self.worker_manager.blur_detection_progress.connect(
            self.handle_blur_detection_progress
        )
        self.worker_manager.blur_detection_status_updated.connect(
            self.handle_blur_status_updated
        )
        self.worker_manager.blur_detection_finished.connect(
            self.handle_blur_detection_finished
        )
        self.worker_manager.blur_detection_error.connect(
            self.handle_blur_detection_error
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

        # Rotation Detection Worker
        self.worker_manager.rotation_detection_progress.connect(
            self.handle_rotation_detection_progress
        )
        self.worker_manager.rotation_detected.connect(self.handle_rotation_detected)
        self.worker_manager.rotation_detection_finished.connect(
            self.handle_rotation_detection_finished
        )
        self.worker_manager.rotation_detection_error.connect(
            self.handle_rotation_detection_error
        )
        self.worker_manager.rotation_model_not_found.connect(
            self.handle_rotation_model_not_found
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

        # Thumbnail Preload Worker
        self.worker_manager.thumbnail_preload_progress.connect(
            self.handle_thumbnail_preload_progress
        )
        self.worker_manager.thumbnail_preload_finished.connect(
            self.handle_thumbnail_preload_complete
        )
        self.worker_manager.thumbnail_preload_error.connect(
            self.handle_thumbnail_preload_error
        )

        # Best Shot Worker
        self.worker_manager.best_shot_progress.connect(
            self.handle_best_shot_progress
        )
        self.worker_manager.best_shot_complete.connect(
            self.handle_best_shot_complete
        )
        self.worker_manager.best_shot_error.connect(self.handle_best_shot_error)
        self.worker_manager.best_shot_models_missing.connect(
            self.handle_best_shot_models_missing
        )

    # --- Public Methods (called from MainWindow) ---

    def load_folder(self, folder_path: str):
        load_folder_start_time = time.perf_counter()
        logger.info("Loading folder: %s", folder_path)
        self.main_window.show_loading_overlay("Preparing to scan folder...")

        add_recent_folder(folder_path)
        self.main_window.menu_manager.update_recent_folders_menu()

        estimated_folder_image_size_bytes = self._calculate_folder_image_size(
            folder_path
        )
        preview_cache_limit_bytes = get_preview_cache_size_bytes()

        logger.debug(
            "Folder Size: %.2f MB",
            estimated_folder_image_size_bytes / (1024 * 1024),
        )
        logger.debug(
            "Preview Cache Limit: %.2f GB",
            preview_cache_limit_bytes / (1024 * 1024 * 1024),
        )
        logger.debug(
            "Current Preview Cache Usage: %.2f MB",
            self.main_window.image_pipeline.preview_cache.volume() / (1024 * 1024),
        )

        # Remove hardcoded factor, now using centralized constant
        estimated_preview_data_needed_for_folder_bytes = int(
            estimated_folder_image_size_bytes * PREVIEW_ESTIMATED_SIZE_FACTOR
        )

        if (
            preview_cache_limit_bytes > 0
            and estimated_preview_data_needed_for_folder_bytes
            > preview_cache_limit_bytes
        ):
            self.main_window.dialog_manager.show_potential_cache_overflow_warning(
                estimated_preview_data_needed_for_folder_bytes,
                preview_cache_limit_bytes,
            )

        self.worker_manager.stop_all_workers()

        self.app_state.clear_all_file_specific_data()
        self.main_window.invalidate_last_displayed_preview()
        self.app_state.current_folder_path = folder_path
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
        self.main_window.menu_manager.cluster_sort_action.setVisible(False)
        self.main_window.cluster_sort_combo.setEnabled(False)
        self.main_window.cluster_sort_combo.setCurrentIndex(0)
        self.main_window.menu_manager.group_by_similarity_action.setEnabled(False)
        self.main_window.menu_manager.group_by_similarity_action.setChecked(False)

        self.main_window.file_system_model.clear()
        self.main_window.file_system_model.setColumnCount(1)

        self.main_window.update_loading_text(
            f"Scanning folder: {os.path.basename(folder_path)}..."
        )
        self.main_window.menu_manager.open_folder_action.setEnabled(False)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(False)
        self.main_window.menu_manager.analyze_best_shots_action.setEnabled(False)
        self.main_window.menu_manager.detect_blur_action.setEnabled(False)
        self.main_window.menu_manager.auto_rotate_action.setEnabled(False)

        logger.debug(
            f"Folder prep complete in {time.perf_counter() - load_folder_start_time:.2f}s. Starting file scan."
        )
        self.worker_manager.start_file_scan(
            folder_path,
            perform_blur_detection=False,
            blur_threshold=self.main_window.blur_detection_threshold,
        )

    def start_similarity_analysis(self):
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

        paths_for_similarity = [fd["path"] for fd in self.app_state.image_files_data]
        if not paths_for_similarity:
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage(
                "No valid image paths for similarity analysis.", 3000
            )
            return

        self.main_window.show_loading_overlay("Starting similarity analysis...")
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(False)
        self.main_window.menu_manager.analyze_best_shots_action.setEnabled(False)
        self.worker_manager.start_similarity_analysis(paths_for_similarity)

    def start_blur_detection_analysis(self):
        logger.info("Starting blur detection analysis.")
        if not self.app_state.image_files_data:
            self.main_window.statusBar().showMessage(
                "No images loaded to analyze for blurriness.", 3000
            )
            return

        if self.worker_manager.is_blur_detection_running():
            self.main_window.statusBar().showMessage(
                "Blur detection is already in progress.", 3000
            )
            return

        self.main_window.show_loading_overlay("Starting blur detection...")
        self.main_window.menu_manager.detect_blur_action.setEnabled(False)

        self.worker_manager.start_blur_detection(
            self.app_state.image_files_data.copy(),
            self.main_window.blur_detection_threshold,
            True,  # Always enable processing for RAW files
        )

    def start_auto_rotation_analysis(self):
        """Start the auto rotation analysis process."""
        logger.info("Starting auto-rotation analysis.")
        if not self.app_state.image_files_data:
            self.main_window.statusBar().showMessage(
                "No images loaded to analyze for rotation.", 3000
            )
            return

        if self.worker_manager.is_rotation_detection_running():
            self.main_window.statusBar().showMessage(
                "Rotation detection is already in progress.", 3000
            )
            return

        self.main_window.show_loading_overlay("Starting rotation analysis...")
        self.main_window.menu_manager.auto_rotate_action.setEnabled(False)

        self.main_window.rotation_suggestions.clear()

        image_paths = [fd["path"] for fd in self.app_state.image_files_data]
        self.worker_manager.start_rotation_detection(
            image_paths,
            self.app_state.exif_disk_cache,
        )

    def _build_cluster_path_map(self) -> Dict[int, List[str]]:
        cluster_map: Dict[int, List[str]] = {}
        for path, cluster_id in self.app_state.cluster_results.items():
            if cluster_id is None:
                continue
            cluster_map.setdefault(cluster_id, []).append(path)
        return cluster_map

    def start_best_shot_analysis(self):
        logger.info("Starting best shot analysis.")
        if self.worker_manager.is_best_shot_worker_running():
            self.main_window.statusBar().showMessage(
                "Best shot analysis is already running.", 3000
            )
            return

        if not self.app_state.cluster_results:
            self.main_window.statusBar().showMessage(
                "Run Analyze Similarity before best shot analysis.", 4000
            )
            return

        cluster_map = self._build_cluster_path_map()
        if not cluster_map:
            self.main_window.statusBar().showMessage(
                "No similarity clusters available for best shot analysis.", 4000
            )
            return

        self.main_window.show_loading_overlay("Analyzing best shots...")
        self.main_window.menu_manager.analyze_best_shots_action.setEnabled(False)
        self.worker_manager.start_best_shot_analysis(cluster_map)

    def start_best_shot_analysis_for_selected(self):
        """Run best-shot analysis on currently selected images only."""
        logger.info("Starting best shot analysis for selected images.")
        
        if self.worker_manager.is_best_shot_worker_running():
            self.main_window.statusBar().showMessage(
                "Best shot analysis is already running.", 3000
            )
            return

        # Get selected images
        selected_paths = self.main_window.get_selected_file_paths()
        if not selected_paths:
            self.main_window.statusBar().showMessage(
                "No images selected. Please select images first.", 3000
            )
            return

        if len(selected_paths) < 2:
            self.main_window.statusBar().showMessage(
                "Please select at least 2 images for comparison.", 3000
            )
            return

        # Create a single "cluster" with ID 0 containing the selected paths
        cluster_map = {0: selected_paths}

        self.main_window.show_loading_overlay(
            f"Analyzing {len(selected_paths)} selected images..."
        )
        self.main_window.menu_manager.analyze_best_shots_selected_action.setEnabled(
            False
        )
        self.worker_manager.start_best_shot_analysis(cluster_map)

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

    def move_to_trash(self, file_path: str):
        """Moves a file to the system's trash."""
        logger.info(f"Moving file to trash: {os.path.basename(file_path)}")
        success, message = ImageFileOperations.move_to_trash(file_path)
        if not success:
            logger.error(
                f"Failed to move file to trash: {os.path.basename(file_path)} - {message}"
            )
            self.main_window.statusBar().showMessage(message, 5000)
        else:
            logger.info(
                f"Successfully moved file to trash: {os.path.basename(file_path)}"
            )

    def rename_image(self, old_path: str, new_path: str):
        """Renames an image file."""
        success, message = ImageFileOperations.rename_image(old_path, new_path)
        if not success:
            self.main_window.statusBar().showMessage(message, 5000)

    # --- Private Helper Methods ---

    def _calculate_folder_image_size(self, folder_path: str) -> int:
        total_size_bytes = 0
        try:
            for root, _, files in os.walk(folder_path):
                for filename in files:
                    ext = os.path.splitext(filename)[1].lower()
                    if ext in SUPPORTED_EXTENSIONS:
                        try:
                            full_path = os.path.join(root, filename)
                            total_size_bytes += os.path.getsize(full_path)
                        except OSError:
                            pass  # Ignore files that can't be accessed
        except Exception:
            logger.error(
                f"Error calculating folder image size for {folder_path}", exc_info=True
            )
        return total_size_bytes

    def _start_preview_preloader(self, image_data_list: List[Dict[str, any]]):
        logger.info(f"Starting preview preloader for {len(image_data_list)} images.")
        if not image_data_list:
            self.main_window.hide_loading_overlay()
            return

        paths_for_preloader = [
            fd["path"]
            for fd in image_data_list
            if fd and isinstance(fd, dict) and "path" in fd
        ]

        if not paths_for_preloader:
            self.main_window.hide_loading_overlay()
            return

        self.main_window.update_loading_text(
            f"Preloading previews ({len(paths_for_preloader)} images)..."
        )
        self.worker_manager.start_preview_preload(
            paths_for_preloader,
        )

    # --- Slots for WorkerManager Signals ---

    def handle_files_found(self, batch_of_file_data: List[Dict[str, any]]):
        self.app_state.image_files_data.extend(batch_of_file_data)
        self.main_window.update_loading_text(
            f"Scanning... {len(self.app_state.image_files_data)} images found"
        )
        self.main_window._update_image_info_label()

    def handle_scan_finished(self):
        self.main_window.update_loading_text(
            "Scan finished. Populating view and starting background loads..."
        )
        self.main_window.menu_manager.open_folder_action.setEnabled(True)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(
            bool(self.app_state.image_files_data)
        )
        self.main_window.menu_manager.analyze_best_shots_selected_action.setEnabled(
            bool(self.app_state.image_files_data)
        )
        self.main_window.menu_manager.detect_blur_action.setEnabled(
            bool(self.app_state.image_files_data)
        )
        self.main_window.menu_manager.auto_rotate_action.setEnabled(
            bool(self.app_state.image_files_data)
        )
        self.main_window.menu_manager.group_by_similarity_action.setEnabled(
            bool(self.app_state.image_files_data)
        )

        self.main_window._rebuild_model_view()

        if self.app_state.image_files_data:
            # Start thumbnail preload in background (non-blocking)
            image_paths = [item["path"] for item in self.app_state.image_files_data]
            self.worker_manager.start_thumbnail_preload(image_paths)

            self.main_window.update_loading_text("Loading Exiftool data...")
            self.worker_manager.start_rating_load(
                self.app_state.image_files_data.copy(),
                self.app_state.rating_disk_cache,
                self.app_state,
            )
        else:
            self.main_window.hide_loading_overlay()

        self.main_window._update_image_info_label()

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

    def handle_rating_load_progress(self, current: int, total: int, basename: str):
        percentage = int((current / total) * 100) if total > 0 else 0
        self.main_window.update_loading_text(
            f"Loading ratings: {percentage}% ({current}/{total}) - {basename}"
        )

    def handle_metadata_batch_loaded(
        self, metadata_batch: List[Tuple[str, Dict[str, Any]]]
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
        logger.info("Rating loading finished. Starting preview preloading.")
        self.main_window.statusBar().showMessage(
            "Background rating loading finished.", 3000
        )

        if not self.app_state.image_files_data:
            self.main_window.hide_loading_overlay()
            return

        self.main_window.update_loading_text("Ratings loaded. Preloading previews...")
        self._start_preview_preloader(self.app_state.image_files_data.copy())

    def handle_rating_load_error(self, message: str):
        logger.error(f"Rating load failed: {message}", exc_info=True)
        self.main_window.statusBar().showMessage(f"Rating Load Error: {message}", 5000)
        if self.app_state.image_files_data:
            self.main_window.update_loading_text(
                "Rating load errors. Preloading previews..."
            )
            self._start_preview_preloader(self.app_state.image_files_data.copy())
        else:
            self.main_window.hide_loading_overlay()

    def handle_preview_progress(self, percentage: int, message: str):
        self.main_window.update_loading_text(message)

    def handle_preview_finished(self):
        self.main_window.statusBar().showMessage("Previews regenerated.", 5000)
        self.main_window.hide_loading_overlay()

        if self.app_state.current_folder_path:
            total_image_size_bytes = self._calculate_folder_image_size(
                self.app_state.current_folder_path
            )
            preview_cache_size_bytes = (
                self.main_window.image_pipeline.preview_cache.volume()
            )
            logger.debug("--- Cache vs. Image Size Diagnostics (Post-Preload) ---")
            logger.debug(
                f"Total Original Image Size: {total_image_size_bytes / (1024 * 1024):.2f} MB"
            )
            logger.debug(
                f"Final Preview Cache Size: {preview_cache_size_bytes / (1024 * 1024):.2f} MB"
            )
            if total_image_size_bytes > 0:
                ratio = (preview_cache_size_bytes / total_image_size_bytes) * 100
                logger.debug(f"Cache-to-Image Size Ratio: {ratio:.2f}%")
            logger.debug("---------------------------------------------------------")

        self.main_window._update_image_info_label()

    def handle_preview_error(self, message: str):
        logger.error(f"Preview preload failed: {message}", exc_info=True)
        self.main_window.statusBar().showMessage(
            f"Preview Preload Error: {message}", 5000
        )
        self.main_window.hide_loading_overlay()

    def handle_similarity_progress(self, percentage, message):
        self.main_window.update_loading_text(f"Similarity: {message} ({percentage}%)")

    def handle_embeddings_generated(self, embeddings_dict):
        self.app_state.embeddings_cache = embeddings_dict
        self.main_window.update_loading_text("Embeddings generated. Clustering...")

    def handle_clustering_complete(self, cluster_results_dict: Dict[str, int]):
        self.app_state.cluster_results = cluster_results_dict
        self.app_state.clear_best_shot_results()
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(
            bool(self.app_state.image_files_data)
        )

        if not self.app_state.cluster_results:
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage(
                "Clustering did not produce results.", 3000
            )
            return

        self.main_window.update_loading_text("Clustering complete. Updating view...")
        cluster_ids = sorted(list(set(self.app_state.cluster_results.values())))
        self.main_window.cluster_filter_combo.clear()
        self.main_window.cluster_filter_combo.addItems(
            ["All Clusters"] + [f"Cluster {cid}" for cid in cluster_ids]
        )
        self.main_window.cluster_filter_combo.setEnabled(True)
        self.main_window.menu_manager.group_by_similarity_action.setChecked(True)
        if (
            self.main_window.menu_manager.group_by_similarity_action.isChecked()
            and self.app_state.cluster_results
        ):
            self.main_window.menu_manager.cluster_sort_action.setVisible(True)
            self.main_window.cluster_sort_combo.setEnabled(True)
        self.main_window.menu_manager.analyze_best_shots_action.setEnabled(True)
        if self.main_window.group_by_similarity_mode:
            self.main_window._rebuild_model_view()
        self.main_window.hide_loading_overlay()

    def handle_similarity_error(self, message):
        logger.error(f"Similarity analysis failed: {message}", exc_info=True)
        self.main_window.statusBar().showMessage(f"Similarity Error: {message}", 8000)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(
            bool(self.app_state.image_files_data)
        )
        self.main_window.menu_manager.analyze_best_shots_action.setEnabled(False)
        self.main_window.hide_loading_overlay()

    def handle_best_shot_progress(self, percentage: int, message: str):
        self.main_window.update_loading_text(
            f"Best shots: {message} ({percentage}%)"
        )

    def handle_best_shot_complete(
        self, rankings_by_cluster: Dict[int, List[Dict[str, Any]]]
    ):
        self.app_state.set_best_shot_results(rankings_by_cluster or {})
        self.main_window.hide_loading_overlay()
        analyzed = len(rankings_by_cluster or {})
        self.main_window.statusBar().showMessage(
            f"Best shot analysis complete for {analyzed} group(s).", 4000
        )
        self.main_window.menu_manager.analyze_best_shots_action.setEnabled(
            bool(self.app_state.cluster_results)
        )
        self.main_window.menu_manager.analyze_best_shots_selected_action.setEnabled(
            True
        )
        self.main_window._rebuild_model_view()

    def handle_best_shot_error(self, message: str):
        logger.error(f"Best shot analysis failed: {message}", exc_info=True)
        self.main_window.hide_loading_overlay()
        self.main_window.statusBar().showMessage(
            f"Best shot analysis error: {message}", 8000
        )
        self.main_window.menu_manager.analyze_best_shots_action.setEnabled(
            bool(self.app_state.cluster_results)
        )
        self.main_window.menu_manager.analyze_best_shots_selected_action.setEnabled(
            True
        )

    def handle_best_shot_models_missing(self, missing_models: list):
        """Handle the case where best-shot models are not found."""
        self.main_window.hide_loading_overlay()
        self.main_window.dialog_manager.show_best_shot_models_missing_dialog(
            missing_models
        )
        self.main_window.statusBar().showMessage(
            "Best shot models not found. Analysis cancelled.", 5000
        )
        self.main_window.menu_manager.analyze_best_shots_action.setEnabled(
            bool(self.app_state.cluster_results)
        )
        self.main_window.menu_manager.analyze_best_shots_selected_action.setEnabled(
            True
        )

    def handle_blur_detection_progress(
        self, current: int, total: int, path_basename: str
    ):
        percentage = int((current / total) * 100) if total > 0 else 0
        self.main_window.update_loading_text(
            f"Detecting blur: {percentage}% ({current}/{total}) - {path_basename}"
        )

    def handle_blur_status_updated(self, image_path: str, is_blurred: bool):
        self.app_state.update_blur_status(image_path, is_blurred)
        self.main_window._update_item_blur_status(image_path, is_blurred)

    def handle_blur_detection_finished(self):
        self.main_window.hide_loading_overlay()
        self.main_window.statusBar().showMessage("Blur detection complete.", 5000)
        self.main_window.menu_manager.detect_blur_action.setEnabled(
            bool(self.app_state.image_files_data)
        )

    def handle_blur_detection_error(self, message: str):
        logger.error(f"Blur detection failed: {message}", exc_info=True)
        self.main_window.hide_loading_overlay()
        self.main_window.statusBar().showMessage(
            f"Blur Detection Error: {message}", 8000
        )
        self.main_window.menu_manager.detect_blur_action.setEnabled(
            bool(self.app_state.image_files_data)
        )

    # --- Rotation Detection Handlers ---

    def handle_rotation_detection_progress(
        self, current: int, total: int, path_basename: str
    ):
        """Handle progress updates from rotation detection."""
        percentage = int((current / total) * 100) if total > 0 else 0
        self.main_window.update_loading_text(
            f"Analyzing rotation: {percentage}% ({current}/{total}) - {path_basename}"
        )

    def handle_rotation_detected(self, image_path: str, suggested_rotation: int):
        """Handle individual rotation detection results."""
        if not hasattr(self.main_window, "rotation_suggestions"):
            self.main_window.rotation_suggestions = {}
        self.main_window.rotation_suggestions[image_path] = suggested_rotation

    def handle_rotation_detection_finished(self):
        """Handle completion of rotation detection analysis."""
        self.main_window.menu_manager.auto_rotate_action.setEnabled(
            bool(self.app_state.image_files_data)
        )

        if not self.main_window.rotation_suggestions:
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage(
                "Rotation analysis complete. No rotation suggestions found.", 5000
            )
            return

        logger.info(
            f"Rotation analysis finished with {len(self.main_window.rotation_suggestions)} suggestions."
        )

        final_suggestions = {
            path: rotation
            for path, rotation in self.main_window.rotation_suggestions.items()
            if rotation != 0
        }

        # Mutate the existing dict so the RotationController keeps the same reference
        self.main_window.rotation_suggestions.clear()
        self.main_window.rotation_suggestions.update(final_suggestions)
        self.main_window.hide_loading_overlay()

        if not self.main_window.rotation_suggestions:
            self.main_window.statusBar().showMessage(
                "Rotation analysis complete. No rotation suggestions found.", 5000
            )
            return

        num_suggestions = len(self.main_window.rotation_suggestions)
        logger.info(f"Displaying rotation view with {num_suggestions} suggestions.")
        self.main_window.statusBar().showMessage(
            f"Rotation analysis finished. Please review the {num_suggestions} suggestions.",
            5000,
        )

        self.main_window.left_panel.view_rotation_icon.setVisible(True)
        self.main_window.left_panel.set_view_mode_rotation()

    def handle_rotation_detection_error(self, message: str):
        """Handle errors during rotation detection."""
        logger.error(f"Rotation detection failed: {message}", exc_info=True)
        self.main_window.hide_loading_overlay()
        self.main_window.statusBar().showMessage(
            f"Rotation Detection Error: {message}", 8000
        )
        self.main_window.menu_manager.auto_rotate_action.setEnabled(
            bool(self.app_state.image_files_data)
        )

    def handle_rotation_model_not_found(self, model_path: str):
        """Handle the case where the rotation model is not found."""
        self.main_window.hide_loading_overlay()
        self.main_window.dialog_manager.show_model_not_found_dialog(model_path)
        self.main_window.statusBar().showMessage(
            "Rotation model not found. Analysis cancelled.", 5000
        )
        self.main_window.menu_manager.auto_rotate_action.setEnabled(
            bool(self.app_state.image_files_data)
        )

    def _apply_approved_rotations(self, approved_rotations: Dict[str, int]):
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

        # Show loading overlay
        self.main_window.show_loading_overlay("Applying rotations...")

        # Start the worker
        self.worker_manager.start_rotation_application(
            approved_rotations=approved_rotations,
            exif_disk_cache=self.app_state.exif_disk_cache,
        )

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

        # If no more rotation suggestions, hide the rotation view
        if not self.main_window.rotation_suggestions:
            self.main_window._hide_rotation_view()

    # --- Rating Writer Handlers ---

    def apply_rating_to_selection(self, rating: int, selected_paths: List[str]):
        """Apply rating to multiple images using background worker."""
        if not selected_paths:
            return

        # Create list of (path, rating) tuples for the worker
        rating_operations = [(path, rating) for path in selected_paths]

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
        else:
            self.main_window.statusBar().showMessage(
                f"Setting rating to {rating} for {len(selected_paths)} images...", 2000
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

        Defers UI-heavy updates (preview regeneration, selection refresh) until batch completion.
        Only performs lightweight cache invalidation here.
        """
        if success:
            # Track for batch processing
            self._pending_rotated_paths.append(file_path)

            # Perform only lightweight cache invalidation (no preview regeneration yet)
            filename = os.path.basename(file_path)
            logger.info(f"Rotation completed for '{filename}' (Lossy: {is_lossy})")
            self.main_window.image_pipeline.preview_cache.delete_all_for_path(file_path)
            self.main_window.image_pipeline.thumbnail_cache.delete_all_for_path(
                file_path
            )

    def handle_rotation_application_finished(
        self, successful_count: int, failed_count: int
    ):
        """Handle completion of all rotation applications.

        Performs batch preview regeneration in parallel, then updates UI.
        """
        logger.info(
            f"Rotation batch finished: {successful_count} successful, {failed_count} failed. "
            f"Processing {len(self._pending_rotated_paths)} rotated images..."
        )

        try:
            if self._pending_rotated_paths:
                # Update loading overlay for preview regeneration
                overlay_text = f"Regenerating previews for {len(self._pending_rotated_paths)} images..."
                self.main_window.show_loading_overlay(overlay_text)

                # Batch regenerate previews in parallel using existing infrastructure
                t1 = time.perf_counter()
                self.main_window.image_pipeline.preload_previews(
                    self._pending_rotated_paths
                )
                t2 = time.perf_counter()
                logger.info(
                    f"Batch preview regeneration for {len(self._pending_rotated_paths)} images "
                    f"completed in {t2 - t1:.2f}s"
                )

                # Batch update thumbnails in UI
                t3 = time.perf_counter()
                self.main_window._batch_update_rotated_thumbnails(
                    self._pending_rotated_paths
                )
                t4 = time.perf_counter()
                logger.info(f"Batch thumbnail update completed in {t4 - t3:.2f}s")

                # Single selection refresh if any rotated images are in current selection
                selected_paths = self.main_window._get_selected_file_paths_from_view()
                if any(path in selected_paths for path in self._pending_rotated_paths):
                    t5 = time.perf_counter()
                    self.main_window.invalidate_last_displayed_preview()
                    self.main_window._handle_file_selection_changed()
                    t6 = time.perf_counter()
                    logger.info(f"Selection refresh completed in {t6 - t5:.2f}s")

        except Exception as e:
            logger.error(
                f"Error during batch post-rotation processing: {e}", exc_info=True
            )
        finally:
            # Clear pending list
            self._pending_rotated_paths.clear()
            self.main_window.hide_loading_overlay()

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
        self.main_window.hide_loading_overlay()
        self.main_window.statusBar().showMessage(
            f"Error applying rotations: {error_message}", 5000
        )

    def handle_thumbnail_preload_progress(self, current: int, total: int, message: str):
        """Handle progress updates from thumbnail preload worker."""
        logger.debug(f"Thumbnail preload: {current}/{total} - {message}")
        # Optional: Update status bar or progress indicator
        # For now, just log - thumbnails load silently in background

    def handle_thumbnail_preload_complete(self):
        """Handle completion of thumbnail preload worker."""
        logger.info("Thumbnail preloading completed - updating tree view icons")
        # Update all tree view items with the cached thumbnails
        self.main_window._update_thumbnails_from_cache()

    def handle_thumbnail_preload_error(self, error_message: str):
        """Handle errors from thumbnail preload worker."""
        logger.warning(f"Thumbnail preload error: {error_message}")
        # Non-critical - don't show to user, thumbnails will load on demand
