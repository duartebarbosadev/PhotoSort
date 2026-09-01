"""Keep worker results out of modal event loops and bulk view mutations."""

from collections import deque
from collections.abc import Callable
from contextlib import contextmanager
import time

from PyQt6.QtCore import QObject, QTimer
from PyQt6.QtWidgets import QApplication

_bulk_update_depth = 0


@contextmanager
def bulk_ui_update():
    """Prevent processEvents inside a rebuild from applying another result."""
    global _bulk_update_depth
    _bulk_update_depth += 1
    try:
        yield
    finally:
        _bulk_update_depth -= 1


class UiResultDispatcher(QObject):
    """Deliver results after modal decisions, preserving order and ownership checks.

    Worker cleanup signals bypass this queue so a dialog never keeps a finished
    thread alive. Callbacks must validate their generation when delivered, not
    only when queued, because accepting a dialog can cancel that generation.
    """

    def __init__(self, parent: QObject):
        super().__init__(parent)
        self._pending: deque[Callable[[], None]] = deque()
        self._delivering = False
        self._timer = QTimer(self)
        self._timer.setInterval(25)
        self._timer.timeout.connect(self._drain)

    @staticmethod
    def _blocked() -> bool:
        return bool(_bulk_update_depth or QApplication.activeModalWidget() is not None)

    def has_pending(self) -> bool:
        """Return whether a worker result is waiting for a safe UI boundary."""
        return bool(self._pending or self._delivering)

    def dispatch(self, callback: Callable[[], None]) -> None:
        if self._blocked() or self._delivering or self._pending:
            self._pending.append(callback)
            self._timer.start()
            return
        self._delivering = True
        try:
            callback()
        finally:
            self._delivering = False

    def _drain(self) -> None:
        if self._blocked() or self._delivering:
            return
        self._delivering = True
        deadline = time.monotonic() + 0.005
        try:
            while self._pending and not self._blocked():
                self._pending.popleft()()
                if time.monotonic() >= deadline:
                    break
        finally:
            self._delivering = False
            if not self._pending:
                self._timer.stop()
