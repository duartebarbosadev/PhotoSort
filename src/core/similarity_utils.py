import logging
import os
from typing import Literal
from collections.abc import Sequence

import numpy as np
from PIL import Image
from PIL.ImageOps import exif_transpose

logger = logging.getLogger(__name__)

Orientation = Literal["portrait", "landscape", "square"]


def cosine_similarity(
    first_values: Sequence[float] | np.ndarray,
    second_values: Sequence[float] | np.ndarray,
) -> float | None:
    """Return cosine similarity for two valid embedding vectors."""

    first = np.asarray(first_values, dtype=np.float32).reshape(-1)
    second = np.asarray(second_values, dtype=np.float32).reshape(-1)
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


def build_orientation_map(file_paths: list[str]) -> dict[str, Orientation]:
    """
    Build a mapping of file paths to their orientations.

    Args:
        file_paths: List of image file paths.

    Returns:
        Dictionary mapping each path to its orientation.
    """
    orientation_map: dict[str, Orientation] = {}
    for path in file_paths:
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


def build_regional_distance_matrix(
    embeddings: dict[str, list[float]],
    regional_embeddings: dict[str, list[list[float]]],
    subset_paths: list[str],
) -> np.ndarray:
    """Build a symmetric distance matrix from shared regional embedding data."""
    region_sets: list[np.ndarray] = []
    for path in subset_paths:
        region_vectors = regional_embeddings.get(path)
        if region_vectors:
            region_matrix = np.asarray(region_vectors, dtype=np.float32)
        else:
            region_matrix = np.asarray([embeddings[path]], dtype=np.float32)
        if region_matrix.ndim != 2 or region_matrix.shape[0] == 0:
            region_matrix = np.asarray([embeddings[path]], dtype=np.float32)
        region_sets.append(region_matrix)

    count = len(subset_paths)
    distances = np.zeros((count, count), dtype=np.float32)
    for first_index in range(count):
        for second_index in range(first_index + 1, count):
            distance = regional_embedding_distance(
                region_sets[first_index], region_sets[second_index]
            )
            distances[first_index, second_index] = distance
            distances[second_index, first_index] = distance
    return distances


def normalize_embedding_vector(values: list[float]) -> tuple[list[float], bool]:
    """Normalize a single embedding vector, returning (normalized_list, changed_flag)."""
    arr = np.asarray(values, dtype=np.float32)
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
