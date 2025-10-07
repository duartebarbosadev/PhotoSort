"""
Rotation Application Worker
Background worker for applying image rotations without blocking the UI.
"""

import logging
import os
import time
from typing import Dict, Optional
from PyQt6.QtCore import QObject, pyqtSignal

from core.metadata_processor import MetadataProcessor
from core.caching.exif_cache import ExifCache

logger = logging.getLogger(__name__)


class RotationApplicationWorker(QObject):
    """Worker for applying image rotations in a background thread."""

    # Signals
    progress = pyqtSignal(int, int, str)  # current, total, filename
    rotation_applied = pyqtSignal(
        str, str, bool, str, bool
    )  # path, direction, success, message, is_lossy
    finished = pyqtSignal(int, int)  # successful_count, failed_count
    error = pyqtSignal(str)

    def __init__(self, exif_disk_cache: Optional[ExifCache] = None):
        super().__init__()
        self.exif_disk_cache = exif_disk_cache
        self._is_running = True

    def stop(self):
        """Signal the worker to stop processing."""
        self._is_running = False

    def apply_rotations(self, approved_rotations: Dict[str, int]):
        """
        Apply rotations to multiple images.

        Args:
            approved_rotations: Dict mapping file_path -> rotation_degrees
        """
        self._is_running = True
        total_rotations = len(approved_rotations)
        successful_rotations = 0
        failed_rotations = 0

        logger.info(f"Starting batch rotation application for {total_rotations} images")

        try:
            for i, (file_path, rotation_degrees) in enumerate(
                approved_rotations.items(), 1
            ):
                if not self._is_running:
                    logger.info("Rotation application stopped by user request")
                    break

                single_file_start_time = time.perf_counter()

                try:
                    filename = os.path.basename(file_path)
                    logger.debug(f"Applying {rotation_degrees}° rotation to {filename}...")
                    self.progress.emit(i, total_rotations, filename)

                    # Convert degrees to direction
                    if rotation_degrees == 90:
                        direction = "clockwise"
                    elif rotation_degrees == -90:
                        direction = "counterclockwise"
                    elif rotation_degrees == 180:
                        direction = "180"
                    else:
                        logger.warning(
                            f"Unsupported rotation angle {rotation_degrees} for {filename}"
                        )
                        failed_rotations += 1
                        self.rotation_applied.emit(
                            file_path,
                            "",
                            False,
                            f"Unsupported rotation angle: {rotation_degrees}",
                            False,
                        )
                        continue

                    # Try metadata rotation first
                    t1 = time.perf_counter()
                    metadata_success, needs_lossy, message = (
                        MetadataProcessor.try_metadata_rotation_first(
                            file_path, direction, self.exif_disk_cache
                        )
                    )
                    t2 = time.perf_counter()
                    logger.debug(
                        f"Metadata rotation for '{filename}' took {t2 - t1:.2f}s. "
                        f"Success: {metadata_success}, Needs Lossy: {needs_lossy}"
                    )

                    if metadata_success:
                        successful_rotations += 1
                        self.rotation_applied.emit(
                            file_path,
                            direction,
                            True,
                            f"Rotated {filename} {rotation_degrees}° (lossless)",
                            False,
                        )
                    elif needs_lossy:
                        # Try lossy rotation
                        logger.info(f"Attempting lossy rotation for '{filename}'.")
                        t3 = time.perf_counter()
                        success = MetadataProcessor.rotate_image(
                            file_path,
                            direction,
                            update_metadata_only=False,
                            exif_disk_cache=self.exif_disk_cache,
                        )
                        t4 = time.perf_counter()
                        logger.debug(
                            f"Lossy rotation for '{filename}' took {t4 - t3:.2f}s."
                        )

                        if success:
                            successful_rotations += 1
                            self.rotation_applied.emit(
                                file_path,
                                direction,
                                True,
                                f"Rotated {filename} {rotation_degrees}° (lossy)",
                                True,
                            )
                        else:
                            failed_rotations += 1
                            logger.error(f"Lossy rotation failed for '{filename}'.")
                            self.rotation_applied.emit(
                                file_path, direction, False, "Rotation failed", False
                            )
                    else:
                        failed_rotations += 1
                        logger.error(f"Rotation not supported for '{filename}': {message}")
                        self.rotation_applied.emit(
                            file_path, direction, False, message, False
                        )

                except Exception as e:
                    failed_rotations += 1
                    logger.error(
                        f"Unhandled error while rotating '{os.path.basename(file_path)}': {e}",
                        exc_info=True,
                    )
                    self.rotation_applied.emit(
                        file_path, "", False, f"Error: {e}", False
                    )
                finally:
                    single_file_end_time = time.perf_counter()
                    logger.debug(
                        f"Finished processing '{os.path.basename(file_path)}' in "
                        f"{single_file_end_time - single_file_start_time:.2f}s."
                    )

            if self._is_running:
                logger.info(
                    f"Rotation application complete: {successful_rotations} successful, "
                    f"{failed_rotations} failed"
                )
                self.finished.emit(successful_rotations, failed_rotations)

        except Exception as e:
            error_msg = f"Unexpected error in rotation application worker: {e}"
            logger.error(error_msg, exc_info=True)
            self.error.emit(error_msg)
