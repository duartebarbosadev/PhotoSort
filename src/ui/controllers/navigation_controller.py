from __future__ import annotations
from typing import List, Optional, Iterable, Protocol, Set
from PyQt6.QtCore import QModelIndex, Qt
from PyQt6.QtWidgets import QAbstractItemView


class NavigationContext(Protocol):
    def get_active_view(self) -> Optional[QAbstractItemView]: ...
    def is_valid_image_index(self, proxy_index: QModelIndex) -> bool: ...
    def map_to_source(self, proxy_index: QModelIndex) -> QModelIndex: ...
    def item_from_source(self, source_index: QModelIndex): ...
    def get_group_sibling_images(self, current_proxy_index: QModelIndex): ...
    def find_first_visible_item(self) -> QModelIndex: ...
    def find_proxy_index_for_path(self, path: str) -> QModelIndex: ...
    def get_all_visible_image_paths(self) -> List[str]: ...
    def get_marked_deleted(self) -> Iterable[str]: ...
    def validate_and_select_image_candidate(
        self, proxy_index: QModelIndex, direction: str, log_skip: bool
    ): ...


def navigate_group_cyclic(
    group_paths: List[str],
    current: Optional[str],
    direction: str,
    skip_deleted: bool,
    deleted_set: Set[str],
) -> Optional[str]:
    if not group_paths:
        return None
    if current not in group_paths:
        return group_paths[0] if direction == "right" else group_paths[-1]
    idx = group_paths.index(current)
    step = -1 if direction == "left" else 1
    for _ in range(len(group_paths)):
        idx = (idx + step) % len(group_paths)
        candidate = group_paths[idx]
        if not skip_deleted or candidate not in deleted_set:
            return candidate
    return None


def navigate_linear(
    all_visible: List[str],
    current: Optional[str],
    direction: str,
    skip_deleted: bool,
    deleted_set: Set[str],
) -> Optional[str]:
    if not all_visible:
        return None
    if current not in all_visible:
        return all_visible[0] if direction == "down" else all_visible[-1]
    idx = all_visible.index(current)
    step = -1 if direction in ("up", "left") else 1
    while True:
        idx += step
        if idx < 0 or idx >= len(all_visible):
            return None
        candidate = all_visible[idx]
        if not skip_deleted or candidate not in deleted_set:
            return candidate


class NavigationController:
    def __init__(self, ctx: NavigationContext):
        self.ctx = ctx

    def navigate_group(self, direction: str, skip_deleted: bool = True):
        active_view = self.ctx.get_active_view()
        if not active_view:
            return
        current_proxy_idx = active_view.currentIndex()
        current_path = None
        if current_proxy_idx.isValid() and self.ctx.is_valid_image_index(
            current_proxy_idx
        ):
            src_idx = self.ctx.map_to_source(current_proxy_idx)
            item = self.ctx.item_from_source(src_idx)
            if item:
                data = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(data, dict):
                    current_path = data.get("path")
        group_paths: List[str] = []
        if current_proxy_idx.isValid():
            _parent_group_idx, group_image_indices, _ = (
                self.ctx.get_group_sibling_images(current_proxy_idx)
            )
            for idx in group_image_indices:
                src_idx = self.ctx.map_to_source(idx)
                item = self.ctx.item_from_source(src_idx)
                if item:
                    d = item.data(Qt.ItemDataRole.UserRole)
                    if isinstance(d, dict) and "path" in d:
                        group_paths.append(d["path"])
        if not group_paths:
            first_item = self.ctx.find_first_visible_item()
            if first_item.isValid():
                sel_model = active_view.selectionModel()
                if sel_model:
                    # Access selection flag dynamically to avoid strict import dependency in tests
                    flag = getattr(sel_model, "SelectionFlag", None)
                    if flag is not None:
                        sel_model.setCurrentIndex(first_item, flag.ClearAndSelect)  # type: ignore[attr-defined]
                    else:
                        sel_model.setCurrentIndex(first_item, 0)
                active_view.setFocus(Qt.FocusReason.ShortcutFocusReason)
            return
        deleted_set = set(self.ctx.get_marked_deleted()) if skip_deleted else set()
        target_path = navigate_group_cyclic(
            group_paths, current_path, direction, skip_deleted, deleted_set
        )
        if target_path:
            proxy_idx = self.ctx.find_proxy_index_for_path(target_path)
            if proxy_idx.isValid():
                self.ctx.validate_and_select_image_candidate(
                    proxy_idx, direction, False
                )

    def navigate_linear(self, direction: str, skip_deleted: bool = True):
        active_view = self.ctx.get_active_view()
        if not active_view:
            return
        all_visible = self.ctx.get_all_visible_image_paths()
        current_path = None
        cur_idx = active_view.currentIndex()
        if cur_idx.isValid() and self.ctx.is_valid_image_index(cur_idx):
            src_idx = self.ctx.map_to_source(cur_idx)
            item = self.ctx.item_from_source(src_idx)
            if item:
                d = item.data(Qt.ItemDataRole.UserRole)
                if isinstance(d, dict):
                    current_path = d.get("path")
        deleted_set = set(self.ctx.get_marked_deleted()) if skip_deleted else set()
        target_path = navigate_linear(
            all_visible, current_path, direction, skip_deleted, deleted_set
        )
        if target_path:
            proxy_idx = self.ctx.find_proxy_index_for_path(target_path)
            if proxy_idx.isValid():
                self.ctx.validate_and_select_image_candidate(
                    proxy_idx, direction, False
                )
