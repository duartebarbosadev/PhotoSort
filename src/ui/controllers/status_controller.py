"""Status-bar and library-context presentation."""

import os
from typing import Any, Protocol


class StatusContext(Protocol):
    app_state: Any
    image_pipeline: Any
    menu_manager: Any

    def statusBar(self) -> Any: ...


class StatusController:
    """Build status text from indexed application state and update the view."""

    def __init__(self, context: StatusContext):
        self.context = context

    def update(self, message_override: str | None = None) -> None:
        ctx = self.context
        if message_override:
            ctx.statusBar().showMessage(message_override)
            left_panel = getattr(ctx, "left_panel", None)
            if left_panel is not None:
                folder_display = self._folder_display_name()
                left_panel.update_context(
                    folder_display,
                    len(ctx.app_state.image_files_data),
                    message_override.replace("Folder: ", "", 1),
                )
            return

        status_text = "No folder loaded. Open a folder to begin."
        sidebar_title = None
        sidebar_subtitle = "Open a folder to start sorting your library."
        sidebar_item_count = 0
        folder_display = self._folder_display_name()

        if ctx.app_state.current_folder_path:
            sidebar_title = folder_display
            scan_active = not ctx.menu_manager.open_folder_action.isEnabled()
            if scan_active:
                sidebar_item_count = len(ctx.app_state.image_files_data)
                status_text = (
                    f"Folder: {folder_display}  |  Scanning... "
                    f"({sidebar_item_count} files found)"
                )
                sidebar_subtitle = (
                    f"Scanning library • {sidebar_item_count} files found"
                )
            elif ctx.app_state.image_files_data:
                summary = ctx.app_state.media_summary()
                total_size_mb = summary.total_size_bytes / (1024 * 1024)
                preview_cache_mb = ctx.image_pipeline.preview_cache.volume() / (
                    1024 * 1024
                )
                status_text = (
                    f"Folder: {folder_display} | Images: {summary.image_count} | "
                    f"Videos: {summary.video_count} ({total_size_mb:.2f} MB) | "
                    f"Preview Cache: {preview_cache_mb:.2f} MB"
                )
                sidebar_item_count = summary.total_items
                sidebar_subtitle = (
                    f"{summary.image_count} images • {summary.video_count} videos • "
                    f"{total_size_mb:.1f} MB"
                )
            else:
                status_text = f"Folder: {folder_display}  |  Files: 0 (0.00 MB)"
                sidebar_subtitle = "No supported media files found yet."

        ctx.statusBar().showMessage(status_text)
        left_panel = getattr(ctx, "left_panel", None)
        if left_panel is not None:
            left_panel.update_context(
                sidebar_title,
                sidebar_item_count,
                sidebar_subtitle,
            )

    def _folder_display_name(self) -> str | None:
        folder_path = self.context.app_state.current_folder_path
        if not folder_path:
            return None
        return os.path.basename(folder_path) or folder_path
