import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

import json
import logging
import re
from pathlib import Path

import pytest

from core import model_provisioning
from core.model_provisioning import (
    AESTHETIC_MODEL,
    EMBEDDING_MODEL,
    MODEL_REGISTRY,
    PICKLED_WEIGHTS_PATTERN,
    SAFETENSORS_PATTERN,
    ModelDownloadError,
    ModelNotInstalledError,
    get_model,
    is_installed,
    missing_models,
    resolve_snapshot,
)


pytestmark = pytest.mark.usefixtures("inline_model_download")


def _valid_snapshot(root: Path, model=EMBEDDING_MODEL, *, model_type="dinov2") -> Path:
    """Build a directory that satisfies the model's validation contract."""

    path = root / model.repo_id.replace("/", "-")
    path.mkdir(parents=True, exist_ok=True)
    (path / "config.json").write_text(
        json.dumps({"model_type": model_type}), encoding="utf-8"
    )
    (path / "preprocessor_config.json").write_text("{}", encoding="utf-8")
    weight = (
        "model.safetensors" if model is EMBEDDING_MODEL else PICKLED_WEIGHTS_PATTERN
    )
    (path / weight).write_bytes(b"weights-placeholder")
    return path


class FakeDownloader:
    """Stand-in for huggingface_hub.snapshot_download recording every call."""

    def __init__(self, *, local_result=None, remote_result=None):
        self.local_result = local_result
        self.remote_result = remote_result
        self.calls = []

    def __call__(self, repo_id, **kwargs):
        self.calls.append((repo_id, kwargs))
        result = (
            self.local_result if kwargs.get("local_files_only") else self.remote_result
        )
        if isinstance(result, Exception):
            raise result
        if result is None:
            raise OSError("snapshot unavailable")
        return str(result)

    @property
    def local_only_flags(self):
        return [kwargs.get("local_files_only") for _repo, kwargs in self.calls]


@pytest.fixture
def install_downloader(monkeypatch):
    def install(downloader):
        monkeypatch.setattr(
            model_provisioning, "_snapshot_download", lambda: downloader
        )
        return downloader

    return install


def test_local_snapshot_is_used_without_any_download(install_downloader, tmp_path):
    snapshot = _valid_snapshot(tmp_path)
    downloader = install_downloader(FakeDownloader(local_result=snapshot))

    assert resolve_snapshot(EMBEDDING_MODEL, allow_download=True) == str(snapshot)
    assert downloader.local_only_flags == [True]


def test_missing_local_snapshot_without_consent_never_hits_network(install_downloader):
    downloader = install_downloader(FakeDownloader())

    with pytest.raises(ModelNotInstalledError, match="has not been downloaded yet"):
        resolve_snapshot(EMBEDDING_MODEL, allow_download=False)

    assert downloader.local_only_flags == [True]


def test_download_is_attempted_only_after_consent(install_downloader, tmp_path):
    snapshot = _valid_snapshot(tmp_path)
    downloader = install_downloader(FakeDownloader(remote_result=snapshot))

    assert resolve_snapshot(EMBEDDING_MODEL, allow_download=True) == str(snapshot)
    assert downloader.local_only_flags == [True, False]


def test_failed_download_raises_model_download_error(install_downloader):
    install_downloader(FakeDownloader(remote_result=OSError("network down")))

    with pytest.raises(ModelDownloadError, match="Could not download"):
        resolve_snapshot(EMBEDDING_MODEL, allow_download=True)


def test_every_call_pins_revision_and_allow_patterns(install_downloader, tmp_path):
    snapshot = _valid_snapshot(tmp_path, AESTHETIC_MODEL, model_type="beit")
    downloader = install_downloader(FakeDownloader(remote_result=snapshot))

    resolve_snapshot(AESTHETIC_MODEL, allow_download=True)

    assert len(downloader.calls) == 2
    for repo_id, kwargs in downloader.calls:
        assert repo_id == AESTHETIC_MODEL.repo_id
        assert kwargs["revision"] == AESTHETIC_MODEL.revision
        assert kwargs["allow_patterns"] == list(AESTHETIC_MODEL.allow_patterns)
        assert kwargs["cache_dir"]


def test_progress_callback_reports_download_start_and_completion(
    install_downloader, tmp_path
):
    snapshot = _valid_snapshot(tmp_path)
    install_downloader(FakeDownloader(remote_result=snapshot))
    events = []

    resolve_snapshot(
        EMBEDDING_MODEL,
        allow_download=True,
        progress_callback=lambda percent, message: events.append((percent, message)),
    )

    assert events[0] == (-1, f"Downloading {EMBEDDING_MODEL.label}")
    assert events[-1] == (100, f"Ready: {EMBEDDING_MODEL.label}")


@pytest.mark.parametrize("removed", ["config.json", "model.safetensors"])
def test_incomplete_downloaded_snapshot_fails_validation(
    install_downloader, tmp_path, removed
):
    snapshot = _valid_snapshot(tmp_path)
    (snapshot / removed).unlink()
    install_downloader(FakeDownloader(remote_result=snapshot))

    # A downloaded but unusable snapshot is an integrity problem, so the error
    # must not misreport it as a connectivity problem.
    with pytest.raises(ModelDownloadError) as error:
        resolve_snapshot(EMBEDDING_MODEL, allow_download=True)
    assert "internet" not in str(error.value)


def test_wrong_architecture_snapshot_fails_validation(install_downloader, tmp_path):
    snapshot = _valid_snapshot(tmp_path, model_type="resnet")
    install_downloader(FakeDownloader(remote_result=snapshot))

    with pytest.raises(ModelDownloadError, match="architecture validation"):
        resolve_snapshot(EMBEDDING_MODEL, allow_download=True)


def test_valid_snapshot_passes_validate_directly(tmp_path):
    EMBEDDING_MODEL.validate(_valid_snapshot(tmp_path))


def test_is_installed_reflects_local_availability(install_downloader, tmp_path):
    snapshot = _valid_snapshot(tmp_path)

    install_downloader(FakeDownloader(local_result=snapshot))
    assert is_installed(EMBEDDING_MODEL) is True

    install_downloader(FakeDownloader())
    assert is_installed(EMBEDDING_MODEL) is False


@pytest.mark.parametrize("error", [ModelNotInstalledError, ModelDownloadError])
def test_is_installed_swallows_provisioning_errors(monkeypatch, error):
    def raiser(*_args, **_kwargs):
        raise error("boom")

    monkeypatch.setattr(model_provisioning, "resolve_snapshot", raiser)

    assert is_installed(EMBEDDING_MODEL) is False


def test_missing_models_preserves_order(monkeypatch):
    monkeypatch.setattr(
        model_provisioning,
        "is_installed",
        lambda model: model is EMBEDDING_MODEL,
    )

    assert missing_models([AESTHETIC_MODEL, EMBEDDING_MODEL]) == (AESTHETIC_MODEL,)


def test_registry_is_keyed_by_model_key():
    assert MODEL_REGISTRY == {
        "embedding": EMBEDDING_MODEL,
        "aesthetic": AESTHETIC_MODEL,
    }
    assert get_model("embedding") is EMBEDDING_MODEL

    with pytest.raises(KeyError, match="Unknown managed model"):
        get_model("nope")


def test_embedding_model_is_pinned_safetensors_dinov2():
    assert EMBEDDING_MODEL.repo_id == "facebook/dinov2-small"
    assert re.fullmatch(r"[0-9a-f]{40}", EMBEDDING_MODEL.revision)
    assert SAFETENSORS_PATTERN in EMBEDDING_MODEL.allow_patterns
    assert PICKLED_WEIGHTS_PATTERN not in EMBEDDING_MODEL.allow_patterns


def test_aesthetic_model_allows_pickled_weights_because_repo_has_no_safetensors():
    assert AESTHETIC_MODEL.repo_id == "cafeai/cafe_aesthetic"
    assert re.fullmatch(r"[0-9a-f]{40}", AESTHETIC_MODEL.revision)
    assert PICKLED_WEIGHTS_PATTERN in AESTHETIC_MODEL.allow_patterns
    assert SAFETENSORS_PATTERN not in AESTHETIC_MODEL.allow_patterns


def test_clear_model_caches_leaves_non_model_directories_alone(tmp_path):
    from core.model_provisioning import clear_model_caches

    model_dir = tmp_path / "models--facebook--dinov2-small" / "snapshots"
    model_dir.mkdir(parents=True)
    (model_dir / "weights.bin").write_bytes(b"x")
    unrelated = tmp_path / "previews"
    unrelated.mkdir()
    tag = tmp_path / "CACHEDIR.TAG"
    tag.write_text("Signature")

    removed = clear_model_caches(tmp_path)

    assert removed == ("models--facebook--dinov2-small",)
    # Only Hugging Face's own directories are eligible for deletion.
    assert unrelated.exists()
    assert tag.exists()


def test_clear_model_caches_is_a_noop_when_cache_dir_is_absent(tmp_path):
    from core.model_provisioning import clear_model_caches

    assert clear_model_caches(tmp_path / "missing") == ()


def _make_model_dir(root: Path, name: str, payload: bytes = b"weights") -> Path:
    directory = root / name / "snapshots"
    directory.mkdir(parents=True)
    (directory / "model.bin").write_bytes(payload)
    return root / name


def test_clear_model_caches_removes_managed_models_too(tmp_path):
    from core.model_provisioning import EMBEDDING_MODEL, clear_model_caches

    managed = _make_model_dir(
        tmp_path, "models--" + EMBEDDING_MODEL.repo_id.replace("/", "--")
    )
    stale = _make_model_dir(tmp_path, "models--facebook--dinov2-base")
    unrelated = tmp_path / "previews"
    unrelated.mkdir()

    removed = clear_model_caches(tmp_path)

    assert set(removed) == {managed.name, stale.name}
    assert not managed.exists()
    assert not stale.exists()
    # Non-model directories must survive: this clears models, not caches.
    assert unrelated.exists()


def test_model_cache_usage_counts_only_model_directories(tmp_path):
    from core.model_provisioning import model_cache_usage_bytes

    _make_model_dir(tmp_path, "models--facebook--dinov2-small", b"x" * 100)
    _make_model_dir(tmp_path, "models--cafeai--cafe_aesthetic", b"y" * 50)
    other = tmp_path / "previews"
    other.mkdir()
    (other / "ignored.bin").write_bytes(b"z" * 999)

    assert model_cache_usage_bytes(tmp_path) == 150


def test_model_cache_usage_is_zero_when_nothing_downloaded(tmp_path):
    from core.model_provisioning import model_cache_usage_bytes

    assert model_cache_usage_bytes(tmp_path / "missing") == 0


def test_corrupt_local_snapshot_is_logged_before_redownloading(
    tmp_path, caplog, monkeypatch
):
    """A silent re-download is a support blind spot, so it must be logged."""

    from core.model_provisioning import EMBEDDING_MODEL, resolve_snapshot

    snapshot = tmp_path / "snapshot"
    snapshot.mkdir()

    def fake_download(repo_id, *, local_files_only=False, **kwargs):
        if local_files_only:
            raise OSError("snapshot is incomplete")
        (snapshot / "config.json").write_text(json.dumps({"model_type": "dinov2"}))
        (snapshot / "preprocessor_config.json").write_text("{}")
        (snapshot / "model.safetensors").write_bytes(b"weights")
        return str(snapshot)

    monkeypatch.setattr(model_provisioning, "_snapshot_download", lambda: fake_download)
    with caplog.at_level(logging.WARNING, logger="core.model_provisioning"):
        resolve_snapshot(EMBEDDING_MODEL, allow_download=True)

    warnings = [r for r in caplog.records if r.levelno == logging.WARNING]
    assert any("missing or unusable" in r.getMessage() for r in warnings)


def test_removal_logs_a_summary_with_space_freed(tmp_path, caplog):
    from core.model_provisioning import clear_model_caches

    directory = tmp_path / "models--old--retired" / "snapshots"
    directory.mkdir(parents=True)
    (directory / "model.bin").write_bytes(b"x" * (2 * 1024 * 1024))

    with caplog.at_level(logging.INFO, logger="core.model_provisioning"):
        clear_model_caches(tmp_path)

    messages = [r.getMessage() for r in caplog.records]
    assert any("freeing 2.0 MB" in message for message in messages)


def test_removal_logs_when_there_is_nothing_to_remove(tmp_path, caplog):
    from core.model_provisioning import clear_model_caches

    with caplog.at_level(logging.INFO, logger="core.model_provisioning"):
        clear_model_caches(tmp_path)

    messages = [r.getMessage() for r in caplog.records]
    assert any("No model caches to remove" in message for message in messages)
