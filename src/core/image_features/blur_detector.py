import os
import cv2
import numpy as np
from PIL import Image, UnidentifiedImageError
from typing import Optional, Tuple

# Assuming these processors are in the new structure
from src.core.image_processing.raw_image_processor import RawImageProcessor, is_raw_extension
from src.core.image_processing.standard_image_processor import StandardImageProcessor, SUPPORTED_STANDARD_EXTENSIONS

# Default size for the image used in blur detection
BLUR_DETECTION_PREVIEW_SIZE: Tuple[int, int] = (640, 480)

class BlurDetector:
    """Detects blurriness in images."""

    @staticmethod
    def _load_image_for_detection(
        image_path: str,
        target_size: Tuple[int, int] = BLUR_DETECTION_PREVIEW_SIZE,
        apply_auto_edits_for_raw: bool = False
    ) -> Optional[Image.Image]:
        """
        Loads and prepares a PIL image (RGB) for blur detection, scaled to target_size.
        Uses RawImageProcessor or StandardImageProcessor based on file type.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img: Optional[Image.Image] = None
        
        try:
            ext = os.path.splitext(normalized_path)[1].lower()

            if is_raw_extension(ext):
                pil_img = RawImageProcessor.load_raw_for_blur_detection(
                    normalized_path,
                    target_size=target_size,
                    apply_auto_edits=apply_auto_edits_for_raw
                )
            elif ext in SUPPORTED_STANDARD_EXTENSIONS:
                pil_img = StandardImageProcessor.load_for_blur_detection(
                    normalized_path,
                    target_size=target_size
                )
            else:
                # Fallback for unknown extensions, try opening with Pillow directly
                # This part might be redundant if StandardImageProcessor handles more or
                # if we decide unsupported types are not processed for blur.
                try:
                    print(f"[BlurDetector] Unknown extension '{ext}', attempting to load with Pillow for blur detection.")
                    img = Image.open(normalized_path)
                    # StandardImageProcessor.load_for_blur_detection already handles exif_transpose
                    # So, if we directly use Image.open, we should also apply it.
                    from src.core.image_processing.image_orientation_handler import ImageOrientationHandler # Local import
                    img = ImageOrientationHandler.exif_transpose(img)
                    img.thumbnail(target_size, Image.Resampling.LANCZOS)
                    pil_img = img.convert("RGB")
                except UnidentifiedImageError:
                    print(f"[BlurDetector] Pillow could not identify unknown image type for blur: {normalized_path}")
                    return None
                except FileNotFoundError:
                    print(f"[BlurDetector] File not found for unknown type blur detection: {normalized_path}")
                    return None
            
            return pil_img

        except Exception as e:
            print(f"[BlurDetector] Error in _load_image_for_detection for {normalized_path}: {e}")
            return None

    @staticmethod
    def is_image_blurred(
        image_path: str,
        threshold: float = 100.0,
        apply_auto_edits_for_raw_preview: bool = False,
        target_size: Tuple[int, int] = BLUR_DETECTION_PREVIEW_SIZE
    ) -> Optional[bool]:
        """
        Detects if an image is blurred using the variance of the Laplacian method.
        Operates on a smaller, efficiently loaded preview of the image.

        Args:
            image_path (str): The path to the image file.
            threshold (float): The threshold for blur detection. Lower values indicate more blur.
            apply_auto_edits_for_raw_preview (bool): If RAW, whether to apply auto edits
                                                     to the preview used for blur detection.
            target_size (Tuple[int, int]): The target size for the image used in blur detection.

        Returns:
            Optional[bool]: True if blurred, False if not, None if an error occurs.
        """
        normalized_path = os.path.normpath(image_path)
        if not os.path.isfile(normalized_path):
            print(f"[BlurDetector] Error: File does not exist for blur detection: {normalized_path}")
            return None

        try:
            pil_image_rgb = BlurDetector._load_image_for_detection(
                normalized_path,
                target_size=target_size,
                apply_auto_edits_for_raw=apply_auto_edits_for_raw_preview
            )

            if pil_image_rgb is None:
                # _load_image_for_detection would have printed an error
                return None

            # Convert PIL Image (RGB) to OpenCV format (BGR)
            open_cv_image = cv2.cvtColor(np.array(pil_image_rgb), cv2.COLOR_RGB2BGR)

            if open_cv_image is None: # Should not happen if pil_image_rgb is valid
                print(f"[BlurDetector] OpenCV could not convert PIL data for {normalized_path}")
                return None

            gray = cv2.cvtColor(open_cv_image, cv2.COLOR_BGR2GRAY)
            laplacian_var = cv2.Laplacian(gray, cv2.CV_64F).var()
            
            # print(f"[BlurDetector] Blur detection for {os.path.basename(normalized_path)}: Laplacian Variance = {laplacian_var:.2f}, Threshold = {threshold}, Blurred = {laplacian_var < threshold}")

            return laplacian_var < threshold

        except UnidentifiedImageError: # Should be caught by _load_image_for_detection
            print(f"[BlurDetector] Pillow could not identify image file: {normalized_path}")
            return None
        except Exception as e:
            print(f"[BlurDetector] Error during blur detection for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

if __name__ == '__main__':
    # Example Usage:
    # Ensure you have sample RAW and standard image files.
    # Create dummy files for testing if needed.
    sample_raw_file = "test_image.arw"  # Replace with your RAW file
    sample_jpg_file = "test_image.jpg"  # Replace with your JPG file

    # Dummy RAW (if not present, create an empty file for path testing)
    if not os.path.exists(sample_raw_file):
        open(sample_raw_file, 'a').close()
        print(f"Created dummy RAW file: {sample_raw_file} (will likely fail processing but tests path logic)")

    # Dummy JPG
    if not os.path.exists(sample_jpg_file):
        try:
            img = Image.new('RGB', (600, 400), color = 'blue')
            img.save(sample_jpg_file)
            print(f"Created dummy JPG file: {sample_jpg_file}")
        except Exception as e:
            print(f"Could not create dummy JPG: {e}")
    
    print(f"\n--- Testing BlurDetector ---")

    if os.path.exists(sample_jpg_file):
        print(f"\nTesting JPG: {sample_jpg_file}")
        is_blurred_jpg = BlurDetector.is_image_blurred(sample_jpg_file, threshold=100.0)
        if is_blurred_jpg is not None:
            print(f"  '{sample_jpg_file}' is blurred: {is_blurred_jpg}")
        else:
            print(f"  Could not determine blurriness for '{sample_jpg_file}'.")
    else:
        print(f"\nSkipping JPG test, file not found: {sample_jpg_file}")

    if os.path.exists(sample_raw_file):
        print(f"\nTesting RAW: {sample_raw_file} (no auto-edits for blur preview)")
        # Note: Processing a dummy RAW will likely fail unless it's a valid RAW file.
        # This tests the pathway more than the actual blur result for a dummy.
        is_blurred_raw = BlurDetector.is_image_blurred(sample_raw_file, threshold=100.0, apply_auto_edits_for_raw_preview=False)
        if is_blurred_raw is not None:
            print(f"  '{sample_raw_file}' is blurred: {is_blurred_raw}")
        else:
            print(f"  Could not determine blurriness for '{sample_raw_file}'. (Expected if dummy file)")
    else:
        print(f"\nSkipping RAW test, file not found: {sample_raw_file}")
        
    print("\n--- BlurDetector Tests Complete ---")

    # Cleanup dummy files if they were created by this test script
    # if "test_image.jpg" in sample_jpg_file and os.path.exists(sample_jpg_file):
    #     if Image.open(sample_jpg_file).size == (600,400) : # Basic check it's our dummy
    #         os.remove(sample_jpg_file)
    # if "test_image.arw" in sample_raw_file and os.path.exists(sample_raw_file):
    #     if os.path.getsize(sample_raw_file) == 0: # Basic check it's our dummy
    #         os.remove(sample_raw_file)