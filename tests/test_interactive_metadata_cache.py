from unittest.mock import Mock

from core.metadata_processor import MetadataProcessor


def test_cached_detailed_metadata_never_falls_back_to_extraction(monkeypatch):
    cache = Mock()
    cached = {"camera": "cached"}
    cache.get.return_value = cached
    monkeypatch.setattr(
        MetadataProcessor,
        "_resolve_path_forms",
        staticmethod(lambda _path: ("operational.arw", "cache-key.arw")),
    )

    result = MetadataProcessor.get_cached_detailed_metadata("image.arw", cache)

    assert result == cached
    cache.get.assert_called_once_with("cache-key.arw")


def test_cached_detailed_metadata_returns_immediately_without_cache(monkeypatch):
    resolver = Mock()
    monkeypatch.setattr(
        MetadataProcessor,
        "_resolve_path_forms",
        staticmethod(resolver),
    )

    assert MetadataProcessor.get_cached_detailed_metadata("image.arw", None) is None
    resolver.assert_not_called()
