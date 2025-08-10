from __future__ import annotations
from typing import List, Optional, Sequence, Iterable

from src.ui.selection_utils import select_next_surviving_path


def compute_next_after_rotation(
    visible_paths_before: Sequence[str],
    accepted_paths: Iterable[str],
    remaining_paths_after: Sequence[str],
    anchor_path: Optional[str] = None,
) -> Optional[str]:
    """Decide which path should be selected after applying rotation(s).

    Delegates core heuristic to ``select_next_surviving_path`` while providing a
    safe default if that returns None. The anchor defaults to the first accepted
    path (stable & deterministic) if not explicitly provided.
    """
    accepted_list = list(dict.fromkeys(accepted_paths))  # de-dupe preserving order
    if not accepted_list:
        return None
    anchor = anchor_path or accepted_list[0]
    candidate = select_next_surviving_path(
        visible_paths_before=visible_paths_before,
        removed_paths=accepted_list,
        anchor_path_before=anchor,
        visible_paths_after=remaining_paths_after,
    )
    if candidate:
        return candidate
    # Fallbacks: try the last remaining, else None
    if remaining_paths_after:
        return remaining_paths_after[min(len(remaining_paths_after) - 1, 0)]
    return None
