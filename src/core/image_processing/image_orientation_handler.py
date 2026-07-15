from PIL import Image, ImageOps
from typing import Any
import logging

logger = logging.getLogger(__name__)


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
            logger.warning("Could not apply EXIF transpose: %s", e, exc_info=True)
            return image

    @staticmethod
    def get_rotation_from_exif(exif_data: dict[str, Any] | None) -> int:
        """
        Gets the rotation in degrees from the EXIF orientation tag.
        Returns 0, 90, 180, or 270. Returns 0 if orientation tag is not present or invalid.
        """
        if not exif_data:
            return 0

        orientation = exif_data.get("Exif.Image.Orientation")
        if not orientation:
            return 0

        try:
            orientation = int(orientation)
        except ValueError, TypeError:
            return 0

        return {
            1: 0,  # Horizontal (normal)
            3: 180,
            6: 90,
            8: 270,
        }.get(orientation, 0)

    @staticmethod
    def get_composite_rotation(
        exif_rotation_degrees: int, model_suggestion_degrees: int
    ) -> int:
        """
        Calculates the net rotation required for an image, considering both its
        EXIF orientation and a suggested rotation from the model.

        The model's suggestion is the rotation needed to correct the raw,
        unrotated image pixels. This function calculates the final rotation
        needed for the image after its EXIF orientation has already been applied.

        Args:
            exif_rotation_degrees (int): The orientation value from get_rotation_from_exif (0, 90, 180, 270).
            model_suggestion_degrees (int): The clockwise rotation suggested by the model (-90, 0, 90, 180).

        Returns:
            int: The net clockwise rotation needed (-90, 0, 90, or 180).
                 Returns 0 if the image is already correctly oriented.
        """
        # This map translates the value from `get_rotation_from_exif` into the
        # effective CLOCKWISE rotation that a standard viewer (like Pillow's
        # ImageOps.exif_transpose) applies to the image data.
        # Both exif_rotation_degrees and model_suggestion_degrees represent the clockwise rotation
        # needed for the RAW image pixels to be upright.
        # The net rotation is the difference between what the model suggests and what EXIF already handles.
        net_rotation = model_suggestion_degrees - exif_rotation_degrees

        # Normalize the angle to the range [-180, 180)
        # e.g. 270 degrees becomes -90
        final_rotation = (net_rotation + 180) % 360 - 180

        # The UI handles -180 as 180
        if final_rotation == -180:
            final_rotation = 180

        return int(final_rotation)
