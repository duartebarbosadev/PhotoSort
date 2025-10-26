import os
import asyncio
import logging
import time
import concurrent.futures
from typing import List, Dict, Any
from PyQt6.QtCore import QObject, pyqtSignal
from .image_pipeline import ImagePipeline
from .image_features.blur_detector import BlurDetector

logger = logging.getLogger(__name__)

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

    def __init__(self, image_pipeline: ImagePipeline, parent=None):
        super().__init__(parent)
        init_start_time = time.perf_counter()
        logger.debug("Initializing FileScanner.")
        self._is_running = True
        self.blur_detection_threshold = 100.0

        self.image_pipeline = image_pipeline
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
                    is_blurred = BlurDetector.is_image_blurred(
                        full_path, threshold=self.blur_detection_threshold
                    )
                    self.files_found.emit(
                        [{"path": full_path, "is_blurred": is_blurred}]
                    )
                    await asyncio.sleep(0)

    def _detect_blur_for_file(
        self, file_path: str, blur_threshold: float
    ) -> Dict[str, Any]:
        """
        Detect blur for a single file (used in parallel processing).
        Returns dict with 'path' and 'is_blurred' keys.
        """
        try:
            is_blurred = BlurDetector.is_image_blurred(
                file_path, threshold=blur_threshold
            )
            return {"path": file_path, "is_blurred": is_blurred}
        except Exception as e:
            logger.warning(
                f"Blur detection failed for {os.path.basename(file_path)}: {e}"
            )
            return {"path": file_path, "is_blurred": None}

    def scan_directory(
        self,
        directory_path: str,
        perform_blur_detection: bool = False,
        blur_threshold: float = 100.0,
    ):
        """
        Starts the directory scanning process.
        Optionally detects blur for each image in parallel.
        perform_blur_detection: bool - If True, performs blur detection in parallel.
        blur_threshold: float - Threshold for blur detection if performed.
        """
        self._is_running = True
        all_file_paths: List[str] = []

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
                    if ext in SUPPORTED_EXTENSIONS:
                        full_path = os.path.normpath(os.path.join(root, filename))

                        # Hard existence check to avoid downstream missing-file errors
                        if not os.path.isfile(full_path):
                            logger.info(
                                f"Skipping missing file during scan: {full_path}"
                            )
                            continue

                        all_file_paths.append(full_path)

                        # Emit found file immediately (with no blur status yet)
                        file_info = {"path": full_path, "is_blurred": None}
                        self.files_found.emit([file_info])
                        logger.debug(f"Found: {os.path.basename(full_path)}")

            if not self._is_running:
                self.error.emit("Scan cancelled after file discovery.")
                return

            logger.info(f"File discovery complete. Found {len(all_file_paths)} images.")

            # Phase 2: Parallel blur detection (if enabled)
            all_file_data: List[Dict[str, Any]] = []

            if perform_blur_detection and all_file_paths:
                logger.info(
                    f"Starting parallel blur detection for {len(all_file_paths)} images..."
                )
                from core.app_settings import calculate_max_workers

                max_workers = calculate_max_workers(min_workers=4, max_workers=8)

                with concurrent.futures.ThreadPoolExecutor(
                    max_workers=max_workers
                ) as executor:
                    # Submit all blur detection tasks
                    future_to_path = {
                        executor.submit(
                            self._detect_blur_for_file, path, blur_threshold
                        ): path
                        for path in all_file_paths
                    }

                    # Process results as they complete
                    completed = 0
                    for future in concurrent.futures.as_completed(future_to_path):
                        if not self._is_running:
                            logger.info("Blur detection cancelled by user")
                            break

                        try:
                            result = future.result()
                            all_file_data.append(result)
                            completed += 1

                            if completed % 10 == 0:  # Log progress every 10 files
                                logger.debug(
                                    f"Blur detection progress: {completed}/{len(all_file_paths)}"
                                )
                        except Exception as e:
                            path = future_to_path[future]
                            logger.error(
                                f"Error in blur detection for {os.path.basename(path)}: {e}"
                            )
                            all_file_data.append({"path": path, "is_blurred": None})

                logger.info(
                    f"Parallel blur detection complete for {len(all_file_data)} images"
                )
            else:
                # No blur detection - just create file data with None blur status
                all_file_data = [
                    {"path": path, "is_blurred": None} for path in all_file_paths
                ]

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
