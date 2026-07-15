"""Prioritized, folder-wide thumbnail scheduling for shared file views."""

from __future__ import annotations

from typing import Iterable, List, Optional
from uuid import uuid4

from PyQt6.QtCore import QObject, QPoint, QTimer, Qt
from PyQt6.QtWidgets import QTreeView

from core.app_settings import (
    THUMBNAIL_PRELOAD_BATCH_SIZE,
    THUMBNAIL_PRELOAD_VISIBLE_MARGIN,
)

THUMBNAIL_SCROLL_IDLE_MS = 75


class ViewportThumbnailLoader(QObject):
    """Materialize a folder while prioritizing paths near active viewports."""

    def __init__(self, context, parent=None):
        super().__init__(parent)
        self.context = context
        self._session_id = ""
        self._all_paths: list[str] = []
        self._all_path_set: set[str] = set()
        self._materialized_paths: set[str] = set()
        self._warm_complete = False
        self._foreground_session_ids: set[str] = set()
        self._load_timer = QTimer(self)
        self._load_timer.setSingleShot(True)
        self._load_timer.setInterval(THUMBNAIL_SCROLL_IDLE_MS)
        self._load_timer.timeout.connect(self._load_visible_batch)
        self._layout_retry_timer = QTimer(self)
        self._layout_retry_timer.setSingleShot(True)
        self._layout_retry_timer.setInterval(250)
        self._layout_retry_timer.timeout.connect(self._load_visible_batch)

        manager = context.worker_manager
        manager.thumbnail_session_batch_ready.connect(self._handle_batch_ready)
        manager.thumbnail_session_progress.connect(self._handle_progress)
        manager.thumbnail_session_finished.connect(self._handle_finished)
        manager.thumbnail_session_error.connect(self._handle_error)

    def start_folder(self, image_paths: Iterable[str]) -> None:
        """Begin one non-blocking warming session for the active folder."""
        self.reset(stop_worker=True)
        self._all_paths = list(dict.fromkeys(path for path in image_paths if path))
        self._all_path_set = set(self._all_paths)
        if not self._all_paths or not self._enabled():
            return
        self._session_id = uuid4().hex
        visible = [
            path
            for path in dict.fromkeys(self._visible_paths())
            if path in self._all_path_set
        ]
        self.context.set_thumbnail_progress(0, len(self._all_paths), 0, False)
        self.context.worker_manager.start_thumbnail_session(
            self._session_id,
            self._all_paths,
            visible,
        )

    def reset(self, *, stop_worker: bool = False) -> None:
        self._load_timer.stop()
        self._layout_retry_timer.stop()
        if stop_worker and self.context.worker_manager.is_thumbnail_preload_running():
            self.context.worker_manager.stop_thumbnail_preload()
        self._session_id = ""
        self._all_paths = []
        self._all_path_set.clear()
        self._materialized_paths.clear()
        self._foreground_session_ids.clear()
        self._warm_complete = False
        self.context.hide_thumbnail_progress()

    def set_enabled(self, enabled: bool) -> None:
        if not enabled:
            self.reset(stop_worker=True)
            return
        paths = [
            item.get("path")
            for item in self.context.app_state.image_files_data
            if item.get("path")
        ]
        self.start_folder(paths)

    def invalidate_paths(self, image_paths) -> None:
        paths = [path for path in dict.fromkeys(image_paths) if path]
        self.context.remove_cached_thumbnail_icons(paths)
        self._materialized_paths.difference_update(paths)
        if not paths or not self._session_id:
            return
        if not self.context.worker_manager.prioritize_thumbnail_paths(
            self._session_id, paths
        ):
            self._start_foreground_session(paths)

    def schedule(self, *_args) -> None:
        if self._session_id:
            self._load_timer.start()

    def model_rebuilt(self) -> None:
        """Request icons for new item objects without restarting folder warming."""
        self._materialized_paths.clear()
        self.schedule()
        # QTreeWidget geometry is not reliable until its first layout pass. A
        # second request applies the initial viewport without requiring a scroll.
        self._layout_retry_timer.start()

    def _enabled(self) -> bool:
        return self.context.menu_manager.toggle_thumbnails_action.isChecked()

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
        if not self._enabled() or not self._session_id:
            return
        visible = [
            path
            for path in dict.fromkeys(self._visible_paths())
            if path in self._all_path_set
        ]
        pending = [path for path in visible if path not in self._materialized_paths]
        if not pending:
            return
        if self.context.worker_manager.prioritize_thumbnail_paths(
            self._session_id, pending
        ):
            return
        self._start_foreground_session(pending)

    def _start_foreground_session(self, paths: list[str]) -> None:
        if self.context.worker_manager.is_thumbnail_preload_running():
            # Cleanup is queued on the UI thread; retry after it releases the slot.
            self._load_timer.start(50)
            return
        session_id = f"{self._session_id}:foreground:{uuid4().hex}"
        if self.context.worker_manager.start_thumbnail_session(
            session_id, paths, paths
        ):
            self._foreground_session_ids.add(session_id)

    def _handle_batch_ready(self, session_id: str, image_paths) -> None:
        if (
            session_id != self._session_id
            and session_id not in self._foreground_session_ids
        ):
            return
        # Apply the worker's exact completion batch. Background batches populate
        # the shared UI-icon cache for models that do not exist yet; foreground
        # batches remain correct even if navigation shifts before this callback.
        applicable = list(dict.fromkeys(image_paths or []))
        if not applicable:
            return
        self.context._update_thumbnails_from_cache(applicable)
        grouping_widget = getattr(self.context, "grouping_step_widget", None)
        if grouping_widget is not None:
            grouping_widget.refresh_cached_thumbnails(applicable)
        # Disk-only background results will not materialize yet. Reprioritizing
        # them promotes the cache hit to memory and produces a second callback.
        materialized = {
            path
            for path in applicable
            if self.context.get_cached_thumbnail_icon(path) is not None
        }
        self._materialized_paths.update(materialized)
        missing = set(applicable) - materialized
        if missing and session_id == self._session_id:
            self.context.worker_manager.prioritize_thumbnail_paths(
                self._session_id, list(missing)
            )

    def _handle_progress(
        self,
        session_id: str,
        attempted: int,
        total: int,
        failures: int,
        paused: bool,
    ) -> None:
        if session_id != self._session_id:
            return
        self.context.set_thumbnail_progress(attempted, total, failures, paused)

    def _handle_finished(self, session_id: str, attempted: int, failures: int) -> None:
        if session_id in self._foreground_session_ids:
            self._foreground_session_ids.discard(session_id)
            return
        if session_id != self._session_id:
            return
        self._warm_complete = True
        self.context.hide_thumbnail_progress()
        self.schedule()

    def _handle_error(self, session_id: str, message: str) -> None:
        if session_id == self._session_id:
            self.context.hide_thumbnail_progress()
