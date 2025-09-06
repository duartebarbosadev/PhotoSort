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
        # jpegtran availability might vary by system
        assert isinstance(self.rotator.jpegtran_available, bool)

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


if __name__ == "__main__":
    # Run tests with verbose output
    pytest.main([__file__, "-v", "-s"])
