"""Off-thread probe for the model execution environment.

Resolving Hugging Face snapshots and importing ``torch`` both take seconds on a
cold process, so they must never run on the GUI thread. Every workflow that needs
a model asks the same question - "what is missing, and what will run it?" - so
one probe answers it for all of them. The result is a stable per-process fact, so
the controller probes once and reuses the answer.
"""

from __future__ import annotations

from collections.abc import Sequence
import logging

from PyQt6.QtCore import QObject, pyqtSignal

logger = logging.getLogger(__name__)


class ModelEnvironmentProbeWorker(QObject):
    """Report which managed models are missing and which device will run them."""

    # (missing model keys, torch device)
    completed = pyqtSignal(tuple, str)
    finished = pyqtSignal()

    def __init__(self, model_keys: Sequence[str], parent: QObject | None = None):
        super().__init__(parent)
        self._model_keys = tuple(model_keys)

    def run(self) -> None:
        missing: tuple[str, ...] = ()
        device = "cpu"
        try:
            from core.app_settings import get_preferred_torch_device
            from core.model_provisioning import get_model, missing_models

            models = [get_model(key) for key in self._model_keys]
            missing = tuple(model.key for model in missing_models(models))
            device = get_preferred_torch_device()
        except Exception:
            # Fail closed. Claiming nothing is missing would skip the download
            # consent prompt and let the workflow start, only to die later with a
            # raw "model is not installed locally" error. Reporting everything as
            # missing means the user is asked, and the download can repair it.
            logger.exception(
                "Failed to probe the model environment; assuming %d model(s) missing.",
                len(self._model_keys),
            )
            missing = self._model_keys
            device = "cpu"
        else:
            logger.info(
                "Model environment probe: requested=%s missing=%s device=%s",
                self._model_keys,
                missing,
                device,
            )
        self.completed.emit(missing, device)
        self.finished.emit()
