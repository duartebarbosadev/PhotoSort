from __future__ import annotations

import logging
from typing import Dict, List, Tuple

import numpy as np
from sklearn.neighbors import NearestNeighbors

logger = logging.getLogger(__name__)


def l2_normalize_rows(matrix: np.ndarray) -> np.ndarray:
    """Return a row-wise L2-normalized copy of the matrix."""
    if matrix.size == 0:
        return matrix
    norms = np.linalg.norm(matrix, axis=1, keepdims=True)
    norms = np.where(norms == 0, 1.0, norms)
    return matrix / norms


def normalize_embedding_vector(values: List[float]) -> Tuple[List[float], bool]:
    """Normalize a single embedding vector, returning (normalized_list, changed_flag)."""
    arr = np.asarray(values, dtype=np.float32)
    norm = float(np.linalg.norm(arr))
    if not np.isfinite(norm) or norm == 0.0:
        return arr.tolist(), False
    if abs(norm - 1.0) <= 1e-4:
        return arr.tolist(), False
    return (arr / norm).tolist(), True


def normalize_embedding_dict(embeddings: Dict[str, List[float]]) -> bool:
    """Normalize all embedding vectors in-place. Returns True if any were updated."""
    updated = False
    for path, vector in list(embeddings.items()):
        if not isinstance(vector, (list, tuple, np.ndarray)):
            continue
        normalized, changed = normalize_embedding_vector(list(vector))
        if changed:
            embeddings[path] = normalized
            updated = True
    return updated


def adaptive_dbscan_eps(
    embedding_matrix: np.ndarray, base_eps: float, min_samples: int
) -> float:
    """Estimate a data-driven epsilon for DBSCAN using cosine k-distances."""
    sample_count = embedding_matrix.shape[0]
    if sample_count <= max(min_samples * 2, 4):
        return base_eps
    neighbor_count = min(
        max(min_samples + 1, min_samples * 3), sample_count
    )  # ensure > min_samples
    try:
        nn = NearestNeighbors(metric="cosine", n_neighbors=neighbor_count)
        nn.fit(embedding_matrix)
        distances, _ = nn.kneighbors(embedding_matrix)
    except Exception:
        logger.exception("Adaptive eps estimation failed; falling back to base epsilon")
        return base_eps

    kth_index = min_samples - 1
    if kth_index < 0:
        return base_eps
    kth_index = min(kth_index, distances.shape[1] - 1)
    kth_distances = distances[:, kth_index]
    finite = kth_distances[np.isfinite(kth_distances)]
    if finite.size == 0:
        return base_eps

    adaptive_component = float(np.percentile(finite, 65))
    adaptive_component = max(0.005, min(0.3, adaptive_component))
    return float((adaptive_component + base_eps) / 2.0)
