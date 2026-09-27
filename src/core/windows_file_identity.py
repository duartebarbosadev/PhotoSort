"""Windows file versions using pywin32, without reading image contents."""

import os
import stat
from datetime import UTC, datetime


_EPOCH = datetime(1970, 1, 1, tzinfo=UTC)


def windows_file_identity(path: str) -> tuple[int, int, int, int, int] | None:
    """Return size, mtime, change time, volume and file ID from one open file.

    pywin32 exposes change time at millisecond precision, which is sufficient
    alongside the other attributes for normal photo editing and replacement.
    """
    import msvcrt
    import pywintypes
    import win32file

    try:
        with open(path, "rb") as source:
            info = os.fstat(source.fileno())
            if not stat.S_ISREG(info.st_mode):
                return None
            basic = win32file.GetFileInformationByHandleEx(
                msvcrt.get_osfhandle(source.fileno()), win32file.FileBasicInfo
            )
            elapsed = basic["ChangeTime"] - _EPOCH
            change_ns = (
                elapsed.days * 86400 + elapsed.seconds
            ) * 1_000_000_000 + elapsed.microseconds * 1000
            if change_ns <= 0:
                return None
            return (
                info.st_size,
                info.st_mtime_ns,
                change_ns,
                info.st_dev,
                info.st_ino,
            )
    except OSError, pywintypes.error:
        return None
