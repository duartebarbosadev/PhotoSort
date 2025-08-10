from __future__ import annotations
from typing import Protocol, List, Dict


class PreviewContext(Protocol):
    worker_manager: object

    def show_loading_overlay(self, text: str) -> None: ...
    def hide_loading_overlay(self) -> None: ...
    def status_message(self, msg: str, timeout: int = 3000) -> None: ...


class PreviewController:
    def __init__(self, ctx: PreviewContext):
        self.ctx = ctx

    def start_preload(self, image_data_list: List[Dict[str, any]]):
        if not image_data_list:
            self.ctx.hide_loading_overlay()
            self.ctx.status_message("No previews to preload.")
            return
        paths = [
            d.get("path")
            for d in image_data_list
            if isinstance(d, dict) and d.get("path")
        ]
        paths = [p for p in paths if p]
        if not paths:
            self.ctx.hide_loading_overlay()
            return
        self.ctx.worker_manager.start_preview_preload(paths)
