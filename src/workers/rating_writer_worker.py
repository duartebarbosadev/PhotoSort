"""
Rating Writer Worker
Background worker for writing image ratings to metadata without blocking the UI.
"""

import logging
import os
from typing import List, Tuple, Optional
from PyQt6.QtCore import QObject, pyqtSignal

from core.metadata_processor import MetadataProcessor
from core.caching.rating_cache import RatingCache
from core.caching.exif_cache import ExifCache

logger = logging.getLogger(__name__)


class RatingWriterWorker(QObject):
    """Worker for writing ratings to image metadata in a background thread."""

    # Signals
    progress = pyqtSignal(int, int, str)  # current, total, filename
    rating_written = pyqtSignal(str, int, bool)  # path, rating, success
    finished = pyqtSignal(int, int)  # successful_count, failed_count
    error = pyqtSignal(str)

    def __init__(
        self,
        rating_disk_cache: Optional[RatingCache] = None,
        exif_disk_cache: Optional[ExifCache] = None,
    ):
        super().__init__()
        self.rating_disk_cache = rating_disk_cache
        self.exif_disk_cache = exif_disk_cache
        self._is_running = True

    def stop(self):
        """Signal the worker to stop processing."""
        self._is_running = False

    def write_ratings(self, rating_operations: List[Tuple[str, int]]):
        """
        Write ratings to multiple images.

        Args:
            rating_operations: List of tuples (file_path, rating)
        """
        self._is_running = True
        total_operations = len(rating_operations)
        successful_count = 0
        failed_count = 0

        logger.info(f"Starting batch rating write for {total_operations} images")

        try:
            for i, (file_path, rating) in enumerate(rating_operations, 1):
                if not self._is_running:
                    logger.info("Rating writer stopped by user request")
                    break

                if not os.path.exists(file_path):
                    logger.warning(f"File not found, skipping: {file_path}")
                    failed_count += 1
                    continue

                filename = os.path.basename(file_path)
                self.progress.emit(i, total_operations, filename)

                try:
                    success = MetadataProcessor.set_rating(
                        file_path,
                        rating,
                        self.rating_disk_cache,
                        self.exif_disk_cache,
                    )

                    if success:
                        successful_count += 1
                        logger.debug(f"Successfully set rating {rating} for {filename}")
                    else:
                        failed_count += 1
                        logger.warning(f"Failed to set rating for {filename}")

                    self.rating_written.emit(file_path, rating, success)

                except Exception as e:
                    failed_count += 1
                    logger.error(f"Error setting rating for {filename}: {e}", exc_info=True)
                    self.rating_written.emit(file_path, rating, False)

            if self._is_running:
                logger.info(
                    f"Rating write complete: {successful_count} successful, {failed_count} failed"
                )
                self.finished.emit(successful_count, failed_count)

        except Exception as e:
            error_msg = f"Unexpected error in rating writer: {e}"
            logger.error(error_msg, exc_info=True)
            self.error.emit(error_msg)
