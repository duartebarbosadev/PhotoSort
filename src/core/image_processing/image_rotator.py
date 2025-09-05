import os
import logging
import subprocess
import tempfile
from typing import Literal, Tuple
from PIL import Image, ImageOps
from core.metadata_io import MetadataIO
from pathlib import Path
from core.image_file_ops import ImageFileOperations
from core.app_settings import JPEGTRAN_TIMEOUT_SECONDS
import piexif  # For EXIF manipulation with Pillow
from pillow_heif import HeifImageFile  # For HEIF/HEIC support

logger = logging.getLogger(__name__)

# Rotation directions
RotationDirection = Literal["clockwise", "counterclockwise", "180"]


class ImageRotator:
    # Define supported image formats for different rotation types
    _RAW_FORMATS_EXIF_ONLY = {
        ".arw",
        ".cr2",
        ".nef",
        ".dng",
        ".orf",
        ".raf",
        ".rw2",
        ".pef",
        ".srw",
    }
    _LOSSLESS_JPEG_FORMATS = {".jpg", ".jpeg"}
    _LOSSLESS_HEIF_FORMATS = {
        ".heif",
        ".heic",
    }  # For lossless HEIF/HEIC rotation via metadata
    _STANDARD_PIXEL_ROTATION_FORMATS = {
        ".png",
        ".tiff",
        ".tif",
        ".bmp",
    }  # Removed .heif, .heic
    _XMP_UPDATE_SUPPORTED_EXTENSIONS = {
        ".jpg",
        ".jpeg",
        ".tiff",
        ".tif",
        ".heif",
        ".heic",
    }
    """
    Handles image rotation with support for:
    1. Lossless rotation for JPEG files (using jpegtran if available)
    2. Standard rotation for PNG and other formats
    3. XMP orientation metadata updates for all supported formats
    """

    def __init__(self):
        self.jpegtran_available = self._check_jpegtran_availability()
        logger.info(f"jpegtran availability: {self.jpegtran_available}")

    def _check_jpegtran_availability(self) -> bool:
        """Check if jpegtran is available in the system PATH."""
        try:
            result = subprocess.run(
                ["jpegtran", "-version"],
                capture_output=True,
                text=True,
                timeout=JPEGTRAN_TIMEOUT_SECONDS,
            )
            return result.returncode == 0
        except (
            subprocess.SubprocessError,
            FileNotFoundError,
            subprocess.TimeoutExpired,
        ):
            return False

    def _get_current_orientation(self, image_path: str) -> int:
        """Get current EXIF orientation value (1-8, default 1)."""
        file_ext = os.path.splitext(image_path)[1].lower()

        if file_ext in self._LOSSLESS_HEIF_FORMATS:
            try:
                with Image.open(image_path) as img:
                    if isinstance(img, HeifImageFile):
                        exif_bytes = img.info.get("exif")
                        if exif_bytes:
                            exif_dict = piexif.load(exif_bytes)
                            orientation = exif_dict["0th"].get(
                                piexif.ImageIFD.Orientation
                            )
                            if orientation:
                                return int(orientation)
            except Exception as e:
                logger.warning(
                    f"Could not read HEIF/HEIC orientation from '{os.path.basename(image_path)}': {e}"
                )
            return 1  # Default if pillow-heif fails or no EXIF

        orientation = MetadataIO.read_exif_orientation(image_path)
        if orientation is not None:
            return orientation
        else:
            logger.warning(
                f"Could not read EXIF orientation from '{os.path.basename(image_path)}' (defaulting to 1)"
            )
        return 1  # Default orientation (no rotation)

    def _calculate_new_orientation(
        self, current_orientation: int, direction: RotationDirection
    ) -> int:
        """
        Calculate new orientation value based on current orientation and rotation direction.
        EXIF orientation values 1-8 represent different rotations and flips.
        """
        # Maps for primary rotation states (1, 6, 3, 8) which correspond to 0°, 90°, 180°, 270°
        rotation_map = {
            "clockwise": {1: 6, 6: 3, 3: 8, 8: 1},
            "counterclockwise": {1: 8, 8: 3, 3: 6, 6: 1},
            "180": {1: 3, 3: 1, 6: 8, 8: 6},
        }
        # Maps for flipped states (2, 7, 4, 5)
        flipped_map = {
            "clockwise": {2: 5, 5: 4, 4: 7, 7: 2},
            "counterclockwise": {2: 7, 7: 4, 4: 5, 5: 2},
            "180": {2: 4, 4: 2, 5: 7, 7: 5},
        }

        if direction not in rotation_map:
            return current_orientation

        # Check if current orientation is a flipped state
        if current_orientation in flipped_map[direction]:
            return flipped_map[direction][current_orientation]
        # Otherwise, use the primary rotation map
        elif current_orientation in rotation_map[direction]:
            return rotation_map[direction][current_orientation]

        return current_orientation

    def _rotate_jpeg_lossless(
        self, image_path: str, direction: RotationDirection
    ) -> bool:
        """
        Perform lossless JPEG rotation using jpegtran.
        Returns True if successful, False otherwise.
        """
        if not self.jpegtran_available:
            return False

        try:
            # Map rotation direction to jpegtran parameters
            if direction == "clockwise":
                transform = "-rotate 90"
            elif direction == "counterclockwise":
                transform = "-rotate 270"
            elif direction == "180":
                transform = "-rotate 180"
            else:
                return False

            # Create temporary file for output
            with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as temp_file:
                temp_path = temp_file.name

            # Execute jpegtran
            cmd = f'jpegtran -copy all -perfect {transform} "{image_path}"'
            result = subprocess.run(
                cmd,
                shell=True,
                capture_output=True,
                stdout=open(temp_path, "wb"),
                timeout=30,
            )

            if result.returncode == 0 and os.path.getsize(temp_path) > 0:
                # Replace original with rotated version
                success, msg = ImageFileOperations.replace_file(temp_path, image_path)
                if success:
                    logger.info(
                        f"Lossless JPEG rotation successful: {os.path.basename(image_path)}."
                    )
                else:
                    logger.error(
                        f"Lossless JPEG rotation failed during file replacement for '{os.path.basename(image_path)}': {msg}"
                    )
                return success
            else:
                logger.warning(
                    "jpegtran command failed for '%s'. Stderr: %s",
                    os.path.basename(image_path),
                    result.stderr.decode(),
                )
                return False

        except Exception as e:
            logger.error(
                "Error during lossless JPEG rotation for '%s': %s",
                os.path.basename(image_path),
                e,
                exc_info=True,
            )
            return False
        finally:
            # Clean up temp file if it exists
            if "temp_path" in locals() and os.path.exists(temp_path):
                try:
                    os.unlink(temp_path)
                except OSError:
                    pass

    def _rotate_image_standard(
        self, image_path: str, direction: RotationDirection
    ) -> bool:
        """
        Perform standard image rotation using PIL.
        Works for PNG, and as fallback for JPEG.
        """
        try:
            # Open image
            with Image.open(image_path) as img:
                # Apply current EXIF orientation first
                img = ImageOps.exif_transpose(img)

                # Perform rotation
                if direction == "clockwise":
                    rotated = img.transpose(
                        Image.Transpose.ROTATE_270
                    )  # PIL rotate_270 = clockwise 90°
                elif direction == "counterclockwise":
                    rotated = img.transpose(
                        Image.Transpose.ROTATE_90
                    )  # PIL rotate_90 = counterclockwise 90°
                elif direction == "180":
                    rotated = img.transpose(Image.Transpose.ROTATE_180)
                else:
                    return False

                # Save the rotated image
                # Preserve original format and quality
                save_kwargs = {}
                if img.format == "JPEG":
                    save_kwargs["quality"] = 95
                    save_kwargs["optimize"] = True
                elif img.format == "PNG":
                    save_kwargs["optimize"] = True
                elif img.format in ["HEIF", "HEIC"]:  # Pillow-heif handles HEIF/HEIC
                    # Pillow-heif handles quality and other parameters automatically
                    pass

                rotated.save(image_path, format=img.format, **save_kwargs)
                logger.info(
                    f"Standard rotation successful for: {os.path.basename(image_path)}."
                )
                return True

        except Exception as e:
            logger.error(
                "Error during standard rotation for '%s': %s",
                os.path.basename(image_path),
                e,
                exc_info=True,
            )
            return False

    def _update_xmp_orientation(self, image_path: str, new_orientation: int) -> bool:
        """
        Update XMP orientation metadata in the image file.
        This ensures proper orientation handling by applications that support XMP.
        Prioritizes pyexiv2, but uses pillow-heif for HEIF/HEIC.
        """
        file_ext = os.path.splitext(image_path)[1].lower()

        if file_ext in self._LOSSLESS_HEIF_FORMATS:
            # Use pillow-heif for HEIF/HEIC metadata update
            return self._update_heif_orientation_lossless(image_path, new_orientation)

        exif_ok = MetadataIO.set_exif_orientation(image_path, new_orientation)
        xmp_ok = False
        if file_ext in self._XMP_UPDATE_SUPPORTED_EXTENSIONS:
            xmp_ok = MetadataIO.set_xmp_orientation(image_path, new_orientation)

        if exif_ok or xmp_ok:
            logger.info(
                "Orientation metadata for '%s' updated to %d.",
                os.path.basename(image_path),
                new_orientation,
            )
            return True
        else:
            logger.warning(
                f"Failed to update orientation metadata for '{os.path.basename(image_path)}'."
            )
            return False

    def _update_heif_orientation_lossless(
        self, image_path: str, new_orientation: int
    ) -> bool:
        """
        Update EXIF orientation metadata for HEIF/HEIC files using pillow-heif and piexif.
        This method aims for lossless metadata update by only modifying the EXIF tag in-place.
        """
        try:
            # 1. Read existing EXIF data using pillow-heif
            with Image.open(image_path) as img:
                if not isinstance(img, HeifImageFile):
                    logger.warning(
                        f"Cannot use HEIF-specific update for non-HEIF file: {os.path.basename(image_path)}"
                    )
                    return False

                exif_bytes = img.info.get("exif")
                if not exif_bytes:
                    logger.warning(
                        f"No EXIF data found in '{os.path.basename(image_path)}'. Cannot update orientation."
                    )
                    return False

            # 2. Modify the Orientation tag using piexif
            exif_dict = piexif.load(exif_bytes)
            exif_dict["0th"][piexif.ImageIFD.Orientation] = new_orientation
            new_exif_bytes = piexif.dump(exif_dict)

            # 3. Insert the new EXIF data back into the file using piexif.insert
            # This should modify the EXIF block in-place without re-encoding the image data.
            piexif.insert(new_exif_bytes, image_path)

            logger.info(
                f"Lossless HEIF/HEIC metadata update successful for: {os.path.basename(image_path)}."
            )
            return True

        except Exception as e:
            logger.error(
                f"Error updating HEIF/HEIC orientation for '{os.path.basename(image_path)}': {e}",
                exc_info=True,
            )
            return False

    def rotate_image(
        self,
        image_path: str,
        direction: RotationDirection,
        update_metadata_only: bool = False,
    ) -> Tuple[bool, str]:
        """
        Rotate an image in the specified direction.

        Args:
            image_path: Path to the image file
            direction: Rotation direction ('clockwise', 'counterclockwise', '180')
            update_metadata_only: If True, only update orientation metadata without rotating pixels

        Returns:
            Tuple of (success: bool, message: str)
        """
        if not os.path.isfile(image_path):
            return False, f"File not found: {image_path}"

        # Validate rotation direction
        valid_directions = ["clockwise", "counterclockwise", "180"]
        if direction not in valid_directions:
            return (
                False,
                f"Invalid rotation direction: {direction}. Must be one of {valid_directions}",
            )

        file_ext = Path(image_path).suffix.lower()
        filename = os.path.basename(image_path)

        # Get current orientation
        current_orientation = self._get_current_orientation(image_path)
        new_orientation = self._calculate_new_orientation(
            current_orientation, direction
        )

        logger.info(
            f"Rotating '{filename}' {direction} (Orientation: {current_orientation} -> {new_orientation})"
        )

        success = False
        method_used = ""

        # Check if this is a RAW format that should only use metadata rotation
        is_raw_format = file_ext in self._RAW_FORMATS_EXIF_ONLY

        if update_metadata_only or is_raw_format:
            # Only update metadata, don't rotate pixels
            success = self._update_xmp_orientation(image_path, new_orientation)
            if is_raw_format:
                method_used = "metadata-only (RAW format)"
            else:
                method_used = "metadata-only"
        else:
            # Rotate the actual image pixels
            if file_ext in self._LOSSLESS_JPEG_FORMATS and self.jpegtran_available:
                # Try lossless JPEG rotation first
                success = self._rotate_jpeg_lossless(image_path, direction)
                method_used = "lossless JPEG"

                if not success:
                    # Fallback to standard rotation (lossy for JPEG)
                    success = self._rotate_image_standard(image_path, direction)
                    method_used = "standard (lossy fallback)"
            elif file_ext in self._LOSSLESS_HEIF_FORMATS:
                # Use metadata-only rotation for HEIF/HEIC (lossless)
                success = self._update_xmp_orientation(image_path, new_orientation)
                method_used = "lossless HEIF/HEIC (metadata-only)"
            elif file_ext in self._STANDARD_PIXEL_ROTATION_FORMATS:
                # Use standard rotation for PNG, TIFF, BMP (lossy re-encoding)
                success = self._rotate_image_standard(image_path, direction)
                method_used = "standard (pixel rotation)"
            else:
                # This case should ideally not be reached if is_rotation_supported is checked first
                success = False
                method_used = "unsupported format for pixel rotation"

            # Update orientation metadata after pixel rotation, resetting to 1
            # This block should only execute if pixel rotation actually occurred.
            if success and method_used not in [
                "lossless HEIF/HEIC (metadata-only)",
                "metadata-only (RAW format)",
                "metadata-only",
            ]:
                self._update_xmp_orientation(image_path, 1)

        if success:
            message = f"Successfully rotated {filename} {direction} using {method_used} method"
            logger.info(message)
            return True, message
        else:
            message = (
                f"Failed to rotate {filename} {direction} using {method_used} method"
            )
            logger.error("Rotation failed for %s: %s", filename, message)
            return False, message

    def rotate_clockwise(
        self, image_path: str, update_metadata_only: bool = False
    ) -> Tuple[bool, str]:
        """Rotate image 90° clockwise."""
        return self.rotate_image(image_path, "clockwise", update_metadata_only)

    def rotate_counterclockwise(
        self, image_path: str, update_metadata_only: bool = False
    ) -> Tuple[bool, str]:
        """Rotate image 90° counterclockwise."""
        return self.rotate_image(image_path, "counterclockwise", update_metadata_only)

    def rotate_180(
        self, image_path: str, update_metadata_only: bool = False
    ) -> Tuple[bool, str]:
        """Rotate image 180°."""
        return self.rotate_image(image_path, "180", update_metadata_only)

    def get_supported_formats(self) -> list[str]:
        """Get list of supported image formats for rotation."""
        formats = list(
            self._LOSSLESS_JPEG_FORMATS.union(self._LOSSLESS_HEIF_FORMATS)
            .union(self._STANDARD_PIXEL_ROTATION_FORMATS)
            .union(self._RAW_FORMATS_EXIF_ONLY)
        )
        if self.jpegtran_available:
            formats.append(".jpg (lossless via jpegtran)")
        formats.append(
            ".heif (lossless via metadata)"
        )  # Add specific mention for HEIF/HEIC lossless
        formats.append(".heic (lossless via metadata)")
        return sorted(
            list(set(formats))
        )  # Use set to remove duplicates, then convert to list and sort

    def try_metadata_rotation_first(
        self, image_path: str, direction: RotationDirection
    ) -> Tuple[bool, bool, str]:
        """
        Try metadata-only rotation first (the preferred lossless method).

        Args:
            image_path: Path to the image file
            direction: Rotation direction

        Returns:
            Tuple of (metadata_rotation_succeeded: bool, needs_lossy_rotation: bool, message: str)
        """
        if not os.path.isfile(image_path):
            return False, False, f"File not found: {image_path}"

        file_ext = Path(image_path).suffix.lower()
        filename = os.path.basename(image_path)

        # Get current orientation
        current_orientation = self._get_current_orientation(image_path)
        new_orientation = self._calculate_new_orientation(
            current_orientation, direction
        )

        # Try metadata-only rotation first
        success = self._update_xmp_orientation(image_path, new_orientation)

        if success:
            message = f"Successfully rotated {filename} {direction} using metadata-only (lossless)"
            logger.info(message)
            return True, False, message
        else:
            # Check if this format supports pixel rotation as a fallback
            # For HEIF/HEIC, if metadata rotation fails, we don't offer lossy pixel rotation.
            if file_ext in self._LOSSLESS_HEIF_FORMATS:
                message = f"Metadata rotation failed for {filename} (HEIF/HEIC). No other lossless rotation method available."
                return False, False, message

            pixel_rotation_formats = self._LOSSLESS_JPEG_FORMATS.union(
                self._STANDARD_PIXEL_ROTATION_FORMATS
            )
            raw_formats = self._RAW_FORMATS_EXIF_ONLY

            if file_ext in pixel_rotation_formats:
                # Pixel rotation is possible but will be lossy (except lossless JPEG)
                if file_ext in [".jpg", ".jpeg"] and self.jpegtran_available:
                    message = f"Metadata rotation failed for {filename}. Lossless JPEG rotation available."
                else:
                    message = f"Metadata rotation failed for {filename}. Lossy pixel rotation available."
                return False, True, message
            elif file_ext in raw_formats:
                # RAW files should only use metadata rotation
                message = f"Metadata rotation failed for {filename} (RAW format). No other rotation method available."
                return False, False, message
            else:
                # Unsupported format
                message = f"Rotation not supported for {filename} (format: {file_ext})"
                return False, False, message

    def is_rotation_supported(self, image_path: str) -> bool:
        """Check if rotation is supported for the given image format."""
        file_ext = Path(image_path).suffix.lower()

        # Standard formats that support pixel rotation
        pixel_rotation_formats = self._LOSSLESS_JPEG_FORMATS.union(
            self._STANDARD_PIXEL_ROTATION_FORMATS
        )

        # Formats that support metadata-only rotation (lossless)
        metadata_only_formats = self._RAW_FORMATS_EXIF_ONLY.union(
            self._LOSSLESS_HEIF_FORMATS
        )

        return file_ext in pixel_rotation_formats or file_ext in metadata_only_formats
