import logging
import os

from PyQt6.QtCore import QObject, pyqtSignal

from core.image_features.model_rotation_detector import ModelNotFoundError
from core.image_features.rotation_detector import RotationDetector

logger = logging.getLogger(__name__)


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
        rotation_detector: RotationDetector,
        parent: QObject | None = None,
    ):
        super().__init__(parent)
        self.image_paths = list(image_paths)
        self.rotation_detector = rotation_detector
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

    def _on_result(self, path: str, angle: int) -> None:
        self._processed += 1
        percent = int((self._processed / max(self._total, 1)) * 100)
        self.progress_update.emit(
            percent,
            f"Checking {os.path.basename(path)}… ({self._processed}/{self._total})",
        )
        if angle != 0:
            self._results[path] = angle

    def _run(self) -> None:
        if not self.image_paths:
            self.completed.emit({})
            return

        self.progress_update.emit(
            0, f"Starting rotation analysis for {self._total} images…"
        )

        self.rotation_detector.detect_rotation_in_batch(
            image_paths=self.image_paths,
            result_callback=self._on_result,
            should_continue_callback=lambda: not self._should_stop,
        )

        if not self._should_stop:
            self.completed.emit(dict(self._results))
