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
    - Determine an anchor index based on the position of the anchor_path_before
      (typically current selected path or first deleted path) in visible_paths_before.
    - Starting from anchor_index, step forward to the next path that exists in
      visible_paths_after.
    - If none found forward, search backward from anchor_index - 1.
    - If no valid path is found, return None.
    """
    if not visible_paths_after:
        return None

    visible_after_set = set(visible_paths_after)

    # 1. If the original selection still exists, don't move the selection.
    if anchor_path_before and anchor_path_before in visible_after_set:
        return anchor_path_before

    # If we're here, the selected item was deleted. Find the best next item.
    anchor_index = 0
    if visible_paths_before:
        if anchor_path_before and anchor_path_before in visible_paths_before:
            anchor_index = visible_paths_before.index(anchor_path_before)
        # Fallback: if anchor is not in the 'before' list, try to find the first deleted item's index
        elif deleted_paths:
            for p in deleted_paths:
                if p in visible_paths_before:
                    anchor_index = visible_paths_before.index(p)
                    break
    
    # 2. Search forward for the next available item
    if visible_paths_before:
        for i in range(anchor_index + 1, len(visible_paths_before)):
            candidate = visible_paths_before[i]
            if candidate in visible_after_set:
                return candidate

    # 3. If no 'next', search backward for the previous available item
    if visible_paths_before:
        for i in range(anchor_index - 1, -1, -1):
            candidate = visible_paths_before[i]
            if candidate in visible_after_set:
                return candidate

    # 4. As a final fallback, return the last available item in the list
    if visible_paths_after:
        return visible_paths_after[-1]

    return None