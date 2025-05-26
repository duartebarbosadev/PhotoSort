from PIL import Image, ImageOps
from typing import Optional
import os
import logging

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
            logging.warning(f"Could not apply EXIF transpose: {e}")
            return image

    @staticmethod
    def rotate_image_file_physically(image_path: str, degrees: int = 90) -> bool:
        """
        Physically rotate an image file by the specified degrees.
        Works for all image types by rotating the actual pixel data.
        
        Args:
            image_path: Path to the image file
            degrees: Rotation degrees (90, 180, 270, or -90)
            
        Returns:
            True if successful, False otherwise
        """
        try:
            if degrees not in [90, 180, 270, -90]:
                logging.error(f"Invalid rotation degrees: {degrees}")
                return False

            # Normalize -90 to 270
            if degrees == -90:
                degrees = 270

            with Image.open(image_path) as img:
                # Apply EXIF orientation first to get correctly oriented image
                img = ImageOps.exif_transpose(img)
                
                # Rotate the image
                if degrees == 90:
                    rotated_img = img.transpose(Image.Transpose.ROTATE_270)  # 90° clockwise
                elif degrees == 180:
                    rotated_img = img.transpose(Image.Transpose.ROTATE_180)
                elif degrees == 270:
                    rotated_img = img.transpose(Image.Transpose.ROTATE_90)   # 270° clockwise
                else:
                    return False

                # Save with good quality
                save_kwargs = {'quality': 95, 'optimize': True}
                
                # For JPEG files, try to preserve EXIF but reset orientation
                if image_path.lower().endswith(('.jpg', '.jpeg')):
                    try:
                        exif_dict = img.getexif()
                        if exif_dict:
                            exif_dict[274] = 1  # Set orientation to normal
                            save_kwargs['exif'] = exif_dict.tobytes()
                    except Exception as e:
                        logging.warning(f"Could not preserve EXIF data: {e}")

                # Save the rotated image
                rotated_img.save(image_path, **save_kwargs)
                
                logging.info(f"Successfully rotated {image_path} by {degrees} degrees")
                return True
                
        except Exception as e:
            logging.error(f"Failed to physically rotate {image_path}: {e}")
            return False

    @staticmethod
    def is_image_rotatable(image_path: str) -> bool:
        """
        Check if an image file can be rotated.
        
        Args:
            image_path: Path to the image file
            
        Returns:
            True if the image can be rotated, False otherwise
        """
        try:
            if not os.path.exists(image_path):
                return False
                
            # Check if it's a supported image format
            supported_formats = ['.jpg', '.jpeg', '.png', '.bmp', '.tiff', '.tif']
            file_ext = os.path.splitext(image_path)[1].lower()
            
            if file_ext not in supported_formats:
                return False
                
            # Try to open the image to verify it's valid
            with Image.open(image_path) as img:
                pass  # Just opening is enough to verify
                
            return True
            
        except Exception as e:
            logging.debug(f"Image {image_path} is not rotatable: {e}")
            return False
