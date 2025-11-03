import copy
import json
import logging
import os
import time
from typing import Dict, List, Optional, Set

import diskcache


logger = logging.getLogger(__name__)

DEFAULT_ANALYSIS_CACHE_DIR = os.path.join(
    os.path.expanduser("~"), ".cache", "photosort_analysis"
)

CACHE_VERSION = 1


def _normalize_folder_path(path: str) -> str:
    return os.path.abspath(os.path.normpath(path))


class AnalysisCache:
    """
    Persists similarity clustering and best-shot analysis results per folder so that
    long-running AI computations can be resumed across application sessions.
    """

    def __init__(self, cache_dir: str = DEFAULT_ANALYSIS_CACHE_DIR):
        os.makedirs(cache_dir, exist_ok=True)
        self._cache = diskcache.Cache(directory=cache_dir, disk_min_file_size=0)

    def close(self) -> None:
        try:
            self._cache.close()
        except Exception:
            logger.exception("Failed to close analysis cache.")

    def load(self, folder_path: str) -> Dict[str, object]:
        key = _normalize_folder_path(folder_path)
        try:
            data = self._cache.get(key)
        except Exception:
            logger.exception("Failed to load analysis cache for %s", folder_path)
            return {}
        if isinstance(data, dict):
            return copy.deepcopy(data)
        return {}

    def save_cluster_results(
        self,
        folder_path: str,
        cluster_results: Dict[str, int],
        *,
        reset_best_shots: bool = True,
    ) -> None:
        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)
        entry["version"] = CACHE_VERSION
        entry["cluster_results"] = dict(cluster_results)
        if reset_best_shots:
            entry.pop("best_shot_rankings", None)
            entry.pop("best_shot_scores_by_path", None)
            entry.pop("best_shot_winners", None)
        entry["updated_at"] = time.time()
        try:
            self._cache.set(key, entry)
        except Exception:
            logger.exception("Failed to persist cluster results for %s", folder_path)

    def update_best_shot_results(
        self,
        folder_path: str,
        cluster_id: int,
        rankings: List[Dict[str, object]],
    ) -> None:
        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)
        serialized_rankings = json.loads(json.dumps(rankings))

        rankings_map = entry.setdefault("best_shot_rankings", {})
        winners_map = entry.setdefault("best_shot_winners", {})
        scores_map = entry.setdefault("best_shot_scores_by_path", {})

        rankings_map[str(cluster_id)] = serialized_rankings
        winner = serialized_rankings[0] if serialized_rankings else None
        if winner:
            winners_map[str(cluster_id)] = winner

        for result in serialized_rankings:
            path = result.get("image_path")
            if path:
                scores_map[path] = result

        entry["version"] = CACHE_VERSION
        entry["updated_at"] = time.time()
        try:
            self._cache.set(key, entry)
        except Exception:
            logger.exception("Failed to persist best-shot results for %s", folder_path)

    def get_completed_best_shot_clusters(self, folder_path: str) -> Set[int]:
        entry = self.load(folder_path)
        rankings_map = entry.get("best_shot_rankings")
        if not isinstance(rankings_map, dict):
            return set()
        completed: Set[int] = set()
        for key in rankings_map.keys():
            try:
                completed.add(int(key))
            except (TypeError, ValueError):
                continue
        return completed

    def clear_best_shot_data(self, folder_path: str) -> None:
        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)
        entry.pop("best_shot_rankings", None)
        entry.pop("best_shot_scores_by_path", None)
        entry.pop("best_shot_winners", None)
        entry["version"] = CACHE_VERSION
        entry["updated_at"] = time.time()
        try:
            self._cache.set(key, entry)
        except Exception:
            logger.exception("Failed to clear best-shot data for %s", folder_path)

    def clear_folder(self, folder_path: str) -> None:
        key = _normalize_folder_path(folder_path)
        try:
            if key in self._cache:
                del self._cache[key]
        except Exception:
            logger.exception("Failed to clear analysis cache for %s", folder_path)

    def clear_all(self) -> None:
        try:
            self._cache.clear()
        except Exception:
            logger.exception("Failed to clear full analysis cache")

    def volume(self) -> int:
        try:
            return self._cache.volume()
        except Exception:
            logger.exception("Failed to get analysis cache volume")
            return 0
