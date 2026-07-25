import logging
import os
from collections.abc import Callable, Iterable
from dataclasses import dataclass, field

from PyQt6.QtCore import QObject, pyqtSignal

from core.caching.path_cache_ops import delete_cached_paths
from core.image_file_ops import ImageFileOperations

logger = logging.getLogger(__name__)


@dataclass(slots=True)
class FileDeletionResult:
    successful_targets: list[str] = field(default_factory=list)
    failures: dict[str, str] = field(default_factory=dict)


class FileDeletionWorker(QObject):
    """Move a deletion batch to Trash and invalidate its disk-cache keys."""

    progress = pyqtSignal(int, int, str)
    completed = pyqtSignal(object)
    finished = pyqtSignal()

    def __init__(
        self,
        targets: Iterable[str],
        *,
        cache_paths_by_target: dict[str, list[str]] | None = None,
        rating_cache=None,
        exif_cache=None,
        analysis_cache=None,
        folder_path: str | None = None,
        trash_operation: Callable[[str], tuple[bool, str]] | None = None,
    ) -> None:
        super().__init__()
        self.targets = list(dict.fromkeys(path for path in targets if path))
        self.cache_paths_by_target = {
            path: list(dict.fromkeys(paths))
            for path, paths in (cache_paths_by_target or {}).items()
        }
        self.rating_cache = rating_cache
        self.exif_cache = exif_cache
        self.analysis_cache = analysis_cache
        self.folder_path = folder_path
        self.trash_operation = trash_operation or ImageFileOperations.move_to_trash
        self._should_stop = False

    def stop(self) -> None:
        self._should_stop = True

    def run(self) -> None:
        result = FileDeletionResult()
        total = len(self.targets)
        deleted_cache_paths: set[str] = set()
        try:
            for current, target in enumerate(self.targets, start=1):
                if self._should_stop:
                    result.failures.update(
                        {
                            pending: "Deletion cancelled."
                            for pending in self.targets[current - 1 :]
                        }
                    )
                    break
                self.progress.emit(current, total, os.path.basename(target) or target)
                try:
                    success, message = self.trash_operation(target)
                except Exception as exc:
                    success, message = False, str(exc)
                if not success:
                    result.failures[target] = message or "Could not move to Trash."
                    continue
                result.successful_targets.append(target)
                deleted_cache_paths.update(
                    self.cache_paths_by_target.get(target, [target])
                )
                try:
                    delete_cached_paths(
                        self.cache_paths_by_target.get(target, [target]),
                        rating_cache=self.rating_cache,
                        exif_cache=self.exif_cache,
                    )
                except Exception:
                    # The file mutation succeeded, so cache cleanup must never
                    # turn it into a reported deletion failure.
                    logger.warning(
                        "Failed to invalidate disk caches after trashing %s",
                        target,
                        exc_info=True,
                    )
            if (
                deleted_cache_paths
                and self.analysis_cache is not None
                and self.folder_path
            ):
                try:
                    self.analysis_cache.remove_paths(
                        self.folder_path,
                        deleted_cache_paths,
                    )
                except Exception:
                    logger.warning(
                        "Failed to invalidate analysis cache after deletion.",
                        exc_info=True,
                    )
            self.completed.emit(result)
        finally:
            self.finished.emit()
