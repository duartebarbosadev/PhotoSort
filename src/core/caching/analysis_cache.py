import copy
import json
import logging
import os
import time
from typing import Dict, List, Optional, Set

import diskcache
from core.runtime_paths import resolve_user_cache_dir


logger = logging.getLogger(__name__)

CACHE_VERSION = 1


def _normalize_folder_path(path: str) -> str:
    return os.path.abspath(os.path.normpath(path))


class AnalysisCache:
    """
    Persists similarity clustering and best-shot analysis results per folder so that
    long-running AI computations can be resumed across application sessions.
    """

    def __init__(self, cache_dir: Optional[str] = None):
        if cache_dir is None:
            cache_dir = resolve_user_cache_dir("photosort_analysis")
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

    # --- Manual Cluster Override Methods ---

    def save_manual_cluster_override(
        self,
        folder_path: str,
        file_path: str,
        cluster_id: int,
    ) -> None:
        """
        Save a single manual cluster assignment.

        This also updates the cluster_results to reflect the change.
        """
        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)

        overrides = entry.setdefault("manual_cluster_overrides", {})
        overrides[file_path] = cluster_id

        # Also update cluster_results
        cluster_results = entry.setdefault("cluster_results", {})
        cluster_results[file_path] = cluster_id

        entry["version"] = CACHE_VERSION
        entry["updated_at"] = time.time()
        try:
            self._cache.set(key, entry)
        except Exception:
            logger.exception(
                "Failed to persist manual cluster override for %s", folder_path
            )

    def save_manual_cluster_overrides(
        self,
        folder_path: str,
        overrides_to_save: Dict[str, int],
    ) -> None:
        """
        Save multiple manual cluster assignments at once.

        This also updates the cluster_results to reflect all changes.
        """
        if not overrides_to_save:
            return

        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)

        overrides = entry.setdefault("manual_cluster_overrides", {})
        overrides.update(overrides_to_save)

        # Also update cluster_results
        cluster_results = entry.setdefault("cluster_results", {})
        cluster_results.update(overrides_to_save)

        entry["version"] = CACHE_VERSION
        entry["updated_at"] = time.time()
        try:
            self._cache.set(key, entry)
        except Exception:
            logger.exception(
                "Failed to persist manual cluster overrides for %s", folder_path
            )

    def get_manual_overrides(self, folder_path: str) -> Dict[str, int]:
        """Get all manual cluster overrides for a folder."""
        entry = self.load(folder_path)
        overrides = entry.get("manual_cluster_overrides", {})
        if isinstance(overrides, dict):
            return dict(overrides)
        return {}

    def clear_manual_override(self, folder_path: str, file_path: str) -> None:
        """Remove a single manual override for a file."""
        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)

        overrides = entry.get("manual_cluster_overrides", {})
        if file_path in overrides:
            del overrides[file_path]
            entry["manual_cluster_overrides"] = overrides
            entry["version"] = CACHE_VERSION
            entry["updated_at"] = time.time()
            try:
                self._cache.set(key, entry)
            except Exception:
                logger.exception(
                    "Failed to clear manual override for %s in %s",
                    file_path,
                    folder_path,
                )

    def clear_all_manual_overrides(self, folder_path: str) -> None:
        """Clear all manual overrides for a folder."""
        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)

        if "manual_cluster_overrides" in entry:
            del entry["manual_cluster_overrides"]
            entry["version"] = CACHE_VERSION
            entry["updated_at"] = time.time()
            try:
                self._cache.set(key, entry)
            except Exception:
                logger.exception(
                    "Failed to clear all manual overrides for %s", folder_path
                )
