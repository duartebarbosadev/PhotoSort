from typing import List, Optional


def select_next_surviving_path(
    visible_paths_before: List[str],
    removed_paths: List[str],
    anchor_path_before: Optional[str],
    visible_paths_after: List[str],
) -> Optional[str]:
    """Determine the most appropriate next path to select after one or more items
    have been removed (rotation accepted, files deleted, filtered out, etc.).

    Generalized from deletion-specific logic; works for any removal scenario.

    Strategy:
    1. Keep current selection if it still exists.
    2. Establish an anchor index in the original ordering:
       - If anchor_path_before exists, use that index.
       - Else first actually removed path that existed.
       - Else heuristically approximate a locality position (name LCP heuristic) or midpoint.
    3. Scan forward from anchor for first surviving candidate.
    4. If none forward, scan backward.
    5. Fallback: last visible item.
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
        if removed_paths:
            for p in removed_paths:
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
