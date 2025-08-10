from __future__ import annotations
from typing import List, Optional, Sequence, Iterable, Set

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
