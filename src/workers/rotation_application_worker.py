"""
Rotation Application Worker
Background worker for applying image rotations without blocking the UI.
Supports parallel processing for batch operations.
"""

import logging
import os
import time
import threading
import concurrent.futures
from typing import Dict, Optional, Tuple
from PyQt6.QtCore import QObject, pyqtSignal

from core.metadata_processor import MetadataProcessor
from core.caching.exif_cache import ExifCache
from core.app_settings import calculate_max_workers

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
        self._progress_lock = threading.Lock()
        self._completed_count = 0

    def stop(self):
        """Signal the worker to stop processing."""
        self._is_running = False

    def _rotate_single_image(
        self, file_path: str, rotation_degrees: int, total_rotations: int
    ) -> Tuple[str, str, bool, str, bool]:
        """
        Rotate a single image (used by both sequential and parallel paths).

        Args:
            file_path: Path to the image file
            rotation_degrees: Rotation in degrees (90, -90, 180)
            total_rotations: Total number of images being rotated

        Returns:
            Tuple of (file_path, direction, success, message, is_lossy)
        """
        single_file_start_time = time.perf_counter()
        filename = os.path.basename(file_path)

        try:
            # Emit progress with thread-safe counter
            with self._progress_lock:
                self._completed_count += 1
                current = self._completed_count
            self.progress.emit(current, total_rotations, filename)

            logger.debug(f"Applying {rotation_degrees}° rotation to {filename}...")

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
                return (
                    file_path,
                    "",
                    False,
                    f"Unsupported rotation angle: {rotation_degrees}",
                    False,
                )

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
                return (
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
                logger.debug(f"Lossy rotation for '{filename}' took {t4 - t3:.2f}s.")

                if success:
                    return (
                        file_path,
                        direction,
                        True,
                        f"Rotated {filename} {rotation_degrees}° (lossy)",
                        True,
                    )
                else:
                    logger.error(f"Lossy rotation failed for '{filename}'.")
                    return (file_path, direction, False, "Rotation failed", False)
            else:
                logger.error(f"Rotation not supported for '{filename}': {message}")
                return (file_path, direction, False, message, False)

        except Exception as e:
            logger.error(
                f"Unhandled error while rotating '{filename}': {e}", exc_info=True
            )
            return (file_path, "", False, f"Error: {e}", False)
        finally:
            single_file_end_time = time.perf_counter()
            logger.debug(
                f"Finished processing '{filename}' in "
                f"{single_file_end_time - single_file_start_time:.2f}s."
            )

    def apply_rotations(self, approved_rotations: Dict[str, int]):
        """
        Apply rotations to multiple images.
        Uses parallel processing for multiple images when max_workers > 1.

        Args:
            approved_rotations: Dict mapping file_path -> rotation_degrees
        """
        self._is_running = True
        self._completed_count = 0  # Reset counter
        total_rotations = len(approved_rotations)
        successful_rotations = 0
        failed_rotations = 0

        max_workers = calculate_max_workers()
        use_parallel = total_rotations > 1 and max_workers > 1

        logger.info(
            f"Starting batch rotation application for {total_rotations} images "
            f"({'parallel' if use_parallel else 'sequential'} mode, max_workers={max_workers})"
        )

        try:
            if use_parallel:
                # Parallel processing path
                with concurrent.futures.ThreadPoolExecutor(
                    max_workers=max_workers
                ) as executor:
                    # Submit all rotation tasks
                    future_to_path = {
                        executor.submit(
                            self._rotate_single_image,
                            file_path,
                            rotation_degrees,
                            total_rotations,
                        ): file_path
                        for file_path, rotation_degrees in approved_rotations.items()
                    }

                    # Process results as they complete
                    for future in concurrent.futures.as_completed(future_to_path):
                        if not self._is_running:
                            logger.info("Rotation application stopped by user request")
                            # Cancel pending futures
                            for f in future_to_path:
                                f.cancel()
                            break

                        try:
                            (
                                file_path,
                                direction,
                                success,
                                message,
                                is_lossy,
                            ) = future.result()

                            # Emit result signal
                            self.rotation_applied.emit(
                                file_path, direction, success, message, is_lossy
                            )

                            # Update counters
                            if success:
                                successful_rotations += 1
                            else:
                                failed_rotations += 1

                        except Exception as e:
                            logger.error(
                                f"Exception getting rotation result: {e}", exc_info=True
                            )
                            failed_rotations += 1

            else:
                # Sequential processing path (single image or max_workers=1)
                for file_path, rotation_degrees in approved_rotations.items():
                    if not self._is_running:
                        logger.info("Rotation application stopped by user request")
                        break

                    result = self._rotate_single_image(
                        file_path, rotation_degrees, total_rotations
                    )
                    file_path, direction, success, message, is_lossy = result

                    # Emit result signal
                    self.rotation_applied.emit(
                        file_path, direction, success, message, is_lossy
                    )

                    # Update counters
                    if success:
                        successful_rotations += 1
                    else:
                        failed_rotations += 1

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
