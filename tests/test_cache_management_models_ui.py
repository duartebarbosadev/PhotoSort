import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from unittest.mock import Mock

from PyQt6.QtWidgets import QApplication, QDialog, QPushButton, QWidget

from ui.dialog_manager import DialogManager


_app = QApplication.instance() or QApplication([])

_CACHE_ACTIONS = (
    "_update_cache_dialog_labels",
    "_clear_thumbnail_cache_action",
    "_clear_preview_cache_action",
    "_clear_analysis_cache_action",
    "_apply_preview_cache_limit_action",
    "_clear_exif_cache_action",
    "_apply_exif_cache_limit_action",
    "_clear_downloaded_models_action",
)


def _open_cache_dialog(monkeypatch, dialog_manager_factory=None):
    captured: dict[str, QDialog] = {}

    def reject_dialog(dialog: QDialog):
        captured["dialog"] = dialog
        return QDialog.DialogCode.Rejected

    monkeypatch.setattr(QDialog, "exec", reject_dialog)
    parent = QWidget()
    for name in _CACHE_ACTIONS:
        setattr(parent, name, Mock())

    manager = DialogManager(parent)
    if dialog_manager_factory is not None:
        dialog_manager_factory(manager)
    manager.show_cache_management_dialog()
    return parent, manager, captured["dialog"]


def test_cache_dialog_exposes_model_management_controls(monkeypatch):
    _parent, _manager, dialog = _open_cache_dialog(monkeypatch)

    assert dialog.findChild(QPushButton, "deleteDownloadedModelsButton") is not None


def test_deleting_all_models_is_skipped_when_the_user_cancels(monkeypatch):
    parent, _manager, dialog = _open_cache_dialog(
        monkeypatch,
        lambda manager: setattr(
            manager, "confirm_model_cache_deletion", Mock(return_value=False)
        ),
    )

    dialog.findChild(QPushButton, "deleteDownloadedModelsButton").click()

    # Deleting models forces a large re-download, so cancelling must be a no-op.
    parent._clear_downloaded_models_action.assert_not_called()


def test_deleting_all_models_proceeds_once_confirmed(monkeypatch):
    parent, _manager, dialog = _open_cache_dialog(
        monkeypatch,
        lambda manager: setattr(
            manager, "confirm_model_cache_deletion", Mock(return_value=True)
        ),
    )

    dialog.findChild(QPushButton, "deleteDownloadedModelsButton").click()

    parent._clear_downloaded_models_action.assert_called_once_with()


def test_model_management_slots_exist_on_the_main_window():
    from ui.main_window import MainWindow

    # The dialog connects to these by name, so a rename must fail loudly here.
    assert callable(MainWindow._clear_downloaded_models_action)


def test_deleting_models_makes_the_next_run_re_check_what_is_installed():
    """Deleting the weights must invalidate the cached "what is missing" answer.

    ``AppController._model_environment`` is resolved once per process. If models
    are deleted behind its back it still reports nothing missing, so the next
    Pick Best or similarity run starts without download consent and dies with
    "has not been downloaded yet" instead of offering the download.
    """

    from ui.controllers.cache_controller import CacheController

    class _AppController:
        def __init__(self):
            self.resets = 0

        def _reset_model_environment(self):
            self.resets += 1

    class _Context:
        def __init__(self):
            self.app_controller = _AppController()
            self.messages = []

        def status_message(self, message, timeout=3000):
            self.messages.append(message)

    class _Controller(CacheController):
        # Label refresh needs the whole cache stack; it is not what is under test.
        def update_labels(self):
            pass

    context = _Context()
    _Controller(context).clear_downloaded_models()

    assert context.app_controller.resets == 1


def test_preview_limit_rejection_does_not_persist_settings(monkeypatch):
    from ui.controllers.cache_controller import CacheController

    context = Mock()
    context.preview_cache_size_combo.currentIndex.return_value = 0
    context.preview_cache_size_combo.itemText.return_value = "1 GB"
    context.preview_cache_size_options_gb = [1]
    context.image_pipeline.preview_cache.set_size_limit.return_value = False
    save = Mock()
    monkeypatch.setattr(
        "ui.controllers.cache_controller.get_preview_cache_size_gb", lambda: 2
    )
    monkeypatch.setattr(
        "ui.controllers.cache_controller.set_preview_cache_size_gb", save
    )
    CacheController(context).apply_preview_cache_limit()
    save.assert_not_called()
    context.image_pipeline.reinitialize_preview_cache_from_settings.assert_not_called()
    context.status_message.assert_called_once()


def test_preview_limit_increase_uses_live_cache(monkeypatch):
    from ui.controllers.cache_controller import CacheController

    context = Mock()
    context.preview_cache_size_combo.currentIndex.return_value = 0
    context.preview_cache_size_combo.itemText.return_value = "2 GB"
    context.preview_cache_size_options_gb = [2]
    context.image_pipeline.preview_cache.set_size_limit.return_value = True
    save = Mock()
    monkeypatch.setattr(
        "ui.controllers.cache_controller.get_preview_cache_size_gb", lambda: 1
    )
    monkeypatch.setattr(
        "ui.controllers.cache_controller.set_preview_cache_size_gb", save
    )
    controller = CacheController(context)
    controller.update_labels = Mock()
    controller.apply_preview_cache_limit()
    context.image_pipeline.preview_cache.set_size_limit.assert_called_once_with(
        2 * 1024**3
    )
    save.assert_called_once_with(2)
    context.image_pipeline.reinitialize_preview_cache_from_settings.assert_not_called()
