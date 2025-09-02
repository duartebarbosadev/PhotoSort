import rawpy
from PIL import Image, ImageOps, UnidentifiedImageError, ImageFile
import io
import os
import logging
import time
from typing import Optional, Set
from PIL import ImageEnhance
from core.app_settings import (
    RAW_AUTO_EDIT_BRIGHTNESS_STANDARD,
    RAW_AUTO_EDIT_BRIGHTNESS_ENHANCED,
    THUMBNAIL_MAX_SIZE,
    PRELOAD_MAX_RESOLUTION,
    BLUR_DETECTION_PREVIEW_SIZE,
)

logger = logging.getLogger(__name__)

# Define a reasonable max size for thumbnails to avoid using too much memory
# These might be passed in by an orchestrator class later, but for now,
# they are here as they were in the original ImageHandler logic for these operations.


# Helper function to check RAW extensions safely
_rawpy_formats_checked = False
_rawpy_supported_set: Set[str] = set()


def is_raw_extension(ext: str) -> bool:
    """Checks if the extension is a supported RAW format, handling rawpy errors."""
    global _rawpy_formats_checked, _rawpy_supported_set
    if not _rawpy_formats_checked:
        check_start_time = time.perf_counter()
        logger.info("Initializing rawpy supported formats set...")
        try:
            _rawpy_supported_set = rawpy.supported_formats()
            logger.info("Successfully retrieved formats from rawpy.")
        except AttributeError:
            _rawpy_supported_set = {
                ".arw",
                ".cr2",
                ".cr3",
                ".nef",
                ".nrw",  # Nikon RAW
                ".dng",
                ".orf",
                ".raf",
                ".rw2",
                ".pef",
                ".srw",
                ".raw",
                ".ptx",
                ".cap",
                ".iiq",
                ".eip",
                ".fff",
                ".mef",
                ".mdc",
                ".mos",
                ".mrw",
                ".erf",
                ".kdc",
                ".dcs",
                ".dcr",
                ".x3f",
                ".rwl",
            }
            logger.warning(
                "rawpy.supported_formats() not available. Using a fallback list of RAW extensions."
            )
        except Exception as e:
            logger.error(
                f"Error getting rawpy supported formats: {e}. Using fallback list."
            )
            _rawpy_supported_set = {
                ".arw",
                ".cr2",
                ".cr3",
                ".nef",
                ".dng",
                ".orf",
                ".raf",
                ".rw2",
                ".pef",
                ".srw",
                ".raw",
            }
        _rawpy_formats_checked = True
        logger.info(
            f"rawpy supported formats set initialized in {time.perf_counter() - check_start_time:.4f}s (Count: {len(_rawpy_supported_set)})"
        )
    return ext.lower() in _rawpy_supported_set


class RawImageProcessor:
    """Handles loading and processing of RAW image formats."""

    @staticmethod
    def process_raw_for_thumbnail(
        image_path: str,
        apply_auto_edits: bool = False,
        thumbnail_max_size: tuple = THUMBNAIL_MAX_SIZE,
    ) -> Optional[Image.Image]:
        """
        Generates a PIL.Image thumbnail from a RAW file.
        Uses embedded thumbnail if suitable, otherwise processes the RAW.
        Applies auto-edits (brightness, contrast) if requested.
        """
        normalized_path = os.path.normpath(image_path)
        final_pil_img: Optional[Image.Image] = None

        try:
            with rawpy.imread(normalized_path) as raw:
                temp_pil_img: Optional[Image.Image] = None
                try:
                    # Attempt to use embedded thumbnail first
                    thumb: ImageFile = raw.extract_thumb()
                    if (
                        thumb.format == rawpy.ThumbFormat.JPEG
                        and thumb.data is not None
                    ):
                        logger.debug(
                            f"Using embedded JPEG thumbnail for: {os.path.basename(normalized_path)}"
                        )
                        temp_pil_img = Image.open(io.BytesIO(thumb.data))
                        temp_pil_img = ImageOps.exif_transpose(
                            temp_pil_img
                        )  # Correct orientation
                        if apply_auto_edits:
                            logger.debug(
                                f"Applying auto-edits to embedded JPEG thumbnail from RAW: {os.path.basename(normalized_path)}"
                            )
                            temp_pil_img = ImageOps.autocontrast(temp_pil_img)
                            enhancer = ImageEnhance.Brightness(temp_pil_img)
                            temp_pil_img = enhancer.enhance(1.1)
                    if temp_pil_img is None:
                        raise rawpy.LibRawNoThumbnailError(
                            "No suitable (JPEG) embedded thumbnail found."
                        )
                except (
                    rawpy.LibRawNoThumbnailError,
                    rawpy.LibRawUnsupportedThumbnailError,
                ):
                    logger.debug(
                        f"No suitable embedded thumbnail for {os.path.basename(normalized_path)}. Post-processing RAW."
                    )
                    # Fallback to processing the main image, optimized with half_size=True
                    postprocess_params = {
                        "use_camera_wb": True,
                        "output_bps": 8,
                        "half_size": True,
                    }
                    if apply_auto_edits:
                        logger.debug(
                            f"Applying auto-edits (bright={RAW_AUTO_EDIT_BRIGHTNESS_STANDARD}) via rawpy for: {os.path.basename(normalized_path)}"
                        )
                        postprocess_params["bright"] = RAW_AUTO_EDIT_BRIGHTNESS_STANDARD
                        postprocess_params["no_auto_bright"] = False
                    else:
                        logger.debug(
                            f"Disabling auto-bright via rawpy for: {os.path.basename(normalized_path)}"
                        )
                        postprocess_params["no_auto_bright"] = True

                    rgb = raw.postprocess(**postprocess_params)
                    temp_pil_img = Image.fromarray(rgb)
                    if apply_auto_edits:
                        logger.debug(
                            f"Applying ImageOps.autocontrast post-rawpy for: {os.path.basename(normalized_path)}"
                        )
                        temp_pil_img = ImageOps.autocontrast(temp_pil_img)

                if temp_pil_img:
                    temp_pil_img.thumbnail(thumbnail_max_size, Image.Resampling.LANCZOS)
                    final_pil_img = temp_pil_img.convert("RGBA")  # Ensure RGBA
            return final_pil_img
        except UnidentifiedImageError:
            logger.error(
                f"Pillow could not identify image from rawpy data for thumbnail: {os.path.basename(normalized_path)}"
            )
            return None
        except rawpy.LibRawIOError as e:
            logger.error(
                "rawpy I/O error for thumbnail '%s': %s",
                os.path.basename(normalized_path),
                e,
            )
            return None
        except rawpy.LibRawUnspecifiedError as e:
            logger.error(
                "rawpy unspecified error for thumbnail '%s': %s",
                os.path.basename(normalized_path),
                e,
            )
            return None
        except Exception as e:
            logger.error(
                f"Failed to process RAW thumbnail for '{os.path.basename(normalized_path)}': {e}",
                exc_info=True,
            )
            return None

    @staticmethod
    def process_raw_for_preview(
        image_path: str,
        apply_auto_edits: bool = False,
        preview_max_resolution: tuple = PRELOAD_MAX_RESOLUTION,
    ) -> Optional[Image.Image]:
        """
        Generates a PIL.Image preview from a RAW file for preloading.
        Attempts to use embedded JPEG or half-size RAW processing for speed.
        Applies auto-edits if requested.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img: Optional[Image.Image] = None
        try:
            with rawpy.imread(normalized_path) as raw:
                # Attempt 1: Extract a large enough embedded JPEG preview
                try:
                    thumb: ImageFile = raw.extract_thumb()
                    if (
                        thumb.format == rawpy.ThumbFormat.JPEG
                        and thumb.data is not None
                    ):
                        temp_img: Image.Image = Image.open(io.BytesIO(thumb.data))
                        temp_img = ImageOps.exif_transpose(temp_img)

                        MIN_EMBEDDED_WIDTH: int = preview_max_resolution[0] // 2
                        MIN_EMBEDDED_HEIGHT: int = preview_max_resolution[1] // 2
                        if (
                            temp_img.width >= MIN_EMBEDDED_WIDTH
                            and temp_img.height >= MIN_EMBEDDED_HEIGHT
                        ):
                            logger.info(
                                f"Using embedded JPEG preview ({temp_img.width}x{temp_img.height}) for: {os.path.basename(normalized_path)}"
                            )
                            if apply_auto_edits:
                                logger.debug(
                                    f"Applying auto-edits to embedded JPEG preview from RAW: {os.path.basename(normalized_path)}"
                                )
                                temp_img = ImageOps.autocontrast(temp_img)
                                enhancer = ImageEnhance.Brightness(temp_img)
                                temp_img = enhancer.enhance(
                                    1.2
                                )  # Example brightness factor
                            pil_img = temp_img.convert("RGBA")
                            if (
                                pil_img.width > preview_max_resolution[0]
                                or pil_img.height > preview_max_resolution[1]
                            ):
                                pil_img.thumbnail(
                                    preview_max_resolution, Image.Resampling.LANCZOS
                                )
                except (
                    rawpy.LibRawNoThumbnailError,
                    rawpy.LibRawUnsupportedThumbnailError,
                ):
                    logger.debug(
                        f"No suitable embedded thumbnail for {os.path.basename(normalized_path)}. Post-processing."
                    )
                    pass  # Fall through to postprocessing
                except Exception as e_thumb:
                    logger.warning(
                        f"Error processing embedded thumbnail for {os.path.basename(normalized_path)}: {e_thumb}"
                    )
                    pass  # Fall through to postprocessing

                # Attempt 2: Fallback to postprocessing (half_size for speed)
                if pil_img is None:
                    logger.debug(
                        f"Falling back to raw.postprocess for: {os.path.basename(normalized_path)}"
                    )
                    postprocess_params = {
                        "use_camera_wb": True,
                        "output_bps": 8,
                        "half_size": True,
                    }
                    if apply_auto_edits:
                        logger.debug(
                            f"Applying auto-edits (bright={RAW_AUTO_EDIT_BRIGHTNESS_STANDARD}) via rawpy for: {os.path.basename(normalized_path)}"
                        )
                        postprocess_params["bright"] = RAW_AUTO_EDIT_BRIGHTNESS_STANDARD
                        postprocess_params["no_auto_bright"] = False
                    else:
                        logger.debug(
                            f"Disabling auto-bright via rawpy for: {os.path.basename(normalized_path)}"
                        )
                        postprocess_params["no_auto_bright"] = True

                    rgb_array = raw.postprocess(**postprocess_params)
                    img_from_raw = Image.fromarray(rgb_array)

                    if apply_auto_edits:
                        logger.debug(
                            f"Applying ImageOps.autocontrast post-rawpy for: {os.path.basename(normalized_path)}"
                        )
                        img_from_raw = ImageOps.autocontrast(img_from_raw)

                    img_from_raw.thumbnail(
                        preview_max_resolution, Image.Resampling.LANCZOS
                    )
                    pil_img = img_from_raw.convert("RGBA")
            return pil_img
        except UnidentifiedImageError:
            logger.error(
                f"Pillow could not identify image from rawpy data for preview: {os.path.basename(normalized_path)}"
            )
            return None
        except rawpy.LibRawIOError as e:
            logger.error(
                "rawpy I/O error for preview '%s': %s",
                os.path.basename(normalized_path),
                e,
            )
            return None
        except rawpy.LibRawUnspecifiedError as e:
            logger.error(
                "rawpy unspecified error for preview '%s': %s",
                os.path.basename(normalized_path),
                e,
            )
            return None
        except Exception as e:
            logger.error(
                f"Failed to process RAW preview for '{os.path.basename(normalized_path)}': {e}",
                exc_info=True,
            )
            return None

    @staticmethod
    def load_raw_as_pil(
        image_path: str,
        target_mode: str = "RGB",
        apply_auto_edits: bool = False,  # For consistency, though 'bright' and 'no_auto_bright' are more direct
        use_camera_wb: bool = True,
        output_bps: int = 8,
        half_size: bool = False,  # Default to full resolution unless specified
        custom_whitebalance: Optional[list] = None,  # e.g. [R, G, B, G2]
        demosaic_algorithm: Optional[
            rawpy.DemosaicAlgorithm
        ] = None,  # e.g. rawpy.DemosaicAlgorithm.AAHD
        force_default_brightness: bool = False,
    ) -> Optional[Image.Image]:
        """
        Loads a RAW image as a PIL Image object, with more granular control over rawpy postprocessing.
        'apply_auto_edits' will enable brightness adjustment and auto-contrast.
        'force_default_brightness' can be used with 'apply_auto_edits' to skip the
        extra brightness factor, which is useful for post-rotation processing.
        """
        normalized_path = os.path.normpath(image_path)
        try:
            with rawpy.imread(normalized_path) as raw:
                postprocess_params = {
                    "use_camera_wb": use_camera_wb,
                    "output_bps": output_bps,
                    "half_size": half_size,
                    "no_auto_bright": False,  # Default to False, allow rawpy's auto brightening
                }
                if demosaic_algorithm:
                    postprocess_params["demosaic_algorithm"] = demosaic_algorithm
                if custom_whitebalance:  # custom_whitebalance overrides use_camera_wb
                    postprocess_params["user_wb"] = custom_whitebalance
                    postprocess_params.pop("use_camera_wb", None)

                if apply_auto_edits:
                    if not force_default_brightness:
                        logger.debug(
                            f"Applying auto-edits (bright={RAW_AUTO_EDIT_BRIGHTNESS_ENHANCED}) via rawpy for: {os.path.basename(normalized_path)}"
                        )
                        postprocess_params["bright"] = (
                            RAW_AUTO_EDIT_BRIGHTNESS_ENHANCED  # Apply custom brightness
                        )
                    else:
                        logger.debug(
                            f"Applying auto-edits but forcing default brightness for: {os.path.basename(normalized_path)}"
                        )
                    postprocess_params["no_auto_bright"] = False
                else:
                    # When auto_edits are OFF, disable rawpy's auto-brightening
                    logger.info(
                        f"Auto-edits OFF. Disabling rawpy auto-bright for: {os.path.basename(normalized_path)}"
                    )
                    postprocess_params["no_auto_bright"] = True

                rgb_array = raw.postprocess(**postprocess_params)
                pil_img = Image.fromarray(rgb_array)

                # Apply PIL enhancements if auto_edits are on, regardless of brightness setting
                if apply_auto_edits:
                    logger.info(
                        f"Applying PIL ImageOps.autocontrast for: {os.path.basename(normalized_path)}"
                    )
                    pil_img = ImageOps.autocontrast(pil_img)

                    logger.info(
                        f"Applying PIL ImageEnhance.Color (1.2) for: {os.path.basename(normalized_path)}"
                    )
                    color_enhancer = ImageEnhance.Color(pil_img)
                    pil_img = color_enhancer.enhance(1.2)  # Enhance color saturation

                return pil_img.convert(target_mode)
        except UnidentifiedImageError:
            logger.error(
                f"Pillow could not process data from rawpy: {os.path.basename(normalized_path)}"
            )
            return None
        except rawpy.LibRawIOError as e:
            logger.error(
                "rawpy I/O error for '%s': %s", os.path.basename(normalized_path), e
            )
            return None
        except rawpy.LibRawUnspecifiedError as e:
            logger.error(
                "rawpy unspecified error for '%s': %s",
                os.path.basename(normalized_path),
                e,
            )
            return None
        except Exception as e:
            logger.error(
                f"Failed to load RAW as PIL for '{os.path.basename(normalized_path)}': {e}",
                exc_info=True,
            )
            return None

    @staticmethod
    def load_raw_for_blur_detection(
        image_path: str,
        target_size: tuple = BLUR_DETECTION_PREVIEW_SIZE,
        apply_auto_edits: bool = False,
    ) -> Optional[Image.Image]:
        """
        Loads and prepares a PIL image (RGB) from a RAW file for blur detection, scaled to target_size.
        Uses efficient methods (embedded or half-size postprocess).
        """
        normalized_path = os.path.normpath(image_path)
        pil_img: Optional[Image.Image] = None
        try:
            with rawpy.imread(normalized_path) as raw:
                temp_pil_img: Optional[Image.Image] = None
                try:  # Attempt embedded thumbnail first
                    thumb: ImageFile = raw.extract_thumb()
                    if (
                        thumb.format == rawpy.ThumbFormat.JPEG
                        and thumb.data is not None
                    ):
                        temp_pil_img = Image.open(io.BytesIO(thumb.data))
                        temp_pil_img = ImageOps.exif_transpose(temp_pil_img)
                except (
                    rawpy.LibRawNoThumbnailError,
                    rawpy.LibRawUnsupportedThumbnailError,
                ):
                    pass  # Fallback to postprocessing

                if temp_pil_img is None:  # Fallback to postprocessing
                    postprocess_params = {
                        "use_camera_wb": True,
                        "output_bps": 8,
                        "half_size": True,
                    }
                    if apply_auto_edits:
                        logger.debug(
                            f"Applying auto-edits (bright={RAW_AUTO_EDIT_BRIGHTNESS_STANDARD}) for blur detection load: {os.path.basename(normalized_path)}"
                        )
                        postprocess_params["bright"] = RAW_AUTO_EDIT_BRIGHTNESS_STANDARD
                        postprocess_params["no_auto_bright"] = False
                    else:
                        logger.debug(
                            f"Disabling auto-bright for blur detection load: {os.path.basename(normalized_path)}"
                        )
                        postprocess_params["no_auto_bright"] = True

                    rgb_array = raw.postprocess(**postprocess_params)
                    temp_pil_img = Image.fromarray(rgb_array)
                    if apply_auto_edits:
                        logger.debug(
                            f"Applying ImageOps.autocontrast for blur detection load: {os.path.basename(normalized_path)}"
                        )
                        temp_pil_img = ImageOps.autocontrast(temp_pil_img)

                if temp_pil_img:
                    # If embedded thumbnail was used and auto-edits applied, do it here too
                    if (
                        apply_auto_edits
                        and raw.extract_thumb().format == rawpy.ThumbFormat.JPEG
                    ):  # A bit of a simplification
                        logger.debug(
                            f"Applying auto-edits to embedded JPEG thumbnail from RAW for blur detection: {os.path.basename(normalized_path)}"
                        )
                        temp_pil_img = ImageOps.autocontrast(temp_pil_img)
                        enhancer = ImageEnhance.Brightness(temp_pil_img)
                        temp_pil_img = enhancer.enhance(1.1)

                    temp_pil_img.thumbnail(target_size, Image.Resampling.LANCZOS)
                    pil_img = temp_pil_img.convert("RGB")
            return pil_img
        except UnidentifiedImageError:
            logger.error(
                f"Pillow could not process data from rawpy for blur detection: {os.path.basename(normalized_path)}"
            )
            return None
        except rawpy.LibRawIOError as e:
            logger.error(
                "rawpy I/O error for blur detection '%s': %s",
                os.path.basename(normalized_path),
                e,
            )
            return None
        except rawpy.LibRawUnspecifiedError as e:
            logger.error(
                "rawpy unspecified error for blur detection '%s': %s",
                os.path.basename(normalized_path),
                e,
            )
            return None
        except Exception:
            logger.error(
                f"Failed to load RAW for blur detection '{os.path.basename(normalized_path)}'",
                exc_info=True,
            )
            return None
