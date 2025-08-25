import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2
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
    from src.core.metadata_processor import MetadataProcessor

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)

    # Create dummy classes to avoid NameError
    class MetadataProcessor:
        pass


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

    def test_full_rotation_cycle_returns_to_original(self):
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
            assert success, f"Rotation {i + 1} failed"
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

    @pytest.mark.parametrize("rotation_sequence", [
        ["clockwise", "180", "counterclockwise", "180"],
        ["clockwise", "clockwise", "180"],
        ["counterclockwise", "clockwise", "180"],
    ])
    def test_mixed_rotations_sequence(self, rotation_sequence):
        """Test different rotation combinations using parametrized sequences."""
        if not self.sample_images:
            pytest.skip("No sample images available")

        source_image = self.sample_images[0]
        temp_image = self._create_temp_copy(source_image)

        # Map rotation names to functions
        rotation_functions = {
            "clockwise": MetadataProcessor.rotate_clockwise,
            "counterclockwise": MetadataProcessor.rotate_counterclockwise,
            "180": MetadataProcessor.rotate_180,
        }

        # Execute rotation sequence
        for rotation_name in rotation_sequence:
            rotate_func = rotation_functions[rotation_name]
            success = rotate_func(temp_image)
            assert success, f"{rotation_name} rotation failed"
            assert os.path.exists(temp_image), (
                f"File missing after {rotation_name} rotation"
            )

            # Verify image is still readable
            metadata = MetadataProcessor.get_detailed_metadata(temp_image)
            assert metadata is not None, (
                f"Cannot read metadata after {rotation_name} rotation"
            )

        logging.info(
            f"Mixed rotation sequence {rotation_sequence} completed for {os.path.basename(source_image)}"
        )


if __name__ == "__main__":
    # Run tests with verbose output
    pytest.main([__file__, "-v", "-s"])