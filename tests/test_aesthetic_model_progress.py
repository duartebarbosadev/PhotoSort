import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

import json
from pathlib import Path

import pytest

from core.best_photo_finder.scorers import HuggingFaceAestheticScorer
from core.model_provisioning import AESTHETIC_MODEL, ModelNotInstalledError


def _snapshot(root: Path) -> str:
    path = root / "cafe-aesthetic"
    path.mkdir()
    (path / "config.json").write_text(json.dumps({"model_type": "beit"}), "utf-8")
    (path / "preprocessor_config.json").write_text("{}", encoding="utf-8")
    (path / "pytorch_model.bin").write_bytes(b"weights-placeholder")
    return str(path)


def _install_downloader(monkeypatch, downloader) -> None:
    monkeypatch.setattr(
        "core.model_provisioning._snapshot_download", lambda: downloader
    )


def test_cached_aesthetic_model_loads_without_downloading(monkeypatch, tmp_path):
    events: list[tuple[int, str]] = []
    calls: list[dict] = []
    snapshot = _snapshot(tmp_path)

    def downloader(repo_id, **kwargs):
        calls.append({"repo_id": repo_id, **kwargs})
        return snapshot

    _install_downloader(monkeypatch, downloader)
    scorer = HuggingFaceAestheticScorer(
        progress_callback=lambda percent, message: events.append((percent, message))
    )

    assert scorer._resolve_model_snapshot() == snapshot
    assert [call["local_files_only"] for call in calls] == [True]
    assert calls[0]["revision"] == AESTHETIC_MODEL.revision
    assert events == [(-1, f"Loading {AESTHETIC_MODEL.label}")]


def test_aesthetic_model_is_not_downloaded_without_consent(monkeypatch):
    calls: list[bool] = []

    def downloader(repo_id, **kwargs):
        calls.append(kwargs["local_files_only"])
        raise FileNotFoundError("not cached")

    _install_downloader(monkeypatch, downloader)
    scorer = HuggingFaceAestheticScorer()

    with pytest.raises(ModelNotInstalledError):
        scorer._resolve_model_snapshot()
    assert calls == [True], "an unapproved scorer must never reach the network"


def test_approved_aesthetic_model_downloads_after_cache_miss(monkeypatch, tmp_path):
    events: list[tuple[int, str]] = []
    calls: list[bool] = []
    snapshot = _snapshot(tmp_path)

    def downloader(repo_id, **kwargs):
        calls.append(kwargs["local_files_only"])
        if kwargs["local_files_only"]:
            raise FileNotFoundError("not cached")
        progress = kwargs["tqdm_class"](total=10, unit="B")
        progress.update(10)
        progress.close()
        return snapshot

    _install_downloader(monkeypatch, downloader)
    scorer = HuggingFaceAestheticScorer(
        allow_download=True,
        progress_callback=lambda percent, message: events.append((percent, message)),
    )

    assert scorer._resolve_model_snapshot() == snapshot
    assert calls == [True, False]
    assert any(
        f"Downloading {AESTHETIC_MODEL.label}" in message for _, message in events
    )
    assert events[-1] == (-1, f"Loading {AESTHETIC_MODEL.label}")
