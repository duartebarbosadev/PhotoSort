from collections.abc import Callable
from ui.helpers.rotation_utils import compute_next_after_rotation


class RotationController:
    """Encapsulates logic for accepting / refusing rotation suggestions.

    Keeps state in a shared rotation_suggestions dict (path -> degrees).
    Delegates actual rotation application to provided callback (typically AppController).
    """

    def __init__(
        self,
        rotation_suggestions: dict[str, int],
        apply_rotations: Callable[[dict[str, int]], None],
    ):
        self._rotation_suggestions = rotation_suggestions
        self._apply_rotations = apply_rotations

    # --- State access ---
    @property
    def rotation_suggestions(self) -> dict[str, int]:
        return self._rotation_suggestions

    def has_suggestions(self) -> bool:
        return bool(self._rotation_suggestions)

    def get_visible_order(self) -> list[str]:
        # Current UI uses dict iteration order as the visual list in rotation view rebuild
        return list(self._rotation_suggestions.keys())

    # --- Accept / Refuse operations ---
    def accept_all(self) -> list[str]:
        if not self._rotation_suggestions:
            return []
        to_apply = dict(self._rotation_suggestions)
        self._apply_rotations(to_apply)
        accepted = list(to_apply.keys())
        self._rotation_suggestions.clear()
        return accepted

    def accept_paths(self, paths: list[str]) -> list[str]:
        to_apply = {
            p: self._rotation_suggestions[p]
            for p in paths
            if p in self._rotation_suggestions
        }
        if not to_apply:
            return []
        self._apply_rotations(to_apply)
        for p in to_apply:
            self._rotation_suggestions.pop(p, None)
        return list(to_apply.keys())

    def refuse_all(self) -> list[str]:
        refused = list(self._rotation_suggestions.keys())
        self._rotation_suggestions.clear()
        return refused

    def refuse_paths(self, paths: list[str]) -> list[str]:
        refused = []
        for p in paths:
            if p in self._rotation_suggestions:
                self._rotation_suggestions.pop(p)
                refused.append(p)
        return refused

    # --- Next selection computation ---
    def compute_next_after_accept(
        self,
        visible_before: list[str],
        accepted_paths: list[str],
        anchor_path: str | None = None,
    ) -> str | None:
        remaining = self.get_visible_order()
        if not accepted_paths:
            return None
        anchor = anchor_path or accepted_paths[0]
        return compute_next_after_rotation(
            visible_paths_before=visible_before,
            accepted_paths=accepted_paths,
            remaining_paths_after=remaining,
            anchor_path=anchor,
        )
