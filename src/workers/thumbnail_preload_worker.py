"""Prioritized background thumbnail loading for one folder session."""

import concurrent.futures
from collections import deque
import logging
import threading
import time
from collections.abc import Callable, Iterable

from PyQt6.QtCore import QObject, pyqtSignal

from core.image_pipeline import ImagePipeline
from core.caching.preview_cache import PreviewCacheCapacityError
from core.image_processing.raw_image_processor import is_raw_extension

logger = logging.getLogger(__name__)
PROGRESS_EMIT_INTERVAL_SECONDS = 0.25


class ThumbnailPreloadWorker(QObject):
    """Prepare canonical review assets while allowing paths to jump ahead."""

    session_batch_ready = pyqtSignal(str, object)
    session_progress = pyqtSignal(str, int, int, int, bool)
    session_finished = pyqtSignal(str, int, int)
    session_error = pyqtSignal(str, str)
    session_capacity_required = pyqtSignal(str, int)
    session_metrics = pyqtSignal(str, object)

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
        self._encoded_bytes = 0
        self._cache_hits = 0
        self._raw_decode_count = 0
        self._capacity_condition = threading.Condition()
        self._capacity_waiting = False
        self._capacity_waiters = 0
        self._capacity_decision: bool | None = None
        self._capacity_cancelled = False
        self._last_progress_emit_at = 0.0

    def stop(self):
        with self._wake:
            self._is_running = False
            self._wake.notify_all()
        with self._capacity_condition:
            self._capacity_decision = False
            self._capacity_condition.notify_all()
        logger.info("Thumbnail preload worker stop requested")

    def resolve_capacity_request(self, approved: bool) -> None:
        """Resume blocked decoders after the UI raises capacity, or cancel them."""
        with self._capacity_condition:
            self._capacity_decision = bool(approved)
            if not approved:
                self._capacity_cancelled = True
            self._capacity_condition.notify_all()

    def _wait_for_capacity(self, required_bytes: int) -> bool:
        emit_request = False
        with self._capacity_condition:
            if not self._capacity_waiting:
                self._capacity_waiting = True
                self._capacity_decision = None
                emit_request = True
            self._capacity_waiters += 1

        if emit_request:
            self.session_capacity_required.emit(self.session_id, required_bytes)

        with self._capacity_condition:
            while self._capacity_decision is None and self._is_running:
                self._capacity_condition.wait(timeout=0.25)
            approved = bool(self._capacity_decision) and self._is_running
            self._capacity_waiters -= 1
            if self._capacity_waiters == 0:
                self._capacity_waiting = False
                self._capacity_decision = None
            return approved

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
        now = time.monotonic()
        should_emit_progress = (
            attempted == 1
            or attempted == self._total
            or attempted % 20 == 0
            or failed_this_batch
            or now - self._last_progress_emit_at >= PROGRESS_EMIT_INTERVAL_SECONDS
        )
        if should_emit_progress:
            self._last_progress_emit_at = now
            self.session_progress.emit(
                self.session_id,
                attempted,
                self._total,
                failures,
                False,
            )

    def _ensure(self, path: str, promote_to_memory: bool) -> bool:
        while self._is_running:
            try:
                result = self.image_pipeline.ensure_review_assets_cached(
                    path,
                    promote_to_memory=promote_to_memory,
                )
                if result.success:
                    with self._lock:
                        encoded_bytes = int(getattr(result, "encoded_bytes", 0))
                        cache_hit = bool(getattr(result, "cache_hit", False))
                        self._encoded_bytes += encoded_bytes
                        self._cache_hits += int(cache_hit)
                        extension = path.rsplit(".", 1)[-1] if "." in path else ""
                        self._raw_decode_count += int(
                            not cache_hit
                            and is_raw_extension(f".{extension.lower()}")
                        )
                return result.success
            except PreviewCacheCapacityError as exc:
                if not self._wait_for_capacity(exc.required_bytes):
                    return False
            except Exception:
                logger.error(
                    "Review-asset preparation failed for %s", path, exc_info=True
                )
                return False
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
        started_at = time.perf_counter()
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
            elapsed = max(0.000001, time.perf_counter() - started_at)
            metrics = {
                "attempted": self._attempted,
                "failures": self._failures,
                "cache_hits": self._cache_hits,
                "cache_hit_rate": (
                    self._cache_hits / self._attempted if self._attempted else 0.0
                ),
                "raw_decode_count": self._raw_decode_count,
                "encoded_bytes": self._encoded_bytes,
                "elapsed_seconds": elapsed,
                "throughput_per_second": self._attempted / elapsed,
            }
            logger.info(
                "Review session %s finished: attempted=%d/%d failures=%d "
                "cache_hits=%d raw_decodes=%d throughput=%.2f/s",
                self.session_id,
                self._attempted,
                self._total,
                self._failures,
                self._cache_hits,
                self._raw_decode_count,
                metrics["throughput_per_second"],
            )
            self.session_metrics.emit(self.session_id, metrics)
            if not self._capacity_cancelled:
                self.session_finished.emit(
                    self.session_id,
                    self._attempted,
                    self._failures,
                )
