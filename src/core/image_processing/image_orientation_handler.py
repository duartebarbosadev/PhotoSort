from PIL import Image, ImageOps
from typing import Optional

class ImageOrientationHandler:
    """Handles EXIF-based image orientation correction."""

    @staticmethod
    def exif_transpose(image: Image.Image) -> Image.Image:
        """
        Apply EXIF orientation to an image.
        If the image has an EXIF Orientation tag, transpose the image
        accordingly. Otherwise, return the image unchanged.
        """
        if image is None:
            return image
        try:
            return ImageOps.exif_transpose(image)
        except Exception as e:
            # Log error or handle as appropriate, for now, return original image
            print(f"Warning: Could not apply EXIF transpose: {e}")
            return image

if __name__ == '__main__':
    # This module is primarily a utility.
    # To test, you would need an image with EXIF orientation data.
    # For example:
    # from PIL import Image
    # try:
    #     # Replace with a path to an image that has EXIF orientation
    #     img_path = "path_to_oriented_image.jpg" 
    #     img = Image.open(img_path)
    #     print(f"Original image size: {img.size}, mode: {img.mode}")
    #
    #     # Check orientation from EXIF
    #     exif_data = img._getexif()
    #     orientation_tag = 274 # EXIF Orientation Tag
    #     if exif_data and orientation_tag in exif_data:
    #         print(f"Original EXIF Orientation: {exif_data[orientation_tag]}")
    #     else:
    #         print("No EXIF orientation tag found in original image.")
    #
    #     corrected_img = ImageOrientationHandler.exif_transpose(img.copy()) # Use a copy
    #     print(f"Corrected image size: {corrected_img.size}, mode: {corrected_img.mode}")
    #
    #     # To visually inspect, you might save or show the image
    #     # img.show(title="Original")
    #     # corrected_img.show(title="Corrected by EXIF Transpose")
    #
    # except FileNotFoundError:
    #     print(f"Test image not found at: {img_path}")
    # except Exception as e:
    #     print(f"An error occurred during testing: {e}")
    pass