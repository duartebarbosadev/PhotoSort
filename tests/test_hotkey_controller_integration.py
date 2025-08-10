from PyQt6.QtCore import Qt
from src.ui.controllers.hotkey_controller import HotkeyController


class DummyCtx:
    def __init__(self):
        self.calls = []

    # Provide required methods; capture skip_deleted
    def navigate_left_in_group(self, skip_deleted=True):
        self.calls.append(("left", skip_deleted))

    def navigate_right_in_group(self, skip_deleted=True):
        self.calls.append(("right", skip_deleted))

    def navigate_up_sequential(self, skip_deleted=True):
        self.calls.append(("up", skip_deleted))

    def navigate_down_sequential(self, skip_deleted=True):
        self.calls.append(("down", skip_deleted))

    def navigate_down_smart(self, skip_deleted=True):
        self.calls.append(("down_smart", skip_deleted))

    def navigate_up_smart(self, skip_deleted=True):
        self.calls.append(("up_smart", skip_deleted))


def test_hotkey_controller_skip_deleted_flag_propagates():
    ctx = DummyCtx()
    hk = HotkeyController(ctx)
    # Simulate pressing Down with skip_deleted False (Ctrl modifier logic is handled in MainWindow; here we just call handle_key)
    hk.handle_key(Qt.Key.Key_Down, skip_deleted=False)
    assert ctx.calls[-1] == ("down_smart", False)
    # Up key propagation
    hk.handle_key(Qt.Key.Key_Up, skip_deleted=False)
    assert ctx.calls[-1] == ("up_smart", False)
