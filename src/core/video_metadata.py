from __future__ import annotations

import logging
import math
import os
from typing import Any, Dict

import cv2

logger = logging.getLogger(__name__)


def _clean_number(value: Any) -> float | None:
    try:
        number = float(value)
    except (TypeError, ValueError):
        return None
    if not math.isfinite(number) or number <= 0:
        return None
    return number


def _fourcc_to_string(code: int) -> str:
    if not code:
        return ""
    chars = []
    for shift in range(0, 32, 8):
        chars.append(chr((code >> shift) & 0xFF))
    return "".join(chars).strip()


def get_basic_video_metadata(file_path: str) -> Dict[str, Any]:
    metadata: Dict[str, Any] = {}
    if not file_path or not os.path.isfile(file_path):
        return metadata

    capture = cv2.VideoCapture(file_path)
    if not capture.isOpened():
        logger.debug("VideoCapture failed to open %s", os.path.basename(file_path))
        return metadata

    try:
        width = _clean_number(capture.get(cv2.CAP_PROP_FRAME_WIDTH))
        height = _clean_number(capture.get(cv2.CAP_PROP_FRAME_HEIGHT))
        fps = _clean_number(capture.get(cv2.CAP_PROP_FPS))
        frame_count = _clean_number(capture.get(cv2.CAP_PROP_FRAME_COUNT))
        fourcc = _clean_number(capture.get(cv2.CAP_PROP_FOURCC))
    finally:
        capture.release()

    if width:
        metadata["video_width"] = int(width)
    if height:
        metadata["video_height"] = int(height)
    if fps:
        metadata["video_fps"] = fps
    if frame_count:
        metadata["video_frame_count"] = int(frame_count)
    if fourcc:
        codec = _fourcc_to_string(int(fourcc))
        if codec:
            metadata["video_codec"] = codec

    duration_seconds = None
    if fps and frame_count:
        duration_seconds = frame_count / fps
        if duration_seconds > 0:
            metadata["video_duration_seconds"] = duration_seconds

    try:
        size_bytes = os.path.getsize(file_path)
    except OSError:
        size_bytes = None

    if size_bytes and duration_seconds:
        metadata["video_bitrate_bps"] = (size_bytes * 8) / duration_seconds

    return metadata


__all__ = ["get_basic_video_metadata"]
