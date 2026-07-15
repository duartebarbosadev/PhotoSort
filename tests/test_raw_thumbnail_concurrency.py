"""Concurrency boundaries for inexpensive and full RAW thumbnail decoding."""

import io
from unittest.mock import Mock, patch

import numpy as np
import rawpy
from PIL import Image

from src.core.image_processing.raw_image_processor import RawImageProcessor


def _raw_context(raw):
    context = Mock()
    context.__enter__ = Mock(return_value=raw)
    context.__exit__ = Mock(return_value=False)
    return context


def test_embedded_raw_thumbnail_does_not_take_full_decode_gate():
    encoded = io.BytesIO()
    Image.new("RGB", (32, 32), "red").save(encoded, format="JPEG")
    raw = Mock()
    raw.extract_thumb.return_value = Mock(
        format=rawpy.ThumbFormat.JPEG,
        data=encoded.getvalue(),
    )
    gate = Mock()

    with patch(
        "src.core.image_processing.raw_image_processor.rawpy.imread",
        return_value=_raw_context(raw),
    ):
        result = RawImageProcessor.process_raw_for_thumbnail(
            "image.arw", fallback_decode_gate=gate
        )

    assert result is not None
    gate.acquire.assert_not_called()
    gate.release.assert_not_called()


def test_raw_fallback_is_serialized_by_full_decode_gate():
    raw = Mock()
    raw.extract_thumb.side_effect = rawpy.LibRawNoThumbnailError()
    raw.postprocess.return_value = np.zeros((32, 32, 3), dtype=np.uint8)
    gate = Mock()

    with patch(
        "src.core.image_processing.raw_image_processor.rawpy.imread",
        return_value=_raw_context(raw),
    ):
        result = RawImageProcessor.process_raw_for_thumbnail(
            "image.arw", fallback_decode_gate=gate
        )

    assert result is not None
    gate.acquire.assert_called_once_with()
    gate.release.assert_called_once_with()
