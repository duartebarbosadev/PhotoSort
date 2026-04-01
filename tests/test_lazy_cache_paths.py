import importlib
import sys
from unittest.mock import Mock


def _reload_module(module_name: str):
    sys.modules.pop(module_name, None)
    return importlib.import_module(module_name)


def test_cache_modules_do_not_resolve_cache_dirs_at_import_time(monkeypatch):
    runtime_paths = importlib.import_module("core.runtime_paths")
    resolver = Mock(side_effect=AssertionError("resolver should not run during import"))
    monkeypatch.setattr(runtime_paths, "resolve_user_cache_dir", resolver)

    _reload_module("core.caching.thumbnail_cache")
    _reload_module("core.caching.rating_cache")
    _reload_module("core.caching.exif_cache")
    _reload_module("core.caching.preview_cache")
    _reload_module("core.caching.analysis_cache")
    _reload_module("core.app_settings")

    assert resolver.call_count == 0


def test_cache_classes_resolve_default_dirs_lazily(monkeypatch, tmp_path):
    thumbnail_cache = importlib.import_module("core.caching.thumbnail_cache")
    rating_cache = importlib.import_module("core.caching.rating_cache")
    exif_cache = importlib.import_module("core.caching.exif_cache")
    preview_cache = importlib.import_module("core.caching.preview_cache")
    analysis_cache = importlib.import_module("core.caching.analysis_cache")

    created_paths = []

    def resolver(subdir: str) -> str:
        path = tmp_path / subdir
        created_paths.append(str(path))
        return str(path)

    monkeypatch.setattr(thumbnail_cache, "resolve_user_cache_dir", resolver)
    monkeypatch.setattr(rating_cache, "resolve_user_cache_dir", resolver)
    monkeypatch.setattr(exif_cache, "resolve_user_cache_dir", resolver)
    monkeypatch.setattr(preview_cache, "resolve_user_cache_dir", resolver)
    monkeypatch.setattr(analysis_cache, "resolve_user_cache_dir", resolver)

    thumb = thumbnail_cache.ThumbnailCache()
    rating = rating_cache.RatingCache()
    exif = exif_cache.ExifCache()
    preview = preview_cache.PreviewCache()
    analysis = analysis_cache.AnalysisCache()

    assert any(path.endswith("photosort_thumbnails") for path in created_paths)
    assert any(path.endswith("photosort_ratings") for path in created_paths)
    assert any(path.endswith("photosort_exif_data") for path in created_paths)
    assert any(path.endswith("photosort_preview_pil_images") for path in created_paths)
    assert any(path.endswith("photosort_analysis") for path in created_paths)

    thumb.close()
    rating.close()
    exif.close()
    preview.close()
    analysis.close()


def test_sentence_transformers_cache_dir_is_resolved_lazily(monkeypatch, tmp_path):
    app_settings = _reload_module("core.app_settings")
    resolver = Mock(return_value=str(tmp_path / "sentence-transformers"))
    monkeypatch.setattr(app_settings, "resolve_user_cache_dir", resolver)

    cache_dir = app_settings.get_sentence_transformers_cache_dir()

    assert cache_dir == str(tmp_path / "sentence-transformers")
    resolver.assert_called_once_with("photosort_hf/sentence-transformers")
