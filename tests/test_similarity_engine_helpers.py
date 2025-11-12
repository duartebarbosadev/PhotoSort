import numpy as np
import pytest

pytest.importorskip("sklearn")

from core.similarity_utils import (
    adaptive_dbscan_eps,
    l2_normalize_rows,
    normalize_embedding_vector,
)


def test_l2_normalize_rows_produces_unit_norm_rows():
    data = np.array([[3.0, 4.0], [1.0, 1.0], [0.0, 0.0]], dtype=np.float32)
    normalized = l2_normalize_rows(data.copy())
    norms = np.linalg.norm(normalized[:2], axis=1)
    assert np.allclose(norms, np.ones_like(norms), atol=1e-6)
    # Zero vector remains zero after normalization
    assert np.allclose(normalized[2], np.zeros_like(normalized[2]))


def test_normalize_embedding_vector_flags_updates():
    vec = [2.0, 0.0]
    normalized, changed = normalize_embedding_vector(vec)
    assert changed is True
    assert np.allclose(np.linalg.norm(normalized), 1.0, atol=1e-6)

    already_unit = [1.0, 0.0]
    normalized_same, changed_same = normalize_embedding_vector(already_unit)
    assert changed_same is False
    assert normalized_same == already_unit


def test_adaptive_eps_distinguishes_dense_and_sparse_sets():
    dense = np.vstack(
        [np.ones(8, dtype=np.float32), np.ones(8, dtype=np.float32) * 1.01]
    )
    dense = l2_normalize_rows(dense)
    sparse = np.eye(8, dtype=np.float32)
    sparse = l2_normalize_rows(sparse)
    base_eps = 0.05
    dense_eps = adaptive_dbscan_eps(dense, base_eps, min_samples=2)
    sparse_eps = adaptive_dbscan_eps(sparse, base_eps, min_samples=2)

    assert 0.005 <= dense_eps <= 0.3
    assert 0.005 <= sparse_eps <= 0.3
    assert dense_eps <= sparse_eps


def test_adaptive_eps_respects_min_samples_neighbor():
    rng = np.random.default_rng(0)
    cluster_a = rng.normal(scale=1e-3, size=(3, 8)).astype(np.float32)
    cluster_a[:, 0] += 1.0
    cluster_b = rng.normal(scale=1e-3, size=(3, 8)).astype(np.float32)
    cluster_b[:, 1] += 1.0
    data = np.vstack([cluster_a, cluster_b])
    data = l2_normalize_rows(data)
    base_eps = 0.05
    eps = adaptive_dbscan_eps(data, base_eps, min_samples=3)
    assert eps < 0.2
