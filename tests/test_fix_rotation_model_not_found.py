import os
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PyQt6.QtWidgets import QApplication
from ui.fix_rotation_step_widget import FixRotationStepWidget

# Ensure a QApplication exists for widget tests.
_app = QApplication.instance() or QApplication([])


def test_fix_rotation_step_widget_shows_download_instructions_when_model_missing():
    widget = FixRotationStepWidget()

    # Initially hidden/shown states
    assert widget._missing_model_widget.isHidden()
    assert not widget._progress_bar.isHidden()

    # Show missing model screen
    dummy_path = "/path/to/missing/model.onnx"
    widget.show_model_not_found(dummy_path)

    assert not widget._missing_model_widget.isHidden()
    assert widget._progress_bar.isHidden()

    # Check that it displays the instructions and path
    assert widget._model_path_label.text() == dummy_path
    instructions_text = widget._instructions_label.text()
    assert "Download Model" in instructions_text
    assert "Open Models Folder" in instructions_text

    # Verify that retry button emits retry_requested signal
    retried = False
    def on_retry():
        nonlocal retried
        retried = True
    widget.retry_requested.connect(on_retry)
    widget._retry_btn.click()
    assert retried is True

    # Show loading again - should hide download widgets
    widget.show_loading("Loading...", 10)
    assert widget._missing_model_widget.isHidden()
    assert not widget._progress_bar.isHidden()
