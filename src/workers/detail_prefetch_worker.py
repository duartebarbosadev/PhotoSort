"""Cancellable source-detail decoding for interactive inspection."""

import concurrent.futures
import logging
import math
from threading import Event
from collections.abc import Iterable

from PyQt6.QtCore import QObject, QRunnable, pyqtSignal

from core.image_pipeline import ImagePipeline

logger = logging.getLogger(__name__)

MAX_PARALLEL_DETAIL_DECODES = 4


class DetailPrefetchSignals(QObject):
    detail_ready = pyqtSignal(str, object, int)
    detail_failed = pyqtSignal(str, int)
    finished = pyqtSignal(int)


class DetailPrefetchWorker(QRunnable):
    """Decode up to four visible images concurrently under one pixel budget."""

    def __init__(
        self,
        image_paths: Iterable[str],
        image_pipeline: ImagePipeline,
        cancel_event: Event,
        request_id: int,
        *,
        max_display_bytes: int,
    ) -> None:
        super().__init__()
        self.image_paths = tuple(dict.fromkeys(path for path in image_paths if path))
        self.image_pipeline = image_pipeline
        self.cancel_event = cancel_event
        self.request_id = request_id
        self.max_display_bytes = max_display_bytes
        self.signals = DetailPrefetchSignals()

    def _target_sizes(self) -> dict[str, tuple[int, int] | None]:
        dimensions = {
            path: self.image_pipeline.get_source_dimensions(path)
            for path in self.image_paths
        }
        known = [size for size in dimensions.values() if size is not None]
        total_bytes = sum(width * height * 4 for width, height in known)
        scale = (
            min(1.0, math.sqrt(self.max_display_bytes / total_bytes))
            if total_bytes > 0
            else 1.0
        )
        return {
            path: (
                (max(1, int(size[0] * scale)), max(1, int(size[1] * scale)))
                if scale < 1.0
                else None
            )
            for path, size in dimensions.items()
            if size is not None
        }

    def run(self) -> None:
        try:
            target_sizes = self._target_sizes()
            pending_paths: list[str] = []
            for path in self.image_paths:
                if path not in target_sizes:
                    if not self.cancel_event.is_set():
                        self.signals.detail_failed.emit(path, self.request_id)
                    continue
                pending_paths.append(path)

            if self.cancel_event.is_set() or not pending_paths:
                return

            worker_count = min(MAX_PARALLEL_DETAIL_DECODES, len(pending_paths))
            path_iterator = iter(pending_paths)
            with concurrent.futures.ThreadPoolExecutor(
                max_workers=worker_count,
                thread_name_prefix="detail-decode",
            ) as executor:
                futures: dict[concurrent.futures.Future, str] = {}

                def submit_until_full() -> None:
                    while (
                        len(futures) < worker_count and not self.cancel_event.is_set()
                    ):
                        try:
                            path = next(path_iterator)
                        except StopIteration:
                            return
                        future = executor.submit(
                            self._decode_detail,
                            path,
                            target_sizes[path],
                        )
                        futures[future] = path

                submit_until_full()
                while futures:
                    done, _pending = concurrent.futures.wait(
                        futures,
                        return_when=concurrent.futures.FIRST_COMPLETED,
                    )
                    for future in done:
                        path = futures.pop(future)
                        image = future.result()
                        if self.cancel_event.is_set():
                            break
                        if image is None:
                            self.signals.detail_failed.emit(path, self.request_id)
                        else:
                            self.signals.detail_ready.emit(path, image, self.request_id)
                    if self.cancel_event.is_set():
                        for future in futures:
                            future.cancel()
                        break
                    submit_until_full()
        finally:
            self.signals.finished.emit(self.request_id)

    def _decode_detail(
        self,
        path: str,
        target_size: tuple[int, int] | None,
    ):
        if self.cancel_event.is_set():
            return None
        try:
            return self.image_pipeline.load_detail_image(path, target_size)
        except Exception:
            logger.error("Detail decode failed for %s", path, exc_info=True)
            return None
