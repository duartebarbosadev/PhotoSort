from types import SimpleNamespace
from unittest.mock import Mock

from src.ui.app_controller import AppController


class _DummyStatusBar:
    def __init__(self):
        self.messages = []

    def showMessage(self, message: str, timeout: int):
        self.messages.append((message, timeout))


def test_load_folder_blocks_while_grouping_workflow_is_running():
    status_bar = _DummyStatusBar()
    controller = SimpleNamespace(
        worker_manager=SimpleNamespace(
            is_grouping_workflow_running=lambda: True,
        ),
        main_window=SimpleNamespace(
            statusBar=lambda: status_bar,
        ),
    )

    AppController.load_folder(controller, "/tmp/demo")

    assert len(status_bar.messages) == 1
    message, timeout = status_bar.messages[0]
    assert timeout == 4000
    assert "Grouping is still moving files" in message
    assert "loading another folder" in message


def test_handle_grouping_workflow_complete_waits_for_thread_shutdown(monkeypatch):
    status_bar = _DummyStatusBar()
    load_calls = []
    close_calls = []
    running_states = [True, False]

    def is_grouping_workflow_running():
        if running_states:
            return running_states.pop(0)
        return False

    controller = SimpleNamespace()
    controller.worker_manager = SimpleNamespace(
        is_grouping_workflow_running=is_grouping_workflow_running,
    )
    controller.app_state = SimpleNamespace(
        update_path=lambda old_path, new_path: None,
        grouping_run_summary=None,
        grouping_output_root=None,
    )
    controller.main_window = SimpleNamespace(
        set_grouping_busy=lambda busy: None,
        hide_loading_overlay=lambda: None,
        grouping_step_widget=SimpleNamespace(
            set_loading_state=lambda message, busy: None,
        ),
        statusBar=lambda: status_bar,
        finish_pending_close_after_grouping=lambda: close_calls.append(True),
    )
    controller.load_folder = (
        lambda folder_path, skip_grouping_step=False, record_as_source=True: (
            load_calls.append((folder_path, skip_grouping_step, record_as_source))
        )
    )
    controller._finalize_grouping_workflow_completion = lambda summary: (
        AppController._finalize_grouping_workflow_completion(controller, summary)
    )

    monkeypatch.setattr(
        "src.ui.app_controller.QTimer.singleShot",
        lambda _delay, callback: callback(),
    )

    summary = SimpleNamespace(
        entries=[],
        mode="current",
        output_root="/tmp/demo",
        manifest_path="/tmp/demo/grouping-manifest.json",
        moved_count=1,
        unassigned_count=0,
        skipped_count=0,
    )

    AppController.handle_grouping_workflow_complete(controller, summary)

    assert load_calls == [("/tmp/demo", True, False)]
    assert close_calls == [True]


def test_handle_thumbnail_preload_complete_refreshes_grouping_widget():
    refresh_cached_thumbnails = Mock()
    update_thumbnails_from_cache = Mock()
    controller = SimpleNamespace(
        _last_thumbnail_preload_logged=5,
        _supports_grouping_workflow_ui=lambda: True,
        main_window=SimpleNamespace(
            _update_thumbnails_from_cache=update_thumbnails_from_cache,
            grouping_step_widget=SimpleNamespace(
                refresh_cached_thumbnails=refresh_cached_thumbnails
            ),
        ),
    )

    AppController.handle_thumbnail_preload_complete(controller)

    assert controller._last_thumbnail_preload_logged == 0
    update_thumbnails_from_cache.assert_called_once_with()
    refresh_cached_thumbnails.assert_called_once_with()
