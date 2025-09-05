import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2
import os
import pytest

try:
    from src.core.metadata_processor import MetadataProcessor

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)

    class MetadataProcessor:  # type: ignore
        pass


class TestMetadataFieldsStability:
    """Verify core detailed metadata is consistently available for sample images.

    This guards against regressions from pyexiv2 or extraction changes by
    asserting stable, normalized fields our UI depends on (resolution, size, MIME).
    """

    @classmethod
    def setup_class(cls):
        cls.test_folder = "tests/samples"
        cls.sample_images = []

        if os.path.exists(cls.test_folder):
            for filename in os.listdir(cls.test_folder):
                lower = filename.lower()
                if lower.endswith((".png", ".jpg", ".jpeg", ".arw")):
                    cls.sample_images.append(os.path.join(cls.test_folder, filename))

        if not IMPORTS_AVAILABLE:
            pytest.skip(f"Cannot import MetadataProcessor: {IMPORT_ERROR}")

        if len(cls.sample_images) == 0:
            pytest.skip(
                f"No sample images found in {cls.test_folder}", allow_module_level=True
            )

    def test_detailed_metadata_has_core_fields(self):
        for image_path in self.sample_images:
            md = MetadataProcessor.get_detailed_metadata(image_path)
            assert md is not None, f"No metadata for {image_path}"

            # If an error key appears, fail loudly to catch regressions
            assert "error" not in md, (
                f"Extraction error for {image_path}: {md.get('error')}"
            )

            # Core keys must exist
            for key in ("pixel_width", "pixel_height", "file_size", "mime_type"):
                assert key in md, f"Missing '{key}' in metadata for {image_path}"

            # Validate types and sensible ranges
            w = md["pixel_width"]
            h = md["pixel_height"]
            size = md["file_size"]
            mime = md["mime_type"]

            assert isinstance(w, int) and w > 0, f"Invalid width for {image_path}: {w}"
            assert isinstance(h, int) and h > 0, f"Invalid height for {image_path}: {h}"
            assert isinstance(size, int) and size > 0, (
                f"Invalid file size for {image_path}: {size}"
            )
            assert isinstance(mime, str) and "/" in mime, (
                f"Invalid MIME type for {image_path}: {mime}"
            )

            # Light sanity: known extensions should map to image/*
            ext = os.path.splitext(image_path)[1].lower()
            if ext in {".jpg", ".jpeg", ".png", ".arw"}:
                assert mime.startswith("image/"), (
                    f"Unexpected MIME for {image_path}: {mime}"
                )
