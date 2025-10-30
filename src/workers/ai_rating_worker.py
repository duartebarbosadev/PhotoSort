import logging
import os
import time
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Dict, Optional, Sequence, TYPE_CHECKING

from PyQt6.QtCore import QObject, pyqtSignal

if TYPE_CHECKING:
    from core.image_pipeline import ImagePipeline

from core.ai.best_shot_pipeline import (
    BaseBestShotStrategy,
    BestShotEngine,
    LLMConfig,
    create_best_shot_strategy,
)
from core.app_settings import calculate_max_workers, get_best_shot_engine

logger = logging.getLogger(__name__)


class AiRatingWorker(QObject):
    """Background worker that requests AI ratings (1-5) for images."""

    progress_update = pyqtSignal(int, str)
    completed = pyqtSignal(object)  # Emits Dict[str, Dict[str, object]]
    error = pyqtSignal(str)
    warning = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        image_paths: Sequence[str],
        models_root: Optional[str] = None,
        image_pipeline: Optional["ImagePipeline"] = None,
        max_workers: Optional[int] = None,
        engine: Optional[str] = None,
        llm_config: Optional[LLMConfig] = None,
        strategy: Optional[BaseBestShotStrategy] = None,
        parent: Optional[QObject] = None,
        max_retries: int = 3,
        retry_delay_seconds: float = 2.0,
    ) -> None:
        super().__init__(parent)
        self.image_paths = list(image_paths)
        self.models_root = models_root
        self._image_pipeline = image_pipeline
        self._strategy = strategy
        self._engine = (engine or get_best_shot_engine() or "local").lower()
        self._llm_config = llm_config
        self._max_workers = max_workers or calculate_max_workers(
            min_workers=2, max_workers=6
        )
        self._should_stop = False
        self._max_retries = max(1, int(max_retries))
        self._retry_delay = max(0.0, float(retry_delay_seconds))

    def stop(self) -> None:
        self._should_stop = True

    def _ensure_strategy(self) -> None:
        if self._strategy is None:
            self._strategy = create_best_shot_strategy(
                self._engine,
                models_root=self.models_root,
                image_pipeline=self._image_pipeline,
                llm_config=self._llm_config,
            )
            if self._strategy.max_workers:
                self._max_workers = min(
                    self._max_workers, max(1, self._strategy.max_workers)
                )
        try:
            self._strategy.validate_connection()
        except Exception as exc:
            logger.error("AI rating strategy validation failed: %s", exc, exc_info=True)
            raise

    def _rate_single(self, image_path: str) -> Optional[Dict[str, object]]:
        if self._should_stop:
            return None
        if self._strategy is None:
            raise RuntimeError("AI rating strategy not initialised")
        attempts = 0
        last_error: Optional[Exception] = None
        while attempts < self._max_retries and not self._should_stop:
            attempts += 1
            try:
                return self._strategy.rate_image(image_path)
            except Exception as exc:  # pragma: no cover - network failures
                last_error = exc
                if attempts < self._max_retries and not self._should_stop:
                    delay = self._retry_delay * attempts
                    warning_msg = (
                        f"Retry {attempts}/{self._max_retries} for {os.path.basename(image_path)}: {exc}"
                    )
                    logger.warning(warning_msg)
                    self.warning.emit(warning_msg)
                    if delay > 0:
                        time.sleep(delay)
        if last_error is not None:
            raise RuntimeError(
                f"Failed to rate {image_path} after {self._max_retries} attempt(s): {last_error}"
            ) from last_error
        return None

    def run(self) -> None:
        try:
            from core.ai.model_checker import check_best_shot_models
        except Exception as exc:  # pragma: no cover
            logger.error("Failed to import model checker: %s", exc, exc_info=True)
            self.error.emit(str(exc))
            self.finished.emit()
            return

        try:
            if self._engine == BestShotEngine.LOCAL.value:
                missing_models = check_best_shot_models(self.models_root)
                if missing_models:
                    logger.warning(
                        "AI rating aborted: %d model(s) missing", len(missing_models)
                    )
                    self.error.emit(
                        "Required best-shot models missing; install models or switch engine."
                    )
                    self.finished.emit()
                    return
            self._ensure_strategy()
        except Exception as exc:
            logger.error("Failed to initialise AI rating strategy: %s", exc, exc_info=True)
            self.error.emit(str(exc))
            self.finished.emit()
            return

        try:
            total = len(self.image_paths)
            if total == 0:
                self.completed.emit({})
                return

            logger.info(f"Starting AI rating of {total} images using {self._engine} engine")
            results: Dict[str, Dict[str, object]] = {}
            failures: list[tuple[str, str]] = []

            with ThreadPoolExecutor(max_workers=self._max_workers) as executor:
                futures = {
                    executor.submit(self._rate_single, path): path
                    for path in self.image_paths
                }

                processed = 0
                for future in as_completed(futures):
                    if self._should_stop:
                        logger.info("AI rating stop requested; skipping remaining results.")
                        break
                    path = futures[future]
                    try:
                        rating_data = future.result()
                    except Exception as exc:
                        message = f"AI rating failed for {os.path.basename(path)}: {exc}"
                        logger.error("%s", message, exc_info=True)
                        failures.append((path, str(exc)))
                        self.warning.emit(message)
                        processed += 1
                        percent = int((processed / total) * 100)
                        self.progress_update.emit(percent, f"Rated {processed}/{total}")
                        continue

                    if rating_data:
                        results[path] = rating_data
                    processed += 1
                    percent = int((processed / total) * 100)
                    self.progress_update.emit(percent, f"Rated {processed}/{total}")

            if not self._should_stop:
                logger.info(f"AI rating completed: {len(results)} successful, {len(failures)} failed")
                if failures:
                    failed_count = len(failures)
                    sample = ", ".join(
                        os.path.basename(path) for path, _ in failures[:3]
                    )
                    summary = (
                        f"AI rating skipped {failed_count} image(s)."
                        + (f" Examples: {sample}" if sample else "")
                    )
                    self.warning.emit(summary)
                self.completed.emit(results)
        finally:
            if self._strategy is not None:
                try:
                    self._strategy.shutdown()
                except Exception:  # pragma: no cover
                    logger.warning("AI rating strategy shutdown failed", exc_info=True)
            self.finished.emit()
