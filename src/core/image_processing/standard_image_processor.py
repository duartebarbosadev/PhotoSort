from PIL import Image, ImageOps, UnidentifiedImageError
import os
from typing import Optional

# Define a reasonable max size for thumbnails to avoid using too much memory
# These might be passed in by an orchestrator class later.
THUMBNAIL_MAX_SIZE = (256, 256)
PRELOAD_MAX_RESOLUTION = (1920, 1200) # Fixed high resolution for preloading
BLUR_DETECTION_PREVIEW_SIZE = (640, 480) # Size for image used in blur detection

# Standard image extensions this processor will handle
SUPPORTED_STANDARD_EXTENSIONS = {'.jpg', '.jpeg', '.png', '.bmp', '.gif', '.tif', '.tiff'}

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
            print(f"Error: Pillow could not identify image file (standard thumbnail gen): {normalized_path}")
            return None
        except FileNotFoundError:
            print(f"Error: File not found (standard thumbnail gen): {normalized_path}")
            return None
        except Exception as e:
            print(f"Error in process_for_thumbnail for {normalized_path}: {e} (Type: {type(e).__name__})")
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
            print(f"Error: Pillow could not identify image file (standard preview gen): {normalized_path}")
            return None
        except FileNotFoundError:
            print(f"Error: File not found (standard preview gen): {normalized_path}")
            return None
        except Exception as e:
            print(f"Error in process_for_preview for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

    @staticmethod
    def load_as_pil(
        image_path: str,
        target_mode: str = "RGB"
    ) -> Optional[Image.Image]:
        """
        Loads a standard image as a PIL Image object.
        """
        normalized_path = os.path.normpath(image_path)
        pil_img = None
        try:
            with Image.open(normalized_path) as img:
                img = ImageOps.exif_transpose(img) # Correct orientation
                pil_img = img.convert(target_mode)
            return pil_img
        except UnidentifiedImageError:
            print(f"Error: Pillow could not identify image file (standard load_as_pil): {normalized_path}")
            return None
        except FileNotFoundError:
            print(f"Error: File not found (standard load_as_pil): {normalized_path}")
            return None
        except Exception as e:
            print(f"Error in load_as_pil for {normalized_path}: {e} (Type: {type(e).__name__})")
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
            print(f"Error: Pillow could not identify image file (standard blur detection load): {normalized_path}")
            return None
        except FileNotFoundError:
            print(f"Error: File not found (standard blur detection load): {normalized_path}")
            return None
        except Exception as e:
            print(f"Error in load_for_blur_detection for {normalized_path}: {e} (Type: {type(e).__name__})")
            return None

if __name__ == '__main__':
    # Example Usage (requires a sample standard image file)
    # Create a dummy JPG/PNG file for testing.
    sample_std_file = "test_image.jpg" # Replace with a path to your standard image file

    # Create a dummy file for testing if it doesn't exist
    if not os.path.exists(sample_std_file):
        try:
            from PIL import ImageDraw
            img = Image.new('RGB', (100, 100), color = 'red')
            d = ImageDraw.Draw(img)
            d.text((10,10), "Hello", fill=(255,255,0))
            img.save(sample_std_file)
            print(f"Created dummy test file: {sample_std_file}")
        except Exception as e:
            print(f"Could not create dummy test file: {e}")


    if not os.path.exists(sample_std_file):
        print(f"Test standard image file not found: {sample_std_file}")
        print("Please provide a sample JPG/PNG file or ensure Pillow can create one.")
    else:
        print(f"--- Testing StandardImageProcessor with: {sample_std_file} ---")

        # Test 1: is_standard_extension
        print(f"\nIs '{sample_std_file}' a standard extension? {StandardImageProcessor.is_standard_extension(os.path.splitext(sample_std_file)[1])}")
        print(f"Is '.cr2' a standard extension? {StandardImageProcessor.is_standard_extension('.cr2')}")

        # Test 2: Process for thumbnail
        print("\nTesting process_for_thumbnail...")
        thumb = StandardImageProcessor.process_for_thumbnail(sample_std_file)
        if thumb:
            print(f"  Success: Thumbnail created with size {thumb.size}, mode {thumb.mode}")
            # thumb.save("test_std_thumb.png")
        else:
            print("  Failed to generate thumbnail.")

        # Test 3: Process for preview
        print("\nTesting process_for_preview...")
        preview = StandardImageProcessor.process_for_preview(sample_std_file)
        if preview:
            print(f"  Success: Preview created with size {preview.size}, mode {preview.mode}")
            # preview.save("test_std_preview.png")
        else:
            print("  Failed to generate preview.")

        # Test 4: Load as PIL (RGB)
        print("\nTesting load_as_pil (RGB)...")
        pil_rgb = StandardImageProcessor.load_as_pil(sample_std_file, target_mode="RGB")
        if pil_rgb:
            print(f"  Success: PIL (RGB) image loaded with size {pil_rgb.size}, mode {pil_rgb.mode}")
        else:
            print("  Failed to load as PIL (RGB).")

        # Test 5: Load for blur detection
        print("\nTesting load_for_blur_detection...")
        blur_img = StandardImageProcessor.load_for_blur_detection(sample_std_file)
        if blur_img:
            print(f"  Success: Image for blur detection loaded with size {blur_img.size}, mode {blur_img.mode}")
        else:
            print("  Failed to load image for blur detection.")
        
        print("\n--- StandardImageProcessor Tests Complete ---")
        # Clean up dummy file
        # if os.path.exists(sample_std_file) and "test_image.jpg" in sample_std_file:
        #     try:
        #         os.remove(sample_std_file)
        #         print(f"Cleaned up dummy test file: {sample_std_file}")
        #     except Exception as e:
        #         print(f"Could not clean up dummy test file: {e}")