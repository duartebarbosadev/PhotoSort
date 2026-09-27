"""Win32 contract tests plus real filesystem regressions on Windows CI."""

import ctypes
import os
import sys
from types import SimpleNamespace
from unittest.mock import Mock

import pytest

from core import windows_file_identity as native


@pytest.fixture
def api(monkeypatch):
    epoch = 116444736000000000
    structures = {
        0: native._FileBasicInfo(1, 2, epoch + 30, epoch + 40, 0),
        1: native._FileStandardInfo(4096, 8, 1, 0, 0),
        18: native._FileIdInfo(17, (ctypes.c_ubyte * 16)(*range(16))),
    }

    def query(handle, info_class, destination, size):
        assert handle == 1 << 40  # Handles must not be truncated to 32 bits.
        source = structures[info_class]
        assert size == ctypes.sizeof(source)
        ctypes.memmove(destination, ctypes.byref(source), size)
        return 1

    result = SimpleNamespace(
        CreateFileW=Mock(return_value=1 << 40),
        GetFileInformationByHandleEx=Mock(side_effect=query),
        CloseHandle=Mock(return_value=1),
        structures=structures,
    )
    monkeypatch.setattr(native, "_kernel32", lambda: result)
    return result


def test_identity_uses_change_time_and_one_attribute_only_handle(api):
    assert native.windows_file_identity("photo.ARW") == (
        8,
        3000,
        4000,
        17,
        int.from_bytes(bytes(range(16)), "little"),
    )
    api.CreateFileW.assert_called_once_with("photo.ARW", 0x80, 7, None, 3, 0, None)
    assert api.GetFileInformationByHandleEx.call_count == 3
    api.CloseHandle.assert_called_once_with(1 << 40)


@pytest.mark.parametrize("info_class", [0, 1, 18])
def test_unsupported_information_disables_reuse_and_closes_handle(api, info_class):
    original = api.GetFileInformationByHandleEx.side_effect
    api.GetFileInformationByHandleEx.side_effect = lambda h, c, p, s: (
        0 if c == info_class else original(h, c, p, s)
    )
    assert native.windows_file_identity("photo.ARW") is None
    api.CloseHandle.assert_called_once()


@pytest.mark.parametrize("handle", [None, ctypes.c_void_p(-1).value])
def test_failed_open_does_not_query_or_close_invalid_handle(api, handle):
    api.CreateFileW.return_value = handle
    assert native.windows_file_identity("missing.ARW") is None
    api.GetFileInformationByHandleEx.assert_not_called()
    api.CloseHandle.assert_not_called()


@pytest.mark.parametrize("field", ["ChangeTime", "Directory", "DeletePending"])
def test_unusable_file_version_is_not_returned(api, field):
    info_class = 0 if field == "ChangeTime" else 1
    setattr(api.structures[info_class], field, 0 if field == "ChangeTime" else 1)
    assert native.windows_file_identity("photo.ARW") is None
    api.CloseHandle.assert_called_once()


def test_query_exception_still_closes_handle(api):
    api.GetFileInformationByHandleEx.side_effect = OSError("unavailable")
    assert native.windows_file_identity("photo.ARW") is None
    api.CloseHandle.assert_called_once()


def test_win32_binding_preserves_handle_width(monkeypatch):
    api = SimpleNamespace(
        CreateFileW=Mock(), GetFileInformationByHandleEx=Mock(), CloseHandle=Mock()
    )
    monkeypatch.setattr(ctypes, "WinDLL", Mock(return_value=api), raising=False)
    native._kernel32.cache_clear()
    try:
        assert native._kernel32() is api
        assert api.CreateFileW.restype is ctypes.c_void_p
        assert api.GetFileInformationByHandleEx.argtypes[0] is ctypes.c_void_p
        assert api.CloseHandle.argtypes == [ctypes.c_void_p]
        assert ctypes.sizeof(native._FileBasicInfo) == 40
        assert ctypes.sizeof(native._FileStandardInfo) == 24
        assert ctypes.sizeof(native._FileIdInfo) == 24
    finally:
        native._kernel32.cache_clear()


@pytest.mark.skipif(
    sys.platform != "win32", reason="requires the Windows filesystem API"
)
@pytest.mark.parametrize("replacement", [False, True])
def test_native_identity_detects_same_size_changes_with_restored_mtime(
    tmp_path, replacement
):
    photo = tmp_path / "Caf\N{LATIN SMALL LETTER E WITH ACUTE}.ARW"
    photo.write_bytes(b"original")
    before = native.windows_file_identity(str(photo))
    assert before is not None
    assert native.windows_file_identity(str(photo)) == before
    stat = photo.stat()
    target = photo.with_suffix(".new") if replacement else photo
    target.write_bytes(b"modified")
    os.utime(target, ns=(stat.st_atime_ns, stat.st_mtime_ns))
    if replacement:
        target.replace(photo)
    after = native.windows_file_identity(str(photo))
    assert after is not None
    assert after[:2] == before[:2]
    assert after != before
    if not replacement:
        assert after[3:] == before[3:]
        assert after[2] != before[2]
    photo.unlink()
    assert native.windows_file_identity(str(photo)) is None
