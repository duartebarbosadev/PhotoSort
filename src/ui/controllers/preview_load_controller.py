"""Coordinates responsive, bounded preview loading during navigation."""

from threading import Event
from collections.abc import Iterable

from PyQt6.QtCore import QObject, QThreadPool, pyqtSignal

from core.image_pipeline import ImagePipeline
from workers.preview_prefetch_worker import PreviewPrefetchWorker
from workers.detail_prefetch_worker import DetailPrefetchWorker

DETAIL_DISPLAY_BUDGET_BYTES = 512 * 1024 * 1024


class PreviewLoadController(QObject):
    """Keep only the newest navigation preview request alive."""

    preview_ready = pyqtSignal(str)
    preview_failed = pyqtSignal(str)
    detail_ready = pyqtSignal(str, object)
    detail_failed = pyqtSignal(str)

    def __init__(
        self,
        image_pipeline: ImagePipeline,
        parent: QObject | None = None,
    ) -> None:
        super().__init__(parent)
        self._image_pipeline = image_pipeline
        self._pool = QThreadPool(self)
        self._pool.setMaxThreadCount(1)
        self._detail_pool = QThreadPool(self)
        self._detail_pool.setMaxThreadCount(1)
        self._request_id = 0
        self._cancel_event: Event | None = None
        self._primary_path: str | None = None
        self._requested_paths: tuple[str, ...] = ()
        self._force_default_brightness = False
        self._detail_request_id = 0
        self._detail_cancel_event: Event | None = None
        self._detail_requested_paths: tuple[str, ...] = ()

    def request(
        self,
        image_paths: Iterable[str],
        *,
        force_default_brightness: bool = False,
    ) -> None:
        ordered_paths = tuple(dict.fromkeys(path for path in image_paths if path))
        if not ordered_paths:
            return

        primary_path = ordered_paths[0]
        if (
            primary_path in self._requested_paths
            and set(ordered_paths).issubset(self._requested_paths)
            and self._cancel_event is not None
            and not self._cancel_event.is_set()
            and force_default_brightness == self._force_default_brightness
        ):
            return

        self.cancel()
        self._request_id += 1
        request_id = self._request_id
        cancel_event = Event()
        self._cancel_event = cancel_event
        self._primary_path = primary_path
        self._requested_paths = ordered_paths
        self._force_default_brightness = force_default_brightness

        worker = PreviewPrefetchWorker(
            ordered_paths,
            self._image_pipeline,
            cancel_event,
            request_id,
            force_default_brightness=force_default_brightness,
        )
        worker.signals.preview_ready.connect(self._handle_preview_ready)
        worker.signals.preview_failed.connect(self._handle_preview_failed)
        worker.signals.finished.connect(self._handle_finished)
        self._pool.start(worker)

    def request_details(
        self,
        image_paths: Iterable[str],
        *,
        max_display_bytes: int = DETAIL_DISPLAY_BUDGET_BYTES,
    ) -> None:
        ordered_paths = tuple(dict.fromkeys(path for path in image_paths if path))
        if not ordered_paths:
            return
        if (
            ordered_paths == self._detail_requested_paths
            and self._detail_cancel_event is not None
            and not self._detail_cancel_event.is_set()
        ):
            return

        self.cancel_details()
        self._detail_request_id += 1
        request_id = self._detail_request_id
        cancel_event = Event()
        self._detail_cancel_event = cancel_event
        self._detail_requested_paths = ordered_paths
        worker = DetailPrefetchWorker(
            ordered_paths,
            self._image_pipeline,
            cancel_event,
            request_id,
            max_display_bytes=max_display_bytes,
        )
        worker.signals.detail_ready.connect(self._handle_detail_ready)
        worker.signals.detail_failed.connect(self._handle_detail_failed)
        worker.signals.finished.connect(self._handle_detail_finished)
        self._detail_pool.start(worker)

    def cancel(self) -> None:
        if self._cancel_event is not None:
            self._cancel_event.set()
        # Drop queued stale work. A decode already in progress exits after its
        # current file, and its result is ignored using the request id.
        self._pool.clear()

    def cancel_details(self) -> None:
        if self._detail_cancel_event is not None:
            self._detail_cancel_event.set()
        self._detail_pool.clear()
        self._detail_request_id += 1
        self._detail_cancel_event = None
        self._detail_requested_paths = ()

    def reset(self) -> None:
        self.cancel()
        self._primary_path = None
        self._requested_paths = ()
        self._force_default_brightness = False
        self.cancel_details()
        self._detail_requested_paths = ()

    def shutdown(self) -> None:
        self.reset()
        self._pool.waitForDone(5000)
        self._detail_pool.waitForDone(5000)

    def _handle_preview_ready(self, image_path: str, request_id: int) -> None:
        if request_id == self._request_id:
            self.preview_ready.emit(image_path)

    def _handle_preview_failed(self, image_path: str, request_id: int) -> None:
        if request_id == self._request_id:
            self.preview_failed.emit(image_path)

    def _handle_finished(self, request_id: int) -> None:
        if request_id == self._request_id:
            self._cancel_event = None

    def _handle_detail_ready(self, image_path: str, image, request_id: int) -> None:
        if request_id == self._detail_request_id:
            self.detail_ready.emit(image_path, image)

    def _handle_detail_failed(self, image_path: str, request_id: int) -> None:
        if request_id == self._detail_request_id:
            self.detail_failed.emit(image_path)

    def _handle_detail_finished(self, request_id: int) -> None:
        if request_id == self._detail_request_id:
            self._detail_cancel_event = None
