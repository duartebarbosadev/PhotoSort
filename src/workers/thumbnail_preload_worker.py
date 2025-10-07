"""
Thumbnail Preload Worker
Background worker for preloading image thumbnails without blocking the UI.
"""

import logging
from typing import List
from PyQt6.QtCore import QObject, pyqtSignal

from core.image_pipeline import ImagePipeline

logger = logging.getLogger(__name__)


class ThumbnailPreloadWorker(QObject):
    """Worker for preloading thumbnails in a background thread."""

    # Signals
    progress = pyqtSignal(int, int, str)  # current, total, message
    finished = pyqtSignal()
    error = pyqtSignal(str)

    def __init__(self, image_pipeline: ImagePipeline):
        super().__init__()
        self.image_pipeline = image_pipeline
        self._is_running = True

    def stop(self):
        """Signal the worker to stop processing."""
        self._is_running = False
        logger.info("Thumbnail preload worker stop requested")

    def preload_thumbnails(self, image_paths: List[str]):
        """
        Preload thumbnails for a list of image paths.

        Args:
            image_paths: List of image file paths to preload thumbnails for
        """
        self._is_running = True
        total = len(image_paths)

        if total == 0:
            logger.info("No images to preload thumbnails for")
            self.finished.emit()
            return

        logger.info(f"Starting thumbnail preload for {total} images")

        def progress_callback(current: int, total_count: int):
            """Called by image_pipeline during preloading"""
            if not self._is_running:
                return
            self.progress.emit(current, total_count, f"Preloading thumbnails: {current}/{total_count}")

        def should_continue():
            """Called by image_pipeline to check if preloading should continue"""
            return self._is_running

        try:
            # Use image_pipeline's existing parallel preloading
            self.image_pipeline.preload_thumbnails(
                image_paths,
                progress_callback=progress_callback,
                should_continue_callback=should_continue
            )

            if self._is_running:
                logger.info(f"Thumbnail preload complete for {total} images")
                self.finished.emit()
            else:
                logger.info("Thumbnail preload stopped by user request")

        except Exception as e:
            error_msg = f"Error during thumbnail preload: {e}"
            logger.error(error_msg, exc_info=True)
            self.error.emit(error_msg)
