import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from unittest.mock import Mock

import pytest
from PyQt6.QtWidgets import QApplication, QDialog, QPushButton

from ui.app_controller import AppController
from ui.dialog_manager import DialogManager
from ui.worker_manager import WorkerManager


_app = QApplication.instance() or QApplication([])


@pytest.mark.parametrize(
    "slot",
    [
        "grouping_preview_thread",
        "easy_delete_thread",
        "fix_rotation_detect_thread",
        "pick_best_thread",
        "cull_grouping_thread",
        "similarity_thread",
        "thumbnail_preload_thread",
        "scanner_thread",
    ],
)
def test_staying_preserves_work_across_all_workflows(slot):
    manager = WorkerManager(Mock())
    setattr(manager, slot, Mock())
    manager.request_stop_all_workers = Mock()
    state = Mock()
    window = Mock()
    window.dialog_manager.confirm_interrupt_for_folder_change.return_value = False
    controller = AppController(window, state, manager)
    consent = controller._model_consent
    consent.approved_downloads.add("aesthetic")
    controller._deferred_starts.arm("pick_best_scoring")
    controller._cull_prerequisites_declined = True

    controller.load_folder("/new-folder")

    window.dialog_manager.confirm_interrupt_for_folder_change.assert_called_once_with(
        "/new-folder"
    )
    manager.request_stop_all_workers.assert_not_called()
    state.clear_all_file_specific_data.assert_not_called()
    window.show_loading_overlay.assert_not_called()
    assert controller._model_consent is consent
    assert consent.approved_downloads == {"aesthetic"}
    assert controller._deferred_starts.is_armed("pick_best_scoring")
    assert controller._cull_prerequisites_declined is True
    assert controller._pending_folder_load_after_workers is None


def test_approved_change_waits_then_scans_once_without_prompting_again(monkeypatch):
    manager = WorkerManager(Mock())
    manager.pick_best_thread = Mock()
    manager.request_stop_all_workers = Mock()
    manager.start_file_scan = Mock()
    state = Mock()
    state.get_marked_files.return_value = []
    window = Mock()
    window._shutdown_in_progress = False
    window.dialog_manager.confirm_interrupt_for_folder_change.return_value = True
    controller = AppController(window, state, manager)
    callbacks = []
    monkeypatch.setattr(
        "ui.app_controller.QTimer.singleShot", lambda _ms, fn: callbacks.append(fn)
    )
    recent = Mock()
    monkeypatch.setattr("ui.app_controller.add_recent_folder", recent)

    controller.load_folder("/new-folder")
    manager.start_file_scan.assert_not_called()
    state.clear_all_file_specific_data.assert_not_called()
    callbacks.pop()()  # still stopping
    manager.start_file_scan.assert_not_called()
    manager.pick_best_thread = None
    callbacks.pop()()

    manager.start_file_scan.assert_called_once_with("/new-folder")
    state.clear_all_file_specific_data.assert_called_once_with()
    recent.assert_called_once_with("/new-folder")
    window.dialog_manager.confirm_interrupt_for_folder_change.assert_called_once()
    assert controller._pending_folder_load_after_workers is None


def test_cancelling_deletion_prompt_also_preserves_consent_and_queued_starts():
    manager = WorkerManager(Mock())
    manager.request_stop_all_workers = Mock()
    state = Mock()
    state.get_marked_files.return_value = ["/old/a.jpg"]
    window = Mock()
    window.dialog_manager.show_folder_change_confirmation_dialog.return_value = "cancel"
    controller = AppController(window, state, manager)
    consent = controller._model_consent
    controller._deferred_starts.arm("pick_best_scoring")

    controller.load_folder("/new-folder")

    assert controller._model_consent is consent
    assert controller._deferred_starts.is_armed("pick_best_scoring")
    manager.request_stop_all_workers.assert_not_called()


@pytest.mark.parametrize("accepted", [False, True])
def test_interrupt_dialog_has_safe_default_and_returns_choice(monkeypatch, accepted):
    def execute(dialog):
        assert dialog.objectName() == "folderInterruptDialog"
        stay = dialog.findChild(QPushButton, "modelConsentCancelButton")
        assert stay.text() == "Keep Working"
        assert stay.isDefault()
        stop = dialog.findChild(QPushButton, "modelConsentAcceptButton")
        assert not stop.isDefault()
        return QDialog.DialogCode.Accepted if accepted else QDialog.DialogCode.Rejected

    monkeypatch.setattr(QDialog, "exec", execute)
    assert (
        DialogManager(None).confirm_interrupt_for_folder_change("/new-folder")
        is accepted
    )
