import logging
import os
from concurrent.futures import ThreadPoolExecutor, as_completed
from typing import Dict, List, Optional, Sequence, TYPE_CHECKING

from PyQt6.QtCore import QObject, pyqtSignal

if TYPE_CHECKING:
    from core.image_pipeline import ImagePipeline

from core.ai.best_shot_pipeline import (
    BaseBestShotStrategy,
    BestShotEngine,
    LLMConfig,
    create_best_shot_strategy,
)
from core.app_settings import (
    calculate_max_workers,
    get_best_shot_engine,
)

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
        engine: Optional[str] = None,
        llm_config: Optional[LLMConfig] = None,
        strategy: Optional[BaseBestShotStrategy] = None,
        parent: Optional[QObject] = None,
    ):
        super().__init__(parent)
        self.cluster_map = cluster_map
        self.models_root = models_root
        self._should_stop = False
        self._image_pipeline = image_pipeline
        self._engine = (engine or get_best_shot_engine() or "local").lower()
        self._llm_config = llm_config
        self._strategy: Optional[BaseBestShotStrategy] = strategy
        self._max_workers = max_workers or calculate_max_workers(
            min_workers=2, max_workers=8
        )

    def stop(self):
        self._should_stop = True

    def _iter_clusters(self):
        for cluster_id in sorted(self.cluster_map.keys()):
            yield cluster_id, self.cluster_map[cluster_id]

    def _ensure_strategy(self):
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

    def _analyze_cluster(
        self, cluster_id: int, paths: Sequence[str]
    ) -> Optional[List[Dict[str, object]]]:
        if self._should_stop:
            return None
        normalized_paths = [p for p in paths if p]
        if not normalized_paths:
            return []
        if self._strategy is None:
            raise RuntimeError("Best-shot strategy not initialised")
        rankings = self._strategy.rank_cluster(cluster_id, normalized_paths)
        return rankings

    def run(self):
        try:
            from core.ai.model_checker import (
                ModelDependencyError,
                check_best_shot_models,
            )
        except Exception as exc:  # pragma: no cover - import failure handled at runtime
            logger.error("BestPhotoSelector import failed: %s", exc, exc_info=True)
            self.error.emit(str(exc))
            self.finished.emit()
            return

        try:
            if self._engine == BestShotEngine.LOCAL.value:
                missing_models = check_best_shot_models(self.models_root)
                if missing_models:
                    logger.warning(
                        "Best-shot analysis aborted: %d model(s) missing", len(missing_models)
                    )
                    self.models_missing.emit(missing_models)
                    self.finished.emit()
                    return
            try:
                self._ensure_strategy()
            except ModelDependencyError as exc:
                logger.error("Failed to initialise best-shot strategy: %s", exc)
                self.error.emit(str(exc))
                self.finished.emit()
                return

            total_clusters = sum(1 for _, paths in self._iter_clusters() if paths)
            if total_clusters == 0:
                self.completed.emit({})
                return

            logger.info(f"Starting best shot analysis of {total_clusters} clusters using {self._engine} engine")
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
                        f"Cluster {cluster_id}: best candidate {os.path.basename(best_path)}",
                    )

            if not self._should_stop:
                total_results = sum(len(results) for results in results.values())
                logger.info(f"Best shot analysis completed: {total_results} results from {len(results)} clusters")
                self.completed.emit(results)
        finally:
            if self._strategy is not None:
                try:
                    self._strategy.shutdown()
                except Exception:  # pragma: no cover - best effort cleanup
                    logger.warning("Best-shot strategy shutdown failed", exc_info=True)
            self.finished.emit()
