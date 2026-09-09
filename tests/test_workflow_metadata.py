from unittest.mock import Mock

from ui.workflow_metadata import build_workflow_metadata_rows


def test_workflow_metadata_rows_use_cached_exif_without_loading_file(monkeypatch):
    exif_cache = object()
    cached_lookup = Mock(
        return_value={
            "Exif.Photo.DateTimeOriginal": "2026:07:18 14:05:06",
            "Exif.Image.Make": "Canon",
            "Exif.Image.Model": "EOS R5",
            "Exif.Photo.LensModel": "RF 50mm F1.2",
            "Exif.Photo.FocalLength": "50",
            "Exif.Photo.FNumber": "1.2",
            "Exif.Photo.ExposureTime": "1/250",
            "Exif.Photo.ISOSpeedRatings": "200",
            "pixel_width": 8192,
            "pixel_height": 5464,
        }
    )
    monkeypatch.setattr(
        "ui.workflow_metadata.MetadataProcessor.get_cached_detailed_metadata",
        cached_lookup,
    )

    rows = build_workflow_metadata_rows("/photos/image.jpg", exif_cache)

    cached_lookup.assert_called_once_with("/photos/image.jpg", exif_cache)
    assert rows == [
        ("Date", "2026-07-18 14:05"),
        ("Camera", "Canon EOS R5"),
        ("Lens", "RF 50mm F1.2"),
        ("Lens", "50 mm  f/1.2"),
        ("Exposure", "1/250s  ISO 200"),
        ("Size", "8192 × 5464"),
    ]
