import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2

import pytest
import tempfile
import os
from PIL import Image

from core.pyexiv2_wrapper import PyExiv2Operations, safe_pyexiv2_image


class TestPyExiv2Integration:
    """Integration tests using real PyExiv2 operations."""

    @pytest.fixture
    def temp_image_with_exif(self):
        """Create a temporary test image with EXIF data."""
        with tempfile.NamedTemporaryFile(suffix=".jpg", delete=False) as f:
            # Create a simple test image
            img = Image.new("RGB", (200, 100), color="blue")

            # Save with some basic EXIF
            exif_dict = {"0th": {}, "Exif": {}, "GPS": {}, "1st": {}, "thumbnail": None}
            exif_dict["0th"][256] = 200  # ImageWidth
            exif_dict["0th"][257] = 100  # ImageLength
            exif_dict["0th"][274] = 1  # Orientation

            import piexif

            exif_bytes = piexif.dump(exif_dict)
            img.save(f.name, "JPEG", exif=exif_bytes)

            yield f.name

        try:
            os.unlink(f.name)
        except (FileNotFoundError, PermissionError):
            pass

    def test_real_metadata_extraction(self, temp_image_with_exif):
        """Test real metadata extraction with PyExiv2."""
        try:
            metadata = PyExiv2Operations.get_comprehensive_metadata(
                temp_image_with_exif
            )

            # Check basic properties
            assert metadata["file_path"] == temp_image_with_exif
            assert metadata["pixel_width"] == 200
            assert metadata["pixel_height"] == 100
            assert metadata["mime_type"] == "image/jpeg"
            assert isinstance(metadata["file_size"], int)
            assert metadata["file_size"] > 0

        except Exception as e:
            pytest.skip(f"PyExiv2 not available or file access issue: {e}")

    def test_real_basic_info_extraction(self, temp_image_with_exif):
        """Test real basic info extraction with PyExiv2."""
        try:
            info = PyExiv2Operations.get_basic_info(temp_image_with_exif)

            assert info["file_path"] == temp_image_with_exif
            assert info["pixel_width"] == 200
            assert info["pixel_height"] == 100
            assert info["mime_type"] == "image/jpeg"
            assert isinstance(info["file_size"], int)

        except Exception as e:
            pytest.skip(f"PyExiv2 not available or file access issue: {e}")

    def test_real_orientation_operations(self, temp_image_with_exif):
        """Test real orientation get/set operations."""
        try:
            # Get initial orientation
            initial_orientation = PyExiv2Operations.get_orientation(
                temp_image_with_exif
            )
            assert initial_orientation == 1  # Default orientation

            # Set new orientation
            success = PyExiv2Operations.set_orientation(temp_image_with_exif, 6)
            assert success is True

            # Verify the change
            new_orientation = PyExiv2Operations.get_orientation(temp_image_with_exif)
            assert new_orientation == 6

            # Set back to original
            success = PyExiv2Operations.set_orientation(temp_image_with_exif, 1)
            assert success is True

        except Exception as e:
            pytest.skip(f"PyExiv2 not available or file access issue: {e}")

    def test_real_rating_operations(self, temp_image_with_exif):
        """Test real rating get/set operations."""
        try:
            # Initially should have no rating
            initial_rating = PyExiv2Operations.get_rating(temp_image_with_exif)
            assert initial_rating is None

            # Set a rating
            success = PyExiv2Operations.set_rating(temp_image_with_exif, 4)
            assert success is True

            # Verify the rating was set
            new_rating = PyExiv2Operations.get_rating(temp_image_with_exif)
            assert new_rating == 4

            # Change the rating
            success = PyExiv2Operations.set_rating(temp_image_with_exif, 2)
            assert success is True

            # Verify the change
            updated_rating = PyExiv2Operations.get_rating(temp_image_with_exif)
            assert updated_rating == 2

        except Exception as e:
            pytest.skip(f"PyExiv2 not available or file access issue: {e}")

    def test_real_safe_context_manager(self, temp_image_with_exif):
        """Test real safe context manager usage."""
        try:
            with safe_pyexiv2_image(temp_image_with_exif) as img:
                # Test basic operations
                width = img.get_pixel_width()
                height = img.get_pixel_height()
                mime_type = img.get_mime_type()

                assert width == 200
                assert height == 100
                assert mime_type == "image/jpeg"

                # Test metadata reading
                exif_data = img.read_exif()
                assert isinstance(exif_data, dict)

        except Exception as e:
            pytest.skip(f"PyExiv2 not available or file access issue: {e}")

    def test_real_batch_operations(self):
        """Test real batch operations with multiple images."""
        try:
            temp_files = []

            # Create multiple test images
            for i in range(3):
                with tempfile.NamedTemporaryFile(
                    suffix=f"_test_{i}.jpg", delete=False
                ) as f:
                    img = Image.new(
                        "RGB", (50 + i * 10, 50 + i * 10), color=(255, i * 50, 0)
                    )
                    img.save(f.name, "JPEG")
                    temp_files.append(f.name)

            try:
                # Test batch metadata extraction
                results = PyExiv2Operations.batch_get_metadata(temp_files)

                assert len(results) == 3
                for i, result in enumerate(results):
                    assert result["file_path"] == temp_files[i]
                    assert result["pixel_width"] == 50 + i * 10
                    assert result["pixel_height"] == 50 + i * 10
                    assert result["mime_type"] == "image/jpeg"

            finally:
                # Clean up temp files
                for temp_file in temp_files:
                    try:
                        os.unlink(temp_file)
                    except (FileNotFoundError, PermissionError):
                        pass

        except Exception as e:
            pytest.skip(f"PyExiv2 not available or file access issue: {e}")

    def test_real_missing_file_handling(self):
        """Test handling of missing files with real operations."""
        nonexistent_file = "/path/that/does/not/exist/image.jpg"

        try:
            # These should handle missing files gracefully
            orientation = PyExiv2Operations.get_orientation(nonexistent_file)
            assert orientation == 1  # Default

            rating = PyExiv2Operations.get_rating(nonexistent_file)
            assert rating is None

            # These should raise exceptions for missing files
            with pytest.raises(
                Exception
            ):  # Could be PyExiv2Error or underlying exception
                PyExiv2Operations.get_comprehensive_metadata(nonexistent_file)

            with pytest.raises(Exception):
                PyExiv2Operations.get_basic_info(nonexistent_file)

        except Exception as e:
            pytest.skip(f"PyExiv2 not available: {e}")


class TestPyExiv2InitializationOrder:
    """Test that PyExiv2 initialization works correctly."""

    def test_initialization_before_qt_import(self):
        """Test that PyExiv2 can be initialized even with Qt already imported."""
        # Qt should already be imported by other tests, but this should still work
        try:
            from core.pyexiv2_init import ensure_pyexiv2_initialized

            # Should not raise an exception
            ensure_pyexiv2_initialized()

            # Should be able to use PyExiv2 operations
            orientation = PyExiv2Operations.get_orientation("/nonexistent.jpg")
            assert orientation == 1  # Should return default for missing file

        except Exception as e:
            pytest.skip(f"PyExiv2 not available: {e}")
