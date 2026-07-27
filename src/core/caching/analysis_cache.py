import copy
import json
import logging
import os
import time

import diskcache
from core.runtime_paths import resolve_user_cache_dir


logger = logging.getLogger(__name__)

CACHE_VERSION = 2


def _normalize_folder_path(path: str) -> str:
    return os.path.abspath(os.path.normpath(path))


class AnalysisCache:
    """
    Persists similarity clustering and best-shot analysis results per folder so that
    long-running AI computations can be resumed across application sessions.
    """

    def __init__(self, cache_dir: str | None = None):
        if cache_dir is None:
            cache_dir = resolve_user_cache_dir("analysis")
        os.makedirs(cache_dir, exist_ok=True)
        self._cache = diskcache.Cache(directory=cache_dir, disk_min_file_size=0)

    def close(self) -> None:
        try:
            self._cache.close()
        except Exception:
            logger.exception("Failed to close analysis cache.")

    def load(self, folder_path: str) -> dict[str, object]:
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
        cluster_results: dict[str, int],
        *,
        signature: str,
        reset_best_shots: bool = True,
    ) -> None:
        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)
        entry["version"] = CACHE_VERSION
        entry["cluster_results"] = dict(cluster_results)
        entry["similarity_signature"] = signature
        if reset_best_shots:
            entry.pop("best_shot_rankings", None)
            entry.pop("best_shot_scores_by_path", None)
            entry.pop("best_shot_winners", None)
        entry["updated_at"] = time.time()
        try:
            self._cache.set(key, entry)
        except Exception:
            logger.exception("Failed to persist cluster results for %s", folder_path)

    def load_valid_cluster_results(
        self,
        folder_path: str,
        *,
        signature: str,
        expected_paths: set[str],
    ) -> dict[str, int] | None:
        """Return a complete cluster map only when its inputs still match."""

        entry = self.load(folder_path)
        if (
            entry.get("version") != CACHE_VERSION
            or entry.get("similarity_signature") != signature
        ):
            return None
        clusters = entry.get("cluster_results")
        if not isinstance(clusters, dict) or set(clusters) != expected_paths:
            return None
        try:
            return {str(path): int(cluster_id) for path, cluster_id in clusters.items()}
        except TypeError, ValueError:
            return None

    def invalidate_similarity(self, folder_path: str) -> None:
        """Invalidate computed similarity state while preserving manual overrides."""

        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)
        if not entry:
            return
        for field in (
            "cluster_results",
            "similarity_signature",
            "best_shot_rankings",
            "best_shot_scores_by_path",
            "best_shot_winners",
            "subject_descriptors",
        ):
            entry.pop(field, None)
        entry["version"] = CACHE_VERSION
        entry["updated_at"] = time.time()
        try:
            self._cache.set(key, entry)
        except Exception:
            logger.exception(
                "Failed to invalidate similarity state for %s", folder_path
            )

    def load_subject_descriptor(
        self,
        folder_path: str,
        file_path: str,
        *,
        fingerprint: tuple[int, int],
        signature: str,
    ) -> dict[str, object] | None:
        """Return a descriptor only when its file and analysis inputs still match."""

        entry = self.load(folder_path)
        descriptors = entry.get("subject_descriptors")
        if not isinstance(descriptors, dict):
            return None
        record = descriptors.get(file_path)
        if not isinstance(record, dict):
            return None
        try:
            cached_fingerprint = tuple(record.get("fingerprint", ()))
        except TypeError:
            return None
        descriptor = record.get("descriptor")
        if (
            record.get("signature") != signature
            or cached_fingerprint != tuple(fingerprint)
            or not isinstance(descriptor, dict)
        ):
            return None
        return copy.deepcopy(descriptor)

    def save_subject_descriptor(
        self,
        folder_path: str,
        file_path: str,
        *,
        fingerprint: tuple[int, int],
        signature: str,
        descriptor: dict[str, object],
    ) -> None:
        """Persist one immutable subject descriptor and its validity inputs."""

        self.save_subject_descriptors_batch(
            folder_path,
            {
                file_path: {
                    "fingerprint": tuple(fingerprint),
                    "signature": signature,
                    "descriptor": descriptor,
                }
            },
        )

    def save_subject_descriptors_batch(
        self,
        folder_path: str,
        records: dict[str, dict[str, object]],
    ) -> None:
        """Persist many subject descriptors with one cache read and write."""

        if not records:
            return
        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)
        descriptors = entry.setdefault("subject_descriptors", {})
        if not isinstance(descriptors, dict):
            descriptors = {}
            entry["subject_descriptors"] = descriptors
        for file_path, record in records.items():
            if not isinstance(record, dict):
                continue
            descriptor = record.get("descriptor")
            fingerprint = record.get("fingerprint")
            signature = record.get("signature")
            if (
                not isinstance(descriptor, dict)
                or not isinstance(fingerprint, (tuple, list))
                or len(fingerprint) != 2
                or not isinstance(signature, str)
            ):
                continue
            descriptors[file_path] = {
                "fingerprint": tuple(fingerprint),
                "signature": signature,
                "descriptor": copy.deepcopy(descriptor),
            }
        entry["version"] = CACHE_VERSION
        entry["updated_at"] = time.time()
        try:
            self._cache.set(key, entry)
        except Exception:
            logger.exception(
                "Failed to persist %d subject descriptors for %s",
                len(records),
                folder_path,
            )

    def update_best_shot_results(
        self,
        folder_path: str,
        cluster_id: int,
        rankings: list[dict[str, object]],
    ) -> None:
        """Persist one cluster through the batch-owned implementation."""

        self.update_best_shot_results_batch(
            folder_path,
            {cluster_id: rankings},
        )

    def update_best_shot_results_batch(
        self,
        folder_path: str,
        rankings_by_cluster: dict[int, list[dict[str, object]]],
    ) -> None:
        """Merge many cluster rankings with one cache read and one write."""

        if not rankings_by_cluster:
            return
        key = _normalize_folder_path(folder_path)
        entry = self.load(folder_path)

        rankings_map = entry.setdefault("best_shot_rankings", {})
        winners_map = entry.setdefault("best_shot_winners", {})
        scores_map = entry.setdefault("best_shot_scores_by_path", {})

        for cluster_id, rankings in rankings_by_cluster.items():
            serialized_rankings = json.loads(json.dumps(rankings))
            rankings_map[str(cluster_id)] = serialized_rankings
            winner = serialized_rankings[0] if serialized_rankings else None
            if winner:
                winners_map[str(cluster_id)] = winner
            else:
                winners_map.pop(str(cluster_id), None)

            for result in serialized_rankings:
                path = result.get("image_path")
                if path:
                    scores_map[path] = result

        entry["version"] = CACHE_VERSION
        entry["updated_at"] = time.time()
        try:
            self._cache.set(key, entry)
        except Exception:
            logger.exception(
                "Failed to persist best-shot result batch for %s", folder_path
            )

    def get_completed_best_shot_clusters(self, folder_path: str) -> set[int]:
        entry = self.load(folder_path)
        rankings_map = entry.get("best_shot_rankings")
        if not isinstance(rankings_map, dict):
            return set()
        completed: set[int] = set()
        for key in rankings_map:
            try:
                completed.add(int(key))
            except TypeError, ValueError:
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

    def migrate_folder_paths(
        self,
        source_folder: str,
        destination_folder: str,
        path_updates: dict[str, str],
    ) -> None:
        """Move one folder's analysis state and remap every stored file path."""

        if not path_updates:
            return
        entry = self.load(source_folder)
        if not entry:
            return

        def remap_mapping_keys(value: object) -> object:
            if not isinstance(value, dict):
                return value
            return {path_updates.get(path, path): item for path, item in value.items()}

        for field in (
            "cluster_results",
            "manual_cluster_overrides",
            "best_shot_scores_by_path",
            "subject_descriptors",
        ):
            if field in entry:
                entry[field] = remap_mapping_keys(entry[field])

        for field in (
            "best_shot_rankings",
            "best_shot_winners",
            "best_shot_scores_by_path",
        ):
            collection = entry.get(field)
            if not isinstance(collection, dict):
                continue
            groups = (
                collection.values()
                if field == "best_shot_rankings"
                else ([value] for value in collection.values())
            )
            for group in groups:
                if not isinstance(group, list):
                    continue
                for result in group:
                    if not isinstance(result, dict):
                        continue
                    path = result.get("image_path")
                    if path in path_updates:
                        result["image_path"] = path_updates[path]

        entry["version"] = CACHE_VERSION
        entry.pop("similarity_signature", None)
        entry["updated_at"] = time.time()
        source_key = _normalize_folder_path(source_folder)
        destination_key = _normalize_folder_path(destination_folder)
        try:
            self._cache.set(destination_key, entry)
            if source_key != destination_key and source_key in self._cache:
                del self._cache[source_key]
        except Exception:
            logger.exception(
                "Failed to migrate analysis cache from %s to %s",
                source_folder,
                destination_folder,
            )

    def remove_paths(self, folder_path: str, paths: set[str]) -> None:
        """Remove deleted files from one analysis entry with one read/write."""

        removed = {path for path in paths if path}
        if not removed:
            return
        entry = self.load(folder_path)
        if not entry:
            return

        for field in (
            "cluster_results",
            "manual_cluster_overrides",
            "best_shot_scores_by_path",
            "subject_descriptors",
        ):
            mapping = entry.get(field)
            if isinstance(mapping, dict):
                for path in removed:
                    mapping.pop(path, None)

        rankings = entry.get("best_shot_rankings")
        winners = entry.get("best_shot_winners")
        if isinstance(rankings, dict):
            for cluster_id, values in list(rankings.items()):
                if not isinstance(values, list):
                    continue
                retained = [
                    result
                    for result in values
                    if not (
                        isinstance(result, dict) and result.get("image_path") in removed
                    )
                ]
                if retained:
                    rankings[cluster_id] = retained
                    if isinstance(winners, dict):
                        winners[cluster_id] = retained[0]
                else:
                    rankings.pop(cluster_id, None)
                    if isinstance(winners, dict):
                        winners.pop(cluster_id, None)
        elif isinstance(winners, dict):
            for cluster_id, winner in list(winners.items()):
                if isinstance(winner, dict) and winner.get("image_path") in removed:
                    winners.pop(cluster_id, None)

        entry["version"] = CACHE_VERSION
        entry.pop("similarity_signature", None)
        entry["updated_at"] = time.time()
        try:
            self._cache.set(_normalize_folder_path(folder_path), entry)
        except Exception:
            logger.exception(
                "Failed to remove deleted paths from analysis cache for %s",
                folder_path,
            )

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
        overrides_to_save: dict[str, int],
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

    def get_manual_overrides(self, folder_path: str) -> dict[str, int]:
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
