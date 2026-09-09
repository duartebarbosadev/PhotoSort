from types import SimpleNamespace
from unittest.mock import Mock

from ui.app_controller import AppController, ROTATION_LOADING_OVERLAY_DELAY_MS


def test_successful_rotation_is_removed_from_shared_review_results():
    path = "/tmp/photo.jpg"
    widget = Mock()
    pipeline = Mock()
    controller = SimpleNamespace(
        main_window=SimpleNamespace(
            fix_rotation_step_widget=widget,
            image_pipeline=pipeline,
        ),
        app_state=SimpleNamespace(fix_rotation_results={path: 90}),
        _pending_rotated_paths=[],
    )

    AppController.handle_rotation_applied(
        controller,
        path,
        "clockwise",
        True,
        "ok",
        False,
    )

    widget.record_apply_result.assert_called_once_with(path, True)
    assert controller.app_state.fix_rotation_results == {}
    assert controller._pending_rotated_paths == [path]
    pipeline.invalidate_path.assert_called_once_with(path)


def test_rotation_batch_finish_never_eagerly_regenerates_all_previews():
    path = "/tmp/photo.jpg"
    pipeline = SimpleNamespace(
        preload_previews=Mock(
            side_effect=AssertionError("rotation finish must stay cache-lazy")
        )
    )
    main_window = SimpleNamespace(
        image_pipeline=pipeline,
        image_inspection_controller=Mock(),
        _sync_workflow_results_after_file_mutation=Mock(),
        _batch_update_rotated_thumbnails=Mock(),
        _get_selected_file_paths_from_view=Mock(return_value=[path]),
        invalidate_last_displayed_preview=Mock(),
        _handle_file_selection_changed=Mock(),
        hide_loading_overlay=Mock(),
        statusBar=lambda: Mock(),
    )
    controller = SimpleNamespace(
        main_window=main_window,
        app_state=SimpleNamespace(invalidate_similarity_for_paths=Mock()),
        _pending_rotated_paths=[path],
        _rotation_loading_overlay_timer=Mock(),
        _rotation_loading_overlay_visible=False,
    )

    AppController.handle_rotation_application_finished(controller, 1, 0)

    pipeline.preload_previews.assert_not_called()
    controller.app_state.invalidate_similarity_for_paths.assert_called_once_with([path])
    main_window._sync_workflow_results_after_file_mutation.assert_called_once_with(
        exclude={"fix_rotation"}
    )
    main_window._batch_update_rotated_thumbnails.assert_called_once_with([path])
    main_window.image_inspection_controller.refresh_paths.assert_called_once_with(
        [path]
    )
    main_window._handle_file_selection_changed.assert_called_once()
    main_window.hide_loading_overlay.assert_not_called()
    assert controller._pending_rotated_paths == []


def test_fast_rotation_never_shows_or_hides_loading_overlay():
    timer = Mock()
    main_window = SimpleNamespace(
        show_loading_overlay=Mock(),
        hide_loading_overlay=Mock(),
    )
    controller = SimpleNamespace(
        main_window=main_window,
        _rotation_loading_overlay_timer=timer,
        _rotation_loading_text="",
        _rotation_loading_overlay_visible=False,
    )

    AppController._begin_rotation_loading_feedback(controller)
    AppController._finish_rotation_loading_feedback(controller)

    timer.start.assert_called_once_with(ROTATION_LOADING_OVERLAY_DELAY_MS)
    main_window.show_loading_overlay.assert_not_called()
    main_window.hide_loading_overlay.assert_not_called()


def test_rotation_apply_schedules_delayed_overlay_before_starting_worker():
    timer = Mock()
    worker_manager = SimpleNamespace(
        is_rotation_application_running=lambda: False,
        start_rotation_application=Mock(),
    )
    main_window = SimpleNamespace(
        show_loading_overlay=Mock(),
        hide_loading_overlay=Mock(),
        statusBar=lambda: Mock(),
    )
    exif_cache = Mock()
    controller = SimpleNamespace(
        main_window=main_window,
        app_state=SimpleNamespace(exif_disk_cache=exif_cache),
        worker_manager=worker_manager,
        _rotation_loading_overlay_timer=timer,
        _rotation_loading_text="",
        _rotation_loading_overlay_visible=False,
    )

    AppController._apply_approved_rotations(controller, {"photo.jpg": 90})

    timer.start.assert_called_once_with(ROTATION_LOADING_OVERLAY_DELAY_MS)
    main_window.show_loading_overlay.assert_not_called()
    worker_manager.start_rotation_application.assert_called_once_with(
        approved_rotations={"photo.jpg": 90},
        exif_disk_cache=exif_cache,
    )


def test_slow_rotation_overlay_starts_with_latest_progress_and_then_updates():
    timer = Mock()
    main_window = SimpleNamespace(
        show_loading_overlay=Mock(),
        update_loading_text=Mock(),
        hide_loading_overlay=Mock(),
    )
    controller = SimpleNamespace(
        main_window=main_window,
        worker_manager=SimpleNamespace(is_rotation_application_running=lambda: True),
        _rotation_loading_overlay_timer=timer,
        _rotation_loading_text="",
        _rotation_loading_overlay_visible=False,
    )

    AppController._begin_rotation_loading_feedback(controller)
    AppController.handle_rotation_application_progress(controller, 1, 3, "first.jpg")
    main_window.update_loading_text.assert_not_called()

    AppController._show_delayed_rotation_loading_overlay(controller)
    main_window.show_loading_overlay.assert_called_once_with("Rotating 1/3: first.jpg")

    AppController.handle_rotation_application_progress(controller, 2, 3, "second.jpg")
    main_window.update_loading_text.assert_called_once_with("Rotating 2/3: second.jpg")

    AppController._finish_rotation_loading_feedback(controller)
    main_window.hide_loading_overlay.assert_called_once_with()
