import os
import logging
import time
from typing import Any
from PyQt6.QtCore import QObject, pyqtSignal
from .image_pipeline import ImagePipeline
from .media_utils import (
    SUPPORTED_MEDIA_EXTENSIONS,
    is_video_extension,
)
from .app_settings import FILE_SCAN_EMIT_BATCH_SIZE

logger = logging.getLogger(__name__)


class FileScanner(QObject):
    """
    Scans a directory recursively for supported media files.
    Designed to be run in a separate thread.
    """

    # Signals
    # Emits batches of found file paths
    files_found = pyqtSignal(
        list
    )  # Emits list of dicts: [{'path': str, 'is_blurred': Optional[bool], 'media_type': str}]
    # Emits progress percentage (0-100) - Optional, can be complex to estimate accurately
    # progress_update = pyqtSignal(int)
    # Emits when scanning is complete
    finished = pyqtSignal()
    # Emits error messages
    error = pyqtSignal(str)
    thumbnail_preload_finished = pyqtSignal(
        list
    )  # New signal, will also emit list of dicts

    def __init__(self, image_pipeline: ImagePipeline, parent=None):
        super().__init__(parent)
        init_start_time = time.perf_counter()
        logger.debug("Initializing FileScanner.")
        self._is_running = True
        self.image_pipeline = image_pipeline
        logger.debug(
            f"FileScanner initialized in {time.perf_counter() - init_start_time:.2f}s."
        )

    def stop(self):
        """Signals the scanner to stop."""
        self._is_running = False

    def scan_directory(self, directory_path: str):
        """Discover supported media without performing workflow analysis."""
        if not self._is_running:
            logger.info("File scan skipped because cancellation was already requested.")
            return
        all_file_data: list[dict[str, Any]] = []
        discovery_batch: list[dict[str, Any]] = []

        try:
            # Phase 1: Fast file discovery
            logger.info(f"Starting file scan in: {directory_path}")
            for root, _, files in os.walk(directory_path):
                if not self._is_running:
                    self.error.emit("Scan cancelled during file discovery.")
                    return
                for filename in files:
                    if not self._is_running:
                        self.error.emit("Scan cancelled during file processing.")
                        return

                    ext = os.path.splitext(filename)[1].lower()
                    if ext in SUPPORTED_MEDIA_EXTENSIONS:
                        full_path = os.path.normpath(os.path.join(root, filename))

                        try:
                            stat_result = os.stat(full_path)
                        except OSError:
                            logger.info(
                                "Skipping inaccessible file during scan: %s", full_path
                            )
                            continue

                        media_type = "video" if is_video_extension(ext) else "image"
                        file_info = {
                            "path": full_path,
                            "is_blurred": None,
                            "media_type": media_type,
                            "file_size": stat_result.st_size,
                            "mtime_ns": stat_result.st_mtime_ns,
                        }
                        all_file_data.append(file_info)
                        discovery_batch.append(dict(file_info))
                        if len(discovery_batch) >= FILE_SCAN_EMIT_BATCH_SIZE:
                            self.files_found.emit(discovery_batch)
                            discovery_batch = []
                        if len(all_file_data) % 100 == 0:
                            logger.debug(
                                f"Discovered {len(all_file_data)} files so far..."
                            )

            if discovery_batch:
                self.files_found.emit(discovery_batch)

            if not self._is_running:
                self.error.emit("Scan cancelled after file discovery.")
                return

            logger.info(f"File discovery complete. Found {len(all_file_data)} files.")

            if not self._is_running:
                self.error.emit("Scan cancelled before completion.")
                return

            # Emit scan results immediately (thumbnail preloading now happens in separate worker)
            if self._is_running:
                logger.debug("File scan complete. Emitting results.")
                # Emit the list of dicts, so the receiver has blur info too
                self.thumbnail_preload_finished.emit(all_file_data)

        except Exception as e:
            error_msg = f"Error during scan: {e}"
            logger.error(error_msg, exc_info=True)
            self.error.emit(error_msg)
        finally:
            if self._is_running:
                logger.info("File scan finished.")
            self.finished.emit()
