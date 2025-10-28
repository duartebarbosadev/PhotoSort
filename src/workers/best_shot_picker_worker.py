"""
Best Shot Picker Worker
Background worker for AI-powered best shot selection without blocking the UI.
"""

import logging
from typing import Any, Dict, List, Optional

from PyQt6.QtCore import QObject, pyqtSignal

from core.ai.best_shot_picker import BestShotPicker, BestShotPickerError, BestShotResult
from core import app_settings

logger = logging.getLogger(__name__)


class BestShotPickerWorker(QObject):
    """Worker for analyzing images and selecting the best shot in a background thread."""

    # Signals
    progress = pyqtSignal(str)  # Progress message
    result_ready = pyqtSignal(object)  # BestShotResult object
    finished = pyqtSignal(bool)  # Success status
    error = pyqtSignal(str)  # Error message

    def __init__(self, parent=None):
        super().__init__(parent)
        self._is_running = True
        self.picker = None

    def stop(self):
        """Signal the worker to stop processing."""
        self._is_running = False

    def analyze_images(
        self,
        image_paths: List[str],
        preview_pil_map: Optional[Dict[str, Dict[str, Any]]] = None,
    ):
        """
        Analyze images and select the best one.

        Args:
            image_paths: List of image file paths to analyze
            preview_pil_map: Optional mapping of image paths to cached preview
                payloads containing PIL images (and optional metadata) that
                should be sent to the AI instead of reloading from disk.
        """
        self._is_running = True

        try:
            # Get settings
            api_url = app_settings.get_ai_best_shot_api_url()
            api_key = app_settings.get_ai_best_shot_api_key()
            model = app_settings.get_ai_best_shot_model()
            timeout = app_settings.get_ai_best_shot_timeout()

            logger.info(
                f"Starting best shot analysis for {len(image_paths)} images "
                f"using API: {api_url}"
            )
            
            # Log the order of images being analyzed
            for idx, path in enumerate(image_paths, 1):
                from pathlib import Path
                logger.info(f"  Image {idx}: {Path(path).name}")

            # Create picker instance
            self.picker = BestShotPicker(
                base_url=api_url, api_key=api_key, model=model, timeout=timeout
            )

            # Emit progress
            self.progress.emit("Testing API connection...")

            # Test connection
            if not self.picker.test_connection():
                error_msg = (
                    "Failed to connect to AI API. Please check your settings and "
                    "ensure LM Studio (or compatible server) is running."
                )
                logger.error(error_msg)
                self.error.emit(error_msg)
                self.finished.emit(False)
                return

            self.progress.emit("Connection established. Preparing analysis...")

            if not self._is_running:
                logger.info("Analysis cancelled by user")
                self.finished.emit(False)
                return

            # Analyze images
            self.progress.emit(
                f"Analyzing {len(image_paths)} image(s) with AI..."
            )

            preview_overrides: Dict[str, Dict[str, Any]] = {}
            if preview_pil_map:
                for image_path, payload in preview_pil_map.items():
                    pil_image = payload.get("pil_image")
                    if pil_image is None:
                        continue

                    mime_type = payload.get("mime_type", "image/jpeg")
                    overlay_label = payload.get("overlay_label")

                    try:
                        encoded_data, effective_mime = self.picker.prepare_preview_payload(
                            image_path=image_path,
                            pil_image=pil_image,
                            overlay_label=overlay_label,
                            mime_type=mime_type,
                        )
                    except BestShotPickerError as encode_error:
                        logger.debug(
                            "Failed to prepare preview override for %s: %s",
                            image_path,
                            encode_error,
                        )
                        continue

                    override_entry: Dict[str, Any] = {
                        "base64": encoded_data,
                        "mime_type": effective_mime,
                    }
                    if overlay_label:
                        override_entry["overlay_label"] = overlay_label

                    preview_overrides[image_path] = override_entry

                if preview_overrides:
                    logger.info(
                        "Prepared %d cached preview override(s) for AI analysis",
                        len(preview_overrides),
                    )

            result = self.picker.select_best_image(
                image_paths, preview_overrides=preview_overrides or None
            )

            if not self._is_running:
                logger.info("Analysis cancelled by user")
                self.finished.emit(False)
                return

            self.progress.emit("Analysis complete.")

            # Emit result
            logger.info(
                f"Best shot selected: {result.best_image_path} "
                f"(confidence: {result.confidence})"
            )
            self.result_ready.emit(result)
            self.finished.emit(True)

        except ValueError as e:
            error_msg = f"Invalid input: {e}"
            logger.error(error_msg)
            self.error.emit(error_msg)
            self.finished.emit(False)

        except BestShotPickerError as e:
            error_msg = f"Analysis failed: {e}"
            logger.error(error_msg)
            self.error.emit(error_msg)
            self.finished.emit(False)

        except Exception as e:
            error_msg = f"Unexpected error during analysis: {e}"
            logger.exception(error_msg)
            self.error.emit(error_msg)
            self.finished.emit(False)
