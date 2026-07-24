"""Prioritized background thumbnail loading for one folder session."""

import concurrent.futures
from collections import deque
import logging
import threading
from collections.abc import Callable, Iterable

from PyQt6.QtCore import QObject, pyqtSignal

from core.image_pipeline import ImagePipeline

logger = logging.getLogger(__name__)


class ThumbnailPreloadWorker(QObject):
    """Warm a folder while allowing viewport paths to jump ahead safely."""

    session_batch_ready = pyqtSignal(str, object)
    session_progress = pyqtSignal(str, int, int, int, bool)
    session_finished = pyqtSignal(str, int, int)
    session_error = pyqtSignal(str, str)

    def __init__(
        self,
        image_pipeline: ImagePipeline,
        *,
        session_id: str = "",
        all_paths: Iterable[str] | None = None,
        foreground_paths: Iterable[str] | None = None,
        should_pause_background: Callable[[], bool] | None = None,
        materialize_background: bool = True,
        max_workers: int | None = None,
    ):
        super().__init__()
        self.image_pipeline = image_pipeline
        self.session_id = session_id
        self._is_running = True
        self._lock = threading.Lock()
        self._wake = threading.Condition(self._lock)
        self._should_pause_background = should_pause_background or (lambda: False)
        self._materialize_background = materialize_background
        configured_workers = getattr(image_pipeline, "thumbnail_worker_count", 4)
        if not isinstance(configured_workers, int):
            configured_workers = 4
        self._max_workers = max(1, max_workers or configured_workers)
        self._background_ready: list[str] = []

        ordered = list(dict.fromkeys(path for path in (all_paths or []) if path))
        foreground = list(
            dict.fromkeys(path for path in (foreground_paths or []) if path)
        )
        foreground_set = set(foreground)
        self._foreground = deque(path for path in foreground if path in ordered)
        self._background = deque(path for path in ordered if path not in foreground_set)
        self._foreground_requested = set(self._foreground)
        self._refresh_only: set[str] = set()
        self._promote_on_complete: set[str] = set()
        self._pending = set(ordered)
        self._inflight: set[str] = set()
        self._total = len(ordered)
        self._attempted = 0
        self._failures = 0

    def stop(self):
        with self._wake:
            self._is_running = False
            self._wake.notify_all()
        logger.info("Thumbnail preload worker stop requested")

    def prioritize(self, image_paths: Iterable[str]) -> None:
        """Move pending paths to the foreground without duplicating work."""
        with self._wake:
            for path in image_paths:
                if path in self._inflight:
                    self._promote_on_complete.add(path)
                elif path not in self._foreground_requested:
                    self._foreground.append(path)
                    self._foreground_requested.add(path)
                    if path not in self._pending:
                        self._refresh_only.add(path)
            self._wake.notify_all()

    def _take_foreground(self, limit: int = 4) -> list[str]:
        paths: list[str] = []
        with self._lock:
            while self._foreground and len(paths) < limit:
                path = self._foreground.popleft()
                self._foreground_requested.discard(path)
                if (
                    path not in self._pending and path not in self._refresh_only
                ) or path in self._inflight:
                    continue
                self._inflight.add(path)
                paths.append(path)
        return paths

    def _take_background(self, limit: int = 4) -> list[str]:
        paths: list[str] = []
        with self._lock:
            while self._background and len(paths) < limit:
                path = self._background.popleft()
                if path in self._foreground_requested:
                    continue
                if path not in self._pending or path in self._inflight:
                    continue
                self._inflight.add(path)
                paths.append(path)
        return paths

    def _record_results(
        self,
        paths: list[str],
        successes: list[str],
        *,
        foreground: bool,
    ) -> None:
        success_set = set(successes)
        failed_this_batch = False
        with self._lock:
            for path in paths:
                self._inflight.discard(path)
                if path in self._refresh_only:
                    self._refresh_only.discard(path)
                    continue
                self._pending.discard(path)
                self._attempted += 1
                if path not in success_set:
                    self._failures += 1
                    failed_this_batch = True
                promote_after_completion = path in self._promote_on_complete
                self._promote_on_complete.discard(path)
                if path in success_set and promote_after_completion:
                    self._foreground.append(path)
                    self._foreground_requested.add(path)
                    self._refresh_only.add(path)
            attempted = self._attempted
            failures = self._failures
        ready_paths: list[str] = []
        if successes and foreground:
            ready_paths = successes
        elif successes and self._materialize_background:
            self._background_ready.extend(successes)
            if len(self._background_ready) >= 20 or attempted == self._total:
                ready_paths = self._background_ready
                self._background_ready = []
        if ready_paths:
            self.session_batch_ready.emit(self.session_id, ready_paths)
        if (
            attempted == 1
            or attempted == self._total
            or attempted % 20 == 0
            or failed_this_batch
        ):
            self.session_progress.emit(
                self.session_id,
                attempted,
                self._total,
                failures,
                False,
            )

    def _ensure(self, path: str, promote_to_memory: bool) -> bool:
        try:
            return self.image_pipeline.ensure_thumbnail_cached(
                path,
                promote_to_memory=promote_to_memory,
            )
        except Exception:
            logger.error("Thumbnail preparation failed for %s", path, exc_info=True)
            return False

    def run_session(self) -> None:
        """Run until the session queue is exhausted or cancellation is requested."""
        if not self.session_id:
            self.session_error.emit("", "Thumbnail session has no identifier")
            self.session_finished.emit("", 0, 0)
            return

        logger.info(
            "Starting thumbnail session %s for %d files (workers=%d)",
            self.session_id,
            self._total,
            self._max_workers,
        )
        paused_emitted = False
        try:
            with concurrent.futures.ThreadPoolExecutor(
                max_workers=self._max_workers
            ) as executor:
                futures: dict[concurrent.futures.Future[bool], tuple[str, bool]] = {}
                while self._is_running:
                    available = self._max_workers - len(futures)
                    foreground = self._take_foreground(available)
                    for path in foreground:
                        future = executor.submit(self._ensure, path, True)
                        futures[future] = (path, True)

                    background_paused = self._should_pause_background()
                    if foreground and paused_emitted:
                        self.session_progress.emit(
                            self.session_id,
                            self._attempted,
                            self._total,
                            self._failures,
                            False,
                        )
                        paused_emitted = False

                    available = self._max_workers - len(futures)
                    if background_paused:
                        if not paused_emitted:
                            self.session_progress.emit(
                                self.session_id,
                                self._attempted,
                                self._total,
                                self._failures,
                                True,
                            )
                            paused_emitted = True
                    elif paused_emitted:
                        self.session_progress.emit(
                            self.session_id,
                            self._attempted,
                            self._total,
                            self._failures,
                            False,
                        )
                        paused_emitted = False

                    foreground_inflight = any(
                        was_foreground for _path, was_foreground in futures.values()
                    )
                    if available and not background_paused and not foreground_inflight:
                        for path in self._take_background(available):
                            future = executor.submit(
                                self._ensure,
                                path,
                                self._materialize_background,
                            )
                            futures[future] = (path, False)

                    with self._lock:
                        has_pending = bool(self._pending)
                    if not futures:
                        if not has_pending:
                            break
                        with self._wake:
                            self._wake.wait(timeout=0.1)
                        continue

                    done, _pending_futures = concurrent.futures.wait(
                        futures,
                        timeout=0.1,
                        return_when=concurrent.futures.FIRST_COMPLETED,
                    )
                    for future in done:
                        path, was_foreground = futures.pop(future)
                        success = future.result()
                        self._record_results(
                            [path],
                            [path] if success else [],
                            foreground=was_foreground,
                        )
        except Exception as exc:
            logger.error("Thumbnail session failed", exc_info=True)
            self.session_error.emit(self.session_id, str(exc))
        finally:
            if self._background_ready:
                self.session_batch_ready.emit(
                    self.session_id,
                    self._background_ready,
                )
                self._background_ready = []
            logger.info(
                "Thumbnail session %s finished: attempted=%d/%d failures=%d",
                self.session_id,
                self._attempted,
                self._total,
                self._failures,
            )
            self.session_finished.emit(
                self.session_id,
                self._attempted,
                self._failures,
            )
