import pyexiv2  # noqa: F401  # Must be imported first to avoid Windows crashes

# Ensure root path (containing src/) is on sys.path for test imports
import sys
import os
import pytest

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

ROOT = os.path.dirname(os.path.abspath(__file__))
PROJECT_ROOT = os.path.abspath(os.path.join(ROOT, os.pardir))
if PROJECT_ROOT not in sys.path:
    sys.path.insert(0, PROJECT_ROOT)

# Also add the src/ directory for src-layout imports like `from src.core...`
SRC_ROOT = os.path.join(PROJECT_ROOT, "src")
if SRC_ROOT not in sys.path:
    sys.path.insert(0, SRC_ROOT)


@pytest.fixture
def inline_model_download(monkeypatch):
    """Keep provisioning contract fakes local; process lifecycle is tested separately."""
    from core import model_provisioning
    from core.huggingface_progress import build_hf_tqdm_class

    def download(repo_id, *, options, label, progress_callback=None, should_cancel=None):
        return model_provisioning._snapshot_download()(
            repo_id,
            local_files_only=False,
            tqdm_class=build_hf_tqdm_class(progress_callback, label=label),
            **options,
        )

    monkeypatch.setattr(model_provisioning, "download_snapshot", download)
