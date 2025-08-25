import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2
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
    from src.core.metadata_processor import MetadataProcessor
    from src.core.caching.exif_cache import ExifCache

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)

    # Create dummy classes to avoid NameError
    class MetadataProcessor:
        pass

    class ExifCache:
        pass


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

    def test_is_rotation_supported_for_sample_images(self):
        """Test MetadataProcessor.is_rotation_supported method for sample images."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        for image_path in self.sample_images:
            supported = MetadataProcessor.is_rotation_supported(image_path)
            assert isinstance(supported, bool)

            # Check based on file extension
            ext = os.path.splitext(image_path)[1].lower()
            if ext in [".jpg", ".jpeg", ".png", ".tiff", ".tif"]:
                assert supported
            logging.info(
                f"Rotation support for {os.path.basename(image_path)} ({ext}): {supported}"
            )

    def test_rotate_clockwise_with_cache_invalidation(self):
        """Test MetadataProcessor.rotate_clockwise method with cache invalidation."""
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

    def test_rotate_counterclockwise_with_cache_invalidation(self):
        """Test MetadataProcessor.rotate_counterclockwise method with cache invalidation."""
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

    def test_rotate_180_with_cache_invalidation(self):
        """Test MetadataProcessor.rotate_180 method with cache invalidation."""
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

    def test_rotation_with_metadata_only_flag(self):
        """Test rotation with metadata_only flag and cache invalidation."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        # Get original file info
        original_size = os.path.getsize(temp_image)

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

    def test_error_handling_nonexistent_file(self):
        """Test error handling for non-existent file."""
        success = MetadataProcessor.rotate_clockwise("/nonexistent/file.jpg")
        assert not success

    def test_error_handling_invalid_path(self):
        """Test error handling for invalid path."""
        success = MetadataProcessor.rotate_counterclockwise("")
        assert not success


if __name__ == "__main__":
    # Run tests with verbose output
    pytest.main([__file__, "-v", "-s"])