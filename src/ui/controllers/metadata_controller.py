from __future__ import annotations
from typing import Protocol, List


class MetadataContext(Protocol):
    metadata_sidebar: object | None
    sidebar_visible: bool

    def get_selected_file_paths(self) -> List[str]: ...
    def ensure_metadata_sidebar(self) -> None: ...


class MetadataController:
    def __init__(self, ctx: MetadataContext):
        self.ctx = ctx
        self._cached_selection: List[str] = []

    def refresh_for_selection(self):
        if not self.ctx.sidebar_visible or not self.ctx.metadata_sidebar:
            return
        paths = self.ctx.get_selected_file_paths()
        if paths == self._cached_selection:
            return
        self._cached_selection = paths
        sidebar = self.ctx.metadata_sidebar
        if hasattr(sidebar, "update_selection"):
            sidebar.update_selection(paths)
