"""
Update Check Worker
Background worker for checking application updates.
"""

import logging
from PyQt6.QtCore import QObject, pyqtSignal

from core.update_checker import UpdateChecker

logger = logging.getLogger(__name__)


class UpdateCheckWorker(QObject):
    """Worker for checking updates in a background thread."""

    # Signals
    update_check_finished = pyqtSignal(
        bool, object, str
    )  # (update_available, update_info, error_message)

    def __init__(self, current_version: str):
        super().__init__()
        self.current_version = current_version
        self.update_checker = UpdateChecker()

    def check_for_updates(self):
        """Check for updates and emit the result."""
        try:
            logger.debug("Starting background update check...")
            update_available, update_info, error_message = (
                self.update_checker.check_for_updates(self.current_version)
            )
            self.update_check_finished.emit(
                update_available, update_info, error_message
            )
        except Exception as e:
            logger.error(f"Unexpected error in update check worker: {e}", exc_info=True)
            self.update_check_finished.emit(False, None, str(e))
