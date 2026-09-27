"""Library adapter tests plus real filesystem regressions on Windows CI."""

import os
import sys
import time
from datetime import UTC, datetime
from types import SimpleNamespace
from unittest.mock import Mock

import pytest

from core import windows_file_identity as native


@pytest.fixture
def api(monkeypatch):
    class WindowsError(Exception):
        pass

    def query(handle, info_class):
        # A real descriptor is used by the adapter, without reading its contents.
        assert os.lseek(handle, 0, os.SEEK_CUR) == 0
        return {"ChangeTime": datetime(2026, 1, 1, tzinfo=UTC)}

    result = SimpleNamespace(
        GetFileInformationByHandleEx=Mock(side_effect=query), FileBasicInfo=0
    )
    monkeypatch.setitem(sys.modules, "win32file", result)
    monkeypatch.setitem(
        sys.modules, "msvcrt", SimpleNamespace(get_osfhandle=lambda fd: fd)
    )
    monkeypatch.setitem(sys.modules, "pywintypes", SimpleNamespace(error=WindowsError))
    result.error = WindowsError
    return result


def test_identity_combines_change_time_with_file_attributes(api, tmp_path):
    photo = tmp_path / "photo.ARW"
    photo.write_bytes(b"original")
    info = photo.stat()
    assert native.windows_file_identity(str(photo)) == (
        8,
        info.st_mtime_ns,
        1767225600000000000,
        info.st_dev,
        info.st_ino,
    )
    handle = api.GetFileInformationByHandleEx.call_args.args[0]
    with pytest.raises(OSError):
        os.fstat(handle)  # The adapter closes the file after querying it.


def test_unavailable_change_time_is_a_miss_and_closes_file(api, tmp_path):
    photo = tmp_path / "photo.ARW"
    photo.touch()
    api.GetFileInformationByHandleEx.side_effect = api.error("unavailable")
    assert native.windows_file_identity(str(photo)) is None
    handle = api.GetFileInformationByHandleEx.call_args.args[0]
    with pytest.raises(OSError):
        os.fstat(handle)


def test_missing_file_is_a_miss(api, tmp_path):
    assert native.windows_file_identity(str(tmp_path / "missing.ARW")) is None
    api.GetFileInformationByHandleEx.assert_not_called()


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
    info = photo.stat()
    # Model normal editing, outside pywin32's millisecond timestamp resolution.
    assert before[3:] == (info.st_dev, info.st_ino)
    assert before[4] != 0
    time.sleep(0.02)
    target = photo.with_suffix(".new") if replacement else photo
    target.write_bytes(b"modified")
    os.utime(target, ns=(info.st_atime_ns, info.st_mtime_ns))
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
