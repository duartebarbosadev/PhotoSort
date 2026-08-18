import logging
import os
import threading
import time
from collections.abc import Iterable, Iterator
from contextlib import contextmanager

import diskcache
from core.runtime_paths import resolve_user_cache_dir


logger = logging.getLogger(__name__)

CACHE_VERSION = 2

MANUAL_OVERRIDE_NAMESPACE_SIMILARITY = "similarity"
MANUAL_OVERRIDE_NAMESPACE_CULL = "cull"

# Cull and similarity clustering produce independent cluster-id namespaces, so
# each owns its own override store and its own cluster map.
_MANUAL_OVERRIDE_FIELDS: dict[str, tuple[str, str]] = {
    MANUAL_OVERRIDE_NAMESPACE_SIMILARITY: (
        "manual_cluster_overrides",
        "cluster_results",
    ),
    MANUAL_OVERRIDE_NAMESPACE_CULL: (
        "cull_manual_cluster_overrides",
        "cull_cluster_results",
    ),
}

_PATH_KEYED_FIELDS = (
    "cluster_results",
    "manual_cluster_overrides",
    "cull_manual_cluster_overrides",
    "subject_descriptors",
    "cull_subject_artifacts",
    "cull_cluster_results",
)

# Cluster assignments depend on the whole photo set, so any change discards them.
_DERIVED_SIMILARITY_FIELDS = (
    "cluster_results",
    "similarity_signature",
    "cull_cluster_results",
    "cull_grouping_signature",
)

# Expensive per-photo results, keyed by path and safe to invalidate selectively.
_PER_PATH_SIMILARITY_FIELDS = (
    "subject_descriptors",
    "cull_subject_artifacts",
    "cull_pair_verifications",
)

_OBSOLETE_FIELDS = (
    "best_shot_rankings",
    "best_shot_scores_by_path",
    "best_shot_winners",
)


def _normalize_folder_path(path: str) -> str:
    return os.path.abspath(os.path.normpath(path))


def _manual_override_fields(namespace: str) -> tuple[str, str]:
    """Return the (overrides field, cluster field) pair owned by a namespace."""

    try:
        return _MANUAL_OVERRIDE_FIELDS[namespace]
    except KeyError:
        raise ValueError(f"Unknown manual override namespace: {namespace}") from None


class _SkipEntryUpdate(Exception):
    """Abandon an in-progress read-modify-write without persisting anything."""


class AnalysisCache:
    """
    Persists similarity clustering and reusable workflow analysis per folder.

    Every mutation is a read-modify-write of one folder entry, so all mutators go
    through :meth:`_mutate`, which serializes concurrent workers (for example the
    long-running Cull grouping worker and the file deletion worker) and prevents
    lost updates.
    """

    def __init__(self, cache_dir: str | None = None):
        if cache_dir is None:
            cache_dir = resolve_user_cache_dir("analysis")
        os.makedirs(cache_dir, exist_ok=True)
        self._cache = diskcache.Cache(directory=cache_dir, disk_min_file_size=0)
        self._lock = threading.RLock()

    def close(self) -> None:
        try:
            self._cache.close()
        except Exception:
            logger.exception("Failed to close analysis cache.")

    def load(self, folder_path: str) -> dict[str, object]:
        """Return one folder entry. diskcache unpickles a private copy per read."""

        key = _normalize_folder_path(folder_path)
        try:
            data = self._cache.get(key)
        except Exception:
            logger.exception("Failed to load analysis cache for %s", folder_path)
            return {}
        if not isinstance(data, dict):
            return {}
        for obsolete_key in _OBSOLETE_FIELDS:
            data.pop(obsolete_key, None)
        return data

    @contextmanager
    def _mutate(
        self, folder_path: str, *, failure_message: str
    ) -> Iterator[dict[str, object]]:
        """Yield one folder entry for exclusive read-modify-write persistence."""

        key = _normalize_folder_path(folder_path)
        with self._lock, self._cache.transact():
            entry = self.load(folder_path)
            try:
                yield entry
            except _SkipEntryUpdate:
                return
            entry["version"] = CACHE_VERSION
            entry["updated_at"] = time.time()
            try:
                self._cache.set(key, entry)
            except Exception:
                logger.exception(failure_message, folder_path)

    def save_cluster_results(
        self,
        folder_path: str,
        cluster_results: dict[str, int],
        *,
        signature: str,
    ) -> None:
        with self._mutate(
            folder_path,
            failure_message="Failed to persist cluster results for %s",
        ) as entry:
            entry["cluster_results"] = dict(cluster_results)
            entry["similarity_signature"] = signature

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

    def invalidate_similarity(
        self, folder_path: str, *, changed_paths: Iterable[str] | None = None
    ) -> None:
        """Invalidate computed similarity state while preserving manual overrides.

        ``changed_paths`` limits the invalidation to the photos whose pixels
        actually changed: their descriptors, artifacts and pair verdicts are
        dropped, while every untouched photo keeps its expensive embedding.
        Group assignments are always discarded because they depend on the whole
        set. Passing ``None`` invalidates everything.
        """

        with self._mutate(
            folder_path,
            failure_message="Failed to invalidate similarity state for %s",
        ) as entry:
            if not entry:
                raise _SkipEntryUpdate
            for field in _DERIVED_SIMILARITY_FIELDS:
                entry.pop(field, None)
            if changed_paths is None:
                for field in (
                    *_PER_PATH_SIMILARITY_FIELDS,
                    "cull_model_signature",
                    "cull_pair_pipeline_version",
                    "cull_pair_context_signature",
                ):
                    entry.pop(field, None)
                return
            stale = {path for path in changed_paths if path}
            if not stale:
                return
            for field in _PER_PATH_SIMILARITY_FIELDS:
                records = entry.get(field)
                if isinstance(records, dict):
                    for path in stale:
                        records.pop(path, None)
            pairs = entry.get("cull_pair_verifications")
            if isinstance(pairs, dict):
                for key in [
                    key for key in pairs if stale.intersection(str(key).split("\0", 1))
                ]:
                    pairs.pop(key, None)

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
        return descriptor

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

    def load_cull_grouping_state(self, folder_path: str) -> dict[str, object]:
        """Return the independently versioned high-precision Cull cache state."""

        entry = self.load(folder_path)
        return {
            key: entry[key]
            for key in (
                "cull_subject_artifacts",
                "cull_pair_verifications",
                "cull_cluster_results",
                "cull_grouping_signature",
                "cull_model_signature",
                "cull_pair_pipeline_version",
                "cull_pair_context_signature",
            )
            if key in entry
        }

    def save_cull_grouping_state(
        self,
        folder_path: str,
        *,
        artifacts: dict[str, object],
        pair_verifications: dict[str, object],
        clusters: dict[str, int],
        grouping_signature: str,
        model_signature: str,
        pair_pipeline_version: str,
        pair_context_signature: str,
    ) -> None:
        """Persist reusable Cull features, pair evidence, and final assignments."""

        with self._mutate(
            folder_path,
            failure_message="Failed to persist Cull grouping state for %s",
        ) as entry:
            entry.update(
                {
                    "cull_subject_artifacts": dict(artifacts),
                    "cull_pair_verifications": dict(pair_verifications),
                    "cull_cluster_results": dict(clusters),
                    "cull_grouping_signature": grouping_signature,
                    "cull_model_signature": model_signature,
                    "cull_pair_pipeline_version": pair_pipeline_version,
                    "cull_pair_context_signature": pair_context_signature,
                }
            )

    def merge_cull_artifacts_checkpoint(
        self,
        folder_path: str,
        *,
        artifacts: dict[str, object],
        model_signature: str,
    ) -> None:
        """Merge newly extracted DINO artifacts without publishing clusters.

        Only artifacts produced since the previous checkpoint are supplied, so a
        long extraction run costs O(new artifacts) per checkpoint instead of
        rewriting the whole accumulated set every time.
        """

        if not artifacts:
            return
        with self._mutate(
            folder_path,
            failure_message="Failed to checkpoint Cull artifacts for %s",
        ) as entry:
            stored = entry.get("cull_subject_artifacts")
            if entry.get("cull_model_signature") != model_signature:
                entry.pop("cull_pair_verifications", None)
                entry.pop("cull_cluster_results", None)
                entry.pop("cull_grouping_signature", None)
                entry.pop("cull_pair_context_signature", None)
                stored = None
            if not isinstance(stored, dict):
                stored = {}
            stored.update(artifacts)
            entry["cull_subject_artifacts"] = stored
            entry["cull_model_signature"] = model_signature

    def save_subject_descriptors_batch(
        self,
        folder_path: str,
        records: dict[str, dict[str, object]],
    ) -> None:
        """Persist many subject descriptors with one cache read and write."""

        if not records:
            return
        with self._mutate(
            folder_path,
            failure_message="Failed to persist subject descriptors for %s",
        ) as entry:
            descriptors = entry.get("subject_descriptors")
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
                    "descriptor": descriptor,
                }

    def clear_folder(self, folder_path: str) -> None:
        key = _normalize_folder_path(folder_path)
        try:
            with self._lock:
                if key in self._cache:
                    del self._cache[key]
        except Exception:
            logger.exception("Failed to clear analysis cache for %s", folder_path)

    def clear_all(self) -> None:
        try:
            with self._lock:
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
        source_key = _normalize_folder_path(source_folder)
        destination_key = _normalize_folder_path(destination_folder)
        with self._lock, self._cache.transact():
            entry = self.load(source_folder)
            if not entry:
                return

            def remap_mapping_keys(value: object) -> object:
                if not isinstance(value, dict):
                    return value
                return {
                    path_updates.get(path, path): item for path, item in value.items()
                }

            for field in _PATH_KEYED_FIELDS:
                if field in entry:
                    entry[field] = remap_mapping_keys(entry[field])

            artifacts = entry.get("cull_subject_artifacts")
            if isinstance(artifacts, dict):
                for path, value in artifacts.items():
                    if isinstance(value, dict):
                        value["path"] = path

            pair_verifications = entry.get("cull_pair_verifications")
            if isinstance(pair_verifications, dict):
                remapped_pairs = {}
                for key, value in pair_verifications.items():
                    parts = str(key).split("\0", 1)
                    if len(parts) != 2:
                        continue
                    first = path_updates.get(parts[0], parts[0])
                    second = path_updates.get(parts[1], parts[1])
                    left, right = (first, second) if first < second else (second, first)
                    if isinstance(value, dict):
                        value["path_a"] = left
                        value["path_b"] = right
                    remapped_pairs[f"{left}\0{right}"] = value
                entry["cull_pair_verifications"] = remapped_pairs

            entry["version"] = CACHE_VERSION
            entry.pop("similarity_signature", None)
            entry.pop("cull_grouping_signature", None)
            entry.pop("cull_pair_context_signature", None)
            entry["updated_at"] = time.time()
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
        with self._mutate(
            folder_path,
            failure_message="Failed to remove deleted paths from analysis cache for %s",
        ) as entry:
            if not entry:
                raise _SkipEntryUpdate

            for field in _PATH_KEYED_FIELDS:
                mapping = entry.get(field)
                if isinstance(mapping, dict):
                    for path in removed:
                        mapping.pop(path, None)

            pairs = entry.get("cull_pair_verifications")
            if isinstance(pairs, dict):
                for key in list(pairs):
                    if any(path in removed for path in str(key).split("\0")):
                        pairs.pop(key, None)

            entry.pop("similarity_signature", None)
            entry.pop("cull_grouping_signature", None)
            entry.pop("cull_pair_context_signature", None)

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
        *,
        namespace: str = MANUAL_OVERRIDE_NAMESPACE_SIMILARITY,
    ) -> None:
        """Save a single manual cluster assignment inside one cluster namespace."""

        self.save_manual_cluster_overrides(
            folder_path, {file_path: cluster_id}, namespace=namespace
        )

    def save_manual_cluster_overrides(
        self,
        folder_path: str,
        overrides_to_save: dict[str, int],
        *,
        namespace: str = MANUAL_OVERRIDE_NAMESPACE_SIMILARITY,
    ) -> None:
        """Save manual cluster assignments and mirror them into that namespace's
        cluster map."""

        if not overrides_to_save:
            return
        overrides_field, clusters_field = _manual_override_fields(namespace)
        with self._mutate(
            folder_path,
            failure_message="Failed to persist manual cluster overrides for %s",
        ) as entry:
            overrides = entry.setdefault(overrides_field, {})
            overrides.update(overrides_to_save)
            cluster_results = entry.setdefault(clusters_field, {})
            cluster_results.update(overrides_to_save)

    def get_manual_overrides(
        self,
        folder_path: str,
        *,
        namespace: str = MANUAL_OVERRIDE_NAMESPACE_SIMILARITY,
    ) -> dict[str, int]:
        """Get all manual cluster overrides for a folder in one namespace."""

        overrides_field, _clusters_field = _manual_override_fields(namespace)
        overrides = self.load(folder_path).get(overrides_field, {})
        if isinstance(overrides, dict):
            return dict(overrides)
        return {}

    def clear_manual_override(
        self,
        folder_path: str,
        file_path: str,
        *,
        namespace: str = MANUAL_OVERRIDE_NAMESPACE_SIMILARITY,
    ) -> None:
        """Remove a single manual override for a file."""

        overrides_field, _clusters_field = _manual_override_fields(namespace)
        with self._mutate(
            folder_path,
            failure_message="Failed to clear a manual override for %s",
        ) as entry:
            overrides = entry.get(overrides_field)
            if not isinstance(overrides, dict) or file_path not in overrides:
                raise _SkipEntryUpdate
            del overrides[file_path]

    def clear_all_manual_overrides(
        self,
        folder_path: str,
        *,
        namespace: str = MANUAL_OVERRIDE_NAMESPACE_SIMILARITY,
    ) -> None:
        """Clear all manual overrides for a folder in one namespace."""

        overrides_field, _clusters_field = _manual_override_fields(namespace)
        with self._mutate(
            folder_path,
            failure_message="Failed to clear all manual overrides for %s",
        ) as entry:
            if overrides_field not in entry:
                raise _SkipEntryUpdate
            del entry[overrides_field]
