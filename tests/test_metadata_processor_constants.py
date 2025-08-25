import pyexiv2  # noqa: F401  # This must be the first import or else it will cause a silent crash on windows
import pytest
import os
import sys

# Add the project root to Python path so we can import src modules
project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if project_root not in sys.path:
    sys.path.insert(0, project_root)

# Try to import the modules we need
try:
    from src.core.metadata_processor import (
        DATE_TAGS_PREFERENCE,
        COMPREHENSIVE_METADATA_TAGS,
    )

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)

    # Create dummy constants to avoid NameError
    DATE_TAGS_PREFERENCE = []
    COMPREHENSIVE_METADATA_TAGS = []


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
