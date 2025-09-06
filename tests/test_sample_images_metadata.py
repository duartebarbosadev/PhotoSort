"""
Test specific metadata values from sample images to ensure consistent extraction.

This test validates that we can reliably extract the expected metadata values
from known sample images. If pyexiv2 or our abstraction layer changes, these
tests will catch any regressions in metadata extraction.
"""

import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash
import pytest
import os
import sys

# Add the src directory to the path
project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
src_path = os.path.join(project_root, "src")
if src_path not in sys.path:
    sys.path.insert(0, src_path)

from core.metadata_processor import MetadataProcessor  # noqa: E402


class TestSampleImagesMetadata:
    """Test metadata extraction from specific sample images with known values."""

    @classmethod
    def setup_class(cls):
        """Setup paths to sample images."""
        cls.test_folder = "tests/samples"
        cls.arw_sample = os.path.join(cls.test_folder, "arw_sample.arw")
        cls.jpg_sample = os.path.join(cls.test_folder, "jpg_sample.jpg")

        # Check if sample files exist
        cls.arw_exists = os.path.exists(cls.arw_sample)
        cls.jpg_exists = os.path.exists(cls.jpg_sample)

        if not cls.arw_exists and not cls.jpg_exists:
            pytest.skip(
                f"No sample images found in {cls.test_folder}. "
                f"Expected: {cls.arw_sample} and/or {cls.jpg_sample}",
                allow_module_level=True,
            )

    def test_arw_sample_metadata(self):
        """Test metadata extraction from arw_sample.ARW with expected values."""
        if not self.arw_exists:
            pytest.skip("arw_sample.arw not found")

        # Extract detailed metadata using MetadataIO
        from core.metadata_io import MetadataIO

        image_data = MetadataIO.read_raw_metadata(self.arw_sample)

        # Validate file properties
        assert image_data.get("file_path") == self.arw_sample
        assert image_data.get("mime_type") == "image/x-sony-arw"

        # Camera information
        assert image_data.get("Exif.Image.Make") == "SONY"
        assert image_data.get("Exif.Image.Model") == "ILCE-7M3"
        assert image_data.get("Exif.Photo.LensModel") == "FE 28-70mm F3.5-5.6 OSS"

        # Camera settings
        focal_length = image_data.get("Exif.Photo.FocalLength")
        assert focal_length == "560/10", (
            f"Expected focal length 560/10, got {focal_length}"
        )

        aperture = image_data.get("Exif.Photo.FNumber")
        assert aperture == "50/10", f"Expected aperture 50/10 (f/5), got {aperture}"

        shutter_speed = image_data.get("Exif.Photo.ExposureTime")
        assert shutter_speed == "1/500", (
            f"Expected shutter speed 1/500, got {shutter_speed}"
        )

        iso = image_data.get("Exif.Photo.ISOSpeedRatings")
        assert iso == "64", f"Expected ISO 64, got {iso}"

        # Flash setting
        flash = image_data.get("Exif.Photo.Flash")
        assert flash == "16", f"Expected flash value 16 (off), got {flash}"

        # Image dimensions
        assert image_data.get("pixel_width") == 6048
        assert image_data.get("pixel_height") == 4024

        # Orientation (rotated 90° CCW should be orientation 8)
        orientation = image_data.get("Exif.Image.Orientation")
        assert orientation == "8", (
            f"Expected orientation 8 (90° CCW), got {orientation}"
        )

        # Technical settings
        white_balance = image_data.get("Exif.Photo.WhiteBalance")
        assert white_balance == "0", (
            f"Expected white balance 0 (auto), got {white_balance}"
        )

        metering_mode = image_data.get("Exif.Photo.MeteringMode")
        assert metering_mode == "5", (
            f"Expected metering mode 5 (multi-segment), got {metering_mode}"
        )

        exposure_mode = image_data.get("Exif.Photo.ExposureMode")
        assert exposure_mode == "1", (
            f"Expected exposure mode 1 (manual), got {exposure_mode}"
        )

        # Software version
        software = image_data.get("Exif.Image.Software")
        assert software == "ILCE-7M3 v3.10", (
            f"Expected 'ILCE-7M3 v3.10', got {software}"
        )

    def test_jpg_sample_metadata(self):
        """Test metadata extraction from jpg_sample.jpg with expected values."""
        if not self.jpg_exists:
            pytest.skip("jpg_sample.jpg not found")

        # Extract detailed metadata using MetadataIO
        from core.metadata_io import MetadataIO

        image_data = MetadataIO.read_raw_metadata(self.jpg_sample)

        # Validate file properties
        assert image_data.get("file_path") == self.jpg_sample
        assert image_data.get("mime_type") == "image/jpeg"

        # Camera information
        assert image_data.get("Exif.Image.Make") == "SONY"
        assert image_data.get("Exif.Image.Model") == "ILCE-7M3"
        assert image_data.get("Exif.Photo.LensModel") == "FE 28-70mm F3.5-5.6 OSS"

        # Camera settings
        focal_length = image_data.get("Exif.Photo.FocalLength")
        assert focal_length == "300/10", (
            f"Expected focal length 300/10, got {focal_length}"
        )

        aperture = image_data.get("Exif.Photo.FNumber")
        assert aperture == "56/10", f"Expected aperture 56/10 (f/5.6), got {aperture}"

        shutter_speed = image_data.get("Exif.Photo.ExposureTime")
        assert shutter_speed == "1/320", (
            f"Expected shutter speed 1/320, got {shutter_speed}"
        )

        iso = image_data.get("Exif.Photo.ISOSpeedRatings")
        assert iso == "100", f"Expected ISO 100, got {iso}"

        # Flash setting
        flash = image_data.get("Exif.Photo.Flash")
        assert flash == "16", f"Expected flash value 16 (off), got {flash}"

        # Image dimensions
        assert image_data.get("pixel_width") == 6000
        assert image_data.get("pixel_height") == 4000

        # Technical settings
        white_balance = image_data.get("Exif.Photo.WhiteBalance")
        assert white_balance == "0", (
            f"Expected white balance 0 (auto), got {white_balance}"
        )

        metering_mode = image_data.get("Exif.Photo.MeteringMode")
        assert metering_mode == "5", (
            f"Expected metering mode 5 (multi-segment), got {metering_mode}"
        )

        exposure_mode = image_data.get("Exif.Photo.ExposureMode")
        assert exposure_mode == "0", (
            f"Expected exposure mode 0 (auto), got {exposure_mode}"
        )

        # Software (Adobe Lightroom processed)
        software = image_data.get("Exif.Image.Software")
        assert software == "Adobe Lightroom 7.1.2 (Windows)", (
            f"Expected 'Adobe Lightroom 7.1.2 (Windows)', got {software}"
        )

    def test_metadata_consistency_across_formats(self):
        """Test that similar camera settings are extracted consistently."""
        if not (self.arw_exists and self.jpg_exists):
            pytest.skip("Both sample images needed for comparison")

        # Extract detailed metadata for both images
        from core.metadata_io import MetadataIO

        arw_data = MetadataIO.read_raw_metadata(self.arw_sample)
        jpg_data = MetadataIO.read_raw_metadata(self.jpg_sample)

        # Both should have the same camera
        assert (
            arw_data.get("Exif.Image.Make") == jpg_data.get("Exif.Image.Make") == "SONY"
        )
        assert (
            arw_data.get("Exif.Image.Model")
            == jpg_data.get("Exif.Image.Model")
            == "ILCE-7M3"
        )
        assert (
            arw_data.get("Exif.Photo.LensModel")
            == jpg_data.get("Exif.Photo.LensModel")
            == "FE 28-70mm F3.5-5.6 OSS"
        )

        # Both should have flash off (value 16)
        assert (
            arw_data.get("Exif.Photo.Flash") == jpg_data.get("Exif.Photo.Flash") == "16"
        )

        # Both should have auto white balance
        assert (
            arw_data.get("Exif.Photo.WhiteBalance")
            == jpg_data.get("Exif.Photo.WhiteBalance")
            == "0"
        )

        # Both should have multi-segment metering
        assert (
            arw_data.get("Exif.Photo.MeteringMode")
            == jpg_data.get("Exif.Photo.MeteringMode")
            == "5"
        )

    def test_calculated_values(self):
        """Test calculated/derived values like megapixels."""
        if not self.arw_exists:
            pytest.skip("arw_sample.arw not found")

        from core.metadata_io import MetadataIO

        image_data = MetadataIO.read_raw_metadata(self.arw_sample)

        # Calculate megapixels
        width = image_data.get("pixel_width", 0)
        height = image_data.get("pixel_height", 0)
        megapixels = (width * height) / 1_000_000

        # Should be approximately 24.3 MP for ARW (6048 × 4024)
        assert abs(megapixels - 24.3) < 0.1, (
            f"Expected ~24.3 MP, calculated {megapixels:.1f} MP"
        )

    def test_metadata_extraction_reliability(self):
        """Test that metadata extraction is reliable and doesn't fail."""
        if not (self.arw_exists or self.jpg_exists):
            pytest.skip("No sample images available")

        test_files = []
        if self.arw_exists:
            test_files.append(self.arw_sample)
        if self.jpg_exists:
            test_files.append(self.jpg_sample)

        # Should not raise any exceptions and extract detailed metadata
        from core.metadata_io import MetadataIO

        # Extract metadata for each file individually
        for file_path in test_files:
            image_data = MetadataIO.read_raw_metadata(file_path)

            # Should have basic file info
            assert "file_path" in image_data
            assert "file_size" in image_data
            assert "mime_type" in image_data

            # Should have dimensions
            assert "pixel_width" in image_data
            assert "pixel_height" in image_data
            assert isinstance(image_data["pixel_width"], int)
            assert isinstance(image_data["pixel_height"], int)
            assert image_data["pixel_width"] > 0
            assert image_data["pixel_height"] > 0

            # Should not have error field
            assert "error" not in image_data, (
                f"Error in metadata extraction: {image_data.get('error')}"
            )

    def test_specific_tag_formats(self):
        """Test that specific metadata tags are in the expected format."""
        if not self.arw_exists:
            pytest.skip("arw_sample.arw not found")

        metadata = MetadataProcessor.get_batch_display_metadata([self.arw_sample])
        norm_path = os.path.normpath(self.arw_sample)
        image_data = metadata[norm_path]

        # Test rational number formats (should be strings like "560/10")
        focal_length = image_data.get("Exif.Photo.FocalLength")
        if focal_length:
            assert "/" in focal_length, (
                f"Focal length should be in rational format, got {focal_length}"
            )

        aperture = image_data.get("Exif.Photo.FNumber")
        if aperture:
            assert "/" in aperture, (
                f"Aperture should be in rational format, got {aperture}"
            )

        # Test string formats
        make = image_data.get("Exif.Image.Make")
        if make:
            assert isinstance(make, str), (
                f"Camera make should be string, got {type(make)}"
            )

        # Test integer formats (converted to strings by pyexiv2)
        iso = image_data.get("Exif.Photo.ISOSpeedRatings")
        if iso:
            assert iso.isdigit(), f"ISO should be numeric string, got {iso}"


if __name__ == "__main__":
    # Run tests with verbose output
    pytest.main([__file__, "-v", "-s"])
