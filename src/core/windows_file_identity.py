"""Read a Windows file version without decoding or hashing image contents.

Python's Windows stat ctime is creation time. FILE_BASIC_INFO.ChangeTime is the
change time needed for cache validation. Keep Win32 bindings isolated here so
the rest of the application does not depend on ctypes or Windows-only imports.
"""

import ctypes
from functools import cache


class _FileBasicInfo(ctypes.Structure):
    _fields_ = [
        ("CreationTime", ctypes.c_int64),
        ("LastAccessTime", ctypes.c_int64),
        ("LastWriteTime", ctypes.c_int64),
        ("ChangeTime", ctypes.c_int64),
        ("FileAttributes", ctypes.c_uint32),
    ]


class _FileStandardInfo(ctypes.Structure):
    _fields_ = [
        ("AllocationSize", ctypes.c_int64),
        ("EndOfFile", ctypes.c_int64),
        ("NumberOfLinks", ctypes.c_uint32),
        ("DeletePending", ctypes.c_ubyte),
        ("Directory", ctypes.c_ubyte),
    ]


class _FileIdInfo(ctypes.Structure):
    _fields_ = [
        ("VolumeSerialNumber", ctypes.c_uint64),
        ("FileId", ctypes.c_ubyte * 16),
    ]


@cache
def _kernel32():
    api = ctypes.WinDLL("kernel32", use_last_error=True)
    api.CreateFileW.argtypes = [
        ctypes.c_wchar_p,
        ctypes.c_uint32,
        ctypes.c_uint32,
        ctypes.c_void_p,
        ctypes.c_uint32,
        ctypes.c_uint32,
        ctypes.c_void_p,
    ]
    api.CreateFileW.restype = ctypes.c_void_p
    api.GetFileInformationByHandleEx.argtypes = [
        ctypes.c_void_p,
        ctypes.c_int,
        ctypes.c_void_p,
        ctypes.c_uint32,
    ]
    api.GetFileInformationByHandleEx.restype = ctypes.c_int32
    api.CloseHandle.argtypes = [ctypes.c_void_p]
    api.CloseHandle.restype = ctypes.c_int32
    return api


def windows_file_identity(path: str) -> tuple[int, int, int, int, int] | None:
    """Return size, mtime, change time, volume and file ID from one handle.

    Missing permissions, unsupported filesystems and unavailable change times
    disable reuse; never fall back to the weaker creation-time identity.
    """
    try:
        api = _kernel32()
        # FILE_READ_ATTRIBUTES; share read/write/delete; OPEN_EXISTING. No data
        # read access is requested, and normal file mutations remain possible.
        handle = api.CreateFileW(path, 0x80, 0x7, None, 3, 0, None)
    except AttributeError, OSError:
        return None
    if handle is None or handle == ctypes.c_void_p(-1).value:
        return None
    try:
        basic, standard, file_id = _FileBasicInfo(), _FileStandardInfo(), _FileIdInfo()
        for info_class, info in ((0, basic), (1, standard), (18, file_id)):
            if not api.GetFileInformationByHandleEx(
                handle, info_class, ctypes.byref(info), ctypes.sizeof(info)
            ):
                return None
        if standard.Directory or standard.DeletePending or basic.ChangeTime <= 0:
            return None
        # FILETIME counts 100 ns intervals since 1601; convert to Unix nanoseconds.
        epoch = 116444736000000000
        return (
            standard.EndOfFile,
            (basic.LastWriteTime - epoch) * 100,
            (basic.ChangeTime - epoch) * 100,
            file_id.VolumeSerialNumber,
            int.from_bytes(bytes(file_id.FileId), "little"),
        )
    except OSError:
        return None
    finally:
        api.CloseHandle(handle)
