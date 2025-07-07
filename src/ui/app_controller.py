import os
import time
import logging
from typing import List, Dict, Any, Tuple

from PyQt6.QtCore import QObject

from src.core.app_settings import add_recent_folder, get_preview_cache_size_bytes
from src.core.caching.exif_cache import ExifCache
from src.core.file_scanner import SUPPORTED_EXTENSIONS
from src.core.image_file_ops import ImageFileOperations
from src.core.image_pipeline import ImagePipeline
from src.core.image_processing.image_orientation_handler import ImageOrientationHandler

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
        """Clears all application caches."""
        start_time = time.perf_counter()
        logging.info("clear_application_caches - Start")
        
        try:
            pipeline = ImagePipeline()
            pipeline.clear_all_image_caches()
            logging.info("Image pipeline caches cleared.")
        except Exception as e:
            logging.error(f"Error clearing image pipeline caches: {e}")

        try:
            from src.core.similarity_engine import SimilarityEngine
            SimilarityEngine.clear_embedding_cache()
        except Exception as e:
            logging.error(f"Error clearing similarity cache: {e}")
        
        try:
            from src.core.caching.exif_cache import ExifCache
            exif_cache = ExifCache()
            exif_cache.clear()
            logging.info("EXIF metadata cache cleared.")
        except Exception as e:
            logging.error(f"Error clearing EXIF metadata cache: {e}")
        
        try:
            from src.core.caching.rating_cache import RatingCache
            rating_cache = RatingCache()
            rating_cache.clear()
            logging.info("Rating cache cleared.")
        except Exception as e:
            logging.error(f"Error clearing rating cache: {e}")

        logging.info(f"clear_application_caches - End: {time.perf_counter() - start_time:.4f}s")
        
    """
    Manages interactions between the WorkerManager, AppState, and the UI (MainWindow).
    This class handles the logic for loading data, running analyses,
    and responding to worker signals, keeping the MainWindow class cleaner
    and focused on UI presentation.
    """
    def __init__(self, main_window: 'MainWindow', app_state: 'AppState', worker_manager: 'WorkerManager', parent=None):
        super().__init__(parent)
        self.main_window = main_window
        self.app_state = app_state
        self.worker_manager = worker_manager

    def connect_signals(self):
        """Connects signals from the WorkerManager to the controller's slots."""
        # File Scan Worker
        self.worker_manager.file_scan_found_files.connect(self.handle_files_found)
        self.worker_manager.file_scan_finished.connect(self.handle_scan_finished)
        self.worker_manager.file_scan_error.connect(self.handle_scan_error)
        self.worker_manager.file_scan_thumbnail_preload_finished.connect(self.handle_thumbnail_preload_finished)


        # Similarity Worker
        self.worker_manager.similarity_progress.connect(self.handle_similarity_progress)
        self.worker_manager.similarity_embeddings_generated.connect(self.handle_embeddings_generated)
        self.worker_manager.similarity_clustering_complete.connect(self.handle_clustering_complete)
        self.worker_manager.similarity_error.connect(self.handle_similarity_error)

        # Preview Preloader Worker
        self.worker_manager.preview_preload_progress.connect(self.handle_preview_progress)
        self.worker_manager.preview_preload_finished.connect(self.handle_preview_finished)
        self.worker_manager.preview_preload_error.connect(self.handle_preview_error)

        # Blur Detection Worker
        self.worker_manager.blur_detection_progress.connect(self.handle_blur_detection_progress)
        self.worker_manager.blur_detection_status_updated.connect(self.handle_blur_status_updated)
        self.worker_manager.blur_detection_finished.connect(self.handle_blur_detection_finished)
        self.worker_manager.blur_detection_error.connect(self.handle_blur_detection_error)
        
        # Rating Loader Worker
        self.worker_manager.rating_load_progress.connect(self.handle_rating_load_progress)
        self.worker_manager.rating_load_metadata_batch_loaded.connect(self.handle_metadata_batch_loaded)
        self.worker_manager.rating_load_finished.connect(self.handle_rating_load_finished)
        self.worker_manager.rating_load_error.connect(self.handle_rating_load_error)

        # Rotation Detection Worker
        self.worker_manager.rotation_detection_progress.connect(self.handle_rotation_detection_progress)
        self.worker_manager.rotation_detected.connect(self.handle_rotation_detected)
        self.worker_manager.rotation_detection_finished.connect(self.handle_rotation_detection_finished)
        self.worker_manager.rotation_detection_error.connect(self.handle_rotation_detection_error)
        self.worker_manager.rotation_model_not_found.connect(self.handle_rotation_model_not_found)

    # --- Public Methods (called from MainWindow) ---

    def load_folder(self, folder_path: str):
        load_folder_start_time = time.perf_counter()
        logging.info(f"AppController.load_folder - Start for: {folder_path}")
        self.main_window.show_loading_overlay(f"Preparing to scan folder...")

        add_recent_folder(folder_path)
        self.main_window.menu_manager.update_recent_folders_menu()

        estimated_folder_image_size_bytes = self._calculate_folder_image_size(folder_path)
        preview_cache_limit_bytes = get_preview_cache_size_bytes()

        logging.info(f"Folder Size (Images): {estimated_folder_image_size_bytes / (1024*1024):.2f} MB")
        logging.info(f"Preview Cache Limit: {preview_cache_limit_bytes / (1024*1024*1024):.2f} GB")
        logging.info(f"Current Preview Cache Usage: {self.main_window.image_pipeline.preview_cache.volume() / (1024*1024):.2f} MB")

        PREVIEW_ESTIMATED_SIZE_FACTOR = 1.35
        estimated_preview_data_needed_for_folder_bytes = int(estimated_folder_image_size_bytes * PREVIEW_ESTIMATED_SIZE_FACTOR)

        if preview_cache_limit_bytes > 0 and \
           estimated_preview_data_needed_for_folder_bytes > preview_cache_limit_bytes:
            self.main_window.dialog_manager.show_potential_cache_overflow_warning(
                estimated_preview_data_needed_for_folder_bytes,
                preview_cache_limit_bytes
            )
        
        self.worker_manager.stop_all_workers()
        
        self.app_state.clear_all_file_specific_data()
        self.app_state.current_folder_path = folder_path
        folder_display_name = os.path.basename(folder_path) if folder_path else "Selected Folder"
        self.main_window._update_image_info_label(status_message_override=f"Folder: {folder_display_name} | Preparing scan...")
        
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
        
        self.main_window.update_loading_text(f"Scanning folder: {os.path.basename(folder_path)}...")
        self.main_window.menu_manager.open_folder_action.setEnabled(False)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(False)
        self.main_window.menu_manager.detect_blur_action.setEnabled(False)
        self.main_window.menu_manager.auto_rotate_action.setEnabled(False)
        
        logging.info(f"AppController.load_folder - Preparing to call start_file_scan. Total time before call: {time.perf_counter() - load_folder_start_time:.4f}s")
        self.worker_manager.start_file_scan(
            folder_path,
            apply_auto_edits=self.main_window.apply_auto_edits_enabled,
            perform_blur_detection=False,
            blur_threshold=self.main_window.blur_detection_threshold
        )
        logging.info(f"AppController.load_folder - start_file_scan called. Total time for sync part: {time.perf_counter() - load_folder_start_time:.4f}s")

    def start_similarity_analysis(self):
        logging.info("AppController.start_similarity_analysis called.")
        if self.worker_manager.is_similarity_worker_running():
            self.main_window.statusBar().showMessage("Similarity analysis is already in progress.", 3000)
            return
        
        if not self.app_state.image_files_data:
            self.main_window.hide_loading_overlay() 
            self.main_window.statusBar().showMessage("No images loaded to analyze similarity.", 3000)
            return
        
        paths_for_similarity = [fd['path'] for fd in self.app_state.image_files_data]
        if not paths_for_similarity:
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage("No valid image paths for similarity analysis.", 3000)
            return
 
        self.main_window.show_loading_overlay("Starting similarity analysis...")
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(False)
        self.worker_manager.start_similarity_analysis(paths_for_similarity, self.main_window.apply_auto_edits_enabled)

    def start_blur_detection_analysis(self):
        logging.info("AppController.start_blur_detection_analysis called.")
        if not self.app_state.image_files_data:
            self.main_window.statusBar().showMessage("No images loaded to analyze for blurriness.", 3000)
            return
        
        if self.worker_manager.is_blur_detection_running():
            self.main_window.statusBar().showMessage("Blur detection is already in progress.", 3000)
            return
 
        self.main_window.show_loading_overlay("Starting blur detection...")
        self.main_window.menu_manager.detect_blur_action.setEnabled(False)
 
        self.worker_manager.start_blur_detection(
            self.app_state.image_files_data.copy(),
            self.main_window.blur_detection_threshold,
            self.main_window.apply_auto_edits_enabled
        )

    def start_auto_rotation_analysis(self):
        """Start the auto rotation analysis process."""
        logging.info("AppController.start_auto_rotation_analysis called.")
        if not self.app_state.image_files_data:
            self.main_window.statusBar().showMessage("No images loaded to analyze for rotation.", 3000)
            return
        
        if self.worker_manager.is_rotation_detection_running():
            self.main_window.statusBar().showMessage("Rotation detection is already in progress.", 3000)
            return

        self.main_window.show_loading_overlay("Starting rotation analysis...")
        self.main_window.menu_manager.auto_rotate_action.setEnabled(False)
        
        # Initialize the rotation suggestions storage
        self.main_window.rotation_suggestions.clear()
        
        image_paths = [fd['path'] for fd in self.app_state.image_files_data]
        self.worker_manager.start_rotation_detection(image_paths, self.app_state.exif_disk_cache, self.main_window.apply_auto_edits_enabled)
    
    def reload_current_folder(self):
        if self.app_state.image_files_data: 
            if self.app_state.image_files_data[0] and 'path' in self.app_state.image_files_data[0]:
                current_dir = os.path.dirname(self.app_state.image_files_data[0]['path'])
                if os.path.isdir(current_dir):
                    self.load_folder(current_dir)
                    return
        self.main_window.statusBar().showMessage("No folder context to reload.", 3000)

    def move_to_trash(self, file_path: str):
        """Moves a file to the system's trash."""
        success, message = ImageFileOperations.move_to_trash(file_path)
        if not success:
            self.main_window.statusBar().showMessage(message, 5000)

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
                            pass
        except Exception as e:
            logging.error(f"Error calculating folder image size for {folder_path}: {e}")
        return total_size_bytes

    def _start_preview_preloader(self, image_data_list: List[Dict[str, any]]):
        logging.info(f"AppController._start_preview_preloader called with {len(image_data_list)} items.")
        if not image_data_list:
            self.main_window.hide_loading_overlay()
            return
        
        paths_for_preloader = [fd['path'] for fd in image_data_list if fd and isinstance(fd, dict) and 'path' in fd]
        
        if not paths_for_preloader:
            self.main_window.hide_loading_overlay()
            return
 
        self.main_window.update_loading_text(f"Preloading previews ({len(paths_for_preloader)} images)...")
        self.worker_manager.start_preview_preload(paths_for_preloader, self.main_window.apply_auto_edits_enabled)

    # --- Slots for WorkerManager Signals ---
    
    def handle_files_found(self, batch_of_file_data: List[Dict[str, any]]):
        self.app_state.image_files_data.extend(batch_of_file_data)
        self.main_window.update_loading_text(f"Scanning... {len(self.app_state.image_files_data)} images found")
        self.main_window._update_image_info_label()

    def handle_scan_finished(self):
        self.main_window.update_loading_text("Scan finished. Populating view and starting background loads...")
        self.main_window.menu_manager.open_folder_action.setEnabled(True)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(bool(self.app_state.image_files_data))
        self.main_window.menu_manager.detect_blur_action.setEnabled(bool(self.app_state.image_files_data))
        self.main_window.menu_manager.auto_rotate_action.setEnabled(bool(self.app_state.image_files_data))
        
        self.main_window._rebuild_model_view()
        
        if self.app_state.image_files_data:
            self.main_window.update_loading_text("Loading Exiftool data...")
            self.worker_manager.start_rating_load(
                self.app_state.image_files_data.copy(),
                self.app_state.rating_disk_cache,
                self.app_state
            )
        else:
            self.main_window.hide_loading_overlay()
        
        self.main_window._update_image_info_label()

    def handle_scan_error(self, message: str):
        self.main_window.statusBar().showMessage(f"Scan Error: {message}")
        self.main_window.menu_manager.open_folder_action.setEnabled(True)
        
        error_folder_display = "N/A"
        if self.app_state.current_folder_path:
            error_folder_display = os.path.basename(self.app_state.current_folder_path)
            if not error_folder_display: error_folder_display = self.app_state.current_folder_path
        self.main_window._update_image_info_label(status_message_override=f"Folder: {error_folder_display} | Scan error.")
        
        self.main_window.hide_loading_overlay()

    def handle_rating_load_progress(self, current: int, total: int, basename: str):
        percentage = int((current / total) * 100) if total > 0 else 0
        self.main_window.update_loading_text(f"Loading ratings: {percentage}% ({current}/{total}) - {basename}")

    def handle_metadata_batch_loaded(self, metadata_batch: List[Tuple[str, Dict[str, Any]]]):
        currently_selected_paths = self.main_window._get_selected_file_paths_from_view()
        needs_active_selection_refresh = False
        
        for image_path, metadata in metadata_batch:
            if not metadata: continue

            for viewer in self.main_window.advanced_image_viewer.image_viewers:
                if viewer.isVisible() and viewer._file_path == image_path:
                    viewer.update_rating_display(metadata.get('rating', 0))

            if image_path in currently_selected_paths:
                needs_active_selection_refresh = True
        
        if needs_active_selection_refresh:
            self.main_window._handle_file_selection_changed()
            
        self.main_window._apply_filter()

    def handle_rating_load_finished(self):
        logging.info("AppController.handle_rating_load_finished: Received RatingLoaderWorker.finished signal.")
        self.main_window.statusBar().showMessage("Background rating loading finished.", 3000)
        
        if not self.app_state.image_files_data:
            self.main_window.hide_loading_overlay()
            return

        self.main_window.update_loading_text("Ratings loaded. Preloading previews...")
        self._start_preview_preloader(self.app_state.image_files_data.copy())

    def handle_rating_load_error(self, message: str):
        logging.error(f"Rating Load Error: {message}")
        self.main_window.statusBar().showMessage(f"Rating Load Error: {message}", 5000)
        if self.app_state.image_files_data:
            self.main_window.update_loading_text("Rating load errors. Preloading previews...")
            self._start_preview_preloader(self.app_state.image_files_data.copy())
        else:
            self.main_window.hide_loading_overlay()

    def handle_preview_progress(self, percentage: int, message: str):
        self.main_window.update_loading_text(message)

    def handle_preview_finished(self):
        auto_edits_status = "enabled" if self.main_window.apply_auto_edits_enabled else "disabled"
        self.main_window.statusBar().showMessage(f"Previews regenerated with Auto RAW edits {auto_edits_status}.", 5000)
        self.main_window.hide_loading_overlay()
        
        if self.app_state.current_folder_path:
            total_image_size_bytes = self._calculate_folder_image_size(self.app_state.current_folder_path)
            preview_cache_size_bytes = self.main_window.image_pipeline.preview_cache.volume()
            logging.info("--- Cache vs. Image Size Diagnostics (Post-Preload) ---")
            logging.info(f"Total Original Image Size: {total_image_size_bytes / (1024*1024):.2f} MB")
            logging.info(f"Final Preview Cache Size: {preview_cache_size_bytes / (1024*1024):.2f} MB")
            if total_image_size_bytes > 0:
                ratio = (preview_cache_size_bytes / total_image_size_bytes) * 100
                logging.info(f"Cache-to-Image Size Ratio: {ratio:.2f}%")
            logging.info("---------------------------------------------------------")
        
        self.main_window._update_image_info_label()
        
    def handle_preview_error(self, message: str):
        logging.error(f"Preview Preload Error: {message}")
        self.main_window.statusBar().showMessage(f"Preview Preload Error: {message}", 5000)
        self.main_window.hide_loading_overlay()

    def handle_similarity_progress(self, percentage, message):
        self.main_window.update_loading_text(f"Similarity: {message} ({percentage}%)")

    def handle_embeddings_generated(self, embeddings_dict):
        self.app_state.embeddings_cache = embeddings_dict
        self.main_window.update_loading_text("Embeddings generated. Clustering...")

    def handle_clustering_complete(self, cluster_results_dict: Dict[str, int]):
        self.app_state.cluster_results = cluster_results_dict
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(bool(self.app_state.image_files_data))

        if not self.app_state.cluster_results:
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage("Clustering did not produce results.", 3000)
            return

        self.main_window.update_loading_text("Clustering complete. Updating view...")
        cluster_ids = sorted(list(set(self.app_state.cluster_results.values())))
        self.main_window.cluster_filter_combo.clear()
        self.main_window.cluster_filter_combo.addItems(["All Clusters"] + [f"Cluster {cid}" for cid in cluster_ids])
        self.main_window.cluster_filter_combo.setEnabled(True)
        self.main_window.menu_manager.group_by_similarity_action.setEnabled(True)
        self.main_window.menu_manager.group_by_similarity_action.setChecked(True)
        if self.main_window.menu_manager.group_by_similarity_action.isChecked() and self.app_state.cluster_results:
            self.main_window.menu_manager.cluster_sort_action.setVisible(True)
            self.main_window.cluster_sort_combo.setEnabled(True)
        if self.main_window.group_by_similarity_mode: self.main_window._rebuild_model_view()
        self.main_window.hide_loading_overlay()

    def handle_similarity_error(self, message):
        self.main_window.statusBar().showMessage(f"Similarity Error: {message}", 8000)
        self.main_window.menu_manager.analyze_similarity_action.setEnabled(bool(self.app_state.image_files_data))
        self.main_window.hide_loading_overlay()

    def handle_blur_detection_progress(self, current: int, total: int, path_basename: str):
        percentage = int((current / total) * 100) if total > 0 else 0
        self.main_window.update_loading_text(f"Detecting blur: {percentage}% ({current}/{total}) - {path_basename}")

    def handle_blur_status_updated(self, image_path: str, is_blurred: bool):
        self.app_state.update_blur_status(image_path, is_blurred) 
        self.main_window._update_item_blur_status(image_path, is_blurred)

    def handle_blur_detection_finished(self):
        self.main_window.hide_loading_overlay()
        self.main_window.statusBar().showMessage("Blur detection complete.", 5000)
        self.main_window.menu_manager.detect_blur_action.setEnabled(bool(self.app_state.image_files_data))

    def handle_blur_detection_error(self, message: str):
        self.main_window.hide_loading_overlay()
        self.main_window.statusBar().showMessage(f"Blur Detection Error: {message}", 8000)
        self.main_window.menu_manager.detect_blur_action.setEnabled(bool(self.app_state.image_files_data))
        
    def handle_thumbnail_preload_finished(self, all_file_data: List[Dict[str, any]]):
        logging.debug("AppController.handle_thumbnail_preload_finished called (now largely deprecated by rating load chain)")
        pass

    # --- Rotation Detection Handlers ---
    
    def handle_rotation_detection_progress(self, current: int, total: int, path_basename: str):
        """Handle progress updates from rotation detection."""
        percentage = int((current / total) * 100) if total > 0 else 0
        self.main_window.update_loading_text(f"Analyzing rotation: {percentage}% ({current}/{total}) - {path_basename}")

    def handle_rotation_detected(self, image_path: str, suggested_rotation: int):
        """Handle individual rotation detection results."""
        if not hasattr(self.main_window, 'rotation_suggestions'):
            self.main_window.rotation_suggestions = {}
        self.main_window.rotation_suggestions[image_path] = suggested_rotation

    def handle_rotation_detection_finished(self):
        """Handle completion of rotation detection analysis."""
        self.main_window.menu_manager.auto_rotate_action.setEnabled(bool(self.app_state.image_files_data))

        if not self.main_window.rotation_suggestions:
            self.main_window.hide_loading_overlay()
            self.main_window.statusBar().showMessage("Rotation analysis complete. No rotation suggestions found.", 5000)
            return

        logging.info(f"Rotation analysis finished. Received {len(self.main_window.rotation_suggestions)} total suggestions.")

        final_suggestions = {path: rotation for path, rotation in self.main_window.rotation_suggestions.items() if rotation != 0}

        self.main_window.rotation_suggestions = final_suggestions
        self.main_window.hide_loading_overlay()
        self.main_window.left_panel.set_view_mode_rotation()

    def handle_rotation_detection_error(self, message: str):
        """Handle errors during rotation detection."""
        self.main_window.hide_loading_overlay()
        self.main_window.statusBar().showMessage(f"Rotation Detection Error: {message}", 8000)
        self.main_window.menu_manager.auto_rotate_action.setEnabled(bool(self.app_state.image_files_data))

    def handle_rotation_model_not_found(self, model_path: str):
        """Handle the case where the rotation model is not found."""
        self.main_window.hide_loading_overlay()
        self.main_window.dialog_manager.show_model_not_found_dialog(model_path)
        self.main_window.statusBar().showMessage("Rotation model not found. Analysis cancelled.", 5000)
        self.main_window.menu_manager.auto_rotate_action.setEnabled(bool(self.app_state.image_files_data))

    def _apply_approved_rotations(self, approved_rotations: Dict[str, int]):
        """Apply the approved rotations to the images."""
        apply_start_time = time.perf_counter()
        logging.info(f"APPLY_ROT: Start for {len(approved_rotations)} approved rotations.")
        from src.core.metadata_processor import MetadataProcessor
        
        total_rotations = len(approved_rotations)
        successful_rotations = 0
        failed_rotations = 0
        
        self.main_window.show_loading_overlay("Applying rotations...")
        
        for i, (file_path, rotation_degrees) in enumerate(approved_rotations.items(), 1):
            single_file_start_time = time.perf_counter()
            try:
                filename = os.path.basename(file_path)
                logging.info(f"APPLY_ROT: Processing {i}/{total_rotations}: {filename} for {rotation_degrees} degrees.")
                progress_text = f"Rotating {i}/{total_rotations}: {filename}"
                self.main_window.update_loading_text(progress_text)
                
                if rotation_degrees == 90: direction = 'clockwise'
                elif rotation_degrees == -90: direction = 'counterclockwise'
                elif rotation_degrees == 180: direction = '180'
                else:
                    logging.warning(f"APPLY_ROT: Unsupported rotation angle {rotation_degrees} for {filename}")
                    continue
                
                t1 = time.perf_counter()
                metadata_success, needs_lossy, message = MetadataProcessor.try_metadata_rotation_first(
                    file_path, direction, self.main_window.app_state.exif_disk_cache
                )
                t2 = time.perf_counter()
                logging.info(f"APPLY_ROT: try_metadata_rotation_first for {filename} took {t2 - t1:.4f}s. Success: {metadata_success}, Needs Lossy: {needs_lossy}")

                if metadata_success:
                    self._handle_successful_rotation(file_path, direction, f"Rotated {filename} {rotation_degrees}° (lossless)", is_lossy=False)
                    successful_rotations += 1
                elif needs_lossy:
                    logging.info(f"APPLY_ROT: Performing lossy rotation for {filename}")
                    t3 = time.perf_counter()
                    success = MetadataProcessor.rotate_image(
                        file_path, direction, update_metadata_only=False,
                        exif_disk_cache=self.main_window.app_state.exif_disk_cache
                    )
                    t4 = time.perf_counter()
                    logging.info(f"APPLY_ROT: lossy rotate_image for {filename} took {t4 - t3:.4f}s. Success: {success}")
                    
                    if success:
                        self._handle_successful_rotation(file_path, direction, f"Rotated {filename} {rotation_degrees}° (lossy)", is_lossy=True)
                        successful_rotations += 1
                    else:
                        logging.error(f"APPLY_ROT: Failed to perform lossy rotation for {filename}")
                        failed_rotations += 1
                else:
                    logging.error(f"APPLY_ROT: Rotation not supported for {filename}. Message: {message}")
                    failed_rotations += 1
                    
            except Exception as e:
                logging.error(f"APPLY_ROT: Unhandled error rotating {file_path}: {e}", exc_info=True)
                failed_rotations += 1
            finally:
                single_file_end_time = time.perf_counter()
                logging.info(f"APPLY_ROT: Finished processing {os.path.basename(file_path)}. Total time for this file: {single_file_end_time - single_file_start_time:.4f}s")

        self.main_window.hide_loading_overlay()
        
        if successful_rotations > 0 and failed_rotations == 0:
            self.main_window.statusBar().showMessage(f"Successfully applied {successful_rotations} rotations.", 5000)
        elif successful_rotations > 0 and failed_rotations > 0:
            self.main_window.statusBar().showMessage(f"Applied {successful_rotations} rotations successfully, {failed_rotations} failed.", 5000)
        elif failed_rotations > 0:
            self.main_window.statusBar().showMessage(f"Failed to apply {failed_rotations} rotations.", 5000)

        apply_end_time = time.perf_counter()
        logging.info(f"APPLY_ROT: End. Total elapsed time: {apply_end_time - apply_start_time:.4f}s")

    def _handle_successful_rotation(self, file_path: str, direction: str, message: str, is_lossy: bool):
        """Handle successful rotation - update caches and UI."""
        handle_start_time = time.perf_counter()
        filename = os.path.basename(file_path)
        logging.info(f"HSR: Start for {filename}. Lossy: {is_lossy}. Message: {message}")

        t1 = time.perf_counter()
        self.main_window.image_pipeline.preview_cache.delete_all_for_path(file_path)
        self.main_window.image_pipeline.thumbnail_cache.delete_all_for_path(file_path)
        t2 = time.perf_counter()
        logging.info(f"HSR: Cache clearing for {filename} took {t2 - t1:.4f}s.")

        t3 = time.perf_counter()
        proxy_idx = self.main_window._find_proxy_index_for_path(file_path)
        t4 = time.perf_counter()
        logging.info(f"HSR: _find_proxy_index_for_path for {filename} took {t4 - t3:.4f}s.")

        if proxy_idx.isValid():
            source_idx = self.main_window.proxy_model.mapToSource(proxy_idx)
            item = self.main_window.file_system_model.itemFromIndex(source_idx)
            if item:
                t5 = time.perf_counter()
                new_thumbnail = self.main_window.image_pipeline.get_thumbnail_qpixmap(
                    file_path, apply_auto_edits=self.main_window.apply_auto_edits_enabled
                )
                t6 = time.perf_counter()
                logging.info(f"HSR: get_thumbnail_qpixmap for {filename} took {t6 - t5:.4f}s.")
                if new_thumbnail:
                    from PyQt6.QtGui import QIcon
                    item.setIcon(QIcon(new_thumbnail))
                    logging.info(f"HSR: Set new icon for {filename}.")
        
        selected_paths = self.main_window._get_selected_file_paths_from_view()
        if file_path in selected_paths:
            logging.info(f"HSR: {filename} is in current selection, triggering selection changed handler.")
            t7 = time.perf_counter()
            self.main_window._handle_file_selection_changed()
            t8 = time.perf_counter()
            logging.info(f"HSR: _handle_file_selection_changed took {t8 - t7:.4f}s.")
        
        logging.info(message) # Log the original user-facing message
        handle_end_time = time.perf_counter()
        logging.info(f"HSR: End for {filename}. Total time: {handle_end_time - handle_start_time:.4f}s")