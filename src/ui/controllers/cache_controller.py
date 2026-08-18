"""Cache-management behavior extracted from the main window."""

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
    app_controller: Any
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
            configured_text = (
                f"{configured_mb / 1024:.2f} GB"
                if configured_mb >= 1024
                else f"{configured_mb} MB"
            )
            ctx.exif_cache_configured_limit_label.setText(configured_text)
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

        models_label = getattr(ctx, "model_cache_usage_label", None)
        if models_label is not None:
            usage_text = "N/A"
            try:
                from core.model_provisioning import model_cache_usage_bytes

                usage_text = f"{model_cache_usage_bytes() / (1024 * 1024):.2f} MB"
            except Exception:
                logger.exception("Failed to read model cache usage.")
                usage_text = "Error"
            models_label.setText(usage_text)

    def clear_thumbnail_cache(self) -> None:
        ctx = self.context
        protected_bytes = ctx.image_pipeline.preview_cache.protected_payload_bytes()
        if isinstance(protected_bytes, int | float) and protected_bytes > 0:
            ctx.status_message(
                "Open another folder or close the current one before clearing prepared review images.",
                5000,
            )
            return
        ctx.image_pipeline.thumbnail_cache.clear()
        ctx.status_message("Thumbnail cache cleared.", 5000)
        self.update_labels()
        ctx._refresh_visible_items_icons()

    def clear_preview_cache(self) -> None:
        ctx = self.context
        protected_bytes = ctx.image_pipeline.preview_cache.protected_payload_bytes()
        if isinstance(protected_bytes, int | float) and protected_bytes > 0:
            ctx.status_message(
                "Open another folder or close the current one before clearing prepared review images.",
                5000,
            )
            return
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
            from core.similarity_engine import SimilarityEngine

            SimilarityEngine.clear_embedding_cache()
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
        cull_clusters = getattr(ctx.app_state, "cull_cluster_results", None)
        if cull_clusters is not None:
            cull_clusters.clear()
        getattr(ctx.app_state, "embeddings_cache", {}).clear()
        getattr(ctx.app_state, "regional_embeddings_cache", {}).clear()
        clear_pick_best = getattr(ctx.app_state, "clear_pick_best_results", None)
        if callable(clear_pick_best):
            clear_pick_best()
        if hasattr(ctx.app_state, "easy_delete_results"):
            ctx.app_state.easy_delete_results = None
        getattr(ctx.app_state, "easy_delete_pair_assessments", {}).clear()
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
        if hasattr(menu, "analyze_similarity_action"):
            menu.analyze_similarity_action.setEnabled(has_media)
        ctx.refresh_navigation_shortcut_actions()
        ctx._rebuild_model_view()

    def _invalidate_model_environment(self) -> None:
        """Make the next model-backed run re-check what is actually installed."""

        app_controller = getattr(self.context, "app_controller", None)
        reset = getattr(app_controller, "_reset_model_environment", None)
        if reset is None:
            return
        reset()

    def clear_downloaded_models(self) -> None:
        """Delete every downloaded model so the next run fetches them again."""

        ctx = self.context
        logger.info("User requested deletion of all downloaded models.")
        try:
            from core.model_provisioning import (
                clear_model_caches,
                model_cache_usage_bytes,
            )

            usage_before = model_cache_usage_bytes()
            removed = clear_model_caches()
        except Exception:
            logger.exception("Failed to delete downloaded models.")
            ctx.status_message("Failed to delete downloaded models.", 5000)
            return

        # The controller resolves "which models are on disk" once per process, so
        # deleting them behind its back would leave it certain nothing is missing.
        # The next model-backed run would then start without download consent and
        # fail with "has not been downloaded yet" instead of prompting.
        self._invalidate_model_environment()

        if removed:
            logger.info(
                "Deleted %d downloaded model(s): %s", len(removed), ", ".join(removed)
            )
            ctx.status_message(
                f"Deleted {len(removed)} model(s), freeing "
                f"{self._format_mb(usage_before)}. They will download again when "
                "next needed.",
                6000,
            )
        else:
            logger.info("No downloaded models to delete.")
            ctx.status_message("No downloaded models to delete.", 5000)
        self.update_labels()

    @staticmethod
    def _format_mb(num_bytes: int) -> str:
        return f"{num_bytes / (1024 * 1024):.0f} MB"

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
            protected_bytes = (
                ctx.image_pipeline.preview_cache.protected_payload_bytes()
            )
            if isinstance(protected_bytes, int | float) and int(
                new_size_gb * 1024**3
            ) < protected_bytes:
                ctx.status_message(
                    "The selected limit is smaller than the active folder's prepared "
                    "review images. Open another folder before reducing it.",
                    6000,
                )
                return
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
                    f"EXIF cache limit set to {new_size_mb / 1024:.2f} GB. "
                    "Cache reinitialized.",
                    5000,
                )
            else:
                ctx.status_message(
                    f"EXIF cache limit is already {new_size_mb / 1024:.2f} GB.",
                    3000,
                )
        self.update_labels()
