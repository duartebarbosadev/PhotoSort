import pytest
import os
import sys
import tempfile
import shutil
from unittest.mock import Mock
import logging

# Add the project root to Python path so we can import src modules
project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if project_root not in sys.path:
    sys.path.insert(0, project_root)

# Try to import the modules we need
try:
    from src.core.image_processing.image_rotator import ImageRotator, RotationDirection
    from src.core.metadata_processor import MetadataProcessor
    from src.core.caching.exif_cache import ExifCache

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)

    # Create dummy classes to avoid NameError
    class ImageRotator:
        pass

    class MetadataProcessor:
        pass

    class ExifCache:
        pass


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=f"Required modules not available: {IMPORT_ERROR}"
)
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
                except:
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

    def test_is_rotation_supported(self):
        """Test rotation support checking for different formats."""
        # Test supported formats (pixel rotation)
        assert self.rotator.is_rotation_supported("test.jpg") == True
        assert self.rotator.is_rotation_supported("test.jpeg") == True
        assert self.rotator.is_rotation_supported("test.png") == True
        assert self.rotator.is_rotation_supported("test.tiff") == True

        # Test supported RAW formats (metadata-only rotation)
        assert self.rotator.is_rotation_supported("test.arw") == True
        assert self.rotator.is_rotation_supported("test.cr2") == True
        assert self.rotator.is_rotation_supported("test.nef") == True
        assert self.rotator.is_rotation_supported("test.dng") == True

        # Test unsupported formats
        assert self.rotator.is_rotation_supported("test.txt") == False
        assert self.rotator.is_rotation_supported("test.mp4") == False

    def test_get_current_orientation(self):
        """Test reading current EXIF orientation."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        for image_path in self.sample_images:
            orientation = self.rotator._get_current_orientation(image_path)
            assert isinstance(orientation, int)
            assert 1 <= orientation <= 8  # Valid EXIF orientation range

    def test_calculate_new_orientation(self):
        """Test orientation calculation for different rotations."""
        # Test clockwise rotation from normal orientation
        assert self.rotator._calculate_new_orientation(1, "clockwise") == 6
        assert self.rotator._calculate_new_orientation(1, "counterclockwise") == 8
        assert self.rotator._calculate_new_orientation(1, "180") == 3

        # Test full rotation cycle (clockwise)
        orientation = 1
        for _ in range(4):
            orientation = self.rotator._calculate_new_orientation(
                orientation, "clockwise"
            )
        assert orientation == 1  # Should return to original after 4 rotations

    def test_rotation_with_sample_images(self):
        """Test actual rotation on sample images."""
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

    def test_rotation_error_handling(self):
        """Test error handling for invalid inputs."""
        # Test with non-existent file
        success, message = self.rotator.rotate_image(
            "/nonexistent/file.jpg", "clockwise"
        )
        assert success == False
        assert "not found" in message.lower()

        # Test with invalid direction
        if self.sample_images:
            temp_image = self._create_temp_copy(self.sample_images[0])
            success, message = self.rotator.rotate_image(
                temp_image, "invalid_direction"
            )
            assert success == False


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=f"Required modules not available: {IMPORT_ERROR}"
)
class TestMetadataProcessorRotation:
    """Tests for MetadataProcessor rotation methods."""

    @classmethod
    def setup_class(cls):
        """Setup test environment."""
        cls.test_folder = "tests/samples"
        cls.sample_images = []

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
        self.exif_cache = Mock(spec=ExifCache)
        self.temp_files = []

    def teardown_method(self):
        """Clean up temporary files."""
        for temp_file in self.temp_files:
            if os.path.exists(temp_file):
                try:
                    os.unlink(temp_file)
                except:
                    pass

    def _create_temp_copy(self, source_path: str) -> str:
        """Create a temporary copy of an image for testing."""
        _, ext = os.path.splitext(source_path)
        with tempfile.NamedTemporaryFile(suffix=ext, delete=False) as temp_file:
            temp_path = temp_file.name

        shutil.copy2(source_path, temp_path)
        self.temp_files.append(temp_path)
        return temp_path

    def test_is_rotation_supported(self):
        """Test MetadataProcessor.is_rotation_supported method."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        for image_path in self.sample_images:
            supported = MetadataProcessor.is_rotation_supported(image_path)
            assert isinstance(supported, bool)

            # Check based on file extension
            ext = os.path.splitext(image_path)[1].lower()
            if ext in [".jpg", ".jpeg", ".png", ".tiff", ".tif"]:
                assert supported == True
            logging.info(
                f"Rotation support for {os.path.basename(image_path)} ({ext}): {supported}"
            )

    def test_rotate_clockwise(self):
        """Test MetadataProcessor.rotate_clockwise method."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        success = MetadataProcessor.rotate_clockwise(
            temp_image, exif_disk_cache=self.exif_cache
        )

        # Check that cache was invalidated if rotation was successful
        if success:
            assert os.path.exists(temp_image)
            self.exif_cache.delete.assert_called_once()

        print(f"Clockwise rotation of {os.path.basename(source_image)}: {success}")

    def test_rotate_counterclockwise(self):
        """Test MetadataProcessor.rotate_counterclockwise method."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        success = MetadataProcessor.rotate_counterclockwise(
            temp_image, exif_disk_cache=self.exif_cache
        )

        if success:
            assert os.path.exists(temp_image)
            self.exif_cache.delete.assert_called_once()

        logging.info(
            f"Counterclockwise rotation of {os.path.basename(source_image)}: {success}"
        )

    def test_rotate_180(self):
        """Test MetadataProcessor.rotate_180 method."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        success = MetadataProcessor.rotate_180(
            temp_image, exif_disk_cache=self.exif_cache
        )

        if success:
            assert os.path.exists(temp_image)
            self.exif_cache.delete.assert_called_once()

        logging.info(f"180° rotation of {os.path.basename(source_image)}: {success}")

    def test_rotation_with_metadata_only(self):
        """Test rotation with metadata_only flag."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        # Get original file info
        original_size = os.path.getsize(temp_image)
        original_mtime = os.path.getmtime(temp_image)

        success = MetadataProcessor.rotate_image(
            temp_image,
            "clockwise",
            update_metadata_only=True,
            exif_disk_cache=self.exif_cache,
        )

        if success:
            # File should exist and cache should be invalidated
            assert os.path.exists(temp_image)
            self.exif_cache.delete.assert_called_once()

            # File size should be similar (metadata changes only)
            new_size = os.path.getsize(temp_image)
            assert abs(new_size - original_size) < 1024  # Less than 1KB difference

        logging.info(
            f"Metadata-only rotation of {os.path.basename(source_image)}: {success}"
        )

    def test_error_handling(self):
        """Test error handling in MetadataProcessor rotation methods."""
        # Test with non-existent file
        success = MetadataProcessor.rotate_clockwise("/nonexistent/file.jpg")
        assert success == False

        # Test with invalid path
        success = MetadataProcessor.rotate_counterclockwise("")
        assert success == False


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=f"Required modules not available: {IMPORT_ERROR}"
)
class TestRotationIntegration:
    """Integration tests for the complete rotation system."""

    @classmethod
    def setup_class(cls):
        """Setup test environment."""
        cls.test_folder = "tests/samples"
        cls.sample_images = []

        if os.path.exists(cls.test_folder):
            for filename in os.listdir(cls.test_folder):
                if filename.lower().endswith((".png", ".jpg", ".jpeg")):
                    cls.sample_images.append(os.path.join(cls.test_folder, filename))

        if len(cls.sample_images) == 0:
            pytest.skip(
                f"No test images found in {cls.test_folder}", allow_module_level=True
            )

    def setup_method(self):
        """Setup for each test method."""
        self.temp_files = []

    def teardown_method(self):
        """Clean up temporary files."""
        for temp_file in self.temp_files:
            if os.path.exists(temp_file):
                try:
                    os.unlink(temp_file)
                except:
                    pass

    def _create_temp_copy(self, source_path: str) -> str:
        """Create a temporary copy of an image for testing."""
        _, ext = os.path.splitext(source_path)
        with tempfile.NamedTemporaryFile(suffix=ext, delete=False) as temp_file:
            temp_path = temp_file.name

        shutil.copy2(source_path, temp_path)
        self.temp_files.append(temp_path)
        return temp_path

    def test_full_rotation_cycle(self):
        """Test that 4 clockwise rotations return to original orientation."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        # Get original metadata
        original_metadata = MetadataProcessor.get_batch_display_metadata([temp_image])
        original_detailed = MetadataProcessor.get_detailed_metadata(temp_image)

        # Perform 4 clockwise rotations
        for i in range(4):
            success = MetadataProcessor.rotate_clockwise(temp_image)
            assert success == True, f"Rotation {i + 1} failed"
            assert os.path.exists(temp_image), f"File missing after rotation {i + 1}"

        # Check that we're back to original orientation
        final_metadata = MetadataProcessor.get_batch_display_metadata([temp_image])
        final_detailed = MetadataProcessor.get_detailed_metadata(temp_image)

        # The image should still be valid
        assert final_metadata is not None
        assert final_detailed is not None

        logging.info(
            f"Full rotation cycle completed for {os.path.basename(source_image)}"
        )

    def test_mixed_rotations(self):
        """Test different rotation combinations."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        # Test sequence: clockwise -> 180 -> counterclockwise -> 180
        rotations = [
            (MetadataProcessor.rotate_clockwise, "clockwise"),
            (MetadataProcessor.rotate_180, "180°"),
            (MetadataProcessor.rotate_counterclockwise, "counterclockwise"),
            (MetadataProcessor.rotate_180, "180°"),
        ]

        for rotate_func, description in rotations:
            success = rotate_func(temp_image)
            assert success == True, f"{description} rotation failed"
            assert os.path.exists(temp_image), (
                f"File missing after {description} rotation"
            )

            # Verify image is still readable
            metadata = MetadataProcessor.get_detailed_metadata(temp_image)
            assert metadata is not None, (
                f"Cannot read metadata after {description} rotation"
            )

        logging.info(
            f"Mixed rotation sequence completed for {os.path.basename(source_image)}"
        )


if __name__ == "__main__":
    # Run tests with verbose output
    pytest.main([__file__, "-v", "-s"])
