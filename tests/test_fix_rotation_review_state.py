from types import SimpleNamespace
from unittest.mock import Mock

from ui.app_controller import AppController


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
        _batch_update_rotated_thumbnails=Mock(),
        _get_selected_file_paths_from_view=Mock(return_value=[path]),
        invalidate_last_displayed_preview=Mock(),
        _handle_file_selection_changed=Mock(),
        hide_loading_overlay=Mock(),
        statusBar=lambda: Mock(),
    )
    controller = SimpleNamespace(
        main_window=main_window,
        _pending_rotated_paths=[path],
    )

    AppController.handle_rotation_application_finished(controller, 1, 0)

    pipeline.preload_previews.assert_not_called()
    main_window._batch_update_rotated_thumbnails.assert_called_once_with([path])
    main_window._handle_file_selection_changed.assert_called_once()
    assert controller._pending_rotated_paths == []
