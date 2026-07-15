import logging
from types import SimpleNamespace

from ui.main_window import MainWindow


def test_workflow_view_change_logs_previous_next_and_item_count(caplog):
    window = SimpleNamespace(
        app_state=SimpleNamespace(
            workflow_step="organize",
            image_files_data=[{"path": "one.jpg"}, {"path": "two.jpg"}],
        )
    )

    with caplog.at_level(logging.INFO, logger="ui.main_window"):
        MainWindow._set_workflow_step(window, "cull")

    assert window.app_state.workflow_step == "cull"
    assert "Workflow view changed: Organize -> Cull (media_items=2)" in caplog.text


def test_showing_current_workflow_does_not_log_duplicate_transition(caplog):
    window = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="organize", image_files_data=[])
    )

    with caplog.at_level(logging.INFO, logger="ui.main_window"):
        MainWindow._set_workflow_step(window, "organize")

    assert "Workflow view changed" not in caplog.text
