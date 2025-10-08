from PIL import Image, ImageOps, UnidentifiedImageError
import os
from typing import Optional
import logging

logger = logging.getLogger(__name__)

# Define a reasonable max size for thumbnails to avoid using too much memory
# These might be passed in by an orchestrator class later.
THUMBNAIL_MAX_SIZE = (256, 256)
PRELOAD_MAX_RESOLUTION = (1920, 1200)  # Fixed high resolution for preloading
BLUR_DETECTION_PREVIEW_SIZE = (640, 480)  # Size for image used in blur detection

# Standard image extensions this processor will handle
SUPPORTED_STANDARD_EXTENSIONS = {
    ".jpg",
    ".jpeg",
    ".png",
    ".bmp",
    ".gif",
    ".tif",
    ".tiff",
    ".heic",
    ".heif",
}


class StandardImageProcessor:
    """Handles loading and processing of standard image formats (JPEG, PNG, etc.)."""

    @staticmethod
    def is_standard_extension(ext: str) -> bool:
        """Checks if the extension is a supported standard image format."""
        return ext.lower() in SUPPORTED_STANDARD_EXTENSIONS

    @staticmethod
    def process_for_thumbnail(
        image_path: str,
        thumbnail_max_size: tuple = THUMBNAIL_MAX_SIZE,
        apply_orientation: bool = True,
    ) -> Optional[Image.Image]:
        """
        Generates a PIL.Image thumbnail from a standard image file.
        """
        normalized_path = os.path.normpath(image_path)
        try:
            img = Image.open(normalized_path)
            if apply_orientation:
                img = ImageOps.exif_transpose(img)  # Correct orientation

            # Two-pass resampling: fast initial downsize, then high-quality final pass
            # This is faster when images are much larger than the target size
            if (
                img.width > thumbnail_max_size[0] * 2
                or img.height > thumbnail_max_size[1] * 2
            ):
                intermediate_size = (
                    thumbnail_max_size[0] * 2,
                    thumbnail_max_size[1] * 2,
                )
                img.thumbnail(intermediate_size, Image.Resampling.BILINEAR)

            img.thumbnail(thumbnail_max_size, Image.Resampling.LANCZOS)
            final_pil_img = img.convert("RGBA")  # RGBA required for Qt compatibility
            return final_pil_img
        except UnidentifiedImageError:
            logger.error(
                f"Pillow could not identify image for thumbnail: {os.path.basename(normalized_path)}",
                exc_info=True,
            )
            return None
        except FileNotFoundError:
            logger.error(
                f"File not found for thumbnail: {os.path.basename(normalized_path)}",
                exc_info=True,
            )
            return None
        except Exception as e:
            logger.error(
                f"Failed to process thumbnail for '{os.path.basename(normalized_path)}': {e}",
                exc_info=True,
            )
            return None

    @staticmethod
    def process_for_preview(
        image_path: str, preview_max_resolution: tuple = PRELOAD_MAX_RESOLUTION
    ) -> Optional[Image.Image]:
        """
        Generates a PIL.Image preview from a standard image file for preloading.
        """
        normalized_path = os.path.normpath(image_path)
        try:
            img = Image.open(normalized_path)
            img = ImageOps.exif_transpose(img)  # Correct orientation

            # Two-pass resampling: fast initial downsize, then high-quality final pass
            # This is faster when images are much larger than the target size
            if (
                img.width > preview_max_resolution[0] * 2
                or img.height > preview_max_resolution[1] * 2
            ):
                intermediate_size = (
                    preview_max_resolution[0] * 2,
                    preview_max_resolution[1] * 2,
                )
                img.thumbnail(intermediate_size, Image.Resampling.BILINEAR)

            img.thumbnail(preview_max_resolution, Image.Resampling.LANCZOS)
            pil_img = img.convert("RGBA")  # RGBA required for Qt compatibility
            return pil_img
        except UnidentifiedImageError:
            logger.error(
                f"Pillow could not identify image for preview: {os.path.basename(normalized_path)}",
                exc_info=True,
            )
            return None
        except FileNotFoundError:
            logger.error(
                f"File not found for preview: {os.path.basename(normalized_path)}",
                exc_info=True,
            )
            return None
        except Exception as e:
            logger.error(
                f"Failed to process preview for '{os.path.basename(normalized_path)}': {e}",
                exc_info=True,
            )
            return None

    @staticmethod
    def load_as_pil(
        image_path: str, target_mode: str = "RGB", apply_exif_transpose: bool = True
    ) -> Optional[Image.Image]:
        """
        Loads a standard image as a PIL Image object.
        """
        normalized_path = os.path.normpath(image_path)
        try:
            img = Image.open(normalized_path)
            if apply_exif_transpose:
                img = ImageOps.exif_transpose(img)  # Correct orientation
            pil_img = img.convert(target_mode)
            return pil_img
        except UnidentifiedImageError:
            logger.error(
                f"Pillow could not identify image: {os.path.basename(normalized_path)}",
                exc_info=True,
            )
            return None
        except FileNotFoundError:
            logger.error(
                f"File not found: {os.path.basename(normalized_path)}", exc_info=True
            )
            return None
        except Exception as e:
            logger.error(
                f"Failed to load image '{os.path.basename(normalized_path)}' as PIL object: {e}",
                exc_info=True,
            )
            return None

    @staticmethod
    def load_for_blur_detection(
        image_path: str, target_size: tuple = BLUR_DETECTION_PREVIEW_SIZE
    ) -> Optional[Image.Image]:
        """
        Loads and prepares a PIL image (RGB) from a standard image file for blur detection,
        scaled to target_size.
        """
        normalized_path = os.path.normpath(image_path)
        try:
            img = Image.open(normalized_path)
            img = ImageOps.exif_transpose(img)
            img.thumbnail(target_size, Image.Resampling.LANCZOS)
            pil_img = img.convert("RGB")
            return pil_img
        except UnidentifiedImageError:
            logger.error(
                f"Pillow could not identify image for blur detection: {os.path.basename(normalized_path)}",
                exc_info=True,
            )
            return None
        except FileNotFoundError:
            logger.error(
                f"File not found for blur detection: {os.path.basename(normalized_path)}",
                exc_info=True,
            )
            return None
        except Exception:
            logger.error(
                f"Failed to load image for blur detection '{os.path.basename(normalized_path)}'",
                exc_info=True,
            )
            return None
