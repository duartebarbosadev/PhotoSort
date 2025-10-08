"""
Preview Preloader Worker
Background worker for preloading preview images.
"""

import logging
from PyQt6.QtCore import QObject, pyqtSignal

from core.image_pipeline import ImagePipeline

logger = logging.getLogger(__name__)

# Emit progress updates every N images to reduce signal/slot overhead
PROGRESS_UPDATE_INTERVAL = 10


class PreviewPreloaderWorker(QObject):
    progress_update = pyqtSignal(int, str)
    finished = pyqtSignal()
    error = pyqtSignal(str)

    def __init__(
        self,
        image_paths,
        max_size,
        image_pipeline_instance: ImagePipeline,
        parent=None,
    ):
        super().__init__(parent)
        self._image_paths = image_paths
        self._max_size = max_size
        self._image_pipeline = image_pipeline_instance
        self._is_running = True

    def stop(self):
        self._is_running = False

    def run(self):
        if not self._image_paths:
            self.finished.emit()
            return

        total_images = len(self._image_paths)
        for i, image_path in enumerate(self._image_paths):
            if not self._is_running:
                break

            try:
                # Preload the preview using the pipeline
                self._image_pipeline.get_preview_qpixmap(
                    image_path, max_size=self._max_size, skip_cache=False
                )
            except Exception as e:
                logger.error(f"Error preloading {image_path}: {e}")
                basename = image_path.split("/")[-1]
                self.error.emit(f"Error preloading {basename}: {str(e)}")

            # Emit progress only every PROGRESS_UPDATE_INTERVAL images or at the end
            if (i + 1) % PROGRESS_UPDATE_INTERVAL == 0 or (i + 1) == total_images:
                basename = image_path.split("/")[-1]
                self.progress_update.emit(i + 1, basename)

        self.finished.emit()
