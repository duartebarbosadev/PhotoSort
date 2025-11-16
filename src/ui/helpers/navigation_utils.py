from __future__ import annotations
from collections import Counter
from typing import Callable, Optional, Sequence, Iterable, Set

# Navigation helpers extracted from MainWindow. These are UI-agnostic and operate on
# ordered path lists plus simple state flags. MainWindow is responsible for mapping
# between QModelIndex <-> paths and invoking these helpers.


def navigate_group_cyclic(
    sibling_paths: Sequence[str],
    current_path: Optional[str],
    direction: str,
    skip_deleted: bool,
    deleted_paths: Set[str] | Iterable[str],
) -> Optional[str]:
    """Return the next path within a logical sibling group (left/right cyclic).

    direction: 'left' or 'right'
    skip_deleted: if True skip any path in deleted_paths
    Wraps around (cyclic) matching original behavior.
    """
    if not sibling_paths:
        return None
    deleted_set = set(deleted_paths)

    # Build the candidate list respecting skip_deleted
    if skip_deleted:
        candidates = [p for p in sibling_paths if p not in deleted_set]
    else:
        candidates = list(sibling_paths)
    if not candidates:
        return None

    if current_path not in candidates:
        # Default to first element for deterministic behavior
        return candidates[0]

    idx = candidates.index(current_path)
    if direction == "left":
        return candidates[(idx - 1) % len(candidates)]
    elif direction == "right":
        return candidates[(idx + 1) % len(candidates)]
    else:  # Unknown direction
        return current_path


def navigate_linear(
    ordered_paths: Sequence[str],
    current_path: Optional[str],
    direction: str,
    skip_deleted: bool,
    deleted_paths: Set[str] | Iterable[str],
) -> Optional[str]:
    """Return next path in a flat ordering (up/down semantics).

    direction: 'up' or 'down'
    Behavior matches legacy:
      - If no current_path: 'down' => first, 'up' => last
      - No wrap-around
      - If skip_deleted, deleted items are skipped.
    """
    if not ordered_paths:
        return None
    deleted_set = set(deleted_paths)

    def is_ok(p: str) -> bool:
        return (p not in deleted_set) if skip_deleted else True

    # Establish starting index
    if current_path in ordered_paths:
        start_idx = ordered_paths.index(current_path)
    else:
        start_idx = -1  # Force selection rule below

    if direction == "down":
        if start_idx == -1:  # No current selection
            # pick first acceptable
            for p in ordered_paths:
                if is_ok(p):
                    return p
            return None
        # iterate forward
        for i in range(start_idx + 1, len(ordered_paths)):
            p = ordered_paths[i]
            if is_ok(p):
                return p
        return None  # No further candidate
    elif direction == "up":
        if start_idx == -1:
            # pick last acceptable
            for p in reversed(ordered_paths):
                if is_ok(p):
                    return p
            return None
        for i in range(start_idx - 1, -1, -1):
            p = ordered_paths[i]
            if is_ok(p):
                return p
        return None
    else:
        return current_path


def _iter_indices(direction: str, current_index: int, total: int):
    if total <= 0:
        return tuple()
    if direction == "down":
        start = current_index + 1 if current_index >= 0 else 0
        return range(max(0, start), total)
    if direction == "up":
        start = current_index - 1 if current_index >= 0 else total - 1
        return range(min(start, total - 1), -1, -1)
    return tuple()


def find_next_rating_match(
    ordered_paths: Sequence[str],
    direction: str,
    current_index: int,
    target_rating: Optional[int],
    rating_lookup: Callable[[str], Optional[int]],
    skip_deleted: bool,
    is_deleted: Optional[Callable[[str], bool]] = None,
) -> Optional[str]:
    if target_rating is None or direction not in {"up", "down"}:
        return None
    total = len(ordered_paths)
    if total == 0:
        return None

    for idx in _iter_indices(direction, current_index, total):
        if idx < 0 or idx >= total:
            continue
        path = ordered_paths[idx]
        if skip_deleted and is_deleted and is_deleted(path):
            continue
        rating = rating_lookup(path) if rating_lookup else None
        if rating == target_rating:
            return path
    return None


def find_next_multi_image_cluster_head(
    ordered_paths: Sequence[str],
    direction: str,
    current_index: int,
    cluster_lookup: Callable[[str], Optional[int]],
    skip_deleted: bool,
    is_deleted: Optional[Callable[[str], bool]] = None,
) -> Optional[str]:
    if direction not in {"up", "down"}:
        return None
    total = len(ordered_paths)
    if total == 0:
        return None
    cluster_values = [
        cluster_lookup(path) if cluster_lookup else None for path in ordered_paths
    ]
    cluster_counts = Counter(cid for cid in cluster_values if cid is not None)
    multi_clusters = {cid for cid, count in cluster_counts.items() if count > 1}
    if not multi_clusters:
        return None

    current_cluster = None
    if 0 <= current_index < total:
        current_cluster = cluster_values[current_index]

    def is_cluster_head(index: int) -> bool:
        cid = cluster_values[index]
        if cid not in multi_clusters:
            return False
        prev_index = index - 1
        while prev_index >= 0:
            prev_path = ordered_paths[prev_index]
            prev_cid = cluster_values[prev_index]
            if skip_deleted and is_deleted and is_deleted(prev_path):
                prev_index -= 1
                continue
            return prev_cid != cid
        return True

    for idx in _iter_indices(direction, current_index, total):
        if idx < 0 or idx >= total:
            continue
        path = ordered_paths[idx]
        if skip_deleted and is_deleted and is_deleted(path):
            continue
        cid = cluster_values[idx]
        if current_cluster is not None and cid == current_cluster:
            continue
        if is_cluster_head(idx):
            return path
    return None


def find_next_in_same_multi_cluster(
    ordered_paths: Sequence[str],
    direction: str,
    current_index: int,
    cluster_lookup: Callable[[str], Optional[int]],
    skip_deleted: bool,
    is_deleted: Optional[Callable[[str], bool]] = None,
) -> Optional[str]:
    """Move within the current multi-image cluster if possible.

    Returns the next path inside the same cluster following display order,
    or None if the current cluster is singleton, unknown, or you are at its edge.
    """
    if direction not in {"up", "down"}:
        return None
    if current_index < 0 or current_index >= len(ordered_paths):
        return None

    current_cluster = cluster_lookup(ordered_paths[current_index])
    if current_cluster is None:
        return None

    # Pre-compute cluster membership for quick lookups
    cluster_values = [
        cluster_lookup(p) if cluster_lookup else None for p in ordered_paths
    ]
    cluster_counts = Counter(cid for cid in cluster_values if cid is not None)
    if cluster_counts.get(current_cluster, 0) <= 1:
        return None

    step = 1 if direction == "down" else -1
    idx = current_index + step
    while 0 <= idx < len(ordered_paths):
        if cluster_values[idx] != current_cluster:
            break
        path = ordered_paths[idx]
        if skip_deleted and is_deleted and is_deleted(path):
            idx += step
            continue
        return path
    return None
