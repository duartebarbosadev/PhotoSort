"""Dependency checker for the IQA-based best-shot pipeline."""

from __future__ import annotations

import importlib.util
import logging
from dataclasses import dataclass
from typing import List, Optional

logger = logging.getLogger(__name__)


class ModelDependencyError(Exception):
    """Raised when one or more required dependencies are missing."""

    def __init__(self, missing_models: List["MissingModelInfo"]):
        self.missing_models = missing_models
        super().__init__(self._format_message())

    def _format_message(self) -> str:
        model_names = ", ".join(m.name for m in self.missing_models)
        return f"Required best-shot dependencies not found: {model_names}"


@dataclass
class MissingModelInfo:
    """Information about a missing runtime dependency."""

    name: str
    description: str
    expected_path: str
    download_url: str


def _module_available(module_name: str) -> bool:
    return importlib.util.find_spec(module_name) is not None


def _dependency_catalog() -> List[tuple[str, MissingModelInfo]]:
    return [
        (
            "torch",
            MissingModelInfo(
                name="PyTorch",
                description=(
                    "Deep learning runtime required by the MUSIQ/MANIQA/LIQE pipeline."
                ),
                expected_path="pip install torch --extra-index-url https://download.pytorch.org/whl/cpu",
                download_url="https://pytorch.org/get-started/locally/",
            ),
        ),
        (
            "pyiqa",
            MissingModelInfo(
                name="pyiqa (MUSIQ/MANIQA/LIQE)",
                description=(
                    "Python Image Quality Assessment package that bundles the"
                    " MUSIQ, MANIQA, and LIQE checkpoints."
                ),
                expected_path="pip install pyiqa",
                download_url="https://github.com/chaofengc/IQA-PyTorch",
            ),
        ),
    ]


def check_best_shot_models(models_root: Optional[str] = None) -> List[MissingModelInfo]:
    """Ensure the IQA pipeline dependencies are present.

    Args:
        models_root: Legacy argument for backwards compatibility (no longer used).
    """

    if models_root:
        logger.debug(
            "models_root argument is ignored for the IQA pipeline: %s", models_root
        )

    missing: List[MissingModelInfo] = []
    for module_name, info in _dependency_catalog():
        if not _module_available(module_name):
            missing.append(info)

    if missing:
        logger.warning(
            "Best-shot dependency check failed: %d missing item(s)",
            len(missing),
        )
    else:
        logger.info("All IQA dependencies detected for best-shot analysis.")

    return missing


def ensure_best_shot_models(models_root: Optional[str] = None) -> None:
    missing = check_best_shot_models(models_root)
    if missing:
        raise ModelDependencyError(missing)


__all__ = [
    "ModelDependencyError",
    "MissingModelInfo",
    "check_best_shot_models",
    "ensure_best_shot_models",
]
