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
