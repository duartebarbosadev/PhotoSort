from __future__ import annotations
from dataclasses import dataclass
from typing import Protocol, Optional

from PyQt6.QtCore import QSortFilterProxyModel


class FilterContext(Protocol):
    proxy_model: QSortFilterProxyModel
    app_state: object  # only passed through to proxy_model for now

    def refresh_filter(self) -> None: ...  # triggers view/model refresh


@dataclass
class FilterState:
    rating_filter: str = "Show All"
    cluster_filter_id: int = -1
    search_text: str = ""


class FilterController:
    """Encapsulates rating/cluster filter manipulation previously handled in MainWindow.

    Responsibilities:
    - Track current rating and cluster filter choices
    - Apply them to the proxy model
    - Provide convenience predicates for tests / external logic
    """

    def __init__(self, ctx: FilterContext):
        self.ctx = ctx
        self.state = FilterState()
        # Track whether we've deferred the initial push because proxy_model was missing
        self._initial_push_pending = True  # start pending until proxy exists

    # --- Public API ---
    def set_rating_filter(self, value: str) -> None:
        if value == self.state.rating_filter:
            return
        self.state.rating_filter = value
        self._push_state_to_proxy()

    def set_cluster_filter(self, cluster_id: int) -> None:
        if cluster_id == self.state.cluster_filter_id:
            return
        self.state.cluster_filter_id = cluster_id
        self._push_state_to_proxy()

    def clear_filters(self) -> None:
        changed = False
        if self.state.rating_filter != "Show All":
            self.state.rating_filter = "Show All"
            changed = True
        if self.state.cluster_filter_id != -1:
            self.state.cluster_filter_id = -1
            changed = True
        if changed:
            self._push_state_to_proxy()

    def get_rating_filter(self) -> str:
        return self.state.rating_filter

    def get_cluster_filter_id(self) -> int:
        return self.state.cluster_filter_id

    def set_search_text(self, text: str) -> None:
        t = (text or "").lower()
        if t == self.state.search_text:
            return
        self.state.search_text = t
        self._apply_search_text()

    # --- Application hooks ---
    def apply_all(self, show_folders: bool, current_view_mode: str) -> None:
        """Push filters + search text + view flags to proxy model."""
        # If initial push was deferred, do it now transparently
        if self._initial_push_pending:
            self._push_state_to_proxy(show_folders, current_view_mode)
            return
        self._push_state_to_proxy(show_folders, current_view_mode)

    # --- Internal helpers ---
    def _apply_search_text(self) -> None:
        proxy = getattr(self.ctx, "proxy_model", None)
        if proxy is None:
            # Context not fully initialized yet; mark for later
            self._initial_push_pending = True
            return
        proxy.setFilterRegularExpression(self.state.search_text)

    def _push_state_to_proxy(
        self,
        show_folders: Optional[bool] = None,
        current_view_mode: Optional[str] = None,
    ) -> None:
        proxy = getattr(self.ctx, "proxy_model", None)
        if proxy is None:
            # MainWindow may not have created proxy_model yet; defer application
            self._initial_push_pending = True
            return
        # Attach AppState reference once (idempotent) if attribute exists
        if getattr(proxy, "app_state_ref", None) is None:
            try:
                proxy.app_state_ref = self.ctx.app_state  # type: ignore[attr-defined]
            except Exception:
                pass
        # Push current filters
        if hasattr(proxy, "current_rating_filter"):
            proxy.current_rating_filter = self.state.rating_filter  # type: ignore[attr-defined]
        if hasattr(proxy, "current_cluster_filter_id"):
            proxy.current_cluster_filter_id = self.state.cluster_filter_id  # type: ignore[attr-defined]
        if show_folders is not None and hasattr(proxy, "show_folders_mode_ref"):
            proxy.show_folders_mode_ref = show_folders  # type: ignore[attr-defined]
        if current_view_mode is not None and hasattr(proxy, "current_view_mode_ref"):
            proxy.current_view_mode_ref = current_view_mode  # type: ignore[attr-defined]
        # search text
        proxy.setFilterRegularExpression(self.state.search_text)
        # Trigger a refresh
        self.ctx.refresh_filter()
        self._initial_push_pending = False

    def ensure_initialized(self, show_folders: bool, current_view_mode: str) -> None:
        """Apply deferred initial state once proxy exists."""
        if self._initial_push_pending:
            self._push_state_to_proxy(show_folders, current_view_mode)
