import rawpy
from PIL import Image, ImageOps, UnidentifiedImageError
import io
import os
import logging # Added for startup logging
import time # Added for startup timing
from typing import Optional, Set

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
            # print(f"Error getting rawpy supported formats: {e}. Using fallback list.") # Replaced
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
                        temp_pil_img = Image.open(io.BytesIO(thumb.data))
                        temp_pil_img = ImageOps.exif_transpose(temp_pil_img) # Correct orientation
                    if temp_pil_img is None:
                       raise rawpy.LibRawNoThumbnailError("No suitable (JPEG) embedded thumbnail found.")
                except (rawpy.LibRawNoThumbnailError, rawpy.LibRawUnsupportedThumbnailError):
                    # Fallback to processing the main image, optimized with half_size=True
                    postprocess_params = {
                        'use_camera_wb': True,
                        'output_bps': 8,
                        'half_size': True
                    }
                    if apply_auto_edits:
                        # print(f"DEBUG: Applying auto_edits (bright=1.15) for RAW thumbnail: {normalized_path}")
                        postprocess_params['bright'] = 1.15
                    else:
                        # print(f"DEBUG: NOT applying auto_edits (no_auto_bright=True) for RAW thumbnail: {normalized_path}")
                        postprocess_params['no_auto_bright'] = True
                    
                    rgb = raw.postprocess(**postprocess_params)
                    temp_pil_img = Image.fromarray(rgb)
                    if apply_auto_edits:
                        # print(f"DEBUG: Applying autocontrast for RAW thumbnail: {normalized_path}")
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
                            # print(f"[RawImageProcessor PRELOAD] Using embedded JPEG preview ({temp_img.width}x{temp_img.height}) for: {normalized_path}")
                            pil_img = temp_img.convert("RGBA")
                            if pil_img.width > preview_max_resolution[0] or pil_img.height > preview_max_resolution[1]:
                                pil_img.thumbnail(preview_max_resolution, Image.Resampling.LANCZOS)
                except (rawpy.LibRawNoThumbnailError, rawpy.LibRawUnsupportedThumbnailError):
                    # print(f"[RawImageProcessor PRELOAD] No suitable embedded thumbnail for: {normalized_path}")
                    pass
                except Exception as e_thumb:
                    print(f"[RawImageProcessor PRELOAD] Error processing embedded thumbnail for {normalized_path}: {e_thumb}")

                # Attempt 2: Fallback to postprocessing (half_size for speed)
                if pil_img is None:
                    # print(f"[RawImageProcessor PRELOAD] Falling back to raw.postprocess for: {normalized_path}")
                    postprocess_params = {
                        'use_camera_wb': True,
                        'output_bps': 8,
                        'half_size': True
                    }
                    if apply_auto_edits:
                        # print(f"DEBUG: Applying auto_edits (bright=1.15) for RAW process_raw_for_preview: {normalized_path}")
                        postprocess_params['bright'] = 1.15
                    else:
                        # print(f"DEBUG: NOT applying auto_edits (no_auto_bright=True) for RAW process_raw_for_preview: {normalized_path}")
                        postprocess_params['no_auto_bright'] = True
                    
                    rgb_array = raw.postprocess(**postprocess_params)
                    img_from_raw = Image.fromarray(rgb_array)

                    if apply_auto_edits:
                        # print(f"DEBUG: Applying autocontrast for RAW process_raw_for_preview: {normalized_path}")
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
                    # Default no_auto_bright to True if not applying auto_edits,
                    # and bright is not explicitly set by apply_auto_edits.
                    'no_auto_bright': not apply_auto_edits 
                }
                if demosaic_algorithm:
                    postprocess_params['demosaic_algorithm'] = demosaic_algorithm
                if custom_whitebalance: # custom_whitebalance overrides use_camera_wb
                    postprocess_params['user_wb'] = custom_whitebalance
                    postprocess_params.pop('use_camera_wb', None)


                if apply_auto_edits:
                    postprocess_params['bright'] = 1.15 # Example brightness adjustment
                    postprocess_params['no_auto_bright'] = False # Allow rawpy's auto brightening if bright param is also used

                rgb_array = raw.postprocess(**postprocess_params)
                pil_img = Image.fromarray(rgb_array)

                if apply_auto_edits: # Apply autocontrast after PIL conversion
                    pil_img = ImageOps.autocontrast(pil_img)
                
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
                        postprocess_params['bright'] = 1.15
                        postprocess_params['no_auto_bright'] = False
                    else:
                        postprocess_params['no_auto_bright'] = True
                    
                    rgb_array = raw.postprocess(**postprocess_params)
                    temp_pil_img = Image.fromarray(rgb_array)
                    if apply_auto_edits:
                        temp_pil_img = ImageOps.autocontrast(temp_pil_img)
                
                if temp_pil_img:
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

if __name__ == '__main__':
    # Example Usage (requires a sample RAW file)
    # Create a dummy RAW file for testing if you don't have one.
    # For example, you can often find sample ARW, CR2, NEF files online.
    sample_raw_file = "test_image.arw" # Replace with a path to your RAW file

    if not os.path.exists(sample_raw_file):
        print(f"Test RAW file not found: {sample_raw_file}")
        print("Please provide a sample RAW file to run tests.")
    else:
        print(f"--- Testing RawImageProcessor with: {sample_raw_file} ---")

        # Test 1: is_raw_extension
        print(f"\nIs '{sample_raw_file}' a RAW extension? {is_raw_extension(os.path.splitext(sample_raw_file)[1])}")
        print(f"Is '.jpg' a RAW extension? {is_raw_extension('.jpg')}")

        # Test 2: Process for thumbnail (no auto-edits)
        print("\nTesting process_raw_for_thumbnail (no auto-edits)...")
        thumb_no_edits = RawImageProcessor.process_raw_for_thumbnail(sample_raw_file, apply_auto_edits=False)
        if thumb_no_edits:
            print(f"  Success: Thumbnail created with size {thumb_no_edits.size}, mode {thumb_no_edits.mode}")
            # thumb_no_edits.save("test_raw_thumb_no_edits.png")
        else:
            print("  Failed to generate thumbnail (no auto-edits).")

        # Test 3: Process for thumbnail (with auto-edits)
        print("\nTesting process_raw_for_thumbnail (with auto-edits)...")
        thumb_with_edits = RawImageProcessor.process_raw_for_thumbnail(sample_raw_file, apply_auto_edits=True)
        if thumb_with_edits:
            print(f"  Success: Thumbnail created with size {thumb_with_edits.size}, mode {thumb_with_edits.mode}")
            # thumb_with_edits.save("test_raw_thumb_with_edits.png")
        else:
            print("  Failed to generate thumbnail (with auto-edits).")

        # Test 4: Process for preview (no auto-edits)
        print("\nTesting process_raw_for_preview (no auto-edits)...")
        preview_no_edits = RawImageProcessor.process_raw_for_preview(sample_raw_file, apply_auto_edits=False)
        if preview_no_edits:
            print(f"  Success: Preview created with size {preview_no_edits.size}, mode {preview_no_edits.mode}")
            # preview_no_edits.save("test_raw_preview_no_edits.png")
        else:
            print("  Failed to generate preview (no auto-edits).")

        # Test 5: Load raw as PIL (default settings, RGB)
        print("\nTesting load_raw_as_pil (default settings)...")
        pil_default = RawImageProcessor.load_raw_as_pil(sample_raw_file)
        if pil_default:
            print(f"  Success: PIL image loaded with size {pil_default.size}, mode {pil_default.mode}")
        else:
            print("  Failed to load raw as PIL (default).")

        # Test 6: Load raw for blur detection
        print("\nTesting load_raw_for_blur_detection (no auto-edits)...")
        blur_img = RawImageProcessor.load_raw_for_blur_detection(sample_raw_file, apply_auto_edits=False)
        if blur_img:
            print(f"  Success: Image for blur detection loaded with size {blur_img.size}, mode {blur_img.mode}")
        else:
            print("  Failed to load image for blur detection.")
        
        print("\n--- RawImageProcessor Tests Complete ---")