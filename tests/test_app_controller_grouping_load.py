from types import SimpleNamespace
from unittest.mock import Mock

from src.ui.app_controller import AppController
from src.ui.controllers.model_prerequisites import DeferredModelStarts


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
        _deferred_starts=DeferredModelStarts(),
    )

    AppController.load_folder(controller, "/tmp/demo")

    assert len(status_bar.messages) == 1
    message, timeout = status_bar.messages[0]
    assert timeout == 4000
    assert "Grouping is still moving files" in message
    assert "loading another folder" in message


def test_load_folder_cancels_analysis_without_blocking(monkeypatch):
    callbacks = []
    worker_manager = SimpleNamespace(
        is_grouping_workflow_running=lambda: False,
        is_file_deletion_running=lambda: False,
        is_rotation_application_running=lambda: False,
        is_rating_writer_running=lambda: False,
        is_any_worker_running=lambda: True,
        request_stop_all_workers=Mock(),
    )
    app_state = SimpleNamespace(
        get_marked_files=lambda: [],
        clear_all_file_specific_data=Mock(),
    )
    main_window = SimpleNamespace(
        dialog_manager=SimpleNamespace(
            confirm_interrupt_for_folder_change=Mock(return_value=True),
        ),
        show_loading_overlay=Mock(),
        update_loading_text=Mock(),
        menu_manager=SimpleNamespace(update_recent_folders_menu=Mock()),
    )
    controller = SimpleNamespace(
        worker_manager=worker_manager,
        app_state=app_state,
        main_window=main_window,
        _pending_folder_load_after_workers=None,
        _finish_folder_load_after_workers=Mock(),
        _deferred_starts=DeferredModelStarts(),
    )
    monkeypatch.setattr("src.ui.app_controller.add_recent_folder", lambda _path: None)
    monkeypatch.setattr(
        "src.ui.app_controller.QTimer.singleShot",
        lambda _delay, callback: callbacks.append(callback),
    )

    AppController.load_folder(controller, "/tmp/demo")

    worker_manager.request_stop_all_workers.assert_called_once_with()
    app_state.clear_all_file_specific_data.assert_not_called()
    assert controller._pending_folder_load_after_workers == (
        "/tmp/demo",
        {
            "skip_grouping_step": False,
            "record_as_source": True,
            "preserve_deletion_marks": False,
            "_interrupt_confirmed": True,
        },
    )
    assert callbacks == [controller._finish_folder_load_after_workers]
    main_window.dialog_manager.confirm_interrupt_for_folder_change.assert_called_once_with(
        "/tmp/demo"
    )


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
        update_paths=Mock(),
        grouping_run_summary=None,
        grouping_output_root=None,
    )
    sync_workflows = Mock()
    controller.main_window = SimpleNamespace(
        set_grouping_busy=lambda busy: None,
        hide_loading_overlay=lambda: None,
        _sync_workflow_results_after_file_mutation=sync_workflows,
        grouping_step_widget=SimpleNamespace(
            set_loading_state=lambda message, busy: None,
        ),
        statusBar=lambda: status_bar,
        finish_pending_close_after_grouping=lambda: close_calls.append(True),
    )
    controller.load_folder = (
        lambda folder_path, skip_grouping_step=False, record_as_source=True, preserve_deletion_marks=False: (
            load_calls.append(
                (
                    folder_path,
                    skip_grouping_step,
                    record_as_source,
                    preserve_deletion_marks,
                )
            )
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
        moved_count=1,
        unassigned_count=0,
        skipped_count=0,
    )

    AppController.handle_grouping_workflow_complete(controller, summary)

    controller.app_state.update_paths.assert_called_once_with(
        {},
        migrate_disk_caches=False,
    )
    sync_workflows.assert_called_once_with()
    assert load_calls == [("/tmp/demo", True, False, True)]
    assert close_calls == [True]


def test_scan_finished_defers_hidden_cull_model_until_cull_is_shown():
    actions = {
        name: Mock()
        for name in (
            "open_folder_action",
            "analyze_similarity_action",
            "group_by_similarity_action",
            "ai_rate_images_action",
        )
    }
    rebuild_model = Mock()
    mark_cull_model_dirty = Mock()
    show_grouping_step = Mock()
    refresh_grouping_preview = Mock()
    controller = SimpleNamespace(
        app_state=SimpleNamespace(
            image_files_data=[{"path": "/tmp/a.jpg"}],
            skip_grouping_step_once=False,
            rating_disk_cache=Mock(),
        ),
        worker_manager=SimpleNamespace(
            start_grouping_preview=Mock(),
            start_rating_load=Mock(),
        ),
        main_window=SimpleNamespace(
            menu_manager=SimpleNamespace(**actions),
            grouping_step_widget=SimpleNamespace(),
            update_grouping_preview=Mock(),
            show_grouping_step=show_grouping_step,
            show_cull_step=Mock(),
            mark_cull_model_dirty=mark_cull_model_dirty,
            _rebuild_model_view=rebuild_model,
            update_loading_text=Mock(),
            hide_loading_overlay=Mock(),
            schedule_visible_thumbnail_load=Mock(),
            _update_image_info_label=Mock(),
        ),
        _get_image_file_data=lambda: [{"path": "/tmp/a.jpg"}],
        _get_media_file_data=lambda: [],
        _restore_analysis_state=Mock(),
        refresh_grouping_preview=refresh_grouping_preview,
        _rating_load_complete=False,
    )
    controller._supports_grouping_workflow_ui = lambda: (
        AppController._supports_grouping_workflow_ui(controller)
    )
    controller._activate_loaded_folder = lambda *, asset_failures: (
        AppController._activate_loaded_folder(controller, asset_failures=asset_failures)
    )

    AppController.handle_scan_finished(controller)

    mark_cull_model_dirty.assert_called_once()
    rebuild_model.assert_not_called()
    show_grouping_step.assert_called_once()
    refresh_grouping_preview.assert_called_once()


def test_grouping_preview_does_not_build_hidden_organize_trees():
    set_preview_plan = Mock()
    controller = SimpleNamespace(
        app_state=SimpleNamespace(
            workflow_step="cull",
            grouping_source_root="/tmp/photos",
            current_folder_path="/tmp/photos",
        ),
        main_window=SimpleNamespace(
            update_grouping_preview=Mock(),
            schedule_visible_thumbnail_load=Mock(),
            notify_thumbnail_items_rebuilt=Mock(),
            grouping_step_widget=SimpleNamespace(
                set_preview_plan=set_preview_plan,
                set_loading_state=Mock(),
            ),
        ),
    )
    plan = SimpleNamespace(
        mode="current",
        groups=[],
        source_root="/tmp/photos",
    )

    AppController.handle_grouping_preview_ready(controller, plan)

    set_preview_plan.assert_not_called()
    controller.main_window.schedule_visible_thumbnail_load.assert_not_called()
    assert controller._pending_grouping_preview == (plan, "/tmp/photos")

    AppController.activate_grouping_preview(controller)

    set_preview_plan.assert_called_once_with(plan, "/tmp/photos")
    controller.main_window.notify_thumbnail_items_rebuilt.assert_called_once()
    assert controller._pending_grouping_preview is None
