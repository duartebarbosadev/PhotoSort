from PIL import Image, ImageOps, UnidentifiedImageError
import os
from typing import Optional
import logging

# Define a reasonable max size for thumbnails to avoid using too much memory
# These might be passed in by an orchestrator class later.
THUMBNAIL_MAX_SIZE = (256, 256)
PRELOAD_MAX_RESOLUTION = (1920, 1200) # Fixed high resolution for preloading
BLUR_DETECTION_PREVIEW_SIZE = (640, 480) # Size for image used in blur detection

# Standard image extensions this processor will handle
SUPPORTED_STANDARD_EXTENSIONS = {'.jpg', '.jpeg', '.png', '.bmp', '.gif', '.tif', '.tiff', '.heic', '.heif'}

class StandardImageProcessor:
    """Handles loading and processing of standard image formats (JPEG, PNG, etc.)."""

    @staticmethod
    def is_standard_extension(ext: str) -> bool:
        """Checks if the extension is a supported standard image format."""
        return ext.lower() in SUPPORTED_STANDARD_EXTENSIONS

    @staticmethod
    def process_for_thumbnail(
        image_path: str,
        thumbnail_max_size: tuple = THUMBNAIL_MAX_SIZE
    ) -> Optional[Image.Image]:
        """
        Generates a PIL.Image thumbnail from a standard image file.
        """
        normalized_path = os.path.normpath(image_path)
        final_pil_img = None
        try:
            with Image.open(normalized_path) as img:
                img = ImageOps.exif_transpose(img) # Correct orientation
                img.thumbnail(thumbnail_max_size, Image.Resampling.LANCZOS)
                final_pil_img = img.convert("RGBA") # Ensure RGBA
            return final_pil_img
        except UnidentifiedImageError:
            logging.error(f"Pillow could not identify image file (standard thumbnail gen): {normalized_path}")
            return None
        except FileNotFoundError:
            logging.error(f"File not found (standard thumbnail gen): {normalized_path}")
            return None
        except Exception as e:
            logging.error(f"Error in process_for_thumbnail for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

    @staticmethod
    def process_for_preview(
        image_path: str,
        preview_max_resolution: tuple = PRELOAD_MAX_RESOLUTION
    ) -> Optional[Image.Image]:
        """
        Generates a PIL.Image preview from a standard image file for preloading.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img = None
        try:
            with Image.open(normalized_path) as img:
                img = ImageOps.exif_transpose(img) # Correct orientation
                img.thumbnail(preview_max_resolution, Image.Resampling.LANCZOS)
                pil_img = img.convert("RGBA") # Ensure RGBA
            return pil_img
        except UnidentifiedImageError:
            logging.error(f"Pillow could not identify image file (standard preview gen): {normalized_path}")
            return None
        except FileNotFoundError:
            logging.error(f"File not found (standard preview gen): {normalized_path}")
            return None
        except Exception as e:
            logging.error(f"Error in process_for_preview for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

    @staticmethod
    def load_as_pil(
        image_path: str,
        target_mode: str = "RGB",
        apply_exif_transpose: bool = True
    ) -> Optional[Image.Image]:
        """
        Loads a standard image as a PIL Image object.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img = None
        try:
            with Image.open(normalized_path) as img:
                if apply_exif_transpose:
                    img = ImageOps.exif_transpose(img) # Correct orientation
                pil_img = img.convert(target_mode)
            return pil_img
        except UnidentifiedImageError:
            logging.error(f"Pillow could not identify image file (standard load_as_pil): {normalized_path}")
            return None
        except FileNotFoundError:
            logging.error(f"File not found (standard load_as_pil): {normalized_path}")
            return None
        except Exception as e:
            logging.error(f"Error in load_as_pil for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

    @staticmethod
    def load_for_blur_detection(
        image_path: str,
        target_size: tuple = BLUR_DETECTION_PREVIEW_SIZE
    ) -> Optional[Image.Image]:
        """
        Loads and prepares a PIL image (RGB) from a standard image file for blur detection,
        scaled to target_size.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img = None
        try:
            with Image.open(normalized_path) as img:
                img = ImageOps.exif_transpose(img)
                img.thumbnail(target_size, Image.Resampling.LANCZOS)
                pil_img = img.convert("RGB")
            return pil_img
        except UnidentifiedImageError:
            logging.error(f"Pillow could not identify image file (standard blur detection load): {normalized_path}")
            return None
        except FileNotFoundError:
            logging.error(f"File not found (standard blur detection load): {normalized_path}")
            return None
        except Exception as e:
            logging.error(f"Error in load_for_blur_detection for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None
