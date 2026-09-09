import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

import json
from pathlib import Path

import pytest

from core.similarity_embedding_model import SimilarityModelNotInstalledError
from core.model_provisioning import EMBEDDING_MODEL
from core.subject_grouping_models import (
    CULL_DINO_MODEL,
    CULL_SUBJECT_MODEL_REVISIONS,
    resolve_subject_model_snapshots,
)


pytestmark = pytest.mark.usefixtures("inline_model_download")


def _snapshot(root: Path, model_id: str) -> str:
    path = root / model_id.replace("/", "-")
    path.mkdir()
    config = {"model_type": "dinov2"}
    (path / "config.json").write_text(json.dumps(config), encoding="utf-8")
    (path / "preprocessor_config.json").write_text("{}", encoding="utf-8")
    (path / "model.safetensors").write_bytes(b"safe-placeholder")
    return str(path)


def test_subject_downloads_are_revision_pinned_and_safe(monkeypatch, tmp_path):
    snapshots = {CULL_DINO_MODEL: _snapshot(tmp_path, CULL_DINO_MODEL)}
    calls = []

    def snapshot_download(model_id, **kwargs):
        calls.append((model_id, kwargs))
        return snapshots[model_id]

    monkeypatch.setattr("huggingface_hub.snapshot_download", snapshot_download)

    assert resolve_subject_model_snapshots(allow_download=False) == snapshots
    for model_id, kwargs in calls:
        assert kwargs["revision"] == CULL_SUBJECT_MODEL_REVISIONS[model_id]
        assert kwargs["local_files_only"] is True
        assert "*.safetensors" in kwargs["allow_patterns"]
        assert "*.bin" not in kwargs["allow_patterns"]
        assert "*.py" not in kwargs["allow_patterns"]


def test_invalid_subject_snapshot_fails_closed(monkeypatch, tmp_path):
    monkeypatch.setattr(
        "huggingface_hub.snapshot_download", lambda *_args, **_kwargs: str(tmp_path)
    )

    with pytest.raises(
        SimilarityModelNotInstalledError, match="has not been downloaded yet"
    ):
        resolve_subject_model_snapshots(allow_download=False)


def test_subject_download_reports_model_and_byte_progress(monkeypatch, tmp_path):
    snapshots = {CULL_DINO_MODEL: _snapshot(tmp_path, CULL_DINO_MODEL)}
    events = []

    def snapshot_download(model_id, **kwargs):
        if kwargs["local_files_only"]:
            raise FileNotFoundError(model_id)
        progress = kwargs["tqdm_class"](total=100, unit="B")
        progress.update(50)
        progress.close()
        return snapshots[model_id]

    monkeypatch.setattr("huggingface_hub.snapshot_download", snapshot_download)

    resolve_subject_model_snapshots(
        allow_download=True,
        progress_callback=lambda percent, message: events.append((percent, message)),
    )

    assert any(
        percent == -1 and f"Downloading {EMBEDDING_MODEL.label}" in message
        for percent, message in events
    )
    assert any("50 B / 100 B" in message for _percent, message in events)
    assert events[-1] == (100, f"Ready: {EMBEDDING_MODEL.label}")
