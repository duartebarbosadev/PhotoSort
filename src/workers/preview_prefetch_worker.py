"""Cancelable background work for the bounded navigation preview buffer."""

from __future__ import annotations

import logging
from threading import Event
from typing import Iterable

from PyQt6.QtCore import QObject, QRunnable, pyqtSignal

from core.image_pipeline import ImagePipeline

logger = logging.getLogger(__name__)


class PreviewPrefetchSignals(QObject):
    preview_ready = pyqtSignal(str, int)
    preview_failed = pyqtSignal(str, int)
    finished = pyqtSignal(int)


class PreviewPrefetchWorker(QRunnable):
    """Populate preview cache entries without ever creating QPixmaps off-thread."""

    def __init__(
        self,
        image_paths: Iterable[str],
        image_pipeline: ImagePipeline,
        cancel_event: Event,
        request_id: int,
        *,
        force_default_brightness: bool = False,
    ) -> None:
        super().__init__()
        self.image_paths = tuple(image_paths)
        self.image_pipeline = image_pipeline
        self.cancel_event = cancel_event
        self.request_id = request_id
        self.force_default_brightness = force_default_brightness
        self.signals = PreviewPrefetchSignals()

    def run(self) -> None:
        try:
            for image_path in self.image_paths:
                if self.cancel_event.is_set():
                    break
                try:
                    if self.force_default_brightness:
                        ready = self.image_pipeline.ensure_preview_cached(
                            image_path,
                            force_default_brightness=True,
                        )
                    else:
                        ready = self.image_pipeline.ensure_preview_cached(image_path)
                except Exception:
                    logger.error(
                        "Navigation preview generation failed for %s",
                        image_path,
                        exc_info=True,
                    )
                    if not self.cancel_event.is_set():
                        self.signals.preview_failed.emit(image_path, self.request_id)
                    continue
                if ready and not self.cancel_event.is_set():
                    self.signals.preview_ready.emit(image_path, self.request_id)
                elif not self.cancel_event.is_set():
                    self.signals.preview_failed.emit(image_path, self.request_id)
        finally:
            self.signals.finished.emit(self.request_id)
