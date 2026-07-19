import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PyQt6.QtWidgets import QApplication, QCheckBox, QDialog, QWidget

from ui.dialog_manager import DialogManager


_app = QApplication.instance() or QApplication([])


def test_preferences_exposes_optional_workflow_step_controls(monkeypatch):
    captured: dict[str, QDialog] = {}

    def reject_dialog(dialog: QDialog):
        captured["dialog"] = dialog
        return QDialog.DialogCode.Rejected

    monkeypatch.setattr(QDialog, "exec", reject_dialog)
    parent = QWidget()
    DialogManager(parent).show_preferences_dialog()
    dialog = captured["dialog"]

    organize = dialog.findChild(QCheckBox, "showOrganizeStepCheckbox")
    easy_delete = dialog.findChild(QCheckBox, "showEasyDeleteStepCheckbox")
    fix_rotation = dialog.findChild(QCheckBox, "showFixRotationStepCheckbox")
    pick_best = dialog.findChild(QCheckBox, "showPickBestStepCheckbox")
    cull = dialog.findChild(QCheckBox, "showCullStepCheckbox")

    assert organize is not None and not organize.isEnabled()
    assert easy_delete is not None and easy_delete.isEnabled()
    assert fix_rotation is not None and fix_rotation.isEnabled()
    assert pick_best is not None and pick_best.isEnabled()
    assert cull is not None and not cull.isEnabled()
