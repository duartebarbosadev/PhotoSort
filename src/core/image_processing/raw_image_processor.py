import rawpy
from PIL import Image, ImageOps, UnidentifiedImageError
import io
import os
import logging
import time
from typing import Optional, Set
from PIL import ImageEnhance # Added for brightness adjustment on PIL images

# Define a reasonable max size for thumbnails to avoid using too much memory
# These might be passed in by an orchestrator class later, but for now,
# they are here as they were in the original ImageHandler logic for these operations.
THUMBNAIL_MAX_SIZE = (256, 256)
PRELOAD_MAX_RESOLUTION = (1920, 1200) # Fixed high resolution for preloading
BLUR_DETECTION_PREVIEW_SIZE = (640, 480) # Size for image used in blur detection


# Helper function to check RAW extensions safely
_rawpy_formats_checked = False
_rawpy_supported_set: Set[str] = set()

def is_raw_extension(ext: str) -> bool:
    """Checks if the extension is a supported RAW format, handling rawpy errors."""
    global _rawpy_formats_checked, _rawpy_supported_set
    if not _rawpy_formats_checked:
        check_start_time = time.perf_counter()
        logging.info("raw_image_processor.is_raw_extension - Initializing rawpy supported formats set...")
        try:
            _rawpy_supported_set = rawpy.supported_formats()
            logging.info("raw_image_processor.is_raw_extension - Successfully retrieved formats from rawpy.supported_formats()")
        except AttributeError:
            _rawpy_supported_set = {
                '.arw', '.cr2', '.cr3', '.nef', '.dng', '.orf', '.raf',
                '.rw2', '.pef', '.srw', '.raw', '.ptx', '.cap', '.iiq',
                '.eip', '.fff', '.mef', '.mdc', '.mos', '.mrw', '.erf',
                '.kdc', '.dcs', '.dcr', '.x3f', '.rwl'
            }
            logging.warning("raw_image_processor.is_raw_extension - rawpy.supported_formats() not available. Using a fallback list of RAW extensions.")
            # print("Warning: rawpy.supported_formats() not available. Using a fallback list of RAW extensions.") # Replaced
        except Exception as e:
            logging.error(f"raw_image_processor.is_raw_extension - Error getting rawpy supported formats: {e}. Using fallback list.")
            _rawpy_supported_set = {'.arw', '.cr2', '.cr3', '.nef', '.dng', '.orf', '.raf', '.rw2', '.pef', '.srw', '.raw'}
        _rawpy_formats_checked = True
        logging.info(f"raw_image_processor.is_raw_extension - rawpy supported formats set initialization complete: {time.perf_counter() - check_start_time:.4f}s. Count: {len(_rawpy_supported_set)}")
    return ext.lower() in _rawpy_supported_set


class RawImageProcessor:
    """Handles loading and processing of RAW image formats."""

    @staticmethod
    def process_raw_for_thumbnail(
        image_path: str, 
        apply_auto_edits: bool = False,
        thumbnail_max_size: tuple = THUMBNAIL_MAX_SIZE
    ) -> Optional[Image.Image]:
        """
        Generates a PIL.Image thumbnail from a RAW file.
        Uses embedded thumbnail if suitable, otherwise processes the RAW.
        Applies auto-edits (brightness, contrast) if requested.
        """
        normalized_path = os.path.normpath(image_path)
        final_pil_img = None
        
        try:
            with rawpy.imread(normalized_path) as raw:
                temp_pil_img = None
                try:
                    # Attempt to use embedded thumbnail first
                    thumb = raw.extract_thumb()
                    if thumb.format == rawpy.ThumbFormat.JPEG and thumb.data is not None:
                        logging.debug(f"[RawImageProcessor THUMB] Using embedded JPEG thumbnail for: {normalized_path}")
                        temp_pil_img = Image.open(io.BytesIO(thumb.data))
                        temp_pil_img = ImageOps.exif_transpose(temp_pil_img) # Correct orientation
                        if apply_auto_edits:
                            logging.info(f"[RawImageProcessor THUMB] Applying auto-edits (autocontrast, brightness) to embedded JPEG for: {normalized_path}")
                            temp_pil_img = ImageOps.autocontrast(temp_pil_img)
                            enhancer = ImageEnhance.Brightness(temp_pil_img)
                            temp_pil_img = enhancer.enhance(1.1)
                    if temp_pil_img is None:
                       raise rawpy.LibRawNoThumbnailError("No suitable (JPEG) embedded thumbnail found.")
                except (rawpy.LibRawNoThumbnailError, rawpy.LibRawUnsupportedThumbnailError):
                    logging.debug(f"[RawImageProcessor THUMB] No suitable embedded thumbnail for {normalized_path}, postprocessing RAW.")
                    # Fallback to processing the main image, optimized with half_size=True
                    postprocess_params = {
                        'use_camera_wb': True,
                        'output_bps': 8,
                        'half_size': True
                    }
                    if apply_auto_edits:
                        logging.info(f"[RawImageProcessor THUMB] Applying auto_edits (bright=1.15) via rawpy for: {normalized_path}")
                        postprocess_params['bright'] = 1.15
                        postprocess_params['no_auto_bright'] = False
                    else:
                        logging.info(f"[RawImageProcessor THUMB] NOT applying auto_edits (no_auto_bright=True) via rawpy for: {normalized_path}")
                        postprocess_params['no_auto_bright'] = True
                    
                    rgb = raw.postprocess(**postprocess_params)
                    temp_pil_img = Image.fromarray(rgb)
                    if apply_auto_edits:
                        logging.info(f"[RawImageProcessor THUMB] Applying ImageOps.autocontrast post-rawpy for: {normalized_path}")
                        temp_pil_img = ImageOps.autocontrast(temp_pil_img)
                
                if temp_pil_img:
                    temp_pil_img.thumbnail(thumbnail_max_size, Image.Resampling.LANCZOS)
                    final_pil_img = temp_pil_img.convert("RGBA") # Ensure RGBA
            return final_pil_img
        except UnidentifiedImageError:
            print(f"Error: Pillow could not identify image file (raw thumbnail gen): {normalized_path}")
            return None
        except rawpy.LibRawIOError as e:
            print(f"Error: rawpy I/O error for {normalized_path} (thumbnail): {e}")
            return None
        except rawpy.LibRawUnspecifiedError as e:
            print(f"Error: rawpy unspecified error for {normalized_path} (thumbnail): {e}")
            return None
        except Exception as e:
            print(f"Error in process_raw_for_thumbnail for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

    @staticmethod
    def process_raw_for_preview(
        image_path: str, 
        apply_auto_edits: bool = False,
        preview_max_resolution: tuple = PRELOAD_MAX_RESOLUTION
    ) -> Optional[Image.Image]:
        """
        Generates a PIL.Image preview from a RAW file for preloading.
        Attempts to use embedded JPEG or half-size RAW processing for speed.
        Applies auto-edits if requested.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img = None
        try:
            with rawpy.imread(normalized_path) as raw:
                # Attempt 1: Extract a large enough embedded JPEG preview
                try:
                    thumb = raw.extract_thumb()
                    if thumb.format == rawpy.ThumbFormat.JPEG and thumb.data is not None:
                        temp_img = Image.open(io.BytesIO(thumb.data))
                        temp_img = ImageOps.exif_transpose(temp_img)
                        
                        MIN_EMBEDDED_WIDTH = preview_max_resolution[0] // 2
                        MIN_EMBEDDED_HEIGHT = preview_max_resolution[1] // 2
                        if temp_img.width >= MIN_EMBEDDED_WIDTH and temp_img.height >= MIN_EMBEDDED_HEIGHT:
                            logging.info(f"[RawImageProcessor PRELOAD] Using embedded JPEG preview ({temp_img.width}x{temp_img.height}) for: {normalized_path}")
                            if apply_auto_edits:
                                logging.info(f"[RawImageProcessor PRELOAD] Applying auto-edits (autocontrast, brightness) to embedded JPEG for: {normalized_path}")
                                temp_img = ImageOps.autocontrast(temp_img)
                                enhancer = ImageEnhance.Brightness(temp_img)
                                temp_img = enhancer.enhance(1.2) # Example brightness factor
                            pil_img = temp_img.convert("RGBA")
                            if pil_img.width > preview_max_resolution[0] or pil_img.height > preview_max_resolution[1]:
                                pil_img.thumbnail(preview_max_resolution, Image.Resampling.LANCZOS)
                except (rawpy.LibRawNoThumbnailError, rawpy.LibRawUnsupportedThumbnailError):
                    logging.debug(f"[RawImageProcessor PRELOAD] No suitable embedded thumbnail for {normalized_path}, will postprocess.")
                    pass # Fall through to postprocessing
                except Exception as e_thumb:
                    logging.warning(f"[RawImageProcessor PRELOAD] Error processing embedded thumbnail for {normalized_path}: {e_thumb}")
                    pass # Fall through to postprocessing

                # Attempt 2: Fallback to postprocessing (half_size for speed)
                if pil_img is None:
                    logging.info(f"[RawImageProcessor PRELOAD] Falling back to raw.postprocess for: {normalized_path}")
                    postprocess_params = {
                        'use_camera_wb': True,
                        'output_bps': 8,
                        'half_size': True
                    }
                    if apply_auto_edits:
                        logging.info(f"[RawImageProcessor PRELOAD] Applying auto_edits (bright=1.15) via rawpy for: {normalized_path}")
                        postprocess_params['bright'] = 1.15
                        postprocess_params['no_auto_bright'] = False
                    else:
                        logging.info(f"[RawImageProcessor PRELOAD] NOT applying auto_edits (no_auto_bright=True) via rawpy for: {normalized_path}")
                        postprocess_params['no_auto_bright'] = True
                    
                    rgb_array = raw.postprocess(**postprocess_params)
                    img_from_raw = Image.fromarray(rgb_array)

                    if apply_auto_edits:
                        logging.info(f"[RawImageProcessor PRELOAD] Applying ImageOps.autocontrast post-rawpy for: {normalized_path}")
                        img_from_raw = ImageOps.autocontrast(img_from_raw)
                    
                    img_from_raw.thumbnail(preview_max_resolution, Image.Resampling.LANCZOS)
                    pil_img = img_from_raw.convert("RGBA")
            return pil_img
        except UnidentifiedImageError:
            print(f"Error: Pillow could not identify image file (raw preview gen): {normalized_path}")
            return None
        except rawpy.LibRawIOError as e:
            print(f"Error: rawpy I/O error for {normalized_path} (preview): {e}")
            return None
        except rawpy.LibRawUnspecifiedError as e:
            print(f"Error: rawpy unspecified error for {normalized_path} (preview): {e}")
            return None
        except Exception as e:
            print(f"Error in process_raw_for_preview for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

    @staticmethod
    def load_raw_as_pil(
        image_path: str,
        target_mode: str = "RGB",
        apply_auto_edits: bool = False, # For consistency, though 'bright' and 'no_auto_bright' are more direct
        use_camera_wb: bool = True,
        output_bps: int = 8,
        half_size: bool = False, # Default to full resolution unless specified
        custom_whitebalance: Optional[list] = None, # e.g. [R, G, B, G2]
        demosaic_algorithm: Optional[rawpy.DemosaicAlgorithm] = None # e.g. rawpy.DemosaicAlgorithm.AAHD
    ) -> Optional[Image.Image]:
        """
        Loads a RAW image as a PIL Image object, with more granular control over rawpy postprocessing.
        'apply_auto_edits' will enable brightness adjustment and auto-contrast.
        For more direct control, use specific rawpy parameters.
        """
        normalized_path = os.path.normpath(image_path)
        try:
            with rawpy.imread(normalized_path) as raw:
                postprocess_params = {
                    'use_camera_wb': use_camera_wb,
                    'output_bps': output_bps,
                    'half_size': half_size,
                    'no_auto_bright': False # Default to False, allow rawpy's auto brightening
                }
                if demosaic_algorithm:
                    postprocess_params['demosaic_algorithm'] = demosaic_algorithm
                if custom_whitebalance: # custom_whitebalance overrides use_camera_wb
                    postprocess_params['user_wb'] = custom_whitebalance
                    postprocess_params.pop('use_camera_wb', None)

                if apply_auto_edits:
                    logging.info(f"[RawImageProcessor.load_raw_as_pil] Applying auto-edits (bright=1.25) via rawpy for: {normalized_path}")
                    postprocess_params['bright'] = 1.25 # Increased brightness
                    postprocess_params['no_auto_bright'] = False # Ensure rawpy's auto bright is not disabled
                    # Other params like 'gamma' can be added if specific adjustments are needed
                else:
                    # When auto_edits are OFF, we still want basic rawpy auto-brightening.
                    # So, 'no_auto_bright' remains False (its default in rawpy or explicit here).
                    # We don't set 'bright' here, letting rawpy manage it.
                    logging.info(f"[RawImageProcessor.load_raw_as_pil] Auto-edits OFF. Using rawpy default auto-bright for: {normalized_path}")

                rgb_array = raw.postprocess(**postprocess_params)
                pil_img = Image.fromarray(rgb_array)

                if apply_auto_edits: # Apply PIL enhancements if auto_edits are on
                    logging.info(f"[RawImageProcessor.load_raw_as_pil] Applying PIL ImageOps.autocontrast for: {normalized_path}")
                    pil_img = ImageOps.autocontrast(pil_img)
                    
                    logging.info(f"[RawImageProcessor.load_raw_as_pil] Applying PIL ImageEnhance.Color (1.2) for: {normalized_path}")
                    color_enhancer = ImageEnhance.Color(pil_img)
                    pil_img = color_enhancer.enhance(1.2) # Enhance color saturation

                    # Optional: Further brightness with PIL if rawpy's `bright` isn't enough
                    # logging.info(f"[RawImageProcessor.load_raw_as_pil] Applying PIL ImageEnhance.Brightness (1.1) for: {normalized_path}")
                    # brightness_enhancer = ImageEnhance.Brightness(pil_img)
                    # pil_img = brightness_enhancer.enhance(1.1)
                
                return pil_img.convert(target_mode)
        except UnidentifiedImageError:
            print(f"Error: Pillow could not process data from rawpy (load_raw_as_pil): {normalized_path}")
            return None
        except rawpy.LibRawIOError as e:
            print(f"Error: rawpy I/O error for {normalized_path} (load_raw_as_pil): {e}")
            return None
        except rawpy.LibRawUnspecifiedError as e:
            print(f"Error: rawpy unspecified error for {normalized_path} (load_raw_as_pil): {e}")
            return None
        except Exception as e:
            print(f"Error in load_raw_as_pil for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

    @staticmethod
    def load_raw_for_blur_detection(
        image_path: str,
        target_size: tuple = BLUR_DETECTION_PREVIEW_SIZE,
        apply_auto_edits: bool = False
    ) -> Optional[Image.Image]:
        """
        Loads and prepares a PIL image (RGB) from a RAW file for blur detection, scaled to target_size.
        Uses efficient methods (embedded or half-size postprocess).
        """
        normalized_path = os.path.normpath(image_path)
        pil_img = None
        try:
            with rawpy.imread(normalized_path) as raw:
                temp_pil_img = None
                try: # Attempt embedded thumbnail first
                    thumb = raw.extract_thumb()
                    if thumb.format == rawpy.ThumbFormat.JPEG and thumb.data is not None:
                        temp_pil_img = Image.open(io.BytesIO(thumb.data))
                        temp_pil_img = ImageOps.exif_transpose(temp_pil_img)
                except (rawpy.LibRawNoThumbnailError, rawpy.LibRawUnsupportedThumbnailError):
                    pass # Fallback to postprocessing

                if temp_pil_img is None: # Fallback to postprocessing
                    postprocess_params = {
                        'use_camera_wb': True, 'output_bps': 8, 'half_size': True
                    }
                    if apply_auto_edits:
                        logging.info(f"[RawImageProcessor BLUR_LOAD] Applying auto_edits (bright=1.15) via rawpy for: {normalized_path}")
                        postprocess_params['bright'] = 1.15
                        postprocess_params['no_auto_bright'] = False
                    else:
                        logging.info(f"[RawImageProcessor BLUR_LOAD] NOT applying auto_edits (no_auto_bright=True) via rawpy for: {normalized_path}")
                        postprocess_params['no_auto_bright'] = True
                    
                    rgb_array = raw.postprocess(**postprocess_params)
                    temp_pil_img = Image.fromarray(rgb_array)
                    if apply_auto_edits:
                        logging.info(f"[RawImageProcessor BLUR_LOAD] Applying ImageOps.autocontrast post-rawpy for: {normalized_path}")
                        temp_pil_img = ImageOps.autocontrast(temp_pil_img)
                
                if temp_pil_img:
                    # If embedded thumbnail was used and auto-edits applied, do it here too
                    if apply_auto_edits and raw.extract_thumb().format == rawpy.ThumbFormat.JPEG : # A bit of a simplification
                        logging.info(f"[RawImageProcessor BLUR_LOAD] Applying auto-edits (autocontrast, brightness) to embedded JPEG for blur detection: {normalized_path}")
                        temp_pil_img = ImageOps.autocontrast(temp_pil_img)
                        enhancer = ImageEnhance.Brightness(temp_pil_img)
                        temp_pil_img = enhancer.enhance(1.1)


                    temp_pil_img.thumbnail(target_size, Image.Resampling.LANCZOS)
                    pil_img = temp_pil_img.convert("RGB")
            return pil_img
        except UnidentifiedImageError:
            print(f"Error: Pillow could not process data from rawpy (blur detection): {normalized_path}")
            return None
        except rawpy.LibRawIOError as e:
            print(f"Error: rawpy I/O error for {normalized_path} (blur detection): {e}")
            return None
        except rawpy.LibRawUnspecifiedError as e:
            print(f"Error: rawpy unspecified error for {normalized_path} (blur detection): {e}")
            return None
        except Exception as e:
            print(f"Error in load_raw_for_blur_detection for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None
