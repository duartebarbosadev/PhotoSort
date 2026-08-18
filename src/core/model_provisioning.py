"""One owner for every downloadable model PhotoSort uses.

Before this module each feature resolved its own weights: the similarity engine
downloaded an unpinned snapshot, Cull downloaded the *same* repository pinned to
an exact commit, and Pick Best downloaded its aesthetic model silently. That
meant several copies of the same weights on disk, several "is it installed?"
answers, and inconsistent consent and progress reporting.

Every model is now declared once in ``MODEL_REGISTRY`` and resolved through
``resolve_snapshot``, so download, revision pinning, validation, progress and the
installed check behave identically everywhere.
"""

from __future__ import annotations

from collections.abc import Callable, Iterable, Sequence
from dataclasses import dataclass
import json
import logging
from pathlib import Path
import shutil
import time

from core.app_settings import get_huggingface_cache_dir
from core.huggingface_progress import ProgressCallback, build_hf_tqdm_class

logger = logging.getLogger(__name__)


class ModelNotInstalledError(RuntimeError):
    """Raised when a managed model is not present in the local cache."""


class ModelDownloadError(RuntimeError):
    """Raised when a managed model cannot be downloaded or fails validation."""


# Configuration files every transformers snapshot needs. Weight patterns are
# declared per model so a repository only ever contributes the files we expect.
CONFIG_PATTERNS: tuple[str, ...] = (
    "*.json",
    "*.txt",
    "*.model",
    "*.yaml",
    "*.yml",
)
SAFETENSORS_PATTERN = "*.safetensors"
# Only used for pinned revisions of repositories that publish no safetensors.
# The revision pin is what makes the pickled checkpoint content-fixed.
PICKLED_WEIGHTS_PATTERN = "pytorch_model.bin"


def _transformers_validator(
    *,
    model_type: str | None,
    weight_glob: str,
    extra_files: Sequence[str] = (),
) -> Callable[[ManagedModel, Path], None]:
    """Build a validator asserting a snapshot holds the architecture we expect."""

    def validate(model: ManagedModel, snapshot: Path) -> None:
        config_path = snapshot / "config.json"
        if not snapshot.is_dir() or not config_path.is_file():
            raise ModelDownloadError(f"Model '{model.repo_id}' is incomplete.")
        if not any(snapshot.glob(weight_glob)):
            raise ModelDownloadError(f"Model '{model.repo_id}' has no usable weights.")
        try:
            config = json.loads(config_path.read_text(encoding="utf-8"))
        except (OSError, UnicodeDecodeError, json.JSONDecodeError) as exc:
            raise ModelDownloadError(
                f"Model '{model.repo_id}' has an invalid configuration."
            ) from exc
        if model_type is not None and config.get("model_type") != model_type:
            raise ModelDownloadError(
                f"Model '{model.repo_id}' failed architecture validation."
            )
        missing = [name for name in extra_files if not (snapshot / name).is_file()]
        if missing:
            raise ModelDownloadError(
                f"Model '{model.repo_id}' is missing {', '.join(missing)}."
            )

    return validate


@dataclass(frozen=True, slots=True)
class ManagedModel:
    """A downloadable model with a pinned identity and a validation contract."""

    key: str
    repo_id: str
    revision: str
    label: str
    validator: Callable[[ManagedModel, Path], None]
    # Size of the files we actually fetch, used to set expectations before a
    # download starts. Approximate on purpose: it never gates the download.
    approx_download_mb: int = 0
    allow_patterns: tuple[str, ...] = (*CONFIG_PATTERNS, SAFETENSORS_PATTERN)
    # Torch models run unacceptably slowly without an accelerator, so workflows
    # warn before starting long runs on CPU.
    warns_on_cpu: bool = True

    def validate(self, snapshot: str | Path) -> None:
        self.validator(self, Path(snapshot))


EMBEDDING_MODEL = ManagedModel(
    key="embedding",
    repo_id="facebook/dinov2-small",
    revision="ed25f3a31f01632728cabb09d1542f84ab7b0056",
    label="Same-subject and similarity model (DINOv2)",
    approx_download_mb=85,
    validator=_transformers_validator(
        model_type="dinov2",
        weight_glob=SAFETENSORS_PATTERN,
        extra_files=("preprocessor_config.json",),
    ),
)

# This repository publishes no safetensors, so the pinned revision is what makes
# its pickled checkpoint content-fixed.
AESTHETIC_MODEL = ManagedModel(
    key="aesthetic",
    repo_id="cafeai/cafe_aesthetic",
    revision="48a343764f786abc1ca8aaddfcd60a688a70da9b",
    label="Pick Best aesthetic scoring model",
    approx_download_mb=360,
    validator=_transformers_validator(
        model_type="beit",
        weight_glob=PICKLED_WEIGHTS_PATTERN,
        extra_files=("preprocessor_config.json",),
    ),
    allow_patterns=(*CONFIG_PATTERNS, PICKLED_WEIGHTS_PATTERN),
)

MODEL_REGISTRY: dict[str, ManagedModel] = {
    model.key: model for model in (EMBEDDING_MODEL, AESTHETIC_MODEL)
}


def get_model(key: str) -> ManagedModel:
    try:
        return MODEL_REGISTRY[key]
    except KeyError as exc:
        raise KeyError(f"Unknown managed model '{key}'.") from exc


def _snapshot_download():
    try:
        from huggingface_hub import snapshot_download
    except ImportError as exc:  # pragma: no cover - packaging failure
        raise ModelDownloadError(
            "Missing dependency 'huggingface_hub'. Install PhotoSort dependencies "
            "and try again."
        ) from exc
    return snapshot_download


def resolve_snapshot(
    model: ManagedModel,
    *,
    allow_download: bool = False,
    progress_callback: ProgressCallback | None = None,
) -> str:
    """Return a validated local snapshot path for one managed model.

    The first lookup is always local-only. The network is used only when the
    caller explicitly allows it, which happens after the user consents.
    """

    download = _snapshot_download()
    common = {
        "revision": model.revision,
        "cache_dir": get_huggingface_cache_dir(),
        "allow_patterns": list(model.allow_patterns),
    }

    try:
        snapshot = download(model.repo_id, local_files_only=True, **common)
        model.validate(snapshot)
        logger.debug("Model %s resolved from local cache: %s", model.repo_id, snapshot)
        return snapshot
    except Exception as local_error:
        # A corrupt or incomplete local snapshot is treated as "not installed"
        # so that an approved run simply re-fetches it instead of dead-ending.
        if not allow_download:
            logger.debug(
                "Model %s is not usable locally: %s", model.repo_id, local_error
            )
            raise ModelNotInstalledError(
                f"The {model.label} has not been downloaded yet."
            ) from local_error
        # Distinguish a first-time download from a re-fetch caused by a damaged
        # snapshot: only the latter is worth a warning in a support log.
        logger.warning(
            "Local snapshot for %s is missing or unusable (%s); downloading again.",
            model.repo_id,
            local_error,
        )

    logger.info("Downloading model snapshot: %s@%s", model.repo_id, model.revision)
    if progress_callback:
        progress_callback(-1, f"Downloading {model.label}")
    download_started = time.perf_counter()
    try:
        snapshot = download(
            model.repo_id,
            local_files_only=False,
            tqdm_class=build_hf_tqdm_class(
                progress_callback, label=f"Downloading {model.label}"
            ),
            **common,
        )
    except Exception as download_error:
        logger.error(
            "Failed to download model %s@%s: %s",
            model.repo_id,
            model.revision,
            download_error,
        )
        raise ModelDownloadError(
            f"Could not download model '{model.repo_id}'. Check your internet "
            "connection and try again."
        ) from download_error
    # Validation errors are raised as-is: a freshly downloaded snapshot that
    # fails inspection is an integrity problem, not a connectivity problem.
    model.validate(snapshot)
    logger.info(
        "Downloaded model %s in %.2fs (%.1f MB) to %s",
        model.repo_id,
        time.perf_counter() - download_started,
        _directory_size_bytes(Path(snapshot)) / (1024 * 1024),
        snapshot,
    )
    if progress_callback:
        progress_callback(100, f"Ready: {model.label}")
    return snapshot


def is_installed(model: ManagedModel) -> bool:
    """Report whether one managed model is usable without network access."""

    try:
        resolve_snapshot(model, allow_download=False)
        return True
    except ModelNotInstalledError, ModelDownloadError:
        return False
    except Exception:
        logger.exception("Failed to check local snapshot for %s.", model.repo_id)
        return False


def missing_models(models: Iterable[ManagedModel]) -> tuple[ManagedModel, ...]:
    """Return the subset of models that still need downloading, in order."""

    return tuple(model for model in models if not is_installed(model))


def _model_cache_root(cache_dir: str | Path | None) -> Path:
    return (
        Path(cache_dir) if cache_dir is not None else Path(get_huggingface_cache_dir())
    )


def _iter_model_cache_dirs(root: Path) -> Iterable[Path]:
    """Yield only the snapshot directories Hugging Face itself created."""

    if not root.is_dir():
        return
    for entry in sorted(root.iterdir()):
        if entry.is_dir() and entry.name.startswith("models--"):
            yield entry


def _directory_size_bytes(directory: Path) -> int:
    """Return the total size of regular files under a directory."""

    total = 0
    for path in directory.rglob("*"):
        try:
            if path.is_file() and not path.is_symlink():
                total += path.stat().st_size
        except OSError:
            continue
    return total


def _remove_model_cache_dirs(root: Path) -> tuple[str, ...]:
    removed: list[str] = []
    freed_bytes = 0
    for entry in _iter_model_cache_dirs(root):
        entry_bytes = _directory_size_bytes(entry)
        try:
            shutil.rmtree(entry)
        except OSError:
            logger.exception("Failed to remove model cache %s.", entry)
            continue
        logger.info(
            "Removed model cache %s (%.1f MB).", entry.name, entry_bytes / (1024 * 1024)
        )
        freed_bytes += entry_bytes
        removed.append(entry.name)
    if removed:
        logger.info(
            "Removed %d model cache(s) from %s, freeing %.1f MB.",
            len(removed),
            root,
            freed_bytes / (1024 * 1024),
        )
    else:
        logger.info("No model caches to remove in %s.", root)
    return tuple(removed)


def model_cache_usage_bytes(cache_dir: str | Path | None = None) -> int:
    """Return the disk space used by downloaded model snapshots."""

    return sum(
        _directory_size_bytes(entry)
        for entry in _iter_model_cache_dirs(_model_cache_root(cache_dir))
    )


def clear_model_caches(cache_dir: str | Path | None = None) -> tuple[str, ...]:
    """Delete every downloaded model, forcing a fresh download on next use.

    This is destructive: the models are re-downloaded the next time a feature
    needs them, so callers must confirm with the user first. Only directories
    that Hugging Face itself created are touched.
    """

    return _remove_model_cache_dirs(_model_cache_root(cache_dir))
