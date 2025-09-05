import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash with pyexiv2
import pytest
from PIL import Image

try:
    from src.core.metadata_io import MetadataIO

    IMPORTS_AVAILABLE = True
    IMPORT_ERROR = None
except ImportError as e:
    IMPORTS_AVAILABLE = False
    IMPORT_ERROR = str(e)


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=lambda: f"Cannot import MetadataIO: {IMPORT_ERROR}"
)
def test_read_raw_metadata_basic(tmp_path):
    # Create a minimal JPEG
    p = tmp_path / "metaio_basic.jpg"
    img = Image.new("RGB", (10, 8), color=(200, 10, 10))
    img.save(p, format="JPEG", quality=85)

    md = MetadataIO.read_raw_metadata(str(p))
    assert isinstance(md, dict)
    assert md.get("file_path") == str(p)
    assert md.get("mime_type") == "image/jpeg"
    assert md.get("pixel_width") == 10
    assert md.get("pixel_height") == 8
    # No error for a valid JPEG
    assert "error" not in md


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=lambda: f"Cannot import MetadataIO: {IMPORT_ERROR}"
)
def test_rating_roundtrip(tmp_path):
    p = tmp_path / "metaio_rating.jpg"
    Image.new("RGB", (6, 6), color=(0, 0, 0)).save(p, format="JPEG", quality=80)

    ok = MetadataIO.set_xmp_rating(str(p), 4)
    assert ok is True

    md = MetadataIO.read_raw_metadata(str(p))
    # Rating may be string or numeric depending on backend; accept both
    rating = md.get("Xmp.xmp.Rating")
    assert rating is not None
    assert str(rating) == "4"


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=lambda: f"Cannot import MetadataIO: {IMPORT_ERROR}"
)
def test_orientation_roundtrip(tmp_path):
    p = tmp_path / "metaio_orient.jpg"
    Image.new("RGB", (7, 9), color=(1, 2, 3)).save(p, format="JPEG", quality=80)

    # No orientation set yet (may return None)
    ori_before = MetadataIO.read_exif_orientation(str(p))
    assert ori_before in (None, 1)  # default/no EXIF

    assert MetadataIO.set_exif_orientation(str(p), 6) is True
    ori_after = MetadataIO.read_exif_orientation(str(p))
    assert ori_after == 6

    o, w, h = MetadataIO.read_orientation_and_dimensions(str(p))
    assert o == 6
    assert w == 7 and h == 9


@pytest.mark.skipif(
    not IMPORTS_AVAILABLE, reason=lambda: f"Cannot import MetadataIO: {IMPORT_ERROR}"
)
def test_missing_file_handling(tmp_path):
    missing = tmp_path / "nope.jpg"
    md = MetadataIO.read_raw_metadata(str(missing))
    assert isinstance(md, dict)
    assert md.get("file_path") == str(missing)
    assert "error" in md
