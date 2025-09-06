"""
PyExiv2 Abstraction Layer

This module provides a safe, centralized interface for all pyexiv2 operations.
It ensures proper initialization, thread safety, and consistent error handling
for all metadata operations.

All pyexiv2 usage in the application should go through this abstraction layer
to prevent DLL conflicts and ensure proper initialization order.
"""

import os
import logging
import threading
from typing import Dict, Any, Optional, List
from contextlib import contextmanager

# Import our initialization module first
from core.pyexiv2_init import ensure_pyexiv2_initialized

# Now import pyexiv2
ensure_pyexiv2_initialized()
import pyexiv2  # noqa: E402  # Must be after initialization to prevent DLL conflicts

logger = logging.getLogger(__name__)

# Global lock to guard all pyexiv2 operations for thread safety
_PYEXIV2_LOCK = threading.Lock()


class PyExiv2Error(Exception):
    """Custom exception for pyexiv2-related errors."""

    pass


class PyExiv2ImageWrapper:
    """
    Safe wrapper for pyexiv2.Image operations.

    This class ensures proper initialization, thread safety, and consistent
    error handling for all pyexiv2 operations.
    """

    def __init__(self, image_path: str, encoding: str = "utf-8"):
        """
        Initialize the wrapper.

        Args:
            image_path: Path to the image file
            encoding: Encoding to use for metadata operations
        """
        self.image_path = image_path
        self.encoding = encoding
        self._img = None

    def __enter__(self):
        """Context manager entry."""
        # Ensure pyexiv2 is initialized in this thread
        ensure_pyexiv2_initialized()

        # Acquire the global lock and create the image
        self._lock = _PYEXIV2_LOCK
        self._lock.acquire()
        try:
            self._img = pyexiv2.Image(self.image_path, encoding=self.encoding)
            return self
        except Exception:
            self._lock.release()
            raise

    def __exit__(self, exc_type, exc_val, exc_tb):
        """Context manager exit."""
        try:
            if self._img is not None:
                self._img.close()
        finally:
            self._lock.release()

    def get_pixel_width(self) -> int:
        """Get image pixel width."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        return self._img.get_pixel_width()

    def get_pixel_height(self) -> int:
        """Get image pixel height."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        return self._img.get_pixel_height()

    def get_mime_type(self) -> str:
        """Get image MIME type."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        return self._img.get_mime_type()

    def read_exif(self) -> Optional[Dict[str, Any]]:
        """Read EXIF metadata."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        return self._img.read_exif()

    def read_iptc(self) -> Optional[Dict[str, Any]]:
        """Read IPTC metadata."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        return self._img.read_iptc()

    def read_xmp(self) -> Optional[Dict[str, Any]]:
        """Read XMP metadata."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        return self._img.read_xmp()

    def modify_exif(self, exif_dict: Dict[str, Any]) -> None:
        """Modify EXIF metadata."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        self._img.modify_exif(exif_dict)

    def modify_iptc(self, iptc_dict: Dict[str, Any]) -> None:
        """Modify IPTC metadata."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        self._img.modify_iptc(iptc_dict)

    def modify_xmp(self, xmp_dict: Dict[str, Any]) -> None:
        """Modify XMP metadata."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        self._img.modify_xmp(xmp_dict)

    def clear_exif(self) -> None:
        """Clear EXIF metadata."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        self._img.clear_exif()

    def clear_iptc(self) -> None:
        """Clear IPTC metadata."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        self._img.clear_iptc()

    def clear_xmp(self) -> None:
        """Clear XMP metadata."""
        if self._img is None:
            raise PyExiv2Error("Image not opened")
        self._img.clear_xmp()


@contextmanager
def safe_pyexiv2_image(image_path: str, encoding: str = "utf-8"):
    """
    Context manager for safe pyexiv2.Image operations.

    This is the recommended way to access pyexiv2 functionality.
    It ensures proper initialization, thread safety, and resource cleanup.

    Args:
        image_path: Path to the image file
        encoding: Encoding to use for metadata operations

    Yields:
        PyExiv2ImageWrapper: Safe wrapper for pyexiv2 operations

    Example:
        with safe_pyexiv2_image(image_path) as img:
            exif_data = img.read_exif()
            width = img.get_pixel_width()
    """
    wrapper = PyExiv2ImageWrapper(image_path, encoding)
    with wrapper as img:
        yield img


class PyExiv2Operations:
    """
    High-level operations using pyexiv2.

    This class provides common metadata operations with built-in error handling
    and logging. All methods are thread-safe and ensure proper initialization.
    """

    @staticmethod
    def get_comprehensive_metadata(image_path: str) -> Dict[str, Any]:
        """
        Get comprehensive metadata for an image.

        Args:
            image_path: Path to the image file

        Returns:
            Dictionary containing all metadata (EXIF, IPTC, XMP) plus basic info

        Raises:
            PyExiv2Error: If metadata extraction fails
        """
        try:
            with safe_pyexiv2_image(image_path) as img:
                metadata = {
                    "file_path": image_path,
                    "pixel_width": img.get_pixel_width(),
                    "pixel_height": img.get_pixel_height(),
                    "mime_type": img.get_mime_type(),
                    "file_size": os.path.getsize(image_path)
                    if os.path.isfile(image_path)
                    else "Unknown",
                }

                # Add EXIF, IPTC, and XMP data
                exif_data = img.read_exif() or {}
                iptc_data = img.read_iptc() or {}
                xmp_data = img.read_xmp() or {}

                metadata.update(exif_data)
                metadata.update(iptc_data)
                metadata.update(xmp_data)

                return metadata

        except Exception as e:
            logger.error(
                f"Failed to extract metadata from {os.path.basename(image_path)}: {e}"
            )
            raise PyExiv2Error(f"Metadata extraction failed: {e}") from e

    @staticmethod
    def get_basic_info(image_path: str) -> Dict[str, Any]:
        """
        Get basic image information (dimensions, MIME type, file size).

        Args:
            image_path: Path to the image file

        Returns:
            Dictionary with basic image information
        """
        try:
            with safe_pyexiv2_image(image_path) as img:
                return {
                    "file_path": image_path,
                    "pixel_width": img.get_pixel_width(),
                    "pixel_height": img.get_pixel_height(),
                    "mime_type": img.get_mime_type(),
                    "file_size": os.path.getsize(image_path)
                    if os.path.isfile(image_path)
                    else "Unknown",
                }
        except Exception as e:
            logger.error(
                f"Failed to get basic info from {os.path.basename(image_path)}: {e}"
            )
            raise PyExiv2Error(f"Basic info extraction failed: {e}") from e

    @staticmethod
    def get_orientation(image_path: str) -> int:
        """
        Get EXIF orientation value.

        Args:
            image_path: Path to the image file

        Returns:
            EXIF orientation value (1-8), defaults to 1 if not found
        """
        try:
            with safe_pyexiv2_image(image_path) as img:
                exif_data = img.read_exif() or {}
                orientation = exif_data.get("Exif.Image.Orientation")
                return int(orientation) if orientation else 1
        except Exception as e:
            logger.warning(
                f"Could not read orientation from {os.path.basename(image_path)}: {e}"
            )
            return 1

    @staticmethod
    def set_orientation(image_path: str, orientation: int) -> bool:
        """
        Set EXIF orientation value.

        Args:
            image_path: Path to the image file
            orientation: EXIF orientation value (1-8)

        Returns:
            True if successful, False otherwise
        """
        try:
            with safe_pyexiv2_image(image_path) as img:
                img.modify_exif({"Exif.Image.Orientation": str(orientation)})
                return True
        except Exception as e:
            logger.error(
                f"Failed to set orientation for {os.path.basename(image_path)}: {e}"
            )
            return False

    @staticmethod
    def get_rating(image_path: str) -> Optional[int]:
        """
        Get image rating from metadata.

        Args:
            image_path: Path to the image file

        Returns:
            Rating value (0-5) or None if not found
        """
        try:
            with safe_pyexiv2_image(image_path) as img:
                exif_data = img.read_exif() or {}
                xmp_data = img.read_xmp() or {}

                # Check multiple possible rating fields
                rating_fields = [
                    "Exif.Image.Rating",
                    "Xmp.xmp.Rating",
                    "Xmp.MicrosoftPhoto.Rating",
                ]

                for field in rating_fields:
                    rating = exif_data.get(field) or xmp_data.get(field)
                    if rating is not None:
                        try:
                            return int(rating)
                        except (ValueError, TypeError):
                            continue

                return None

        except Exception as e:
            logger.error(
                f"Failed to get rating from {os.path.basename(image_path)}: {e}"
            )
            return None

    @staticmethod
    def set_rating(image_path: str, rating: int) -> bool:
        """
        Set image rating in metadata.

        Args:
            image_path: Path to the image file
            rating: Rating value (0-5)

        Returns:
            True if successful, False otherwise
        """
        if not 0 <= rating <= 5:
            raise ValueError("Rating must be between 0 and 5")

        try:
            with safe_pyexiv2_image(image_path) as img:
                # Set rating in both EXIF and XMP for compatibility
                img.modify_exif({"Exif.Image.Rating": str(rating)})
                img.modify_xmp({"Xmp.xmp.Rating": str(rating)})
                return True
        except Exception as e:
            logger.error(
                f"Failed to set rating for {os.path.basename(image_path)}: {e}"
            )
            return False

    @staticmethod
    def batch_get_metadata(image_paths: List[str]) -> List[Dict[str, Any]]:
        """
        Get metadata for multiple images.

        Args:
            image_paths: List of image file paths

        Returns:
            List of metadata dictionaries
        """
        results = []
        for path in image_paths:
            try:
                metadata = PyExiv2Operations.get_comprehensive_metadata(path)
                results.append(metadata)
            except PyExiv2Error:
                # Add error entry for failed extractions
                results.append(
                    {"file_path": path, "error": "Metadata extraction failed"}
                )
        return results


# Convenience function for backward compatibility
def create_safe_image_context(image_path: str, encoding: str = "utf-8"):
    """
    Create a safe pyexiv2 image context.

    This is an alias for safe_pyexiv2_image for backward compatibility.
    """
    return safe_pyexiv2_image(image_path, encoding)


# Initialize on module import
ensure_pyexiv2_initialized()
