import logging
import os
import time
import threading
from datetime import timedelta
from concurrent.futures import ThreadPoolExecutor, as_completed, Future
from typing import Dict, Iterable, List, Optional, Sequence, TYPE_CHECKING

from PyQt6.QtCore import QObject, pyqtSignal

if TYPE_CHECKING:
    from core.image_pipeline import ImagePipeline
    from core.caching.analysis_cache import AnalysisCache

from core.ai.best_shot_pipeline import (
    BaseBestShotStrategy,
    BestShotEngine,
    LLMConfig,
    create_best_shot_strategy,
)
from core.app_settings import (
    calculate_max_workers,
    get_best_shot_batch_size,
)

logger = logging.getLogger(__name__)


class BestShotWorker(QObject):
    """Background worker that ranks images per similarity cluster."""

    progress_update = pyqtSignal(int, str)
    completed = pyqtSignal(object)  # Emits Dict[int, List[dict]]
    error = pyqtSignal(str)
    finished = pyqtSignal()

    def __init__(
        self,
        cluster_map: Dict[int, Sequence[str]],
        image_pipeline: Optional["ImagePipeline"] = None,
        max_workers: Optional[int] = None,
        llm_config: Optional[LLMConfig] = None,
        strategy: Optional[BaseBestShotStrategy] = None,
        parent: Optional[QObject] = None,
        best_shot_batch_size: Optional[int] = None,
        folder_path: Optional[str] = None,
        analysis_cache: Optional["AnalysisCache"] = None,
    ):
        super().__init__(parent)
        self.cluster_map = cluster_map
        self._should_stop = False
        self._image_pipeline = image_pipeline
        self._engine = BestShotEngine.LLM.value
        self._llm_config = llm_config
        self._strategy: Optional[BaseBestShotStrategy] = strategy
        self._max_workers = max_workers or calculate_max_workers(
            min_workers=2, max_workers=8
        )
        self._best_shot_batch_size = max(
            2, int(best_shot_batch_size or get_best_shot_batch_size())
        )
        self._folder_path = folder_path
        self._analysis_cache = analysis_cache
        self._executor_lock = threading.Lock()
        self._executor: Optional[ThreadPoolExecutor] = None
        self._futures: Dict[Future, int] = {}

    def stop(self):
        self._should_stop = True
        self._shutdown_executor(cancel=True)
        if self._strategy is not None:
            try:
                self._strategy.request_cancel()
            except Exception:
                logger.debug("Failed to cancel best-shot strategy", exc_info=True)

    def _shutdown_executor(self, *, cancel: bool) -> None:
        executor: Optional[ThreadPoolExecutor] = None
        futures: List[Future] = []
        with self._executor_lock:
            executor = self._executor
            if executor is not None:
                futures = list(self._futures.keys())
            self._executor = None
            self._futures = {}
        if executor is None:
            return
        if cancel:
            for future in futures:
                future.cancel()
            executor.shutdown(wait=False, cancel_futures=True)
        else:
            executor.shutdown(wait=True, cancel_futures=False)

    def _iter_clusters(self):
        for cluster_id in sorted(self.cluster_map.keys()):
            yield cluster_id, self.cluster_map[cluster_id]

    @staticmethod
    def _normalize_detail(exc: Exception) -> str:
        message = str(exc).strip()
        return message or exc.__class__.__name__

    @staticmethod
    def _looks_like_connectivity_issue(message: str) -> bool:
        lowered = message.lower()
        connectivity_keywords = [
            "unable to reach",
            "connection",
            "timeout",
            "timed out",
            "unreachable",
            "refused",
            "reset",
            "broken pipe",
            "disconnected",
            "closed by peer",
            "host is down",
        ]
        return any(keyword in lowered for keyword in connectivity_keywords)

    def _describe_connectivity_issue(
        self,
        detail: str,
        *,
        phase: str,
        cluster_id: Optional[int] = None,
    ) -> str:
        cluster_hint = f" for cluster {cluster_id}" if cluster_id is not None else ""
        if phase == "initialization":
            prefix = "AI service is unreachable"
        else:
            prefix = "AI service became unreachable"
        guidance = (
            f"{prefix} during best-shot analysis{cluster_hint}. "
            "Verify your AI server and network connection, then try again."
        )
        if detail:
            return f"{guidance} Details: {detail}"
        return guidance

    def _format_strategy_error(self, exc: Exception) -> str:
        detail = self._normalize_detail(exc)
        if (
            self._engine == BestShotEngine.LLM.value
            and self._looks_like_connectivity_issue(detail)
        ):
            return self._describe_connectivity_issue(
                detail,
                phase="initialization",
            )
        return detail

    def _format_cluster_error(self, exc: Exception, cluster_id: int) -> str:
        detail = self._normalize_detail(exc)
        if (
            self._engine == BestShotEngine.LLM.value
            and self._looks_like_connectivity_issue(detail)
        ):
            return self._describe_connectivity_issue(
                detail,
                phase="processing",
                cluster_id=cluster_id,
            )
        return f"Cluster {cluster_id} ranking failed: {detail}"

    def _ensure_strategy(self):
        if self._strategy is None:
            self._strategy = create_best_shot_strategy(
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
            logger.error("Best shot strategy validation failed: %s", exc, exc_info=True)
            raise RuntimeError(self._format_strategy_error(exc)) from exc

    def _chunk_paths(self, paths: Sequence[str]) -> Iterable[List[str]]:
        step = self._best_shot_batch_size
        for i in range(0, len(paths), step):
            yield list(paths[i : i + step])

    def _update_results_cache(
        self,
        results_by_path: Dict[str, Dict[str, object]],
        batch_results: List[Dict[str, object]],
        *,
        batch_index: int,
    ) -> None:
        for rank_idx, entry in enumerate(batch_results):
            path = entry.get("image_path")
            if not path:
                continue
            enriched = dict(entry)
            enriched.setdefault("image_path", path)
            enriched["batch_index"] = batch_index
            enriched["batch_rank"] = rank_idx
            existing = results_by_path.get(path)
            if existing is None or enriched.get("composite_score", 0.0) >= existing.get(
                "composite_score", 0.0
            ):
                results_by_path[path] = enriched

    def _determine_global_winner(
        self,
        cluster_id: int,
        candidate_paths: List[str],
        results_by_path: Dict[str, Dict[str, object]],
    ) -> Optional[str]:
        champion: Optional[str] = None
        for candidate in candidate_paths:
            if not candidate:
                continue
            if champion == candidate:
                continue
            if self._should_stop:
                break
            if champion is None:
                champion = candidate
                continue
            comparison_results = self._strategy.rank_cluster(
                cluster_id, [champion, candidate]
            )
            self._update_results_cache(
                results_by_path,
                comparison_results,
                batch_index=-1,
            )
            champion = comparison_results[0].get("image_path", champion)
        return champion

    def _rank_cluster_with_batching(
        self, cluster_id: int, paths: Sequence[str]
    ) -> List[Dict[str, object]]:
        if len(paths) <= self._best_shot_batch_size:
            return self._strategy.rank_cluster(cluster_id, paths)

        results_by_path: Dict[str, Dict[str, object]] = {}
        batch_winners: List[str] = []

        for batch_index, batch_paths in enumerate(self._chunk_paths(paths)):
            if self._should_stop:
                break
            try:
                batch_results = self._strategy.rank_cluster(cluster_id, batch_paths)
            except Exception as exc:
                logger.error(
                    "Best shot ranking failed for cluster %s batch %s: %s",
                    cluster_id,
                    batch_index,
                    exc,
                    exc_info=True,
                )
                raise

            if not batch_results:
                continue

            self._update_results_cache(
                results_by_path,
                batch_results,
                batch_index=batch_index,
            )

            top_entry = batch_results[0]
            top_path = top_entry.get("image_path")
            if top_path:
                batch_winners.append(top_path)

        if not results_by_path:
            return []

        champion = self._determine_global_winner(
            cluster_id,
            batch_winners,
            results_by_path,
        )

        if champion is None:
            champion = max(
                results_by_path.values(),
                key=lambda entry: entry.get("composite_score", 0.0),
            ).get("image_path")

        final_results = list(results_by_path.values())
        final_results.sort(
            key=lambda entry: (
                entry.get("image_path") != champion,
                -entry.get("composite_score", 0.0),
                entry.get("batch_index", 0),
                entry.get("batch_rank", 0),
            )
        )
        if self._analysis_cache and self._folder_path:
            try:
                self._analysis_cache.update_best_shot_results(
                    self._folder_path, cluster_id, final_results
                )
            except Exception:
                logger.exception(
                    "Failed to persist best-shot results for cluster %s", cluster_id
                )
        return final_results

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
        if len(normalized_paths) <= 1:
            single_results: List[Dict[str, object]] = [
                {
                    "image_path": normalized_paths[0],
                    "composite_score": 1.0,
                    "metrics": {"llm_selected": True},
                    "analysis": "",
                }
            ]
            if self._analysis_cache and self._folder_path:
                try:
                    self._analysis_cache.update_best_shot_results(
                        self._folder_path, cluster_id, single_results
                    )
                except Exception:
                    logger.exception(
                        "Failed to persist single-image best-shot result for cluster %s",
                        cluster_id,
                    )
            return single_results
        rankings = self._rank_cluster_with_batching(cluster_id, normalized_paths)
        return rankings

    def run(self):
        try:
            self._ensure_strategy()
        except Exception as exc:
            logger.error(
                "Failed to initialise best-shot strategy: %s", exc, exc_info=True
            )
            self.error.emit(str(exc))
            self.finished.emit()
            return

        try:
            total_clusters = sum(1 for _, paths in self._iter_clusters() if paths)
            if total_clusters == 0:
                self.completed.emit({})
                return

            logger.info(
                f"Starting best shot analysis of {total_clusters} clusters using {self._engine} engine"
            )
            results: Dict[int, List[dict]] = {}
            futures: Dict[Future, int] = {}
            executor = ThreadPoolExecutor(max_workers=self._max_workers)
            with self._executor_lock:
                self._executor = executor
                self._futures = {}
            start_time = time.perf_counter()
            try:
                for cluster_id, paths in self._iter_clusters():
                    if self._should_stop:
                        logger.info("Best shot worker stop requested before submit.")
                        break
                    if not paths:
                        continue
                    future = executor.submit(self._analyze_cluster, cluster_id, paths)
                    with self._executor_lock:
                        futures[future] = cluster_id
                        self._futures[future] = cluster_id

                total_jobs = len(futures)
                if total_jobs == 0:
                    self.completed.emit({})
                    return

                processed = 0
                for future in as_completed(futures):
                    if self._should_stop:
                        logger.info(
                            "Best shot worker stop requested. Skipping remaining results."
                        )
                        break
                    cluster_id = futures.get(future)
                    if cluster_id is None:
                        continue
                    try:
                        cluster_results = future.result()
                    except Exception as exc:
                        logger.error(
                            "Best shot ranking failed for cluster %s: %s",
                            cluster_id,
                            exc,
                            exc_info=True,
                        )
                        self.error.emit(self._format_cluster_error(exc, cluster_id))
                        self._should_stop = True
                        with self._executor_lock:
                            self._futures.pop(future, None)
                        break

                    if cluster_results is None:
                        with self._executor_lock:
                            self._futures.pop(future, None)
                        continue

                    results[cluster_id] = cluster_results
                    processed += 1
                    percent = int((processed / total_jobs) * 100)
                    elapsed = max(0.0, time.perf_counter() - start_time)
                    avg = elapsed / processed
                    remaining = max(0, total_jobs - processed)
                    eta_seconds = avg * remaining
                    eta_text = self._format_eta(eta_seconds)
                    best_path = (
                        cluster_results[0]["image_path"]
                        if cluster_results
                        else "No result"
                    )
                    self.progress_update.emit(
                        percent,
                        f"Cluster {cluster_id}: best candidate {os.path.basename(best_path)} • ETA {eta_text}",
                    )
                    with self._executor_lock:
                        self._futures.pop(future, None)
            finally:
                self._shutdown_executor(cancel=self._should_stop)

            if not self._should_stop:
                total_results = sum(len(results) for results in results.values())
                logger.info(
                    f"Best shot analysis completed: {total_results} results from {len(results)} clusters"
                )
                self.completed.emit(results)
        finally:
            if self._strategy is not None:
                try:
                    self._strategy.shutdown()
                except Exception:  # pragma: no cover - best effort cleanup
                    logger.warning("Best-shot strategy shutdown failed", exc_info=True)
            self.finished.emit()

    @staticmethod
    def _format_eta(seconds: float) -> str:
        bounded = max(0, int(round(seconds)))
        return str(timedelta(seconds=bounded))
