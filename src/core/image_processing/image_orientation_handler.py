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
