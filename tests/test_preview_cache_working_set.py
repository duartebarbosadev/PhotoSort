import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from unittest.mock import patch

import pytest
from PIL import Image

from core.caching.preview_cache import PreviewCache, PreviewCacheCapacityError


def _cache(tmp_path, limit: int) -> PreviewCache:
    with patch.dict(
        PreviewCache.__init__.__globals__,
        {"get_preview_cache_size_bytes": lambda: limit},
    ):
        return PreviewCache(str(tmp_path / "preview"))


def test_active_folder_proxy_survives_cache_pressure(tmp_path):
    cache = _cache(tmp_path, 250_000)
    image = Image.new("RGB", (64, 64), "teal")
    old_key = ("old.jpg", "preview", 3)
    active_key = ("active.jpg", "preview", 3)

    with patch.dict(
        PreviewCache.set.__globals__,
        {"encode_cached_image": lambda *_args, **_kwargs: b"PSJ1" + (b"x" * 100_000)},
    ):
        cache.set(old_key, image)
        cache.begin_working_set(["active.jpg"])
        cache.set(active_key, image)
        cache.set(("incoming.jpg", "preview", 3), image)

    assert active_key in cache
    assert old_key not in cache


def test_cache_refuses_to_evict_protected_working_set(tmp_path):
    cache = _cache(tmp_path, 200_000)
    image = Image.new("RGB", (64, 64), "teal")
    first_key = ("first.jpg", "preview", 3)
    second_key = ("second.jpg", "preview", 3)
    cache.begin_working_set(["first.jpg", "second.jpg"])

    with patch.dict(
        PreviewCache.set.__globals__,
        {"encode_cached_image": lambda *_args, **_kwargs: b"PSJ1" + (b"x" * 120_000)},
    ):
        cache.set(first_key, image)
        with pytest.raises(PreviewCacheCapacityError):
            cache.set(second_key, image)

    assert first_key in cache
    assert second_key not in cache


def test_released_folder_entries_become_evictable(tmp_path):
    cache = _cache(tmp_path, 200_000)
    image = Image.new("RGB", (64, 64), "teal")
    first_key = ("first.jpg", "preview", 3)
    second_key = ("second.jpg", "preview", 3)

    with patch.dict(
        PreviewCache.set.__globals__,
        {"encode_cached_image": lambda *_args, **_kwargs: b"PSJ1" + (b"x" * 120_000)},
    ):
        cache.begin_working_set(["first.jpg"])
        cache.set(first_key, image)
        cache.end_working_set()
        cache.set(second_key, image)

    assert first_key not in cache
    assert second_key in cache


def test_unwritten_working_set_rejects_clear_and_limit_reduction(tmp_path):
    cache = _cache(tmp_path, 200_000)
    cache.begin_working_set(["active.jpg"])
    assert cache.protected_payload_bytes() == 0
    assert cache.clear() is False
    assert cache.set_size_limit(100_000) is False
    assert cache.size_limit_bytes == 200_000
    key = ("active.jpg", "preview", 3)
    written = cache.set(key, Image.new("RGB", (32, 32), "teal"))
    assert cache.protected_payload_bytes() == written > 0
    assert cache.clear() is False
    assert key in cache
    cache.end_working_set()
    assert cache.set_size_limit(100_000) is True
    assert cache.clear() is True
    assert cache.logical_payload_bytes() == 0


def test_reopen_rebuilds_sizes_without_reading_preview_payloads(tmp_path, monkeypatch):
    import diskcache

    directory = str(tmp_path / "preview")
    old_cache = diskcache.Cache(directory, disk_min_file_size=1024)
    small, large = ("small.jpg", "preview", 3), ("large.jpg", "preview", 3)
    old_cache.set(small, b"x" * 30)
    old_cache.set(large, b"x" * 300_000)
    old_cache.set("index_small.jpg", [small])
    old_cache.close()

    def forbid_payload_fetch(*_args, **_kwargs):
        raise AssertionError("Accounting must not fetch encoded previews")

    monkeypatch.setattr(diskcache.Disk, "fetch", forbid_payload_fetch)
    cache = _cache(tmp_path, 500_000)
    assert cache.payload_size(small) == 30
    assert cache.payload_size(large) == 300_000
    assert cache.logical_payload_bytes() == 300_030
    cache.begin_working_set(["large.jpg"])
    assert cache.protected_payload_bytes() == 300_000


def test_live_settings_keep_cache_open_during_preparation(tmp_path, monkeypatch):
    import threading

    cache = _cache(tmp_path, 200_000)
    cache.begin_working_set(["active.jpg"])
    started, release = threading.Event(), threading.Event()
    original_cache = cache._cache
    original_encode = PreviewCache.set.__globals__["encode_cached_image"]

    def encode(*args, **kwargs):
        started.set()
        assert release.wait(3)
        return original_encode(*args, **kwargs)

    monkeypatch.setitem(PreviewCache.set.__globals__, "encode_cached_image", encode)
    monkeypatch.setitem(
        PreviewCache.__init__.__globals__,
        "get_preview_cache_size_bytes",
        lambda: 400_000,
    )
    key = ("active.jpg", "preview", 3)
    results = []
    writer = threading.Thread(
        target=lambda: results.append(
            cache.set(key, Image.new("RGB", (32, 32), "teal"))
        )
    )
    writer.start()
    try:
        assert started.wait(1)
        assert cache.reinitialize_from_settings() is True
        assert cache._cache is original_cache
        assert cache.size_limit_bytes == 400_000
    finally:
        release.set()
        writer.join(3)
    assert results and results[0] > 0
    assert key in cache
    assert cache.protected_payload_bytes() == results[0]


@pytest.mark.parametrize("reader", ["protected_payload_bytes", "logical_payload_bytes"])
def test_accounting_readers_wait_for_inflight_mutation(tmp_path, reader):
    import threading

    cache = _cache(tmp_path, 200_000)
    started, finished = threading.Event(), threading.Event()
    values = []

    def read():
        started.set()
        values.append(getattr(cache, reader)())
        finished.set()

    with cache._capacity_lock:
        thread = threading.Thread(target=read)
        thread.start()
        assert started.wait(1)
        did_finish_while_locked = finished.wait(0.05)
        cache.begin_working_set(["active.jpg"])
        written = cache.set(("active.jpg", "preview", 3), Image.new("RGB", (16, 16)))
    thread.join(2)
    assert not did_finish_while_locked
    assert values == [written]


def test_thumbnail_clear_preserves_previews_and_rejects_active_folder(tmp_path):
    from unittest.mock import Mock
    from core.image_pipeline import ImagePipeline

    cache = _cache(tmp_path, 200_000)
    key = ("old.jpg", "preview", 3)
    cache.set(key, Image.new("RGB", (16, 16)))
    pipeline = ImagePipeline.__new__(ImagePipeline)
    pipeline.preview_cache = cache
    pipeline.thumbnail_cache = Mock()
    assert pipeline.clear_thumbnail_cache() is True
    assert key in cache
    pipeline.thumbnail_cache.clear.assert_called_once()
    pipeline.thumbnail_cache.clear.reset_mock()
    cache.begin_working_set(["new.jpg"])
    assert pipeline.clear_thumbnail_cache() is False
    assert pipeline.clear_all_image_caches() is False
    pipeline.thumbnail_cache.clear.assert_not_called()
    assert key in cache


def test_video_only_folder_protects_shared_preparation(tmp_path):
    from unittest.mock import Mock
    from core.image_pipeline import ImagePipeline

    pipeline = ImagePipeline.__new__(ImagePipeline)
    pipeline.preview_cache = _cache(tmp_path, 200_000)
    pipeline.thumbnail_cache = Mock()
    pipeline.begin_active_review_working_set(["clip.mp4"])
    assert pipeline.preview_cache.protected_payload_bytes() == 0
    assert pipeline.clear_thumbnail_cache() is False
    pipeline.thumbnail_cache.clear.assert_not_called()
    pipeline.end_active_review_working_set()
    assert pipeline.clear_thumbnail_cache() is True
