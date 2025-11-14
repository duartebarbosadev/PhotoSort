from ui.app_controller import AppController


def _make_fake_cache(label: str, calls: list[str]):
    class _Cache:
        def __init__(self):
            calls.append(f"{label}_init")

        def clear(self):
            calls.append(f"{label}_clear")

        def close(self):
            calls.append(f"{label}_close")

    return _Cache


def test_clear_application_caches_clears_every_cache(monkeypatch):
    calls: list[str] = []

    for module_path, class_name in (
        ("core.caching.thumbnail_cache", "ThumbnailCache"),
        ("core.caching.preview_cache", "PreviewCache"),
        ("core.caching.exif_cache", "ExifCache"),
        ("core.caching.rating_cache", "RatingCache"),
    ):
        label = class_name.replace("Cache", "").lower()
        fake_cls = _make_fake_cache(label, calls)
        monkeypatch.setattr(f"{module_path}.{class_name}", fake_cls)

    class FakeAnalysisCache:
        def __init__(self):
            calls.append("analysis_init")

        def clear_all(self):
            calls.append("analysis_clear_all")

        def close(self):
            calls.append("analysis_close")

    monkeypatch.setattr(
        "core.caching.analysis_cache.AnalysisCache",
        FakeAnalysisCache,
    )

    def fake_clear_embedding_cache():
        calls.append("similarity_clear_embeddings")

    monkeypatch.setattr(
        "core.similarity_engine.SimilarityEngine.clear_embedding_cache",
        staticmethod(fake_clear_embedding_cache),
    )

    AppController.clear_application_caches()

    assert "analysis_clear_all" in calls
    assert "similarity_clear_embeddings" in calls

    for cache_name in ("thumbnail", "preview", "exif", "rating"):
        assert f"{cache_name}_clear" in calls
        assert f"{cache_name}_close" in calls
