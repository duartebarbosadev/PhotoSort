import os
import time
import logging

logger = logging.getLogger(__name__)
from PyQt6.QtCore import QObject, pyqtSignal
from typing import List, Dict, Any, Optional

from src.core.metadata_processor import MetadataProcessor
from src.core.caching.rating_cache import RatingCache
from src.core.app_settings import METADATA_EMIT_BATCH_SIZE
from src.ui.app_state import AppState

# DEFAULT_METADATA_WORKERS is no longer needed as batching is handled by MetadataProcessor


class RatingLoaderWorker(QObject):
    """
    Worker to load ratings and other essential metadata for images in the background.
    Metadata fetching is now done in a single batch call.
    """

    progress_update = pyqtSignal(int, int, str)  # current, total, basename
    # Emit a batch of metadata dictionaries
    metadata_batch_loaded = pyqtSignal(
        list
    )  # List of tuples: [(image_path, metadata_dict), ...]
    finished = pyqtSignal()
    error = pyqtSignal(str)

    def __init__(
        self,
        image_data_list: List[Dict[str, Any]],  # Expects list of dicts with 'path'
        rating_disk_cache: RatingCache,
        app_state: AppState,
        parent: Optional[QObject] = None,
    ):
        super().__init__(parent)
        self._image_data_list = image_data_list
        self._rating_disk_cache = rating_disk_cache
        self._app_state = app_state
        self._is_running = True

    def stop(self):
        self._is_running = False
        logger.info("Stop requested.")

    def run_load(self):
        self._is_running = True
        image_paths_to_process = [
            fd["path"]
            for fd in self._image_data_list
            if fd
            and isinstance(fd, dict)
            and "path" in fd
            and os.path.isfile(fd["path"])
        ]
        total_files = len(image_paths_to_process)
        processed_count = 0

        if not image_paths_to_process:
            logger.warning("No valid image paths to process.")
            self.finished.emit()
            return

        if not self._rating_disk_cache:
            self.error.emit("Rating disk cache is not available.")
            self.finished.emit()
            return
        if not self._app_state or not self._app_state.exif_disk_cache:
            self.error.emit("Application state or EXIF disk cache is not available.")
            self.finished.emit()
            return

        total_load_start_time = time.perf_counter()
        logger.info(f"Starting metadata load for {total_files} files.")

        try:
            # Single batch call to the refactored MetadataHandler
            batch_results = MetadataProcessor.get_batch_display_metadata(
                image_paths_to_process,
                self._rating_disk_cache,
                self._app_state.exif_disk_cache,
            )

            # Use centralized batch size constant
            PROGRESS_EMIT_INTERVAL = (
                20  # Emit progress every 20 files, or if it's the last one
            )

            metadata_batch_to_emit = []

            for i, image_path_norm in enumerate(image_paths_to_process):
                if not self._is_running:
                    logger.info(f"Processing stopped by request at index {i}.")
                    break

                metadata = batch_results.get(image_path_norm)
                basename = os.path.basename(image_path_norm)
                processed_count += 1

                current_metadata_tuple = None
                if metadata:
                    # Update AppState's in-memory caches directly here
                    self._app_state.rating_cache[image_path_norm] = metadata.get(
                        "rating", 0
                    )
                    if metadata.get("date"):
                        self._app_state.date_cache[image_path_norm] = metadata["date"]
                    else:
                        self._app_state.date_cache.pop(image_path_norm, None)
                    current_metadata_tuple = (image_path_norm, metadata)
                else:
                    logger.warning(
                        f"No metadata returned for {os.path.basename(image_path_norm)} from batch call."
                    )
                    # Still add to batch for UI to know it was processed, with default values
                    current_metadata_tuple = (
                        image_path_norm,
                        {"rating": 0, "date": None},
                    )

                if current_metadata_tuple:
                    metadata_batch_to_emit.append(current_metadata_tuple)

                if (
                    len(metadata_batch_to_emit) >= METADATA_EMIT_BATCH_SIZE
                    or processed_count == total_files
                ):
                    if metadata_batch_to_emit:
                        logger.debug(
                            f"Emitting metadata batch with {len(metadata_batch_to_emit)} items."
                        )
                        self.metadata_batch_loaded.emit(
                            list(metadata_batch_to_emit)
                        )  # Emit a copy
                        metadata_batch_to_emit.clear()

                if (
                    processed_count % PROGRESS_EMIT_INTERVAL == 0
                    or processed_count == total_files
                    or processed_count == 1
                ):
                    self.progress_update.emit(processed_count, total_files, basename)

                logger.debug(f"Processed {processed_count}/{total_files}: {basename}")

            # Ensure any remaining items in metadata_batch_to_emit are sent
            if metadata_batch_to_emit:
                logger.debug(
                    f"Emitting final metadata batch with {len(metadata_batch_to_emit)} items."
                )
                self.metadata_batch_loaded.emit(list(metadata_batch_to_emit))
                metadata_batch_to_emit.clear()

        except Exception as e:
            error_msg = f"An error occurred during metadata loading: {e}"
            logger.error(error_msg, exc_info=True)
            self.error.emit(error_msg)

        total_load_duration = time.perf_counter() - total_load_start_time
        avg_time_per_file = total_load_duration / total_files if total_files > 0 else 0
        logger.info(
            f"Finished metadata processing for {processed_count}/{total_files} files in {total_load_duration:.2f}s."
        )

        try:
            logger.debug("Emitting finished signal.")
            self.finished.emit()
            logger.debug("Finished signal emitted.")
        except Exception as e_finish:
            logger.error(
                f"Exception during finish sequence: {e_finish}",
                exc_info=True,
            )
            self.error.emit(f"Exception in finish sequence: {e_finish}")
        finally:
            logger.debug("Exiting run_load method.")
