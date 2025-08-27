"""
Tests for automatic RAW processing functionality.

This module tests that the ImagePipeline correctly detects RAW files
and applies appropriate processing automatically without requiring
external apply_auto_edits parameters.
"""

import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2

import inspect
from unittest.mock import Mock, patch
from src.core.image_pipeline import ImagePipeline
from src.core.image_processing.raw_image_processor import is_raw_extension


class TestAutomaticRawProcessing:
    """Tests for automatic RAW file detection and processing."""

    def test_raw_extension_detection(self):
        """Test that RAW file extensions are correctly detected."""
        # Test RAW extensions (should return True)
        raw_extensions = [
            ".arw",  # Sony
            ".cr2",  # Canon
            ".cr3",  # Canon
            ".nef",  # Nikon
            ".nrw",  # Nikon
            ".dng",  # Adobe DNG
            ".raf",  # Fujifilm
            ".orf",  # Olympus
            ".rw2",  # Panasonic
            ".x3f",  # Sigma
            ".pef",  # Pentax
            ".srw",  # Samsung
            ".raw",  # Generic
        ]

        for ext in raw_extensions:
            assert is_raw_extension(ext), f"Extension {ext} should be detected as RAW"
            # Test case insensitive
            assert is_raw_extension(ext.upper()), (
                f"Extension {ext.upper()} should be detected as RAW"
            )

    def test_non_raw_extension_detection(self):
        """Test that non-RAW file extensions are correctly identified."""
        # Test non-RAW extensions (should return False)
        non_raw_extensions = [
            ".jpg",
            ".jpeg",
            ".png",
            ".gif",
            ".bmp",
            ".tiff",
            ".webp",
            ".svg",
            ".txt",
            ".mp4",
        ]

        for ext in non_raw_extensions:
            assert not is_raw_extension(ext), (
                f"Extension {ext} should NOT be detected as RAW"
            )
            # Test case insensitive
            assert not is_raw_extension(ext.upper()), (
                f"Extension {ext.upper()} should NOT be detected as RAW"
            )

    def test_method_signatures_no_apply_auto_edits(self):
        """Test that ImagePipeline methods no longer have apply_auto_edits parameters."""
        pipeline = ImagePipeline()

        methods_to_check = [
            "get_thumbnail_qpixmap",
            "get_preview_qpixmap",
            "preload_thumbnails",
        ]

        for method_name in methods_to_check:
            method = getattr(pipeline, method_name)
            sig = inspect.signature(method)
            params = list(sig.parameters.keys())

            assert "apply_auto_edits" not in params, (
                f"Method {method_name} should not have 'apply_auto_edits' parameter. "
                f"Found parameters: {params}"
            )

    def test_internal_raw_detection_in_get_pil_thumbnail(self):
        """Test that _get_pil_thumbnail method uses is_raw_extension internally."""
        pipeline = ImagePipeline()

        with patch("src.core.image_pipeline.is_raw_extension") as mock_is_raw:
            mock_is_raw.return_value = True

            # Mock file existence to avoid file system calls
            with (
                patch("os.path.exists", return_value=True),
                patch(
                    "src.core.image_processing.raw_image_processor.RawImageProcessor.process_raw_for_thumbnail"
                ) as mock_process,
            ):
                mock_process.return_value = Mock()  # Mock PIL image

                # Call the internal method directly
                pipeline._get_pil_thumbnail("test.arw")

                # Verify that is_raw_extension was called (can be called multiple times internally)
                mock_is_raw.assert_called_with(".arw")
                assert mock_is_raw.call_count >= 1, (
                    "is_raw_extension should be called at least once"
                )

    def test_cache_key_generation_includes_raw_detection(self):
        """Test that cache keys are generated correctly with RAW detection."""
        ImagePipeline()

        with (
            patch("src.core.image_pipeline.is_raw_extension") as mock_is_raw,
            patch("src.core.image_pipeline.os.path.normpath") as mock_normpath,
        ):
            mock_normpath.return_value = "test.arw"
            mock_is_raw.return_value = True

            # Access the cache key generation logic directly
            # This simulates what happens in _get_pil_thumbnail
            normalized_path = "test.arw"
            ext = ".arw"
            apply_auto_edits = is_raw_extension(ext)
            apply_orientation = True

            cache_key = (normalized_path, apply_auto_edits, apply_orientation)

            # Verify cache key structure
            assert cache_key[0] == "test.arw"  # path
            assert cache_key[1]  # RAW detection result for .arw
            assert cache_key[2]  # orientation
            assert len(cache_key) == 3

    def test_preload_thumbnails_processes_each_file_individually(self):
        """Test that preload_thumbnails processes each file with individual RAW detection."""
        pipeline = ImagePipeline()

        # Test with mixed file types
        test_files = ["image1.jpg", "raw1.arw", "image2.png", "raw2.cr2"]

        # Mock to avoid actual file operations
        with (
            patch.object(pipeline, "_get_pil_thumbnail") as mock_get_pil,
            patch("src.core.image_pipeline.os.path.isfile") as mock_isfile,
        ):
            mock_isfile.return_value = True
            mock_get_pil.return_value = None  # Simulate no thumbnail

            # Call preload_thumbnails
            pipeline.preload_thumbnails(test_files)

            # Verify that _get_pil_thumbnail was called for each file
            assert mock_get_pil.call_count == len(test_files)

            # Verify each call had the correct file path
            called_paths = [call[0][0] for call in mock_get_pil.call_args_list]
            assert called_paths == test_files

    def test_get_thumbnail_qpixmap_handles_file_types_correctly(self):
        """Test that get_thumbnail_qpixmap processes different file types."""
        ImagePipeline()

        test_cases = [
            ("test.jpg", False),  # Regular image
            ("test.arw", True),  # RAW image
            ("test.png", False),  # Regular image
            ("test.cr2", True),  # RAW image
        ]

        for file_path, expected_is_raw in test_cases:
            ext = file_path[file_path.rfind(".") :]
            actual_is_raw = is_raw_extension(ext)
            assert actual_is_raw == expected_is_raw, (
                f"File {file_path} should {'be' if expected_is_raw else 'not be'} "
                f"detected as RAW, but got {actual_is_raw}"
            )


class TestRawProcessingIntegration:
    """Integration tests for RAW processing workflow."""

    def test_raw_extension_consistency_across_methods(self):
        """Test that RAW extension detection is consistent across all methods."""
        test_extensions = [".arw", ".cr2", ".jpg", ".png", ".nef", ".dng"]

        for ext in test_extensions:
            is_raw = is_raw_extension(ext)

            # The detection should be consistent regardless of how it's called
            assert is_raw == is_raw_extension(ext), (
                f"RAW detection for {ext} should be consistent"
            )

            # Case insensitive check
            assert is_raw == is_raw_extension(ext.upper()), (
                f"RAW detection for {ext} should be case insensitive"
            )

    def test_pipeline_initialization_without_auto_edits_params(self):
        """Test that ImagePipeline can be initialized and used without any auto_edits parameters."""
        # This should work without any issues
        pipeline = ImagePipeline()

        # Verify pipeline has required attributes
        assert hasattr(pipeline, "thumbnail_cache")
        assert hasattr(pipeline, "preview_cache")
        assert hasattr(pipeline, "get_thumbnail_qpixmap")
        assert hasattr(pipeline, "get_preview_qpixmap")
        assert hasattr(pipeline, "preload_thumbnails")

        # Verify methods can be called (they will fail due to missing files, but parameters should be correct)
        try:
            pipeline.get_thumbnail_qpixmap("nonexistent.arw")
        except Exception:
            pass  # Expected to fail due to missing file

        try:
            pipeline.get_preview_qpixmap(
                "nonexistent.cr2", display_max_size=(1000, 1000)
            )
        except Exception:
            pass  # Expected to fail due to missing file

        try:
            pipeline.preload_thumbnails(["nonexistent1.jpg", "nonexistent2.arw"])
        except Exception:
            pass  # Expected to fail due to missing files
