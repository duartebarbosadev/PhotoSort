import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from PyQt6.QtWidgets import (
    QApplication,
    QDialog,
    QLabel,
    QPushButton,
    QWidget,
)

from core.model_provisioning import AESTHETIC_MODEL, EMBEDDING_MODEL
from ui.dialog_manager import DialogManager


_app = QApplication.instance() or QApplication([])
# Parented dialogs die with their parent, so the owner must outlive the test.
_parent = QWidget()


def _capture(monkeypatch, result=QDialog.DialogCode.Rejected):
    captured: dict[str, QDialog] = {}

    def _exec(dialog: QDialog):
        captured["dialog"] = dialog
        return result

    monkeypatch.setattr(QDialog, "exec", _exec)
    return captured


def _texts(dialog: QDialog) -> str:
    return "\n".join(label.text() for label in dialog.findChildren(QLabel))


def test_download_prompt_names_every_model_and_its_repository(monkeypatch):
    """Users must see exactly which weights are fetched, and from where."""
    captured = _capture(monkeypatch)

    DialogManager(_parent).confirm_model_download(
        [EMBEDDING_MODEL.key, AESTHETIC_MODEL.key],
        feature="Pick Best scoring",
    )

    body = _texts(captured["dialog"])
    for model in (EMBEDDING_MODEL, AESTHETIC_MODEL):
        assert model.label in body
        assert model.repo_id in body
    assert "Pick Best scoring" in body
    total_mb = EMBEDDING_MODEL.approx_download_mb + AESTHETIC_MODEL.approx_download_mb
    assert f"About {total_mb} MB" in body


def test_download_prompt_uses_the_styled_dialog_shell(monkeypatch):
    """The generic QMessageBox look (giant '?' icon) is not acceptable here."""
    captured = _capture(monkeypatch)

    DialogManager(_parent).confirm_model_download(
        [EMBEDDING_MODEL.key], feature="visual similarity grouping"
    )

    dialog = captured["dialog"]
    assert dialog.objectName() == "modelDownloadDialog"
    assert dialog.findChild(QPushButton, "modelConsentAcceptButton") is not None
    assert dialog.findChild(QPushButton, "modelConsentCancelButton") is not None
    assert dialog.findChild(QLabel, "dialogHeaderTitle") is not None


def test_download_prompt_includes_the_workflow_fallback(monkeypatch):
    captured = _capture(monkeypatch)

    DialogManager(_parent).confirm_model_download(
        [EMBEDDING_MODEL.key],
        feature="same-subject grouping",
        fallback="If you cancel, Cull remains available without similarity groups.",
    )

    assert "Cull remains available" in _texts(captured["dialog"])


def test_download_prompt_returns_the_users_answer(monkeypatch):
    _capture(monkeypatch, QDialog.DialogCode.Accepted)
    assert (
        DialogManager(_parent).confirm_model_download(
            [EMBEDDING_MODEL.key], feature="Pick Best scoring"
        )
        is True
    )

    _capture(monkeypatch, QDialog.DialogCode.Rejected)
    assert (
        DialogManager(_parent).confirm_model_download(
            [EMBEDDING_MODEL.key], feature="Pick Best scoring"
        )
        is False
    )


def test_cpu_warning_shares_the_same_styled_shell(monkeypatch):
    captured = _capture(monkeypatch, QDialog.DialogCode.Accepted)

    approved = DialogManager(_parent).confirm_slow_cpu_processing("Pick Best scoring")

    dialog = captured["dialog"]
    assert approved is True
    assert dialog.objectName() == "modelConsentDialog"
    assert dialog.findChild(QPushButton, "modelConsentAcceptButton") is not None
    assert "Pick Best scoring" in _texts(dialog)


def test_single_model_prompt_reports_size_once(monkeypatch):
    """Repeating the size in the summary and the card reads like a bug."""
    captured = _capture(monkeypatch)

    DialogManager(_parent).confirm_model_download(
        [AESTHETIC_MODEL.key], feature="Pick Best scoring"
    )

    body = _texts(captured["dialog"])
    assert body.count(f"{AESTHETIC_MODEL.approx_download_mb} MB") == 1
    assert "in total" not in body
