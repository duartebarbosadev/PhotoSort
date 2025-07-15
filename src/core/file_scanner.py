import os
import asyncio
import logging

logger = logging.getLogger(__name__)
import time
from PyQt6.QtCore import QObject, pyqtSignal
from .image_pipeline import ImagePipeline
from .image_features.blur_detector import BlurDetector

# Define supported image extensions (case-insensitive)
SUPPORTED_EXTENSIONS = {
    ".jpg",
    ".jpeg",
    ".png",
    ".tif",
    ".tiff",  # Standard formats
    ".heic",
    ".heif",  # HEIC/HEIF formats
    ".arw",
    ".cr2",
    ".cr3",
    ".nef",
    ".dng",  # Sony, Canon, Nikon, Adobe RAW
    ".orf",
    ".raf",
    ".rw2",
    ".pef",
    ".srw",  # Olympus, Fuji, Panasonic, Pentax, Samsung RAW
    ".raw",  # Generic RAW
}


class FileScanner(QObject):
    """
    Scans a directory recursively for supported image files.
    Designed to be run in a separate thread.
    """

    # Signals
    # Emits batches of found file paths
    files_found = pyqtSignal(
        list
    )  # Emits list of dicts: [{'path': str, 'is_blurred': Optional[bool]}]
    # Emits progress percentage (0-100) - Optional, can be complex to estimate accurately
    # progress_update = pyqtSignal(int)
    # Emits when scanning is complete
    finished = pyqtSignal()
    # Emits error messages
    error = pyqtSignal(str)
    thumbnail_preload_finished = pyqtSignal(
        list
    )  # New signal, will also emit list of dicts

    def __init__(self, parent=None):
        super().__init__(parent)
        init_start_time = time.perf_counter()
        logger.debug("Initializing FileScanner.")
        self._is_running = True
        self.blur_detection_threshold = 100.0

        self.image_pipeline = ImagePipeline()
        logger.debug(
            f"FileScanner initialized in {time.perf_counter() - init_start_time:.2f}s."
        )

    def stop(self):
        """Signals the scanner to stop."""
        self._is_running = False

    async def _scan_directory_async(self, directory_path):
        """Asynchronous directory scanning."""
        # This async version is not currently used by the main application flow
        # but is kept for potential future use.
        # If used, it would also need to incorporate blur detection.
        for root, _, files in os.walk(directory_path):
            if not self._is_running:
                self.error.emit("Scan cancelled.")
                return

            for filename in files:
                if not self._is_running:
                    return
                ext = os.path.splitext(filename)[1].lower()
                if ext in SUPPORTED_EXTENSIONS:
                    full_path = os.path.normpath(os.path.join(root, filename))
                    # Blur detection would be added here if this method were active
                    # Assuming self.apply_auto_edits_for_raw_preview is available if this method is used
                    is_blurred = BlurDetector.is_image_blurred(
                        full_path,
                        threshold=self.blur_detection_threshold,
                        apply_auto_edits_for_raw_preview=getattr(
                            self, "apply_auto_edits_for_raw_preview", False
                        ),  # Fallback
                    )
                    self.files_found.emit(
                        [{"path": full_path, "is_blurred": is_blurred}]
                    )
                    await asyncio.sleep(0)

    def scan_directory(
        self,
        directory_path: str,
        apply_auto_edits: bool = False,
        perform_blur_detection: bool = False,
        blur_threshold: float = 100.0,
    ):
        """
        Starts the directory scanning process.
        Optionally detects blur for each image.
        apply_auto_edits: bool - Flag for thumbnail preloading AND for RAW preview used in blur detection.
        perform_blur_detection: bool - If True, performs blur detection.
        blur_threshold: float - Threshold for blur detection if performed.
        """
        self._is_running = True
        # self.blur_detection_threshold = blur_threshold # Threshold is passed directly to is_image_blurred if needed
        # Store apply_auto_edits for use in async or other methods if needed
        self.apply_auto_edits_for_raw_preview = apply_auto_edits
        all_file_data = []  # Collect all file data (path and blur status)
        thumbnail_paths_only = []  # For ImageHandler.preload_thumbnails

        try:
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
                    if ext in SUPPORTED_EXTENSIONS:
                        full_path = os.path.normpath(os.path.join(root, filename))

                        is_blurred = None  # Initialize as None
                        if perform_blur_detection:
                            # Perform blur detection
                            # Pass the apply_auto_edits flag to control RAW preview generation for blur detection
                            logger.debug(
                                f"Performing blur detection for: {os.path.basename(full_path)} (Threshold: {blur_threshold})"
                            )
                            is_blurred = BlurDetector.is_image_blurred(
                                full_path,
                                threshold=blur_threshold,
                                apply_auto_edits_for_raw_preview=apply_auto_edits,
                            )

                        file_info = {"path": full_path, "is_blurred": is_blurred}
                        all_file_data.append(file_info)
                        thumbnail_paths_only.append(full_path)

                        self.files_found.emit([file_info])
                        logger.debug(
                            f"Found: {os.path.basename(full_path)}, Blurred: {is_blurred}"
                        )

            if not self._is_running:
                self.error.emit("Scan cancelled before thumbnail preloading.")
                return

            # Preload thumbnails after scanning all files
            if thumbnail_paths_only:
                logger.info(
                    f"Preloading {len(thumbnail_paths_only)} thumbnails (Auto-Edits: {apply_auto_edits})."
                )
                # TODO: Consider if preload_thumbnails needs should_continue_callback
                self.image_pipeline.preload_thumbnails(
                    thumbnail_paths_only, apply_auto_edits=apply_auto_edits
                )
            else:
                logger.warning("No supported image files found to preload.")

            if not self._is_running:
                self.error.emit("Scan cancelled during thumbnail preloading.")
            else:
                logger.debug("Thumbnail preloading complete. Emitting signal.")
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
