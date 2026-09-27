"""Small presentation-only helpers shared by every workflow bookmark."""

from PyQt6.QtCore import QItemSelectionModel


def scroll_state(view) -> list[int]:
    return [view.horizontalScrollBar().value(), view.verticalScrollBar().value()]


def restore_scroll(view, value) -> None:
    if isinstance(value, list) and len(value) == 2:
        for bar, position in zip(
            (view.horizontalScrollBar(), view.verticalScrollBar()), value, strict=True
        ):
            if type(position) is int and position >= 0:
                bar.setValue(position)


def string_paths(value) -> list[str]:
    return (
        [path for path in value if isinstance(path, str)]
        if isinstance(value, list)
        else []
    )


def restore_tree_selection(tree, items, paths, current) -> None:
    """Restore surviving selections using existing indexed items, without I/O."""
    selected = [items[path] for path in string_paths(paths) if path in items]
    if not selected:
        return
    was_blocked = tree.blockSignals(True)
    try:
        tree.clearSelection()
        for item in selected:
            parent = item.parent()
            while parent is not None:
                parent.setExpanded(True)
                parent = parent.parent()
            item.setSelected(True)
        item = items.get(current) if isinstance(current, str) else None
        tree.setCurrentItem(
            item or selected[0], 0, QItemSelectionModel.SelectionFlag.NoUpdate
        )
    finally:
        tree.blockSignals(was_blocked)
