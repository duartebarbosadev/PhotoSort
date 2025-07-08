import os
import logging
import subprocess
import tempfile
from typing import Literal, Tuple
from PIL import Image, ImageOps
import pyexiv2
from pathlib import Path
from src.core.image_file_ops import ImageFileOperations
import piexif  # For EXIF manipulation with Pillow
from pillow_heif import HeifImageFile  # For HEIF/HEIC support

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
        logging.info(f"[ImageRotator] jpegtran available: {self.jpegtran_available}")

    def _check_jpegtran_availability(self) -> bool:
        """Check if jpegtran is available in the system PATH."""
        try:
            result = subprocess.run(
                ["jpegtran", "-version"], capture_output=True, text=True, timeout=5
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
                logging.warning(
                    f"[ImageRotator] Could not read HEIF/HEIC orientation from {os.path.basename(image_path)} using pillow-heif: {e}"
                )
            return 1  # Default if pillow-heif fails or no EXIF

        try:
            with pyexiv2.Image(image_path, encoding="utf-8") as img:
                exif_data = img.read_exif()
                orientation = exif_data.get("Exif.Image.Orientation")
                if orientation:
                    return int(orientation)
        except Exception as e:
            logging.warning(
                f"[ImageRotator] Could not read orientation from {os.path.basename(image_path)}: {e}"
            )
        return 1  # Default orientation (no rotation)

    def _calculate_new_orientation(
        self, current_orientation: int, direction: RotationDirection
    ) -> int:
        """
        Calculate new orientation value based on current orientation and rotation direction.
        EXIF orientation values 1-8 represent different rotations and flips.
        """
        # Orientation transformation matrices for 90° rotations
        # These handle the 8 possible EXIF orientation states
        if direction == "clockwise":
            orientation_map = {1: 6, 2: 7, 3: 8, 4: 5, 5: 2, 6: 3, 7: 4, 8: 1}
        elif direction == "counterclockwise":
            orientation_map = {1: 8, 2: 5, 3: 6, 4: 7, 5: 4, 6: 1, 7: 2, 8: 3}
        elif direction == "180":
            orientation_map = {1: 3, 2: 4, 3: 1, 4: 2, 5: 6, 6: 5, 7: 8, 8: 7}
        else:
            return current_orientation

        return orientation_map.get(current_orientation, 1)

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
                    logging.info(
                        f"[ImageRotator] Lossless JPEG rotation successful: {os.path.basename(image_path)}"
                    )
                else:
                    logging.error(
                        f"[ImageRotator] Lossless JPEG rotation failed during file replacement: {msg}"
                    )
                return success
            else:
                logging.warning(
                    f"[ImageRotator] jpegtran failed for {os.path.basename(image_path)}: {result.stderr.decode()}"
                )
                return False

        except Exception as e:
            logging.error(
                f"[ImageRotator] Error in lossless JPEG rotation for {os.path.basename(image_path)}: {e}"
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
                logging.info(
                    f"[ImageRotator] Standard rotation successful: {os.path.basename(image_path)}"
                )
                return True

        except Exception as e:
            logging.error(
                f"[ImageRotator] Error in standard rotation for {os.path.basename(image_path)}: {e}"
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

        pyexiv2_success = False
        try:
            with pyexiv2.Image(image_path, encoding="utf-8") as img:
                # Update EXIF orientation
                try:
                    img.modify_exif({"Exif.Image.Orientation": str(new_orientation)})
                    logging.debug(
                        f"[ImageRotator] Updated EXIF orientation for {os.path.basename(image_path)} using pyexiv2"
                    )
                    pyexiv2_success = True
                except Exception as e:
                    logging.warning(
                        f"[ImageRotator] pyexiv2 could not update EXIF orientation for {os.path.basename(image_path)}: {e}"
                    )

                # Try to set XMP orientation if the format typically supports it via pyexiv2
                if file_ext in self._XMP_UPDATE_SUPPORTED_EXTENSIONS:
                    try:
                        # Check if image already has XMP data or if it's a JPEG (which can always get XMP)
                        xmp_data = img.read_xmp()
                        if xmp_data or file_ext in [".jpg", ".jpeg"]:
                            img.modify_xmp(
                                {"Xmp.tiff.Orientation": str(new_orientation)}
                            )
                            logging.debug(
                                f"[ImageRotator] Updated XMP orientation for {os.path.basename(image_path)} using pyexiv2"
                            )
                            pyexiv2_success = True
                    except Exception as e:
                        logging.debug(
                            f"[ImageRotator] pyexiv2 XMP orientation not updated for {os.path.basename(image_path)}: {e}"
                        )

        except Exception as e:
            logging.warning(
                f"[ImageRotator] pyexiv2 could not open/process {os.path.basename(image_path)} for metadata update: {e}"
            )

        if pyexiv2_success:
            logging.info(
                f"[ImageRotator] Orientation metadata updated to {new_orientation}: {os.path.basename(image_path)}"
            )
            return True
        else:
            logging.warning(
                f"[ImageRotator] Failed to update orientation metadata for {os.path.basename(image_path)} using pyexiv2."
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
                    logging.warning(
                        f"[ImageRotator] Not a HEIF/HEIC file, cannot use pillow-heif specific update: {os.path.basename(image_path)}"
                    )
                    return False

                exif_bytes = img.info.get("exif")
                if not exif_bytes:
                    logging.warning(
                        f"[ImageRotator] No EXIF data found in {os.path.basename(image_path)}. Cannot update orientation."
                    )
                    return False

            # 2. Modify the Orientation tag using piexif
            exif_dict = piexif.load(exif_bytes)
            exif_dict["0th"][piexif.ImageIFD.Orientation] = new_orientation
            new_exif_bytes = piexif.dump(exif_dict)

            # 3. Insert the new EXIF data back into the file using piexif.insert
            # This should modify the EXIF block in-place without re-encoding the image data.
            piexif.insert(new_exif_bytes, image_path)

            logging.info(
                f"[ImageRotator] Lossless HEIF/HEIC metadata update successful using piexif.insert: {os.path.basename(image_path)}"
            )
            return True

        except Exception as e:
            logging.error(
                f"[ImageRotator] Error updating HEIF/HEIC orientation with piexif.insert for {os.path.basename(image_path)}: {e}"
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

        logging.info(
            f"[ImageRotator] Rotating {filename} {direction} (orientation: {current_orientation} -> {new_orientation})"
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
            logging.info(f"[ImageRotator] {message}")
            return True, message
        else:
            message = f"Failed to rotate {filename}"
            logging.error(f"[ImageRotator] {message}")
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
            logging.info(f"[ImageRotator] {message}")
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
