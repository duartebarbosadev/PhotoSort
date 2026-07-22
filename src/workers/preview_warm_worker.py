"""Cancellable folder-wide high-quality preview warming."""

import logging
import threading
from collections.abc import Iterable

from PyQt6.QtCore import QObject, pyqtSignal

from core.image_pipeline import ImagePipeline

logger = logging.getLogger(__name__)


class PreviewWarmWorker(QObject):
    """Populate the shared preview cache for one folder in the background."""

    progress = pyqtSignal(int, int)
    finished = pyqtSignal(int, int)
    error = pyqtSignal(str)

    def __init__(
        self,
        image_pipeline: ImagePipeline,
        image_paths: Iterable[str],
    ) -> None:
        super().__init__()
        self.image_pipeline = image_pipeline
        self.image_paths = tuple(dict.fromkeys(path for path in image_paths if path))
        self._stop_event = threading.Event()
        self._processed = 0

    def stop(self) -> None:
        self._stop_event.set()
        logger.info("Preview warming stop requested")

    def _handle_progress(self, processed: int, total: int) -> None:
        self._processed = processed
        if processed == 1 or processed == total or processed % 20 == 0:
            self.progress.emit(processed, total)

    def run(self) -> None:
        total = len(self.image_paths)
        try:
            self.image_pipeline.preload_previews(
                list(self.image_paths),
                progress_callback=self._handle_progress,
                should_continue_callback=lambda: not self._stop_event.is_set(),
            )
        except Exception as exc:
            logger.error("Folder preview warming failed", exc_info=True)
            self.error.emit(str(exc))
        finally:
            self.finished.emit(self._processed, total)
