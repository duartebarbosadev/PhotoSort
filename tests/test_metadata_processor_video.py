import os
import unicodedata
from unittest.mock import Mock, patch

from src.core.metadata_processor import MetadataProcessor


def test_batch_display_metadata_skips_pyexiv2_for_video(tmp_path):
    video_path = tmp_path / "clip.mp4"
    video_path.write_bytes(b"video")

    exif_cache = Mock()
    exif_cache.get.return_value = None

    with patch(
        "src.core.metadata_processor.PyExiv2Operations.get_comprehensive_metadata"
    ) as mock_pyexiv2:
        results = MetadataProcessor.get_batch_display_metadata(
            [str(video_path)],
            rating_disk_cache=None,
            exif_disk_cache=exif_cache,
        )

    key = unicodedata.normalize("NFC", os.path.normpath(str(video_path)))
    assert key in results
    assert results[key]["rating"] == 0
    assert results[key]["date"] is not None
    mock_pyexiv2.assert_not_called()

    exif_cache.set.assert_called_once()
    cache_key, payload = exif_cache.set.call_args.args
    assert cache_key == key
    assert payload.get("media_type") == "video"
    assert payload.get("file_path") == key
