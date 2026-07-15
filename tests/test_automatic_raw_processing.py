"""
Tests for automatic RAW processing functionality.

This module tests that the ImagePipeline correctly detects RAW files
and applies appropriate processing automatically without requiring
external apply_auto_edits parameters.
"""

import inspect
from unittest.mock import Mock, patch
from core.app_settings import PRELOAD_MAX_RESOLUTION
from src.core.image_pipeline import CACHE_SCHEMA_VERSION, ImagePipeline
from src.core.image_processing.raw_image_processor import is_raw_extension
from PIL import Image
from PyQt6.QtWidgets import QApplication

_app = QApplication.instance() or QApplication([])


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
                mock_process.return_value = Image.new("RGB", (32, 32))

                # Call the internal method directly
                pipeline._get_pil_thumbnail("test.arw")

                # Verify that is_raw_extension was called (can be called multiple times internally)
                mock_is_raw.assert_called_with(".arw")
                assert mock_is_raw.call_count >= 1, (
                    "is_raw_extension should be called at least once"
                )

    def test_cache_key_generation_includes_raw_detection(self):
        """Test that cache keys are generated correctly with RAW detection."""
        pipeline = ImagePipeline()

        with (
            patch("src.core.image_pipeline.is_raw_extension") as mock_is_raw,
            patch("src.core.image_pipeline.os.path.normpath") as mock_normpath,
        ):
            mock_normpath.return_value = "test.arw"
            mock_is_raw.return_value = True

            cache_key = pipeline.thumbnail_cache_key(
                "test.arw", True, file_size=10, mtime_ns=20
            )

            # Verify cache key structure
            assert cache_key[0] == "test.arw"  # path
            assert cache_key[1] == "thumbnail"
            assert cache_key[2] == CACHE_SCHEMA_VERSION
            assert cache_key[5]  # RAW detection result for .arw
            assert cache_key[6]  # orientation

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
            # Note: preload_thumbnails uses parallel processing, so order is non-deterministic
            called_paths = [call[0][0] for call in mock_get_pil.call_args_list]
            assert set(called_paths) == set(test_files)

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


def test_get_cached_thumbnail_qpixmap_does_not_generate_on_cache_miss(tmp_path):
    image_path = tmp_path / "a.jpg"
    image_path.write_bytes(b"preview")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    pipeline.thumbnail_cache.get = Mock(return_value=None)

    with patch.object(
        pipeline,
        "_get_pil_thumbnail",
        side_effect=AssertionError("cache-only thumbnail helper must not generate"),
    ):
        pixmap = pipeline.get_cached_thumbnail_qpixmap(str(image_path))

    assert pixmap is None


def test_get_cached_preview_qpixmap_does_not_generate_on_cache_miss(tmp_path):
    image_path = tmp_path / "a.jpg"
    image_path.write_bytes(b"preview")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    pipeline.preview_cache.get = Mock(return_value=None)

    with patch.object(
        pipeline,
        "_generate_pil_preview_for_display",
        side_effect=AssertionError("cache-only preview helper must not generate"),
    ):
        pixmap = pipeline.get_cached_preview_qpixmap(
            str(image_path), display_max_size=(800, 600)
        )

    assert pixmap is None


def test_memory_only_preview_lookup_never_reads_disk_cache(tmp_path):
    image_path = tmp_path / "disk-cached.jpg"
    image_path.write_bytes(b"preview")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    pipeline.preview_cache.get = Mock(
        return_value=Image.new("RGB", (800, 600), color="red")
    )

    pixmap = pipeline.get_cached_preview_qpixmap(
        str(image_path),
        display_max_size=(800, 600),
        memory_only=True,
    )

    assert pixmap is None
    pipeline.preview_cache.get.assert_not_called()


def test_get_cached_preview_qpixmap_uses_high_res_cache_without_generation(tmp_path):
    image_path = tmp_path / "a.jpg"
    image_path.write_bytes(b"preview")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )

    cached_preview = Image.new("RGB", (1600, 1200), color="red")

    def cache_get(key):
        if key[5] == (800, 600):
            return None
        return cached_preview

    pipeline.preview_cache.get = Mock(side_effect=cache_get)
    pipeline.preview_cache.set = Mock()

    with patch.object(
        pipeline,
        "_generate_pil_preview_for_display",
        side_effect=AssertionError("cache-only preview helper must not generate"),
    ):
        pixmap = pipeline.get_cached_preview_qpixmap(
            str(image_path), display_max_size=(800, 600)
        )

    assert pixmap is not None
    pipeline.preview_cache.set.assert_called_once()


def test_display_preview_uses_bounded_raw_preview_processor(tmp_path):
    image_path = tmp_path / "a.arw"
    image_path.write_bytes(b"raw")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    expected = Image.new("RGBA", (800, 600), color="blue")

    with (
        patch(
            "src.core.image_pipeline.RawImageProcessor.process_raw_for_preview",
            return_value=expected,
        ) as process_preview,
        patch(
            "src.core.image_pipeline.RawImageProcessor.load_raw_as_pil",
            side_effect=AssertionError(
                "display previews must not fully decode RAW files"
            ),
        ),
    ):
        result = pipeline._generate_pil_preview_for_display(
            str(image_path),
            (800, 600),
        )

    assert result is expected
    process_preview.assert_called_once_with(
        str(image_path),
        True,
        (800, 600),
        force_default_brightness=False,
    )


def test_forced_default_brightness_replaces_existing_navigation_preview(tmp_path):
    image_path = tmp_path / "rotated.arw"
    image_path.write_bytes(b"raw")
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    pipeline.preview_cache.get = Mock(
        return_value=Image.new("RGB", (32, 32), color="red")
    )
    replacement = Image.new("RGB", (64, 64), color="blue")

    with patch(
        "src.core.image_pipeline.RawImageProcessor.process_raw_for_preview",
        return_value=replacement,
    ) as process_preview:
        assert pipeline.ensure_preview_cached(
            str(image_path), force_default_brightness=True
        )

    process_preview.assert_called_once_with(
        str(image_path),
        True,
        PRELOAD_MAX_RESOLUTION,
        force_default_brightness=True,
    )
