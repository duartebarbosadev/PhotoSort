import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

import os
import unicodedata
from unittest.mock import Mock

import pytest

from core.caching.exif_cache import ExifCache
from core.metadata_processor import MetadataProcessor


@pytest.fixture
def cache(tmp_path):
    instance = ExifCache(str(tmp_path / "cache"))
    yield instance
    instance.close()


@pytest.fixture
def photo(tmp_path):
    path = tmp_path / "DSC00001.ARW"
    path.write_bytes(b"original")
    return path


def test_same_name_in_different_folders_is_independent(cache, photo, tmp_path):
    other = tmp_path / "other" / photo.name
    other.parent.mkdir()
    other.write_bytes(photo.read_bytes())
    cache.set(str(photo), {"rating": 1})
    cache.set(str(other), {"rating": 5})

    assert cache.get(str(photo)) == {"rating": 1}
    assert cache.get(str(other)) == {"rating": 5}


def test_relative_and_absolute_paths_share_one_entry(cache, photo, monkeypatch):
    monkeypatch.chdir(photo.parent)
    cache.set(photo.name, {"rating": 3})
    assert cache.get(str(photo)) == {"rating": 3}
    assert cache.dataset_residency([photo.name, str(photo)]) == (1, 1)
    cache.delete(photo.name)
    assert cache.get(str(photo)) is None


def test_unicode_path_forms_share_identity(cache, tmp_path):
    path = tmp_path / "Cafe\N{COMBINING ACUTE ACCENT}.ARW"
    path.write_bytes(b"original")
    cache.set(str(path), {"rating": 2})
    assert cache.get(unicodedata.normalize("NFC", str(path))) == {"rating": 2}


@pytest.mark.parametrize("change", ["size", "mtime", "replacement", "removed"])
def test_changed_file_is_a_miss(cache, photo, change):
    cache.set(str(photo), {"rating": 1})
    original = photo.stat()
    if change == "size":
        photo.write_bytes(b"a new longer image")
    elif change == "mtime":
        os.utime(photo, ns=(original.st_atime_ns, original.st_mtime_ns + 1_000_000_000))
    elif change == "replacement":
        replacement = photo.with_suffix(".new")
        replacement.write_bytes(b"replaced")  # Same size and preserved mtime.
        os.utime(replacement, ns=(original.st_atime_ns, original.st_mtime_ns))
        replacement.replace(photo)
    else:
        photo.unlink()

    assert cache.get(str(photo)) is None
    assert str(photo) not in cache


def test_same_size_edit_with_restored_mtime_is_a_miss(cache, photo):
    cache.set(str(photo), {"rating": 1})
    original = photo.stat()
    photo.write_bytes(b"modified")
    os.utime(photo, ns=(original.st_atime_ns, original.st_mtime_ns))
    assert cache.get(str(photo)) is None


def test_legacy_or_unknown_entry_requires_refresh(cache, photo):
    identity = cache.file_identity(str(photo))
    for entry in (
        {"rating": 5},
        (1, identity, {"rating": 5}),
        (99, identity, {"rating": 5}),
    ):
        cache._cache.set(str(photo), entry)
        assert cache.get(str(photo)) is None
    cache.set(str(photo), {"rating": 2})
    assert cache.get(str(photo)) == {"rating": 2}


def test_windows_change_time_invalidates_shared_metadata(cache, photo, monkeypatch):
    monkeypatch.setattr("core.caching.exif_cache._IS_WINDOWS", True)
    identity = Mock(return_value=(8, 100, 200, 1, 42))
    monkeypatch.setattr("core.caching.exif_cache.windows_file_identity", identity)
    extract = Mock(side_effect=[{"rating": 1}, {"rating": 5}])
    monkeypatch.setattr(
        "core.metadata_processor.PyExiv2Operations.get_comprehensive_metadata", extract
    )
    assert MetadataProcessor.get_detailed_metadata(str(photo), cache) == {"rating": 1}
    assert MetadataProcessor.get_detailed_metadata(str(photo), cache) == {"rating": 1}
    assert extract.call_count == 1
    identity.return_value = (8, 100, 201, 1, 42)
    assert MetadataProcessor.get_detailed_metadata(str(photo), cache) == {"rating": 5}
    assert MetadataProcessor.get_detailed_metadata(str(photo), cache) == {"rating": 5}
    assert extract.call_count == 2


def test_windows_identity_failure_never_falls_back_to_stat(cache, photo, monkeypatch):
    cache.set(str(photo), {"rating": 1})
    monkeypatch.setattr("core.caching.exif_cache._IS_WINDOWS", True)
    monkeypatch.setattr("core.caching.exif_cache.windows_file_identity", lambda _: None)
    assert cache.get(str(photo)) is None
    cache.delete(str(photo))
    cache.set(str(photo), {"rating": 5})
    assert str(photo) not in cache._cache


def test_missing_file_is_not_cached(cache, photo):
    photo.unlink()
    cache.set(str(photo), {"error": "missing"})
    assert str(photo) not in cache._cache


def test_cache_survives_restart(cache, photo):
    cache.set(str(photo), {"rating": 4})
    cache.close()
    reopened = ExifCache(cache._cache_dir)
    try:
        assert reopened.get(str(photo)) == {"rating": 4}
    finally:
        reopened.close()


@pytest.mark.parametrize("batch", [False, True])
def test_shared_metadata_reads_reuse_extraction_until_file_changes(
    cache, photo, monkeypatch, batch
):
    extract = Mock(
        side_effect=lambda path: {
            "file_path": path,
            "Exif.Image.Model": photo.read_text(),
        }
    )
    monkeypatch.setattr(
        "core.metadata_processor.PyExiv2Operations.get_comprehensive_metadata", extract
    )

    def read():
        if batch:
            return MetadataProcessor.get_batch_display_metadata(
                [str(photo)], exif_disk_cache=cache
            )[str(photo)]["raw_metadata"]
        return MetadataProcessor.get_detailed_metadata(str(photo), cache)

    assert read()["Exif.Image.Model"] == "original"
    assert read()["Exif.Image.Model"] == "original"
    assert extract.call_count == 1
    photo.write_bytes(b"new image contents")
    assert read()["Exif.Image.Model"] == "new image contents"
    assert read()["Exif.Image.Model"] == "new image contents"
    assert extract.call_count == 2


@pytest.mark.parametrize("batch", [False, True])
def test_file_changed_during_extraction_is_not_cached(cache, photo, monkeypatch, batch):
    def extract(path):
        photo.write_bytes(b"replacement during extraction")
        return {"file_path": path, "Exif.Image.Model": "old camera"}

    monkeypatch.setattr(
        "core.metadata_processor.PyExiv2Operations.get_comprehensive_metadata", extract
    )
    if batch:
        MetadataProcessor.get_batch_display_metadata(
            [str(photo)], exif_disk_cache=cache
        )
    else:
        MetadataProcessor.get_detailed_metadata(str(photo), cache)
    assert cache.get(str(photo)) is None
