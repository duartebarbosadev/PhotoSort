import pytest
import os
import sys
import tempfile
import shutil
import logging

# Add the project root to Python path so we can import src modules
project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if project_root not in sys.path:
    sys.path.insert(0, project_root)

# Try to import the modules we need
try:
    from src.core.image_processing.image_rotator import ImageRotator

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)

    # Create dummy classes to avoid NameError
    class ImageRotator:
        pass


class TestImageRotator:
    """Tests for the ImageRotator class."""

    @classmethod
    def setup_class(cls):
        """Setup test environment with sample images."""
        cls.test_folder = "tests/samples"
        cls.sample_images = []

        # Find test images
        if os.path.exists(cls.test_folder):
            for filename in os.listdir(cls.test_folder):
                if filename.lower().endswith(
                    (".png", ".jpg", ".jpeg", ".arw", ".cr2", ".nef")
                ):
                    cls.sample_images.append(os.path.join(cls.test_folder, filename))

        if len(cls.sample_images) == 0:
            pytest.skip(
                f"No test images found in {cls.test_folder}", allow_module_level=True
            )

    def setup_method(self):
        """Setup for each test method."""
        self.rotator = ImageRotator()
        self.temp_files = []

    def teardown_method(self):
        """Clean up temporary files."""
        for temp_file in self.temp_files:
            if os.path.exists(temp_file):
                try:
                    os.unlink(temp_file)
                except Exception:
                    pass

    def _create_temp_copy(self, source_path: str) -> str:
        """Create a temporary copy of an image for testing."""
        _, ext = os.path.splitext(source_path)
        with tempfile.NamedTemporaryFile(suffix=ext, delete=False) as temp_file:
            temp_path = temp_file.name

        shutil.copy2(source_path, temp_path)
        self.temp_files.append(temp_path)
        return temp_path

    def test_rotator_initialization(self):
        """Test that ImageRotator initializes correctly."""
        assert isinstance(self.rotator, ImageRotator)
        # ImageRotator should initialize without issues

    def test_get_supported_formats(self):
        """Test that supported formats are returned correctly."""
        formats = self.rotator.get_supported_formats()
        assert isinstance(formats, list)
        assert ".jpg" in formats
        assert ".png" in formats
        assert ".tiff" in formats or ".tif" in formats

    @pytest.mark.parametrize(
        "filename,expected",
        [
            ("test.jpg", True),
            ("test.jpeg", True),
            ("test.png", True),
            ("test.tiff", True),
            ("test.arw", True),
            ("test.cr2", True),
            ("test.nef", True),
            ("test.dng", True),
            ("test.txt", False),
            ("test.mp4", False),
        ],
    )
    def test_is_rotation_supported(self, filename, expected):
        """Test rotation support checking for different formats."""
        assert self.rotator.is_rotation_supported(filename) == expected

    def test_get_current_orientation_with_sample_images(self):
        """Test reading current EXIF orientation from sample images."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        for image_path in self.sample_images:
            orientation = self.rotator._get_current_orientation(image_path)
            assert isinstance(orientation, int)
            assert 1 <= orientation <= 8  # Valid EXIF orientation range

    @pytest.mark.parametrize(
        "initial,direction,expected",
        [
            (1, "clockwise", 6),
            (1, "counterclockwise", 8),
            (1, "180", 3),
            (3, "clockwise", 8),
            (6, "clockwise", 3),
            (8, "clockwise", 1),
        ],
    )
    def test_calculate_new_orientation(self, initial, direction, expected):
        """Test orientation calculation for different rotations."""
        assert self.rotator._calculate_new_orientation(initial, direction) == expected

    def test_calculate_new_orientation_full_cycle(self):
        """Test that 4 clockwise rotations return to original orientation."""
        orientation = 1
        for _ in range(4):
            orientation = self.rotator._calculate_new_orientation(
                orientation, "clockwise"
            )
        assert orientation == 1  # Should return to original after 4 rotations

    def test_rotation_with_sample_images_clockwise(self):
        """Test clockwise rotation on sample images."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        # Test with first available image
        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        # Test clockwise rotation
        success, message = self.rotator.rotate_clockwise(temp_image)
        logging.info(f"Clockwise rotation: {success}, {message}")

        if success:
            # Verify file still exists and is valid
            assert os.path.exists(temp_image)
            assert os.path.getsize(temp_image) > 0

    def test_metadata_only_rotation(self):
        """Test metadata-only rotation (no pixel changes)."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        # Get original file size
        original_size = os.path.getsize(temp_image)

        # Perform metadata-only rotation
        success, message = self.rotator.rotate_image(
            temp_image, "clockwise", update_metadata_only=True
        )
        logging.info(f"Metadata-only rotation: {success}, {message}")

        # File should still exist with same or very similar size
        assert os.path.exists(temp_image)
        new_size = os.path.getsize(temp_image)
        # Allow small size difference due to metadata changes
        assert abs(new_size - original_size) < 1024  # Less than 1KB difference

    def test_rotation_error_handling_nonexistent_file(self):
        """Test error handling for non-existent file."""
        success, message = self.rotator.rotate_image(
            "/nonexistent/file.jpg", "clockwise"
        )
        assert not success
        assert "not found" in message.lower()

    def test_rotation_error_handling_invalid_direction(self):
        """Test error handling for invalid rotation direction."""
        if self.sample_images:
            temp_image = self._create_temp_copy(self.sample_images[0])
            success, message = self.rotator.rotate_image(
                temp_image, "invalid_direction"
            )
            assert not success

    def test_jpeg_metadata_first_approach(self):
        """Test that JPEG files attempt metadata rotation first."""
        if not any(
            img.lower().endswith((".jpg", ".jpeg")) for img in self.sample_images
        ):
            pytest.skip("No JPEG sample images available")

        # Find a JPEG sample
        jpeg_sample = next(
            (
                img
                for img in self.sample_images
                if img.lower().endswith((".jpg", ".jpeg"))
            ),
            None,
        )
        if not jpeg_sample:
            pytest.skip("No JPEG sample found")

        temp_image = self._create_temp_copy(jpeg_sample)

        # Test the try_metadata_rotation_first method directly
        metadata_success, pixel_available, message = (
            self.rotator.try_metadata_rotation_first(temp_image, "clockwise")
        )

        # Should either succeed with metadata or indicate pixel rotation is available
        assert metadata_success or pixel_available
        assert "metadata" in message.lower() or "pixel" in message.lower()
        logging.info(f"JPEG metadata-first test: {message}")

    def test_jpeg_rotation_method_reporting(self):
        """Test that JPEG rotation reports the correct method used."""
        if not any(
            img.lower().endswith((".jpg", ".jpeg")) for img in self.sample_images
        ):
            pytest.skip("No JPEG sample images available")

        # Find a JPEG sample
        jpeg_sample = next(
            (
                img
                for img in self.sample_images
                if img.lower().endswith((".jpg", ".jpeg"))
            ),
            None,
        )
        if not jpeg_sample:
            pytest.skip("No JPEG sample found")

        temp_image = self._create_temp_copy(jpeg_sample)

        # Perform rotation and check the method reported
        success, message = self.rotator.rotate_clockwise(temp_image)

        assert success
        # Should report either metadata-only (lossless) or quality=95 fallback
        assert "metadata-only" in message.lower() or "quality=95" in message.lower()
        logging.info(f"JPEG rotation method: {message}")

    def test_raw_format_metadata_only(self):
        """Test that RAW formats only use metadata rotation."""
        raw_extensions = [".arw", ".cr2", ".nef", ".dng"]
        raw_sample = None

        for img in self.sample_images:
            if any(img.lower().endswith(ext) for ext in raw_extensions):
                raw_sample = img
                break

        if not raw_sample:
            pytest.skip("No RAW sample images available")

        temp_image = self._create_temp_copy(raw_sample)
        original_size = os.path.getsize(temp_image)

        # Perform rotation on RAW file
        success, message = self.rotator.rotate_clockwise(temp_image)

        # Should succeed and use metadata-only
        assert success
        assert "metadata-only" in message.lower()
        assert "raw format" in message.lower()

        # File size should be nearly identical (only metadata changed)
        new_size = os.path.getsize(temp_image)
        assert abs(new_size - original_size) < 1024  # Less than 1KB difference
        logging.info(f"RAW rotation: {message}")


if __name__ == "__main__":
    # Run tests with verbose output
    pytest.main([__file__, "-v", "-s"])
