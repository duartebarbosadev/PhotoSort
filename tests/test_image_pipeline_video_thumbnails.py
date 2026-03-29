from unittest.mock import Mock, patch

import numpy as np

from src.core.image_pipeline import ImagePipeline


class _FakeCapture:
    def __init__(self, opened=True, frame=None):
        self._opened = opened
        self._frame = frame
        self.released = False

    def isOpened(self):
        return self._opened

    def read(self):
        if not self._opened or self._frame is None:
            return False, None
        return True, self._frame

    def release(self):
        self.released = True


def test_video_thumbnail_generates_first_frame_with_overlay(tmp_path):
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    pipeline.thumbnail_cache.get = Mock(return_value=None)
    pipeline.thumbnail_cache.set = Mock()

    frame = np.full((120, 200, 3), 100, dtype=np.uint8)
    fake_capture = _FakeCapture(opened=True, frame=frame)

    with patch(
        "src.core.image_pipeline.cv2.VideoCapture", return_value=fake_capture
    ) as mock_capture:
        thumbnail = pipeline._get_pil_thumbnail("/tmp/video.mp4")

    assert thumbnail is not None
    assert thumbnail.width <= 256
    assert thumbnail.height <= 256
    center_pixel = thumbnail.getpixel((thumbnail.width // 2, thumbnail.height // 2))
    assert center_pixel != (100, 100, 100)
    assert fake_capture.released is True
    mock_capture.assert_called_once_with("/tmp/video.mp4")
    pipeline.thumbnail_cache.set.assert_called_once()


def test_video_thumbnail_falls_back_when_frame_unavailable(tmp_path):
    pipeline = ImagePipeline(
        thumbnail_cache_dir=str(tmp_path / "thumb"),
        preview_cache_dir=str(tmp_path / "preview"),
    )
    pipeline.thumbnail_cache.get = Mock(return_value=None)
    pipeline.thumbnail_cache.set = Mock()

    fake_capture = _FakeCapture(opened=False, frame=None)
    with patch("src.core.image_pipeline.cv2.VideoCapture", return_value=fake_capture):
        thumbnail = pipeline._get_pil_thumbnail("/tmp/video.mp4")

    assert thumbnail is None
    assert pipeline.thumbnail_cache.set.call_count == 0
