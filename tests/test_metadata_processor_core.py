import pyexiv2  # noqa: F401  # This must be the first import or else it will cause a silent crash on windows
import pytest
import os
import sys
import tempfile
import shutil
import unicodedata
from datetime import date
from unittest.mock import Mock
import logging

# Add the project root to Python path so we can import src modules
project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if project_root not in sys.path:
    sys.path.insert(0, project_root)

# Try to import the modules we need
try:
    from src.core.metadata_processor import MetadataProcessor
    from src.core.caching.rating_cache import RatingCache
    from src.core.caching.exif_cache import ExifCache

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)

    # Create dummy classes to avoid NameError
    class MetadataProcessor:
        pass

    class RatingCache:
        pass

    class ExifCache:
        pass


class TestMetadataProcessor:
    """Comprehensive tests for MetadataProcessor using sample images."""

    @classmethod
    def setup_class(cls):
        """Setup test environment with sample images."""
        cls.test_folder = "tests/samples"
        cls.sample_images = []

        # Find all images in the test folder
        if os.path.exists(cls.test_folder):
            for filename in os.listdir(cls.test_folder):
                if filename.lower().endswith(
                    (".png", ".jpg", ".jpeg", ".arw", ".cr2", ".nef")
                ):
                    cls.sample_images.append(os.path.join(cls.test_folder, filename))

        # Ensure we have the expected test files
        expected_extensions = [".png", ".jpg", ".arw"]
        found_extensions = [
            os.path.splitext(img)[1].lower() for img in cls.sample_images
        ]

        logging.info(f"Found test images: {cls.sample_images}")
        logging.info(f"Expected extensions: {expected_extensions}")
        logging.info(f"Found extensions: {found_extensions}")

        if len(cls.sample_images) == 0:
            pytest.skip(
                f"No test images found in {cls.test_folder}", allow_module_level=True
            )

    def setup_method(self):
        """Setup for each test method."""
        # Create mock caches
        self.rating_cache = Mock(spec=RatingCache)
        self.exif_cache = Mock(spec=ExifCache)

        # Configure cache mocks to return None (cache miss) by default
        self.rating_cache.get.return_value = None
        self.exif_cache.get.return_value = None

    def test_batch_display_metadata_basic(self):
        """Test basic batch metadata extraction."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        results = MetadataProcessor.get_batch_display_metadata(
            self.sample_images,
            rating_disk_cache=self.rating_cache,
            exif_disk_cache=self.exif_cache,
        )

        # Verify results structure
        assert isinstance(results, dict)
        assert len(results) == len(self.sample_images)

        for image_path in self.sample_images:
            norm_path = unicodedata.normalize("NFC", os.path.normpath(image_path))
            assert norm_path in results

            metadata = results[norm_path]
            assert "rating" in metadata
            assert "date" in metadata

            # Rating should be 0-5
            assert 0 <= metadata["rating"] <= 5

            # Date can be None or date object
            assert metadata["date"] is None or isinstance(metadata["date"], date)

    def test_batch_metadata_different_file_types(self):
        """Test metadata extraction on different file types (PNG, JPG, ARW)."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        results = MetadataProcessor.get_batch_display_metadata(self.sample_images)

        # Group results by file extension
        by_extension = {}
        for image_path in self.sample_images:
            ext = os.path.splitext(image_path)[1].lower()
            norm_path = unicodedata.normalize("NFC", os.path.normpath(image_path))
            by_extension[ext] = results[norm_path]

        # Test that each file type returns valid metadata
        for ext, metadata in by_extension.items():
            logging.info(f"Testing {ext} file metadata:")
            logging.info(f"  Rating: {metadata['rating']}")
            logging.info(f"  Date: {metadata['date']}")

            assert isinstance(metadata["rating"], int)
            assert 0 <= metadata["rating"] <= 5

    def test_detailed_metadata_extraction(self):
        """Test detailed metadata extraction for each sample image."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        for image_path in self.sample_images:
            logging.info(
                f"\nTesting detailed metadata for: {os.path.basename(image_path)}"
            )

            metadata = MetadataProcessor.get_detailed_metadata(
                image_path, exif_disk_cache=self.exif_cache
            )

            assert metadata is not None
            assert isinstance(metadata, dict)

            # Should have basic file info
            assert "file_path" in metadata
            assert metadata["file_path"] == unicodedata.normalize(
                "NFC", os.path.normpath(image_path)
            )

            # Print some metadata for inspection
            logging.info(f"  File size: {metadata.get('file_size', 'Unknown')}")
            logging.info(
                f"  Dimensions: {metadata.get('pixel_width', '?')}x{metadata.get('pixel_height', '?')}"
            )
            logging.info(f"  MIME type: {metadata.get('mime_type', 'Unknown')}")

            # Check for common EXIF fields if present
            common_fields = [
                "Exif.Image.Make",
                "Exif.Image.Model",
                "Exif.Photo.DateTimeOriginal",
            ]
            found_fields = [field for field in common_fields if field in metadata]
            logging.info(f"  Common EXIF fields found: {found_fields}")

    @pytest.mark.parametrize("rating_value", [0, 3, 5])
    def test_set_and_get_rating(self, rating_value):
        """Test setting and getting ratings on sample images."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        # Use first image for rating test
        test_image = self.sample_images[0]

        # Create a temporary copy for testing
        with tempfile.NamedTemporaryFile(
            suffix=os.path.splitext(test_image)[1], delete=False
        ) as tmp:
            shutil.copy2(test_image, tmp.name)
            temp_image_path = tmp.name

        try:
            success = MetadataProcessor.set_rating(
                temp_image_path,
                rating_value,
                rating_disk_cache=self.rating_cache,
                exif_disk_cache=self.exif_cache,
            )

            assert success, f"Failed to set rating {rating_value}"

            # Verify rating was set by reading it back
            results = MetadataProcessor.get_batch_display_metadata(
                [temp_image_path]
            )
            norm_path = os.path.normpath(temp_image_path)
            assert results[norm_path]["rating"] == rating_value

            logging.info(f"Successfully set and verified rating {rating_value}")

        finally:
            # Clean up temporary file
            if os.path.exists(temp_image_path):
                os.unlink(temp_image_path)

    def test_caching_integration(self):
        """Test that caching works correctly."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        # First call - should populate cache
        results1 = MetadataProcessor.get_batch_display_metadata(
            self.sample_images,
            rating_disk_cache=self.rating_cache,
            exif_disk_cache=self.exif_cache,
        )

        # Verify cache.set was called for each image
        assert self.exif_cache.set.call_count == len(self.sample_images)

        # Configure cache to return data (simulate cache hit)
        def cache_side_effect(path):
            norm_path = os.path.normpath(path)
            if norm_path in results1:
                return {"file_path": norm_path, "cached": True}
            return None

        self.exif_cache.get.side_effect = cache_side_effect

        # Second call - should use cache
        MetadataProcessor.get_batch_display_metadata(
            self.sample_images,
            rating_disk_cache=self.rating_cache,
            exif_disk_cache=self.exif_cache,
        )

        # Verify cache.get was called
        assert self.exif_cache.get.call_count >= len(self.sample_images)

    def test_parallel_processing(self):
        """Test that parallel processing works with multiple images."""
        if len(self.sample_images) < 2:
            pytest.skip("Need at least 2 sample images for parallel processing test")

        # Test with all sample images
        results = MetadataProcessor.get_batch_display_metadata(
            self.sample_images * 3,  # Duplicate list to test chunking
            rating_disk_cache=self.rating_cache,
            exif_disk_cache=self.exif_cache,
        )

        # Should handle duplicates correctly
        assert len(results) == len(self.sample_images)

        # All results should be valid
        for norm_path, metadata in results.items():
            assert "rating" in metadata
            assert "date" in metadata

    def test_error_handling_nonexistent_file(self):
        """Test error handling for nonexistent files."""
        fake_paths = ["/nonexistent/file1.jpg", "/fake/path/file2.png"]

        results = MetadataProcessor.get_batch_display_metadata(fake_paths)

        assert len(results) == len(fake_paths)
        for fake_path in fake_paths:
            norm_path = os.path.normpath(fake_path)
            assert norm_path in results
            # Should still return valid structure with defaults
            assert results[norm_path]["rating"] == 0

    @pytest.mark.parametrize("invalid_rating", [-1, 6, 10, "invalid", None])
    def test_invalid_rating_values(self, invalid_rating):
        """Test error handling for invalid rating values."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        test_image = self.sample_images[0]

        success = MetadataProcessor.set_rating(test_image, invalid_rating)
        assert success is False, f"Should reject invalid rating {invalid_rating}"


if __name__ == "__main__":
    # Run tests with verbose output
    pytest.main([__file__, "-v", "-s"])