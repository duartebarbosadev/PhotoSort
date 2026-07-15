"""Viewport-aware thumbnail scheduling for the main file views."""

from __future__ import annotations

from typing import List, Optional

from PyQt6.QtCore import QObject, QPoint, QTimer, Qt
from PyQt6.QtWidgets import QTreeView

from core.app_settings import (
    THUMBNAIL_PRELOAD_BATCH_SIZE,
    THUMBNAIL_PRELOAD_VISIBLE_MARGIN,
)

THUMBNAIL_SCROLL_IDLE_MS = 75


class ViewportThumbnailLoader(QObject):
    """Requests only thumbnails near the active viewport and deduplicates work."""

    def __init__(self, context, parent=None):
        super().__init__(parent)
        self.context = context
        self._requested_paths: set[str] = set()
        self._load_timer = QTimer(self)
        self._load_timer.setSingleShot(True)
        self._load_timer.setInterval(THUMBNAIL_SCROLL_IDLE_MS)
        self._load_timer.timeout.connect(self._load_visible_batch)

    def reset(self) -> None:
        self._load_timer.stop()
        self._requested_paths.clear()

    def invalidate_paths(self, image_paths) -> None:
        """Allow regenerated files to be requested again without resetting the folder."""
        for path in image_paths:
            self._requested_paths.discard(path)
        self.schedule()

    def schedule(self, *_args) -> None:
        # Restarting the timer lets a held arrow key or scrollbar gesture stay
        # responsive. Thumbnail work resumes as soon as navigation goes idle.
        self._load_timer.start()

    def _visible_paths(self) -> List[str]:
        workflow_provider = getattr(
            self.context,
            "get_workflow_visible_thumbnail_paths",
            None,
        )
        limit = THUMBNAIL_PRELOAD_BATCH_SIZE + (THUMBNAIL_PRELOAD_VISIBLE_MARGIN * 2)
        if callable(workflow_provider):
            workflow_paths: Optional[List[str]] = workflow_provider(limit)
            if workflow_paths is not None:
                return workflow_paths[:limit]

        view = self.context._get_active_file_view()
        if view is None:
            return []

        paths: List[str] = []
        index = view.indexAt(QPoint(1, 1))
        if isinstance(view, QTreeView):
            if not index.isValid():
                index = view.model().index(0, 0)
            for _ in range(THUMBNAIL_PRELOAD_VISIBLE_MARGIN):
                previous = view.indexAbove(index)
                if not previous.isValid():
                    break
                index = previous
            for _ in range(limit):
                if not index.isValid():
                    break
                item_data = index.data(Qt.ItemDataRole.UserRole)
                if isinstance(item_data, dict) and item_data.get("path"):
                    paths.append(item_data["path"])
                index = view.indexBelow(index)
            return paths

        model = view.model()
        start_row = max(
            0,
            (index.row() if index.isValid() else 0) - THUMBNAIL_PRELOAD_VISIBLE_MARGIN,
        )
        for row in range(start_row, min(model.rowCount(), start_row + limit)):
            item_data = model.index(row, 0).data(Qt.ItemDataRole.UserRole)
            if isinstance(item_data, dict) and item_data.get("path"):
                paths.append(item_data["path"])
        return paths

    def _load_visible_batch(self) -> None:
        context = self.context
        if not context.menu_manager.toggle_thumbnails_action.isChecked():
            return
        if context.worker_manager.is_thumbnail_preload_running():
            return

        candidates = self._visible_paths()
        if not candidates:
            candidates = [
                item.get("path")
                for item in context.app_state.image_files_data[
                    :THUMBNAIL_PRELOAD_BATCH_SIZE
                ]
                if item.get("path")
            ]
        pending = [path for path in candidates if path not in self._requested_paths]
        if not pending:
            return
        self._requested_paths.update(pending)
        context.worker_manager.start_thumbnail_preload(pending)
