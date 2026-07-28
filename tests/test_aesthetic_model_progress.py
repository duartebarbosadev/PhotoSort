from core.best_photo_finder.scorers import HuggingFaceAestheticScorer


def test_cached_aesthetic_model_reports_loading_without_downloading(monkeypatch):
    events: list[tuple[int, str]] = []
    calls: list[dict] = []
    scorer = HuggingFaceAestheticScorer(
        progress_callback=lambda percent, message: events.append((percent, message))
    )
    monkeypatch.setattr(
        "core.best_photo_finder.scorers.get_huggingface_cache_dir",
        lambda: "/models",
    )

    def snapshot_download(model_name, **kwargs):
        calls.append({"model_name": model_name, **kwargs})
        return "/models/cached-snapshot"

    result = scorer._resolve_model_snapshot(snapshot_download)

    assert result == "/models/cached-snapshot"
    assert calls == [
        {
            "model_name": "cafeai/cafe_aesthetic",
            "cache_dir": "/models",
            "local_files_only": True,
        }
    ]
    assert events == [(-1, "Loading cafeai/cafe_aesthetic")]


def test_aesthetic_model_reports_download_only_after_cache_miss(monkeypatch):
    events: list[tuple[int, str]] = []
    calls: list[dict] = []
    scorer = HuggingFaceAestheticScorer(
        progress_callback=lambda percent, message: events.append((percent, message))
    )
    monkeypatch.setattr(
        "core.best_photo_finder.scorers.get_huggingface_cache_dir",
        lambda: "/models",
    )

    def snapshot_download(model_name, **kwargs):
        calls.append({"model_name": model_name, **kwargs})
        if kwargs["local_files_only"]:
            raise FileNotFoundError("not cached")
        progress = kwargs["tqdm_class"](total=10, unit="B")
        progress.update(10)
        progress.close()
        return "/models/downloaded-snapshot"

    result = scorer._resolve_model_snapshot(snapshot_download)

    assert result == "/models/downloaded-snapshot"
    assert [call["local_files_only"] for call in calls] == [True, False]
    assert any("Downloading cafeai/cafe_aesthetic" in message for _, message in events)
    assert events[-1] == (-1, "Loading cafeai/cafe_aesthetic")
