import logging
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Dict, List, Optional, Sequence, TYPE_CHECKING

from PyQt6.QtCore import QObject, pyqtSignal
from PIL import Image, ImageOps

if TYPE_CHECKING:
    from core.image_pipeline import ImagePipeline

from core.app_settings import calculate_max_workers

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
        image_pipeline: Optional["ImagePipeline"] = None,
        max_workers: Optional[int] = None,
        parent: Optional[QObject] = None,
    ):
        super().__init__(parent)
        self.cluster_map = cluster_map
        self.models_root = models_root
        self._should_stop = False
        self._image_pipeline = image_pipeline
        self._max_workers = max_workers or calculate_max_workers(
            min_workers=2, max_workers=8
        )
        self._selector = None

    def stop(self):
        self._should_stop = True

    def _iter_clusters(self):
        for cluster_id in sorted(self.cluster_map.keys()):
            yield cluster_id, self.cluster_map[cluster_id]

    def _load_image_for_analysis(self, image_path: str) -> Image.Image:
        preview_image: Optional[Image.Image] = None
        if self._image_pipeline is not None:
            try:
                preview_image = self._image_pipeline.get_preview_image(image_path)
            except Exception:
                logger.error(
                    "ImagePipeline preview lookup failed for %s", image_path, exc_info=True
                )
        if preview_image is None:
            with Image.open(image_path) as img:
                prepared = ImageOps.exif_transpose(img).convert("RGB")
                preview_image = prepared.copy()
        preview_image.info.setdefault("source_path", image_path)
        preview_image.info.setdefault("region", "full")
        return preview_image

    def _analyze_cluster(
        self, cluster_id: int, paths: Sequence[str]
    ) -> Optional[List[Dict[str, object]]]:
        if self._should_stop:
            return None
        normalized_paths = [p for p in paths if p]
        if not normalized_paths:
            return []
        selector = self._selector
        if selector is None:
            raise RuntimeError("Selector not initialized")
        rankings = selector.rank_images(normalized_paths)
        return [r.to_dict() for r in rankings]

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
            selector = BestPhotoSelector(
                models_root=self.models_root,
                image_loader=self._load_image_for_analysis,
            )
            self._selector = selector

            total_clusters = sum(1 for _, paths in self._iter_clusters() if paths)
            if total_clusters == 0:
                self.completed.emit({})
                return

            results: Dict[int, List[dict]] = {}
            futures = {}
            with ThreadPoolExecutor(max_workers=self._max_workers) as executor:
                for cluster_id, paths in self._iter_clusters():
                    if self._should_stop:
                        logger.info("Best shot worker stop requested before submit.")
                        break
                    if not paths:
                        continue
                    futures[
                        executor.submit(self._analyze_cluster, cluster_id, paths)
                    ] = cluster_id

                total_jobs = len(futures)
                if total_jobs == 0:
                    self.completed.emit({})
                    return

                processed = 0
                for future in as_completed(futures):
                    if self._should_stop:
                        logger.info("Best shot worker stop requested. Skipping remaining results.")
                        break
                    cluster_id = futures[future]
                    try:
                        cluster_results = future.result()
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
                        self._should_stop = True
                        break

                    if cluster_results is None:
                        continue

                    results[cluster_id] = cluster_results
                    processed += 1
                    percent = int((processed / total_jobs) * 100)
                    best_path = (
                        cluster_results[0]["image_path"]
                        if cluster_results
                        else "No result"
                    )
                    self.progress_update.emit(
                        percent,
                        f"Cluster {cluster_id}: best candidate {best_path}",
                    )

            if not self._should_stop:
                self.completed.emit(results)
        finally:
            self.finished.emit()
