import logging
import math
import os
import struct
from datetime import UTC, datetime, timedelta
from typing import Any, BinaryIO

import cv2

logger = logging.getLogger(__name__)

QUICKTIME_CREATION_DATE_KEY = "QuickTime.CreationDate"
QUICKTIME_EXTENSIONS = frozenset({".mp4", ".mov", ".m4v", ".3gp", ".3g2"})

_QUICKTIME_EPOCH = datetime(1904, 1, 1, tzinfo=UTC)
_UNIX_EPOCH = datetime(1970, 1, 1, tzinfo=UTC)
_MAX_ATOMS_PER_LEVEL = 4096


def _clean_number(value: Any) -> float | None:
    try:
        number = float(value)
    except TypeError, ValueError:
        return None
    if not math.isfinite(number) or number <= 0:
        return None
    return number


def _fourcc_to_string(code: int) -> str:
    if not code:
        return ""
    chars = [chr((code >> shift) & 0xFF) for shift in range(0, 32, 8)]
    return "".join(chars).strip()


def get_basic_video_metadata(file_path: str) -> dict[str, Any]:
    metadata: dict[str, Any] = {}
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


def _iter_atoms(handle: BinaryIO, start: int, end: int):
    offset = start
    for _ in range(_MAX_ATOMS_PER_LEVEL):
        if offset + 8 > end:
            return
        handle.seek(offset)
        header = handle.read(8)
        if len(header) < 8:
            return
        size, atom_type = struct.unpack(">I4s", header)
        header_size = 8
        if size == 1:
            extended = handle.read(8)
            if len(extended) < 8:
                return
            size = struct.unpack(">Q", extended)[0]
            header_size = 16
        elif size == 0:
            size = end - offset
        if size < header_size:
            return
        yield atom_type, offset + header_size, min(offset + size, end)
        offset += size


def _read_movie_header_seconds(handle: BinaryIO, start: int) -> int | None:
    handle.seek(start)
    version_flags = handle.read(4)
    if len(version_flags) < 4:
        return None
    if version_flags[0] == 1:
        raw = handle.read(8)
        return struct.unpack(">Q", raw)[0] if len(raw) == 8 else None
    raw = handle.read(4)
    return struct.unpack(">I", raw)[0] if len(raw) == 4 else None


def read_quicktime_creation_date(file_path: str) -> datetime | None:
    """Return the recording time from a QuickTime-family movie header.

    Only the ``moov/mvhd`` atom is read, so no decoder is involved. Cameras and
    phones store the value in UTC; it is returned as naive local time so videos
    sort consistently with photo EXIF dates, which are local.
    """

    if os.path.splitext(file_path)[1].lower() not in QUICKTIME_EXTENSIONS:
        return None
    try:
        with open(file_path, "rb") as handle:
            handle.seek(0, os.SEEK_END)
            file_end = handle.tell()
            for atom_type, body_start, body_end in _iter_atoms(handle, 0, file_end):
                if atom_type != b"moov":
                    continue
                for child_type, child_start, _child_end in _iter_atoms(
                    handle, body_start, body_end
                ):
                    if child_type != b"mvhd":
                        continue
                    seconds = _read_movie_header_seconds(handle, child_start)
                    if not seconds:
                        return None
                    created = _QUICKTIME_EPOCH + timedelta(seconds=seconds)
                    if created <= _UNIX_EPOCH:
                        return None
                    return created.astimezone().replace(tzinfo=None)
                return None
    except OSError, OverflowError, struct.error, ValueError:
        logger.debug("Could not read movie header of %s", file_path, exc_info=True)
        return None
    return None


__all__ = [
    "QUICKTIME_CREATION_DATE_KEY",
    "get_basic_video_metadata",
    "read_quicktime_creation_date",
]
