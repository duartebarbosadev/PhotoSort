"""Cache-management behavior extracted from the main window."""

from __future__ import annotations

import logging
from typing import Any, Protocol

from core.app_settings import (
    get_preview_cache_size_gb,
    set_exif_cache_size_mb,
    set_preview_cache_size_gb,
)

logger = logging.getLogger(__name__)


class CacheContext(Protocol):
    """View operations and state required by :class:`CacheController`."""

    image_pipeline: Any
    app_state: Any
    menu_manager: Any
    cluster_filter_combo: Any
    cluster_sort_combo: Any
    group_by_similarity_mode: bool
    preview_cache_size_combo: Any
    preview_cache_size_options_gb: list[float]
    exif_cache_size_combo: Any
    exif_cache_size_options_mb: list[int]

    def status_message(self, message: str, timeout: int = 3000) -> None: ...
    def refresh_navigation_shortcut_actions(self) -> None: ...
    def _rebuild_model_view(self) -> None: ...
    def _refresh_visible_items_icons(self) -> None: ...
    def _refresh_current_selection_preview(self) -> None: ...


class CacheController:
    """Coordinate cache settings, clearing, and the resulting view refreshes."""

    def __init__(self, context: CacheContext):
        self.context = context

    def update_labels(self) -> None:
        ctx = self.context
        thumbnail_bytes = ctx.image_pipeline.thumbnail_cache.volume()
        ctx.thumb_cache_usage_label.setText(f"{thumbnail_bytes / (1024 * 1024):.2f} MB")

        configured_gb = get_preview_cache_size_gb()
        ctx.preview_cache_configured_limit_label.setText(f"{configured_gb:.2f} GB")
        preview_bytes = ctx.image_pipeline.preview_cache.volume()
        ctx.preview_cache_usage_label.setText(f"{preview_bytes / (1024 * 1024):.2f} MB")

        exif_cache = getattr(ctx.app_state, "exif_disk_cache", None)
        if exif_cache:
            configured_mb = exif_cache.get_current_size_limit_mb()
            ctx.exif_cache_configured_limit_label.setText(f"{configured_mb} MB")
            ctx.exif_cache_usage_label.setText(
                f"{exif_cache.volume() / (1024 * 1024):.2f} MB"
            )
        else:
            ctx.exif_cache_configured_limit_label.setText("N/A")
            ctx.exif_cache_usage_label.setText("N/A")

        analysis_label = getattr(ctx, "analysis_cache_usage_label", None)
        if analysis_label is not None:
            usage_text = "N/A"
            analysis_cache = getattr(ctx.app_state, "analysis_cache", None)
            if analysis_cache:
                try:
                    usage_text = f"{analysis_cache.volume() / (1024 * 1024):.2f} MB"
                except Exception:
                    logger.exception("Failed to read analysis cache usage.")
                    usage_text = "Error"
            analysis_label.setText(usage_text)

    def clear_thumbnail_cache(self) -> None:
        ctx = self.context
        ctx.image_pipeline.thumbnail_cache.clear()
        ctx.status_message("Thumbnail cache cleared.", 5000)
        self.update_labels()
        ctx._refresh_visible_items_icons()

    def clear_preview_cache(self) -> None:
        ctx = self.context
        ctx.image_pipeline.preview_cache.clear()
        ctx.status_message("Preview cache cleared. Previews will regenerate.", 5000)
        self.update_labels()
        ctx._refresh_current_selection_preview()

    def clear_analysis_cache(self) -> None:
        ctx = self.context
        analysis_cache = getattr(ctx.app_state, "analysis_cache", None)
        if not analysis_cache:
            ctx.status_message("Analysis cache is not available.")
            return

        try:
            analysis_cache.clear_all()
            ctx.status_message("Analysis cache cleared.", 5000)
        except Exception:
            logger.exception("Failed to clear analysis cache.")
            ctx.status_message("Failed to clear analysis cache.", 5000)
        finally:
            self._reset_analysis_ui()
            self.update_labels()

    def _reset_analysis_ui(self) -> None:
        ctx = self.context
        ctx.group_by_similarity_mode = False
        ctx.app_state.cluster_results.clear()
        ctx.app_state.clear_best_shot_results()
        ctx.cluster_filter_combo.clear()
        ctx.cluster_filter_combo.addItem("All Clusters")
        ctx.cluster_filter_combo.setEnabled(False)

        has_media = bool(ctx.app_state.image_files_data)
        menu = ctx.menu_manager
        menu.group_by_similarity_action.setChecked(False)
        menu.group_by_similarity_action.setEnabled(has_media)
        menu.set_cluster_sort_menu_visible(False)
        menu.set_cluster_sort_menu_enabled(False)
        ctx.cluster_sort_combo.setEnabled(False)
        menu.analyze_best_shots_action.setEnabled(False)
        menu.stop_best_shots_action.setEnabled(False)
        if hasattr(menu, "analyze_best_shots_selected_action"):
            menu.analyze_best_shots_selected_action.setEnabled(has_media)
        if hasattr(menu, "analyze_similarity_action"):
            menu.analyze_similarity_action.setEnabled(has_media)
        ctx.refresh_navigation_shortcut_actions()
        ctx._rebuild_model_view()

    def apply_preview_cache_limit(self) -> None:
        ctx = self.context
        selected_index = ctx.preview_cache_size_combo.currentIndex()
        selected_text = ctx.preview_cache_size_combo.itemText(selected_index)
        if selected_text.endswith("(Custom)"):
            new_size_gb = float(selected_text.split(" ")[0])
        elif 0 <= selected_index < len(ctx.preview_cache_size_options_gb):
            new_size_gb = ctx.preview_cache_size_options_gb[selected_index]
        else:
            ctx.status_message("Invalid selection for cache size.")
            return

        current_size_gb = get_preview_cache_size_gb()
        if new_size_gb != current_size_gb:
            set_preview_cache_size_gb(new_size_gb)
            ctx.image_pipeline.reinitialize_preview_cache_from_settings()
            ctx.status_message(
                f"Preview cache limit set to {new_size_gb:.2f} GB. "
                "Cache reinitialized.",
                5000,
            )
        else:
            ctx.status_message(
                f"Preview cache limit is already {new_size_gb:.2f} GB.", 3000
            )
        self.update_labels()

    def clear_exif_cache(self) -> None:
        ctx = self.context
        if not ctx.app_state.exif_disk_cache:
            return
        ctx.app_state.exif_disk_cache.clear()
        ctx.app_state.rating_disk_cache.clear()
        ctx.status_message("EXIF and rating caches cleared.", 5000)
        self.update_labels()
        ctx._refresh_current_selection_preview()

    def apply_exif_cache_limit(self) -> None:
        ctx = self.context
        selected_index = ctx.exif_cache_size_combo.currentIndex()
        selected_text = ctx.exif_cache_size_combo.itemText(selected_index)
        if selected_text.endswith("(Custom)"):
            new_size_mb = int(selected_text.split(" ")[0])
        elif 0 <= selected_index < len(ctx.exif_cache_size_options_mb):
            new_size_mb = ctx.exif_cache_size_options_mb[selected_index]
        else:
            ctx.status_message("Invalid selection for EXIF cache size.")
            return

        exif_cache = ctx.app_state.exif_disk_cache
        if exif_cache:
            current_size_mb = exif_cache.get_current_size_limit_mb()
            if new_size_mb != current_size_mb:
                set_exif_cache_size_mb(new_size_mb)
                exif_cache.reinitialize_from_settings()
                ctx.status_message(
                    f"EXIF cache limit set to {new_size_mb} MB. Cache reinitialized.",
                    5000,
                )
            else:
                ctx.status_message(
                    f"EXIF cache limit is already {new_size_mb} MB.", 3000
                )
        self.update_labels()
