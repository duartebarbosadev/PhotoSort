from __future__ import annotations

import logging
from typing import Protocol


logger = logging.getLogger(__name__)


class ActiveImageContext(Protocol):
    app_state: object

    def get_active_image_adapter(self, workflow_step: str): ...


class ActiveImageController:
    """Own the application-wide active image and synchronize workflow views."""

    WORKFLOW_STEPS = (
        "organize",
        "easy_delete",
        "fix_rotation",
        "pick_best",
        "cull",
    )

    def __init__(self, context: ActiveImageContext) -> None:
        self._context = context
        self._syncing = False

    @property
    def active_path(self) -> str | None:
        return getattr(self._context.app_state, "focused_image_path", None)

    def publish(self, path: str | None, *, source: str | None = None) -> bool:
        """Publish a user focus change and synchronize every available adapter."""

        if self._syncing:
            return False
        active_workflow = getattr(self._context.app_state, "workflow_step", None)
        if source != active_workflow:
            adapter = self._context.get_active_image_adapter(active_workflow)
            has_changes = getattr(adapter, "has_unconfirmed_changes", None)
            if callable(has_changes) and has_changes():
                show_required = getattr(adapter, "show_confirm_or_reset_required", None)
                if callable(show_required):
                    show_required()
                return False
        normalized = str(path) if path else None
        changed = normalized != self.active_path
        self._context.app_state.focused_image_path = normalized
        if changed and normalized:
            self.sync_all(exclude=source)
        return changed

    def sync_all(self, *, exclude: str | None = None) -> None:
        path = self.active_path
        if not path or self._syncing:
            return
        self._syncing = True
        try:
            for workflow_step in self.WORKFLOW_STEPS:
                if workflow_step != exclude:
                    self._sync_adapter(workflow_step, path)
        finally:
            self._syncing = False

    def sync_workflow(self, workflow_step: str) -> bool:
        path = self.active_path
        if not path or self._syncing:
            return False
        self._syncing = True
        try:
            return self._sync_adapter(workflow_step, path)
        finally:
            self._syncing = False

    def clear_if_active(self, path: str) -> bool:
        if path != self.active_path:
            return False
        self._context.app_state.focused_image_path = None
        return True

    def path_updated(self, old_path: str, new_path: str) -> None:
        if self.active_path == old_path:
            self._context.app_state.focused_image_path = new_path
            self.sync_all()

    def _sync_adapter(self, workflow_step: str, path: str) -> bool:
        adapter = self._context.get_active_image_adapter(workflow_step)
        if adapter is None:
            return False
        focus_image = getattr(adapter, "focus_image", None)
        if not callable(focus_image):
            return False
        try:
            return bool(focus_image(path))
        except RuntimeError:
            logger.debug(
                "Active-image adapter became unavailable for %s", workflow_step
            )
            return False
