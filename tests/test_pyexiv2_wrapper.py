import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2

import pytest
import tempfile
import os
from unittest.mock import patch, MagicMock
from PIL import Image
import threading

from core.pyexiv2_wrapper import (  # noqa: E402
    PyExiv2ImageWrapper,
    safe_pyexiv2_image,
    PyExiv2Operations,
    PyExiv2Error,
    create_safe_image_context,
)


class TestPyExiv2ImageWrapper:
    """Test cases for PyExiv2ImageWrapper class."""

    @pytest.fixture
    def temp_image(self):
        """Create a temporary test image."""
        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as f:
            # Create a simple test image
            img = Image.new("RGB", (100, 100), color="red")
            img.save(f.name, "JPEG")
            yield f.name
        os.unlink(f.name)

    def test_wrapper_context_manager(self, temp_image):
        """Test that wrapper works as a context manager."""
        wrapper = PyExiv2ImageWrapper(temp_image)

        with wrapper as img:
            # Should have access to pyexiv2 methods
            assert hasattr(img, "get_pixel_width")
            assert hasattr(img, "get_pixel_height")
            assert hasattr(img, "read_exif")

    def test_wrapper_error_without_context(self):
        """Test that wrapper raises error when used without context manager."""
        wrapper = PyExiv2ImageWrapper("dummy_path")

        with pytest.raises(PyExiv2Error, match="Image not opened"):
            wrapper.get_pixel_width()

    @patch("core.pyexiv2_wrapper.pyexiv2.Image")
    def test_wrapper_thread_safety(self, mock_pyexiv2_image):
        """Test that wrapper properly handles thread safety."""
        mock_img = MagicMock()
        mock_pyexiv2_image.return_value = mock_img

        wrapper = PyExiv2ImageWrapper("test_path")

        with wrapper:
            # Should acquire and release lock
            pass

        # Should have created pyexiv2.Image
        mock_pyexiv2_image.assert_called_once_with("test_path", encoding="utf-8")
        mock_img.close.assert_called_once()

    def test_wrapper_exception_cleanup(self, temp_image):
        """Test that wrapper properly cleans up even when exceptions occur."""
        with patch("core.pyexiv2_wrapper.pyexiv2.Image") as mock_pyexiv2_image:
            mock_img = MagicMock()
            mock_img.get_pixel_width.side_effect = RuntimeError("Test error")
            mock_pyexiv2_image.return_value = mock_img

            wrapper = PyExiv2ImageWrapper(temp_image)

            with pytest.raises(RuntimeError):
                with wrapper as img:
                    img.get_pixel_width()

            # Should still call close
            mock_img.close.assert_called_once()


class TestSafePyExiv2Image:
    """Test cases for safe_pyexiv2_image context manager."""

    @pytest.fixture
    def temp_image(self):
        """Create a temporary test image."""
        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as f:
            img = Image.new("RGB", (100, 100), color="blue")
            img.save(f.name, "JPEG")
            yield f.name
        os.unlink(f.name)

    def test_safe_context_manager_basic(self, temp_image):
        """Test basic functionality of safe context manager."""
        with safe_pyexiv2_image(temp_image) as img:
            # Should be able to access wrapper methods
            assert hasattr(img, "get_pixel_width")
            assert hasattr(img, "get_pixel_height")

    def test_safe_context_manager_encoding(self):
        """Test that encoding parameter is passed correctly."""
        with patch("core.pyexiv2_wrapper.PyExiv2ImageWrapper") as mock_wrapper:
            mock_instance = MagicMock()
            mock_wrapper.return_value = mock_instance
            mock_instance.__enter__.return_value = mock_instance

            with safe_pyexiv2_image("test_path", encoding="latin-1"):
                pass

            mock_wrapper.assert_called_once_with("test_path", "latin-1")

    def test_create_safe_image_context_alias(self):
        """Test that create_safe_image_context works as an alias for safe_pyexiv2_image."""
        from unittest.mock import patch

        with patch("core.pyexiv2_wrapper.safe_pyexiv2_image") as mock_safe:
            create_safe_image_context("test_path", "utf-8")
            mock_safe.assert_called_once_with("test_path", "utf-8")


class TestPyExiv2Operations:
    """Test cases for PyExiv2Operations class."""

    @pytest.fixture
    def temp_image(self):
        """Create a temporary test image."""
        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as f:
            img = Image.new("RGB", (200, 150), color="green")
            img.save(f.name, "JPEG")
            yield f.name
        os.unlink(f.name)

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_get_comprehensive_metadata(self, mock_safe_image, temp_image):
        """Test comprehensive metadata extraction."""
        # Mock the image wrapper
        mock_img = MagicMock()
        mock_img.get_pixel_width.return_value = 200
        mock_img.get_pixel_height.return_value = 150
        mock_img.get_mime_type.return_value = "image/jpeg"
        mock_img.read_exif.return_value = {"Exif.Image.Make": "Canon"}
        mock_img.read_iptc.return_value = {"Iptc.Application2.Caption": "Test"}
        mock_img.read_xmp.return_value = {"Xmp.dc.title": "Test Title"}

        mock_safe_image.return_value.__enter__.return_value = mock_img

        with patch("os.path.getsize", return_value=12345):
            metadata = PyExiv2Operations.get_comprehensive_metadata(temp_image)

        # Check metadata structure
        assert metadata["file_path"] == temp_image
        assert metadata["pixel_width"] == 200
        assert metadata["pixel_height"] == 150
        assert metadata["mime_type"] == "image/jpeg"
        assert metadata["file_size"] == 12345
        assert metadata["Exif.Image.Make"] == "Canon"
        assert metadata["Iptc.Application2.Caption"] == "Test"
        assert metadata["Xmp.dc.title"] == "Test Title"

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_get_basic_info(self, mock_safe_image, temp_image):
        """Test basic info extraction."""
        mock_img = MagicMock()
        mock_img.get_pixel_width.return_value = 300
        mock_img.get_pixel_height.return_value = 200
        mock_img.get_mime_type.return_value = "image/png"

        mock_safe_image.return_value.__enter__.return_value = mock_img

        with patch("os.path.getsize", return_value=54321):
            info = PyExiv2Operations.get_basic_info(temp_image)

        assert info["file_path"] == temp_image
        assert info["pixel_width"] == 300
        assert info["pixel_height"] == 200
        assert info["mime_type"] == "image/png"
        assert info["file_size"] == 54321

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_get_orientation_with_value(self, mock_safe_image, temp_image):
        """Test orientation extraction when value exists."""
        mock_img = MagicMock()
        mock_img.read_exif.return_value = {"Exif.Image.Orientation": "6"}

        mock_safe_image.return_value.__enter__.return_value = mock_img

        orientation = PyExiv2Operations.get_orientation(temp_image)
        assert orientation == 6

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_get_orientation_default(self, mock_safe_image, temp_image):
        """Test orientation extraction when no value exists."""
        mock_img = MagicMock()
        mock_img.read_exif.return_value = {}

        mock_safe_image.return_value.__enter__.return_value = mock_img

        orientation = PyExiv2Operations.get_orientation(temp_image)
        assert orientation == 1  # Default

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_set_orientation_success(self, mock_safe_image, temp_image):
        """Test setting orientation successfully."""
        mock_img = MagicMock()
        mock_safe_image.return_value.__enter__.return_value = mock_img

        result = PyExiv2Operations.set_orientation(temp_image, 8)

        assert result is True
        mock_img.modify_exif.assert_called_once_with({"Exif.Image.Orientation": "8"})

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_set_orientation_failure(self, mock_safe_image, temp_image):
        """Test setting orientation failure."""
        mock_img = MagicMock()
        mock_img.modify_exif.side_effect = RuntimeError("Write error")
        mock_safe_image.return_value.__enter__.return_value = mock_img

        result = PyExiv2Operations.set_orientation(temp_image, 6)

        assert result is False

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_get_rating_from_exif(self, mock_safe_image, temp_image):
        """Test rating extraction from EXIF."""
        mock_img = MagicMock()
        mock_img.read_exif.return_value = {"Exif.Image.Rating": "5"}
        mock_img.read_xmp.return_value = {}

        mock_safe_image.return_value.__enter__.return_value = mock_img

        rating = PyExiv2Operations.get_rating(temp_image)
        assert rating == 5

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_get_rating_from_xmp(self, mock_safe_image, temp_image):
        """Test rating extraction from XMP."""
        mock_img = MagicMock()
        mock_img.read_exif.return_value = {}
        mock_img.read_xmp.return_value = {"Xmp.xmp.Rating": "3"}

        mock_safe_image.return_value.__enter__.return_value = mock_img

        rating = PyExiv2Operations.get_rating(temp_image)
        assert rating == 3

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_get_rating_none(self, mock_safe_image, temp_image):
        """Test rating extraction when no rating exists."""
        mock_img = MagicMock()
        mock_img.read_exif.return_value = {}
        mock_img.read_xmp.return_value = {}

        mock_safe_image.return_value.__enter__.return_value = mock_img

        rating = PyExiv2Operations.get_rating(temp_image)
        assert rating is None

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_set_rating_success(self, mock_safe_image, temp_image):
        """Test setting rating successfully."""
        mock_img = MagicMock()
        mock_safe_image.return_value.__enter__.return_value = mock_img

        result = PyExiv2Operations.set_rating(temp_image, 4)

        assert result is True
        mock_img.modify_exif.assert_called_once_with({"Exif.Image.Rating": "4"})
        mock_img.modify_xmp.assert_called_once_with({"Xmp.xmp.Rating": "4"})

    def test_set_rating_invalid_value(self, temp_image):
        """Test setting invalid rating value."""
        with pytest.raises(ValueError, match="Rating must be between 0 and 5"):
            PyExiv2Operations.set_rating(temp_image, 6)

        with pytest.raises(ValueError, match="Rating must be between 0 and 5"):
            PyExiv2Operations.set_rating(temp_image, -1)

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_set_rating_failure(self, mock_safe_image, temp_image):
        """Test setting rating failure."""
        mock_img = MagicMock()
        mock_img.modify_exif.side_effect = RuntimeError("Write error")
        mock_safe_image.return_value.__enter__.return_value = mock_img

        result = PyExiv2Operations.set_rating(temp_image, 3)

        assert result is False

    @patch("core.pyexiv2_wrapper.PyExiv2Operations.get_comprehensive_metadata")
    def test_batch_get_metadata_success(self, mock_get_metadata):
        """Test batch metadata extraction with all successes."""
        mock_get_metadata.side_effect = [
            {"file_path": "image1.jpg", "pixel_width": 100},
            {"file_path": "image2.jpg", "pixel_width": 200},
        ]

        results = PyExiv2Operations.batch_get_metadata(["image1.jpg", "image2.jpg"])

        assert len(results) == 2
        assert results[0]["file_path"] == "image1.jpg"
        assert results[1]["file_path"] == "image2.jpg"

    @patch("core.pyexiv2_wrapper.PyExiv2Operations.get_comprehensive_metadata")
    def test_batch_get_metadata_with_errors(self, mock_get_metadata):
        """Test batch metadata extraction with some errors."""
        mock_get_metadata.side_effect = [
            {"file_path": "image1.jpg", "pixel_width": 100},
            PyExiv2Error("Failed to read"),
        ]

        results = PyExiv2Operations.batch_get_metadata(["image1.jpg", "image2.jpg"])

        assert len(results) == 2
        assert results[0]["file_path"] == "image1.jpg"
        assert results[1]["file_path"] == "image2.jpg"
        assert results[1]["error"] == "Metadata extraction failed"


class TestPyExiv2Error:
    """Test cases for PyExiv2Error exception."""

    def test_pyexiv2_error_creation(self):
        """Test that PyExiv2Error can be created and raised."""
        error = PyExiv2Error("Test error message")
        assert str(error) == "Test error message"

    def test_pyexiv2_error_inheritance(self):
        """Test that PyExiv2Error inherits from Exception."""
        error = PyExiv2Error("Test")
        assert isinstance(error, Exception)


class TestThreadSafety:
    """Test thread safety of the wrapper."""

    @pytest.fixture
    def temp_image(self):
        """Create a temporary test image."""
        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as f:
            img = Image.new("RGB", (100, 100), color="red")
            img.save(f.name, "JPEG")
            yield f.name
        os.unlink(f.name)

    def test_concurrent_operations(self, temp_image):
        """Test that multiple threads can safely use the wrapper."""
        results = []
        errors = []

        def worker():
            try:
                with safe_pyexiv2_image(temp_image) as img:
                    # Simulate some work
                    width = img.get_pixel_width()
                    height = img.get_pixel_height()
                    results.append((width, height))
            except Exception as e:
                errors.append(e)

        # Create multiple threads
        threads = []
        for _ in range(5):
            thread = threading.Thread(target=worker)
            threads.append(thread)
            thread.start()

        # Wait for all threads to complete
        for thread in threads:
            thread.join()

        # All operations should succeed
        assert len(results) == 5
        assert len(errors) == 0
        # All should have the same dimensions
        for width, height in results:
            assert width == 100
            assert height == 100


class TestErrorHandling:
    """Test error handling in the wrapper."""

    def test_missing_file_error(self):
        """Test handling of missing file."""
        with pytest.raises(PyExiv2Error):
            PyExiv2Operations.get_comprehensive_metadata("/nonexistent/file.jpg")

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_pyexiv2_runtime_error(self, mock_safe_image):
        """Test handling of PyExiv2 runtime errors."""
        mock_safe_image.side_effect = RuntimeError("PyExiv2 error")

        with pytest.raises(PyExiv2Error):
            PyExiv2Operations.get_basic_info("test.jpg")

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_get_orientation_with_error(self, mock_safe_image):
        """Test orientation handling when error occurs."""
        mock_safe_image.side_effect = RuntimeError("Read error")

        # Should return default orientation on error
        orientation = PyExiv2Operations.get_orientation("test.jpg")
        assert orientation == 1

    @patch("core.pyexiv2_wrapper.safe_pyexiv2_image")
    def test_get_rating_with_error(self, mock_safe_image):
        """Test rating handling when error occurs."""
        mock_safe_image.side_effect = RuntimeError("Read error")

        # Should return None on error
        rating = PyExiv2Operations.get_rating("test.jpg")
        assert rating is None
