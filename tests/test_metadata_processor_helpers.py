import pyexiv2  # noqa: F401  # This must be the first import or else it will cause a silent crash on windows
import pytest
import os
import sys
from datetime import date

# Add the project root to Python path so we can import src modules
project_root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if project_root not in sys.path:
    sys.path.insert(0, project_root)

# Try to import the modules we need
try:
    from src.core.metadata_processor import (
        _parse_exif_date,
        _parse_date_from_filename,
        _parse_rating,
    )

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)

    # Create dummy functions to avoid NameError
    def _parse_exif_date(*args):
        return None

    def _parse_date_from_filename(*args):
        return None

    def _parse_rating(*args):
        return 0


class TestHelperFunctions:
    """Test helper functions used by MetadataProcessor."""

    @pytest.mark.parametrize(
        "date_string,expected",
        [
            ("2023:12:25 14:30:45", date(2023, 12, 25)),
            ("2023-12-25 14:30:45", date(2023, 12, 25)),
            ("2023-12-25T14:30:45", date(2023, 12, 25)),
            ("2023:12:25", date(2023, 12, 25)),
            ("2023-12-25", date(2023, 12, 25)),
            ("invalid", None),
            ("", None),
            (None, None),
        ],
    )
    def test_parse_exif_date(self, date_string, expected):
        """Test EXIF date parsing function."""
        result = _parse_exif_date(date_string)
        assert result == expected, f"Failed for '{date_string}'"

    @pytest.mark.parametrize(
        "filename,expected",
        [
            ("IMG_20231225_143045.jpg", date(2023, 12, 25)),
            ("2023-12-25_photo.jpg", date(2023, 12, 25)),
            ("20231225_143045.jpg", date(2023, 12, 25)),
            ("photo_2023.12.25.jpg", date(2023, 12, 25)),
            ("random_filename.jpg", None),
            ("IMG_20991301_invalid.jpg", None),  # Invalid date
        ],
    )
    def test_parse_date_from_filename(self, filename, expected):
        """Test filename date parsing function."""
        result = _parse_date_from_filename(filename)
        assert result == expected, f"Failed for '{filename}'"

    @pytest.mark.parametrize(
        "value,expected",
        [
            (3, 3),
            ("4", 4),
            ("5.0", 5),
            (0, 0),
            (6, 5),  # Should clamp to 5
            (-1, 0),  # Should clamp to 0
            ("invalid", 0),
            (None, 0),
        ],
    )
    def test_parse_rating(self, value, expected):
        """Test rating parsing function."""
        result = _parse_rating(value)
        assert result == expected, f"Failed for {value}"


if __name__ == "__main__":
    # Run tests with verbose output
    pytest.main([__file__, "-v", "-s"])
