import logging
from typing import Dict, List, Optional, Sequence

from PyQt6.QtCore import QObject, pyqtSignal

logger = logging.getLogger(__name__)


class BestShotWorker(QObject):
    """Background worker that ranks images per similarity cluster."""

    progress_update = pyqtSignal(int, str)
    completed = pyqtSignal(object)  # Emits Dict[int, List[dict]]
    error = pyqtSignal(str)
    models_missing = pyqtSignal(object)  # Emits List[MissingModelInfo]
    finished = pyqtSignal()

    def __init__(
        self,
        cluster_map: Dict[int, Sequence[str]],
        models_root: Optional[str] = None,
        parent: Optional[QObject] = None,
    ):
        super().__init__(parent)
        self.cluster_map = cluster_map
        self.models_root = models_root
        self._should_stop = False

    def stop(self):
        self._should_stop = True

    def _iter_clusters(self):
        for cluster_id in sorted(self.cluster_map.keys()):
            yield cluster_id, self.cluster_map[cluster_id]

    def run(self):
        try:
            from core.ai.best_photo_selector import BestPhotoSelector
            from core.ai.model_checker import (
                ModelDependencyError,
                check_best_shot_models,
            )
        except Exception as exc:  # pragma: no cover - import failure handled at runtime
            logger.error("BestPhotoSelector import failed: %s", exc, exc_info=True)
            self.error.emit(str(exc))
            self.finished.emit()
            return

        # Check for missing models before attempting to instantiate selector
        missing_models = check_best_shot_models(self.models_root)
        if missing_models:
            logger.warning(
                "Best-shot analysis aborted: %d model(s) missing", len(missing_models)
            )
            self.models_missing.emit(missing_models)
            self.finished.emit()
            return

        try:
            selector = BestPhotoSelector(models_root=self.models_root)
            total_clusters = sum(1 for _, paths in self._iter_clusters() if paths)
            if total_clusters == 0:
                self.completed.emit({})
                return

            results: Dict[int, List[dict]] = {}
            processed = 0
            for cluster_id, paths in self._iter_clusters():
                if self._should_stop:
                    logger.info("Best shot worker stop requested. Exiting loop.")
                    break
                normalized_paths = [p for p in paths if p]
                if not normalized_paths:
                    continue
                try:
                    rankings = selector.rank_images(normalized_paths)
                except Exception as exc:
                    logger.error(
                        "Best shot ranking failed for cluster %s: %s",
                        cluster_id,
                        exc,
                        exc_info=True,
                    )
                    self.error.emit(
                        f"Cluster {cluster_id} ranking failed: {exc}"
                    )
                    return
                results[cluster_id] = [r.to_dict() for r in rankings]
                processed += 1
                percent = int((processed / total_clusters) * 100)
                best_path = rankings[0].image_path if rankings else "No result"
                self.progress_update.emit(
                    percent,
                    f"Cluster {cluster_id}: best candidate {best_path}",
                )

            if not self._should_stop:
                self.completed.emit(results)
        finally:
            self.finished.emit()
