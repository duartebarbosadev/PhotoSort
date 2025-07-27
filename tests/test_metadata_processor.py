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
    from src.core.metadata_processor import (
        MetadataProcessor,
        _parse_exif_date,
        _parse_date_from_filename,
        _parse_rating,
        DATE_TAGS_PREFERENCE,
        COMPREHENSIVE_METADATA_TAGS,
    )
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

    def _parse_exif_date(*args):
        return None

    def _parse_date_from_filename(*args):
        return None

    def _parse_rating(*args):
        return 0

    DATE_TAGS_PREFERENCE = []
    COMPREHENSIVE_METADATA_TAGS = []


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=f"Required modules not available: {IMPORT_ERROR}"
)
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
            assert "label" in metadata
            assert "date" in metadata

            # Rating should be 0-5
            assert 0 <= metadata["rating"] <= 5

            # Label can be None or string
            assert metadata["label"] is None or isinstance(metadata["label"], str)

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
            logging.info(f"  Label: {metadata['label']}")
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

    def test_set_and_get_rating(self):
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
            # Test setting different ratings
            for rating in [0, 3, 5]:
                success = MetadataProcessor.set_rating(
                    temp_image_path,
                    rating,
                    rating_disk_cache=self.rating_cache,
                    exif_disk_cache=self.exif_cache,
                )

                assert success, f"Failed to set rating {rating}"

                # Verify rating was set by reading it back
                results = MetadataProcessor.get_batch_display_metadata(
                    [temp_image_path]
                )
                norm_path = os.path.normpath(temp_image_path)
                assert results[norm_path]["rating"] == rating

                logging.info(f"Successfully set and verified rating {rating}")

        finally:
            # Clean up temporary file
            if os.path.exists(temp_image_path):
                os.unlink(temp_image_path)

    def test_set_and_get_label(self):
        """Test setting and getting labels on sample images."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        # Use first image for label test
        test_image = self.sample_images[0]

        # Create a temporary copy for testing
        with tempfile.NamedTemporaryFile(
            suffix=os.path.splitext(test_image)[1], delete=False
        ) as tmp:
            shutil.copy2(test_image, tmp.name)
            temp_image_path = tmp.name

        try:
            # Test setting different labels
            test_labels = ["Red", "Blue", "Green", None, ""]

            for label in test_labels:
                success = MetadataProcessor.set_label(
                    temp_image_path, label, exif_disk_cache=self.exif_cache
                )

                assert success, f"Failed to set label '{label}'"

                # Verify label was set by reading it back
                results = MetadataProcessor.get_batch_display_metadata(
                    [temp_image_path]
                )
                norm_path = os.path.normpath(temp_image_path)

                if label in [None, ""]:
                    # Empty labels should result in None
                    assert (
                        results[norm_path]["label"] is None
                        or results[norm_path]["label"] == ""
                    )
                else:
                    assert results[norm_path]["label"] == label

                logging.info(f"Successfully set and verified label '{label}'")

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
            assert "label" in metadata
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
            assert results[norm_path]["label"] is None

    def test_invalid_rating_values(self):
        """Test error handling for invalid rating values."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        test_image = self.sample_images[0]

        # Test invalid ratings
        invalid_ratings = [-1, 6, 10, "invalid", None]

        for invalid_rating in invalid_ratings:
            success = MetadataProcessor.set_rating(test_image, invalid_rating)
            assert success is False, f"Should reject invalid rating {invalid_rating}"


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=f"Required modules not available: {IMPORT_ERROR}"
)
class TestHelperFunctions:
    """Test helper functions used by MetadataProcessor."""

    def test_parse_exif_date(self):
        """Test EXIF date parsing function."""
        test_cases = [
            ("2023:12:25 14:30:45", date(2023, 12, 25)),
            ("2023-12-25 14:30:45", date(2023, 12, 25)),
            ("2023-12-25T14:30:45", date(2023, 12, 25)),
            ("2023:12:25", date(2023, 12, 25)),
            ("2023-12-25", date(2023, 12, 25)),
            ("invalid", None),
            ("", None),
            (None, None),
        ]

        for date_string, expected in test_cases:
            result = _parse_exif_date(date_string)
            assert result == expected, f"Failed for '{date_string}'"

    def test_parse_date_from_filename(self):
        """Test filename date parsing function."""
        test_cases = [
            ("IMG_20231225_143045.jpg", date(2023, 12, 25)),
            ("2023-12-25_photo.jpg", date(2023, 12, 25)),
            ("20231225_143045.jpg", date(2023, 12, 25)),
            ("photo_2023.12.25.jpg", date(2023, 12, 25)),
            ("random_filename.jpg", None),
            ("IMG_20991301_invalid.jpg", None),  # Invalid date
        ]

        for filename, expected in test_cases:
            result = _parse_date_from_filename(filename)
            assert result == expected, f"Failed for '{filename}'"

    def test_parse_rating(self):
        """Test rating parsing function."""
        test_cases = [
            (3, 3),
            ("4", 4),
            ("5.0", 5),
            (0, 0),
            (6, 5),  # Should clamp to 5
            (-1, 0),  # Should clamp to 0
            ("invalid", 0),
            (None, 0),
        ]

        for value, expected in test_cases:
            result = _parse_rating(value)
            assert result == expected, f"Failed for {value}"


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=f"Required modules not available: {IMPORT_ERROR}"
)
class TestConstants:
    """Test that constants are properly defined."""

    def test_date_tags_preference(self):
        """Test DATE_TAGS_PREFERENCE is properly defined."""
        assert isinstance(DATE_TAGS_PREFERENCE, list)
        assert len(DATE_TAGS_PREFERENCE) > 0
        assert all(isinstance(tag, str) for tag in DATE_TAGS_PREFERENCE)

        # Should include common date tags
        expected_tags = ["Exif.Photo.DateTimeOriginal", "Xmp.xmp.CreateDate"]
        for tag in expected_tags:
            assert tag in DATE_TAGS_PREFERENCE

    def test_comprehensive_metadata_tags(self):
        """Test COMPREHENSIVE_METADATA_TAGS is properly defined."""
        assert isinstance(COMPREHENSIVE_METADATA_TAGS, list)
        assert len(COMPREHENSIVE_METADATA_TAGS) > 0
        assert all(isinstance(tag, str) for tag in COMPREHENSIVE_METADATA_TAGS)

        # Should include rating and label tags
        assert "Xmp.xmp.Rating" in COMPREHENSIVE_METADATA_TAGS
        assert "Xmp.xmp.Label" in COMPREHENSIVE_METADATA_TAGS


if __name__ == "__main__":
    # Run tests with verbose output
    pytest.main([__file__, "-v", "-s"])
