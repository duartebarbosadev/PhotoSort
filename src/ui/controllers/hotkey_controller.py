from __future__ import annotations
from typing import Protocol, Dict, Tuple, Callable
from PyQt6.QtCore import Qt


class HotkeyContext(Protocol):
    def navigate_left_in_group(self) -> None: ...
    def navigate_right_in_group(self) -> None: ...
    def navigate_up_sequential(self) -> None: ...
    def navigate_down_sequential(self) -> None: ...
    def navigate_down_smart(
        self,
    ) -> None: ...  # New: group-cyclic if in group else sequential


class HotkeyController:
    def __init__(self, ctx: HotkeyContext):
        self.ctx = ctx
        # Bindings map key -> (label, callable accepting skip_deleted)
        self.bindings: Dict[int, Tuple[str, Callable[[bool], None]]] = {
            Qt.Key.Key_Left: ("LEFT/H", self.ctx.navigate_left_in_group),
            Qt.Key.Key_H: ("LEFT/H", self.ctx.navigate_left_in_group),
            Qt.Key.Key_Right: ("RIGHT/L", self.ctx.navigate_right_in_group),
            Qt.Key.Key_L: ("RIGHT/L", self.ctx.navigate_right_in_group),
            Qt.Key.Key_Up: (
                "UP/K",
                getattr(self.ctx, "navigate_up_smart", self.ctx.navigate_up_sequential),
            ),
            Qt.Key.Key_K: (
                "UP/K",
                getattr(self.ctx, "navigate_up_smart", self.ctx.navigate_up_sequential),
            ),
            Qt.Key.Key_Down: (
                "DOWN/J",
                getattr(
                    self.ctx, "navigate_down_smart", self.ctx.navigate_down_sequential
                ),
            ),
            Qt.Key.Key_J: (
                "DOWN/J",
                getattr(
                    self.ctx, "navigate_down_smart", self.ctx.navigate_down_sequential
                ),
            ),
        }

    def handle_key(self, key: int, skip_deleted: bool = True) -> bool:
        """Handle a navigation key.

        skip_deleted = True  -> skip images marked for deletion (default)
        skip_deleted = False -> include images marked for deletion (Ctrl/Cmd modified navigation)
        """
        entry = self.bindings.get(key)
        if not entry:
            return False
        _, fn = entry
        try:
            fn(skip_deleted)  # New signature (wrappers updated in MainWindow)
        except TypeError:
            # Backwards compatibility if wrapper not yet updated
            fn()
        return True
