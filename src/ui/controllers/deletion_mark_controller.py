# Renamed from deletion_controller.py to align with class name DeletionMarkController
from __future__ import annotations
from typing import Optional, List, Callable, Iterable, Tuple
import os
from PyQt6.QtGui import QStandardItem, QColor
from PyQt6.QtWidgets import QApplication
from PyQt6.QtCore import Qt

from ui.helpers.deletion_utils import build_presentation


class DeletionMarkController:
    """Encapsulates *non-destructive* deletion mark & blur presentation logic.

    Split from FileDeletionController (destructive / filesystem). This class only:
      * toggles mark state in AppState
      * updates in-model QStandardItem presentation (text + color + blur flag)
    Keeping destructive actions separate lowers risk and keeps tests fast & pure.
    """

    ORANGE = QColor("#FFB366")

    def __init__(self, app_state, is_marked_func):
        self.app_state = app_state
        self._is_marked_func = is_marked_func

    # --- Presentation helpers ---
    def apply_presentation(
        self, item: QStandardItem, file_path: str, is_blurred: Optional[bool]
    ):
        # Optimization: Skip marked check if nothing is marked (common case on initial load)
        is_marked = (
            self._is_marked_func(file_path)
            if len(self.app_state.marked_for_deletion) > 0
            else False
        )

        basename = os.path.basename(file_path)
        is_best = file_path in getattr(self.app_state, "best_shot_paths", set())
        pres = build_presentation(
            basename=basename,
            is_marked=is_marked,
            is_blurred=is_blurred,
            is_best=is_best,
        )
        data = item.data(Qt.ItemDataRole.UserRole)
        if isinstance(data, dict):
            data["is_best_shot"] = is_best
            item.setData(data, Qt.ItemDataRole.UserRole)
        if pres.is_marked:
            item.setForeground(self.ORANGE)
        elif pres.is_blurred:
            item.setForeground(QColor(Qt.GlobalColor.red))
        else:
            item.setForeground(QApplication.palette().text().color())
        item.setText(pres.text)

    # --- Mark / Unmark operations ---
    def toggle_mark(self, file_path: str):
        if self._is_marked_func(file_path):
            self.app_state.unmark_for_deletion(file_path)
        else:
            self.app_state.mark_for_deletion(file_path)

    def mark(self, file_path: str):
        if not self._is_marked_func(file_path):
            self.app_state.mark_for_deletion(file_path)

    def unmark(self, file_path: str):
        if self._is_marked_func(file_path):
            self.app_state.unmark_for_deletion(file_path)

    def mark_others(self, keep_path: str, paths: List[str]) -> int:
        count = 0
        for p in paths:
            if p != keep_path and not self._is_marked_func(p):
                self.app_state.mark_for_deletion(p)
                count += 1
        return count

    def unmark_others(self, keep_path: str, paths: List[str]) -> int:
        count = 0
        for p in paths:
            if p != keep_path and self._is_marked_func(p):
                self.app_state.unmark_for_deletion(p)
                count += 1
        return count

    def clear_all(self) -> int:
        marked = list(self.app_state.get_marked_files())
        self.app_state.clear_all_deletion_marks()
        return len(marked)

    # --- Batch / UI integrated operations (delegated from MainWindow) ---
    def _resolve_item(
        self,
        file_path: str,
        find_proxy_index: Callable[[str], object],
        file_system_model,
        proxy_model,
    ) -> Tuple[Optional[QStandardItem], Optional[bool]]:
        proxy_idx = find_proxy_index(file_path)
        if proxy_idx and proxy_idx.isValid():  # type: ignore[attr-defined]
            source_idx = proxy_model.mapToSource(proxy_idx)
            item = file_system_model.itemFromIndex(source_idx)
            if item:
                data = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(data, dict):
                    return item, data.get("is_blurred")
                return item, None
        return None, None

    def _update_item_presentation(
        self,
        item: Optional[QStandardItem],
        file_path: str,
        is_blurred: Optional[bool],
    ):
        if item:
            self.apply_presentation(item, file_path, is_blurred)

    def toggle_paths(
        self,
        paths: Iterable[str],
        find_proxy_index: Callable[[str], object],
        file_system_model,
        proxy_model,
    ) -> int:
        count = 0
        for p in paths:
            if self._is_marked_func(p):
                self.app_state.unmark_for_deletion(p)
            else:
                self.app_state.mark_for_deletion(p)
            item, is_blurred = self._resolve_item(
                p, find_proxy_index, file_system_model, proxy_model
            )
            self._update_item_presentation(item, p, is_blurred)
            count += 1
        return count

    def mark_others_in_collection(
        self,
        keep_path: str,
        collection_paths: Iterable[str],
        find_proxy_index: Callable[[str], object],
        file_system_model,
        proxy_model,
    ) -> int:
        count = 0
        for p in collection_paths:
            if p == keep_path:
                continue
            if not self._is_marked_func(p):
                self.app_state.mark_for_deletion(p)
                count += 1
            item, is_blurred = self._resolve_item(
                p, find_proxy_index, file_system_model, proxy_model
            )
            self._update_item_presentation(item, p, is_blurred)
        return count

    def unmark_others_in_collection(
        self,
        keep_path: str,
        collection_paths: Iterable[str],
        find_proxy_index: Callable[[str], object],
        file_system_model,
        proxy_model,
    ) -> int:
        count = 0
        for p in collection_paths:
            if p == keep_path:
                continue
            if self._is_marked_func(p):
                self.app_state.unmark_for_deletion(p)
                count += 1
            item, is_blurred = self._resolve_item(
                p, find_proxy_index, file_system_model, proxy_model
            )
            self._update_item_presentation(item, p, is_blurred)
        return count

    def clear_all_and_update(
        self,
        find_proxy_index: Callable[[str], object],
        file_system_model,
        proxy_model,
    ) -> int:
        marked_files = list(self.app_state.get_marked_files())
        if not marked_files:
            return 0
        self.app_state.clear_all_deletion_marks()
        for p in marked_files:
            item, is_blurred = self._resolve_item(
                p, find_proxy_index, file_system_model, proxy_model
            )
            self._update_item_presentation(item, p, is_blurred)
        return len(marked_files)

    def update_blur_status(
        self,
        image_path: str,
        is_blurred: bool,
        find_proxy_index: Callable[[str], object],
        file_system_model,
        proxy_model,
        active_view_getter: Callable[[], object],
        selection_changed_callback: Callable[[], None],
    ):
        item, _prev_blur = self._resolve_item(
            image_path, find_proxy_index, file_system_model, proxy_model
        )
        if item:
            data = item.data(Qt.ItemDataRole.UserRole)
            if isinstance(data, dict):
                data["is_blurred"] = is_blurred
                item.setData(data, Qt.ItemDataRole.UserRole)
            self.apply_presentation(item, image_path, is_blurred)
            active_view = active_view_getter()
            if active_view and active_view.currentIndex().isValid():  # type: ignore[attr-defined]
                cur_proxy = active_view.currentIndex()  # type: ignore[attr-defined]
                cur_source = proxy_model.mapToSource(cur_proxy)
                selected_item = file_system_model.itemFromIndex(cur_source)
                if selected_item == item:
                    selection_changed_callback()
