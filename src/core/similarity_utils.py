import logging
import os
from typing import Literal
from collections.abc import Callable, Sequence

import numpy as np
from PIL import Image
from PIL.ImageOps import exif_transpose

logger = logging.getLogger(__name__)

Orientation = Literal["portrait", "landscape", "square"]


class SimilarityAnalysisCancelled(Exception):
    """Raised when a cancellable similarity computation is asked to stop."""


REGIONAL_DISTANCE_BLOCK_TARGET_BYTES = 64 * 1024 * 1024


def cosine_similarity(
    first_values: Sequence[float] | np.ndarray,
    second_values: Sequence[float] | np.ndarray,
) -> float | None:
    """Return cosine similarity for two valid embedding vectors."""

    first: np.ndarray = np.asarray(first_values, dtype=np.float32).reshape(-1)
    second: np.ndarray = np.asarray(second_values, dtype=np.float32).reshape(-1)
    if first.size == 0 or first.shape != second.shape:
        return None
    denominator = float(np.linalg.norm(first) * np.linalg.norm(second))
    if not np.isfinite(denominator) or denominator == 0.0:
        return None
    similarity = float(np.dot(first, second) / denominator)
    if not np.isfinite(similarity):
        return None
    return max(-1.0, min(1.0, similarity))


def _get_raw_dimensions(image_path: str) -> tuple[int, int] | None:
    """Return orientation-corrected RAW dimensions when rawpy supports the file."""
    try:
        from core.image_processing.raw_image_processor import is_raw_extension

        ext = os.path.splitext(image_path)[1].lower()
        if not is_raw_extension(ext):
            return None

        import rawpy

        with rawpy.imread(image_path) as raw:
            sizes = getattr(raw, "sizes", None)
            if sizes is None:
                return None

            width = int(getattr(sizes, "width", 0) or getattr(sizes, "iwidth", 0) or 0)
            height = int(
                getattr(sizes, "height", 0) or getattr(sizes, "iheight", 0) or 0
            )
            flip = int(getattr(sizes, "flip", 0) or 0)
            if flip in {5, 6, 7, 8}:
                width, height = height, width

            if width > 0 and height > 0:
                return width, height
    except Exception:
        logger.warning(
            "Failed to classify RAW orientation for %s, falling back to Pillow",
            image_path,
        )
    return None


def classify_orientation(image_path: str) -> Orientation:
    """
    Classify an image as 'portrait', 'landscape', or 'square'.

    Uses PIL to load the image and applies EXIF orientation correction
    before determining the aspect ratio.

    Args:
        image_path: Path to the image file.

    Returns:
        'portrait' if height > width, 'landscape' if width > height,
        'square' if approximately equal (within 10% ratio).
    """
    raw_dimensions = _get_raw_dimensions(image_path)
    if raw_dimensions is not None:
        width, height = raw_dimensions
    else:
        try:
            with Image.open(image_path) as img:
                # Apply EXIF orientation to get actual visual dimensions
                transposed = exif_transpose(img)
                if transposed is not None:
                    width, height = transposed.size
                else:
                    width, height = img.size
        except Exception:
            logger.warning(
                "Failed to classify orientation for %s, defaulting to landscape",
                image_path,
            )
            return "landscape"

    if width == 0 or height == 0:
        return "landscape"  # Default for invalid dimensions

    aspect_ratio = width / height

    # Square threshold: within 10% of 1:1 ratio
    if 0.9 <= aspect_ratio <= 1.1:
        return "square"
    elif aspect_ratio < 1.0:
        return "portrait"
    else:
        return "landscape"


def build_orientation_map(
    file_paths: list[str],
    should_cancel: Callable[[], bool] | None = None,
) -> dict[str, Orientation]:
    """
    Build a mapping of file paths to their orientations.

    Args:
        file_paths: List of image file paths.

    Returns:
        Dictionary mapping each path to its orientation.
    """
    orientation_map: dict[str, Orientation] = {}
    for path in file_paths:
        if should_cancel is not None and should_cancel():
            raise SimilarityAnalysisCancelled
        orientation_map[path] = classify_orientation(path)
    return orientation_map


def l2_normalize_rows(matrix: np.ndarray) -> np.ndarray:
    """Return a row-wise L2-normalized copy of the matrix."""
    if matrix.size == 0:
        return matrix
    norms = np.linalg.norm(matrix, axis=1, keepdims=True)
    norms = np.where(norms == 0, 1.0, norms)
    return matrix / norms


def regional_embedding_distance(
    first_regions: np.ndarray, second_regions: np.ndarray
) -> float:
    """Return a subject-aware distance between two ordered regional embeddings.

    Region generation is deterministic: the whole image and each large crop occupy
    the same position in every matrix. Comparing aligned regions prevents one
    coincidentally similar background crop from making otherwise different photos
    appear equivalent. Older or partial caches can contain unequal region counts;
    retain the previous best-pair fallback for those incomplete records.
    """
    first = l2_normalize_rows(np.asarray(first_regions, dtype=np.float32))
    second = l2_normalize_rows(np.asarray(second_regions, dtype=np.float32))
    if first.ndim != 2 or second.ndim != 2 or not len(first) or not len(second):
        return 2.0

    similarities = first @ second.T
    if first.shape[0] == second.shape[0] and first.shape[0] > 1:
        similarity = float(np.mean(np.diag(similarities)))
    else:
        similarity = float(np.max(similarities))
    return max(0.0, min(2.0, 1.0 - similarity))


def _normalized_region_sets(
    embeddings: dict[str, list[float]],
    regional_embeddings: dict[str, list[list[float]]],
    subset_paths: list[str],
    should_cancel: Callable[[], bool] | None = None,
) -> list[np.ndarray]:
    """Prepare every regional matrix once for all subsequent comparisons."""

    region_sets: list[np.ndarray] = []
    for path in subset_paths:
        if should_cancel is not None and should_cancel():
            raise SimilarityAnalysisCancelled
        region_vectors = regional_embeddings.get(path)
        if region_vectors:
            region_matrix: np.ndarray = np.asarray(region_vectors, dtype=np.float32)
        else:
            region_matrix = np.asarray([embeddings[path]], dtype=np.float32)
        if region_matrix.ndim != 2 or region_matrix.shape[0] == 0:
            region_matrix = np.asarray([embeddings[path]], dtype=np.float32)
        region_sets.append(l2_normalize_rows(region_matrix))
    return region_sets


def _uniform_regional_features(
    region_sets: list[np.ndarray],
) -> np.ndarray | None:
    """Return flattened exact regional features when all records are compatible.

    Each regional row is already unit-normalized. Dividing the concatenated rows
    by sqrt(region_count) makes a dot product equal to the existing mean of the
    corresponding regional cosine similarities.
    """

    if not region_sets:
        return np.empty((0, 0), dtype=np.float32)
    expected_shape = region_sets[0].shape
    if len(expected_shape) != 2 or any(
        region_set.shape != expected_shape for region_set in region_sets
    ):
        return None
    region_count = expected_shape[0]
    if region_count <= 0:
        return None
    stacked = np.stack(region_sets).astype(np.float32, copy=False)
    return np.ascontiguousarray(
        stacked.reshape(len(region_sets), -1) / np.sqrt(float(region_count))
    )


def _regional_distance_from_normalized(
    first_regions: np.ndarray, second_regions: np.ndarray
) -> float:
    """Apply the established regional-distance rule to normalized matrices."""

    if first_regions.shape == second_regions.shape and len(first_regions) > 1:
        similarity = float(np.mean(np.sum(first_regions * second_regions, axis=1)))
    else:
        similarity = float(np.max(first_regions @ second_regions.T))
    return max(0.0, min(2.0, 1.0 - similarity))


def _distance_block_rows(count: int, target_bytes: int) -> int:
    if count <= 0:
        return 1
    return max(
        1, min(count, target_bytes // max(1, count * np.dtype(np.float32).itemsize))
    )


def build_regional_distance_matrix(
    embeddings: dict[str, list[float]],
    regional_embeddings: dict[str, list[list[float]]],
    subset_paths: list[str],
    should_cancel: Callable[[], bool] | None = None,
    progress_callback: Callable[[int], None] | None = None,
) -> np.ndarray:
    """Build a symmetric distance matrix from shared regional embedding data.

    The matrix is quadratic in the number of images, so callers running in a
    worker thread can provide a cancellation predicate. It is checked once per
    row to keep cancellation responsive without adding work to every pair.
    """
    region_sets = _normalized_region_sets(
        embeddings, regional_embeddings, subset_paths, should_cancel
    )
    count = len(subset_paths)
    distances: np.ndarray = np.zeros((count, count), dtype=np.float32)
    uniform_features = _uniform_regional_features(region_sets)
    if uniform_features is not None:
        block_rows = _distance_block_rows(count, REGIONAL_DISTANCE_BLOCK_TARGET_BYTES)
        for start in range(0, count, block_rows):
            if should_cancel is not None and should_cancel():
                raise SimilarityAnalysisCancelled
            end = min(count, start + block_rows)
            similarities = uniform_features[start:end] @ uniform_features.T
            distances[start:end] = np.clip(1.0 - similarities, 0.0, 2.0).astype(
                np.float32, copy=False
            )
            if progress_callback is not None and count:
                progress_callback(int(end / count * 100))
        np.fill_diagonal(distances, 0.0)
        return distances

    for first_index in range(count):
        if should_cancel is not None and should_cancel():
            raise SimilarityAnalysisCancelled
        for second_index in range(first_index + 1, count):
            distance = _regional_distance_from_normalized(
                region_sets[first_index], region_sets[second_index]
            )
            distances[first_index, second_index] = distance
            distances[second_index, first_index] = distance
        if progress_callback is not None and count:
            progress_callback(int((first_index + 1) / count * 100))
    return distances


def build_regional_neighborhood_graph(
    embeddings: dict[str, list[float]],
    regional_embeddings: dict[str, list[list[float]]],
    subset_paths: list[str],
    eps: float,
    should_cancel: Callable[[], bool] | None = None,
    progress_callback: Callable[[int], None] | None = None,
):
    """Build an exact sparse epsilon-neighbour graph for regional DBSCAN.

    The graph contains every distance at or below ``eps``, including explicit
    zero-distance edges between distinct identical images. Work is performed in
    bounded row blocks so large libraries do not require a dense NxN allocation.
    """

    from scipy.sparse import coo_matrix

    region_sets = _normalized_region_sets(
        embeddings, regional_embeddings, subset_paths, should_cancel
    )
    uniform_features = _uniform_regional_features(region_sets)
    count = len(subset_paths)
    if uniform_features is None:
        row_values = list(range(count))
        column_values = list(range(count))
        distance_values = [0.0] * count
        for first_index in range(count):
            if should_cancel is not None and should_cancel():
                raise SimilarityAnalysisCancelled
            for second_index in range(first_index + 1, count):
                distance = _regional_distance_from_normalized(
                    region_sets[first_index], region_sets[second_index]
                )
                if distance <= eps:
                    row_values.extend((first_index, second_index))
                    column_values.extend((second_index, first_index))
                    distance_values.extend((distance, distance))
            if progress_callback is not None and count:
                progress_callback(int((first_index + 1) / count * 100))
        return coo_matrix(
            (
                np.asarray(distance_values, dtype=np.float32),
                (
                    np.asarray(row_values, dtype=np.int64),
                    np.asarray(column_values, dtype=np.int64),
                ),
            ),
            shape=(count, count),
            dtype=np.float32,
        ).tocsr()

    row_parts: list[np.ndarray] = []
    column_parts: list[np.ndarray] = []
    value_parts: list[np.ndarray] = []
    block_rows = _distance_block_rows(count, REGIONAL_DISTANCE_BLOCK_TARGET_BYTES)
    for start in range(0, count, block_rows):
        if should_cancel is not None and should_cancel():
            raise SimilarityAnalysisCancelled
        end = min(count, start + block_rows)
        distances = np.clip(
            1.0 - (uniform_features[start:end] @ uniform_features.T),
            0.0,
            2.0,
        ).astype(np.float32, copy=False)
        local_rows, columns = np.nonzero(distances <= eps)
        row_parts.append(local_rows.astype(np.int64, copy=False) + start)
        column_parts.append(columns.astype(np.int64, copy=False))
        value_parts.append(distances[local_rows, columns])
        if progress_callback is not None and count:
            progress_callback(int(end / count * 100))

    if not row_parts:
        return coo_matrix((count, count), dtype=np.float32).tocsr()
    graph = coo_matrix(
        (
            np.concatenate(value_parts),
            (np.concatenate(row_parts), np.concatenate(column_parts)),
        ),
        shape=(count, count),
        dtype=np.float32,
    ).tocsr()
    graph.sort_indices()
    return graph


def normalize_embedding_vector(values: list[float]) -> tuple[list[float], bool]:
    """Normalize a single embedding vector, returning (normalized_list, changed_flag)."""
    arr: np.ndarray = np.asarray(values, dtype=np.float32)
    norm = float(np.linalg.norm(arr))
    if not np.isfinite(norm) or norm == 0.0:
        return arr.tolist(), False
    if abs(norm - 1.0) <= 1e-4:
        return arr.tolist(), False
    return (arr / norm).tolist(), True


def normalize_embedding_dict(embeddings: dict[str, list[float]]) -> bool:
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
        from sklearn.neighbors import NearestNeighbors

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
