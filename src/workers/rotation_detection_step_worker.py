import logging
import os
from concurrent.futures import Future, ThreadPoolExecutor, as_completed
from typing import Protocol

from PyQt6.QtCore import QObject, pyqtSignal

from core.app_settings import ROTATION_MODEL_IMAGE_SIZE, calculate_max_workers
from core.image_features.model_rotation_detector import (
    ModelNotFoundError,
    ModelRotationDetector,
)
from core.image_pipeline import ImagePipeline

logger = logging.getLogger(__name__)


class _RotationModel(Protocol):
    def predict_rotation_angle(self, image_path: str, image: object | None = None) -> int: ...


class RotationDetectionStepWorker(QObject):
    """Detects wrongly-rotated images for the Fix Rotation step.

    Emits completed with {path: angle} for all images where angle != 0.
    """

    progress_update = pyqtSignal(int, str)
    completed = pyqtSignal(dict)  # {path: angle_degrees}  (only non-zero)
    model_not_found = pyqtSignal(str)
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        image_paths: list[str],
        image_pipeline: ImagePipeline,
        model_detector: _RotationModel | None = None,
        num_workers: int | None = None,
        parent: QObject | None = None,
    ):
        super().__init__(parent)
        self.image_paths = list(image_paths)
        self.image_pipeline = image_pipeline
        self.model_detector = model_detector or ModelRotationDetector()
        self.num_workers = num_workers or calculate_max_workers(
            min_workers=4, max_workers=8
        )
        self._should_stop = False
        self._results: dict[str, int] = {}
        self._processed = 0
        self._total = len(image_paths)

    def stop(self) -> None:
        self._should_stop = True

    def run(self) -> None:
        try:
            self._run()
        except ModelNotFoundError as exc:
            logger.warning(f"Rotation model not found: {exc}")
            self.model_not_found.emit(str(exc))
        except Exception as exc:
            logger.error("RotationDetectionStepWorker: unexpected error", exc_info=True)
            self.error.emit(str(exc))
        finally:
            self.finished.emit()

    def _record_result(self, path: str, angle: int) -> None:
        self._processed += 1
        percent = int((self._processed / max(self._total, 1)) * 100)
        self.progress_update.emit(
            percent,
            f"Checking {os.path.basename(path)}… ({self._processed}/{self._total})",
        )
        if angle != 0:
            self._results[path] = angle

    def _detect_rotation(self, path: str) -> int:
        model_input_size = ROTATION_MODEL_IMAGE_SIZE + 32
        image = self.image_pipeline.get_analysis_image(
            path,
            target_size=(model_input_size, model_input_size),
        )
        if image is None:
            return 0
        return self.model_detector.predict_rotation_angle(path, image=image)

    def _run(self) -> None:
        if not self.image_paths:
            self.completed.emit({})
            return

        self.progress_update.emit(
            0, f"Starting rotation analysis for {self._total} images…"
        )

        executor = ThreadPoolExecutor(max_workers=self.num_workers)
        futures: dict[Future[int], str] = {}
        try:
            futures = {
                executor.submit(self._detect_rotation, path): path
                for path in self.image_paths
                if not self._should_stop
            }
            for future in as_completed(futures):
                if self._should_stop:
                    break
                path = futures[future]
                try:
                    angle = future.result()
                except ModelNotFoundError:
                    for pending in futures:
                        pending.cancel()
                    raise
                except Exception:
                    logger.error(
                        "Fix Rotation failed to analyze %s",
                        os.path.basename(path),
                        exc_info=True,
                    )
                    angle = 0
                self._record_result(path, angle)
        finally:
            if self._should_stop:
                for pending in futures:
                    pending.cancel()
            # ONNX inference cannot be interrupted once it has entered the runtime.
            # On cancellation, release the workflow thread immediately and let only
            # already-running calls finish in the executor; their results are stale
            # and are never emitted. A normal completion still joins every task.
            executor.shutdown(
                wait=not self._should_stop,
                cancel_futures=self._should_stop,
            )

        if not self._should_stop:
            self.completed.emit(dict(self._results))
