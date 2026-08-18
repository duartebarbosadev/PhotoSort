"""Shared ownership of similarity cluster results and their warm cache.

Both the similarity view and the Organize grouping pipeline run the same
embedding + DBSCAN analysis. Keeping the signature, the cache read, the manual
overrides and the cache write here means either workflow warms the cache for the
other instead of re-running an expensive analysis.
"""

from __future__ import annotations

from collections.abc import Callable
import logging

from core.app_settings import DBSCAN_MIN_SAMPLES, get_similarity_clustering_eps
from core.caching.analysis_cache import MANUAL_OVERRIDE_NAMESPACE_SIMILARITY
from core.similarity_cache import (
    FileFingerprint,
    SimilarityClusteringResult,
    build_similarity_signature,
    normalize_cluster_results,
    normalize_fingerprints,
)

logger = logging.getLogger(__name__)


def build_signature(
    engine,
    file_paths: list[str],
    fingerprints: dict[str, FileFingerprint],
) -> str:
    """Describe the inputs of a clustering run, including its tunable settings."""

    return build_similarity_signature(
        file_paths,
        fingerprints,
        model_cache_key=engine.model.cache_key,
        regional_cache_key=engine.model.region_cache_key,
        clustering_eps=get_similarity_clustering_eps(),
        min_samples=DBSCAN_MIN_SAMPLES,
    )


def load_cached_clusters(
    analysis_cache,
    folder_path: str | None,
    *,
    signature: str,
    expected_paths: set[str],
) -> dict[str, int] | None:
    """Return previously computed assignments when their inputs still match."""

    if analysis_cache is None or not folder_path:
        return None
    try:
        return analysis_cache.load_valid_cluster_results(
            folder_path,
            signature=signature,
            expected_paths=expected_paths,
        )
    except Exception:
        logger.exception("Failed to read cached similarity results.")
        return None


def persist_clusters(
    analysis_cache,
    folder_path: str | None,
    clusters: dict[str, int],
    *,
    signature: str,
) -> dict[str, int]:
    """Apply manual overrides and persist the result for every workflow."""

    results = normalize_cluster_results(clusters)
    if analysis_cache is None or not folder_path:
        return results
    try:
        overrides = normalize_cluster_results(
            analysis_cache.get_manual_overrides(
                folder_path,
                namespace=MANUAL_OVERRIDE_NAMESPACE_SIMILARITY,
            )
        )
        for path, cluster_id in overrides.items():
            if path in results:
                results[path] = cluster_id
        analysis_cache.save_cluster_results(folder_path, results, signature=signature)
    except Exception:
        logger.exception("Failed to persist similarity results.")
    return results


def cluster_paths_with_cache(
    engine,
    file_paths: list[str],
    *,
    fingerprints: dict[str, FileFingerprint] | None = None,
    analysis_cache=None,
    folder_path: str | None = None,
    progress_callback: Callable[[int, str], None] | None = None,
) -> SimilarityClusteringResult:
    """Cluster ``file_paths``, reusing and warming the shared analysis cache."""

    caching = analysis_cache is not None and bool(folder_path)
    # Signatures fingerprint every file, so they are only built when a cache
    # exists to store them in.
    signature = ""
    if caching:
        if fingerprints is None:
            fingerprints = normalize_fingerprints(list(file_paths))
        signature = build_signature(engine, file_paths, fingerprints)
        cached = load_cached_clusters(
            analysis_cache,
            folder_path,
            signature=signature,
            expected_paths=set(file_paths),
        )
        if cached is not None:
            logger.info(
                "Reusing %d cached similarity cluster assignments.", len(cached)
            )
            return SimilarityClusteringResult(
                clusters=cached, signature=signature, reused=True
            )

    _embeddings, raw_clusters = engine.run_analysis_sync(
        list(file_paths),
        progress_callback=progress_callback,
    )
    results = persist_clusters(
        analysis_cache, folder_path, raw_clusters, signature=signature
    )
    return SimilarityClusteringResult(
        clusters=results, signature=signature, reused=False
    )
