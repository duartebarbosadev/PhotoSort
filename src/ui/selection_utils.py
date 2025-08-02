from typing import List, Optional


def find_next_visible_path_after_deletions(
    visible_paths_before: List[str],
    deleted_paths: List[str],
    anchor_path_before: Optional[str],
    visible_paths_after: List[str],
) -> Optional[str]:
    """
    From the pre-deletion ordering and the set of deleted paths, find the next
    valid (non-deleted) path to select after deletions occur.

    Strategy:
    - Prefer keeping the current selection if it still exists.
    - Choose an anchor index from the pre-deletion ordering:
        * If the anchor_path_before exists in visible_paths_before, use its index.
        * Else, use the first deleted path that appears in visible_paths_before.
        * Else, use the position where the anchor_path_before would have been inserted
          in visible_paths_before (nearest-neighbor fallback) to keep locality.
    - Search outward from the anchor index, choosing the nearest surviving neighbor:
        * Check forward (+1, +2, ...) first to advance naturally.
        * If none forward, search backward (-1, -2, ...).
    - As a final fallback, return the closest end item (last item).
    """
    if not visible_paths_after:
        return None

    # Fast path: if current selection still visible, keep it.
    visible_after_set = set(visible_paths_after)
    if anchor_path_before and anchor_path_before in visible_after_set:
        return anchor_path_before

    if not visible_paths_before:
        # Nothing to reference; return last item as a stable fallback
        return visible_paths_after[-1]

    # Determine a stable anchor index in the 'before' ordering
    anchor_index: int

    if anchor_path_before and anchor_path_before in visible_paths_before:
        anchor_index = visible_paths_before.index(anchor_path_before)
    else:
        # Try to anchor at the first deleted item that was present before
        anchor_index = -1
        if deleted_paths:
            for p in deleted_paths:
                if p in visible_paths_before:
                    anchor_index = visible_paths_before.index(p)
                    break

        if anchor_index == -1:
            # Nearest-neighbor fallback: compute where the anchor would have been inserted
            # and use that vicinity to pick the next path to select.
            # This keeps selection local even when neither anchor nor deleted paths are present.
            # If anchor_path_before is None or not comparable, fall back to the middle.
            try:
                if anchor_path_before:
                    # Find nearest index by lexicographic proximity on normalized strings
                    ap = anchor_path_before
                    # Simple binary-search-like insertion point emulation
                    # Since we don't rely on ordering, approximate with min distance by name
                    # Choose the closest index by minimal absolute difference of indices
                    # based on name similarity heuristic: pick index of the first item
                    # whose basename shares the longest common prefix with the anchor's basename.
                    import os as _os

                    anchor_base = _os.path.basename(ap)
                    best_idx, best_score = 0, -1
                    for idx, candidate in enumerate(visible_paths_before):
                        cand_base = _os.path.basename(candidate)
                        # Longest common prefix length as a cheap locality proxy
                        lcp = 0
                        for a_ch, c_ch in zip(anchor_base, cand_base):
                            if a_ch == c_ch:
                                lcp += 1
                            else:
                                break
                        if lcp > best_score:
                            best_score, best_idx = lcp, idx
                    anchor_index = best_idx
                else:
                    anchor_index = max(
                        0,
                        min(
                            len(visible_paths_before) - 1,
                            len(visible_paths_before) // 2,
                        ),
                    )
            except Exception:
                anchor_index = 0

    # Outward nearest-neighbor search preferring forward movement first
    n = len(visible_paths_before)
    # First, try forward from anchor_index + 1
    for i in range(anchor_index + 1, n):
        candidate = visible_paths_before[i]
        if candidate in visible_after_set:
            return candidate

    # If nothing forward, try backward from anchor_index - 1
    for i in range(anchor_index - 1, -1, -1):
        candidate = visible_paths_before[i]
        if candidate in visible_after_set:
            return candidate

    # Final fallback: last available item
    return visible_paths_after[-1] if visible_paths_after else None
