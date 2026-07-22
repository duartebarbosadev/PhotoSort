from types import SimpleNamespace
from unittest.mock import Mock

from src.ui.main_window import MainWindow


def _cache_only_pipeline():
    return SimpleNamespace(
        get_immediate_review_qpixmap=Mock(return_value=(None, False)),
        get_cached_preview_qpixmap=Mock(return_value=None),
        get_cached_thumbnail_qpixmap=Mock(return_value=None),
        get_preview_qpixmap=Mock(side_effect=AssertionError("synchronous preview")),
        get_thumbnail_qpixmap=Mock(side_effect=AssertionError("synchronous thumbnail")),
        invalidate_path=Mock(),
    )


def _selection_sync_context(current_path: str, proxy_index=None):
    viewport = Mock()
    selection = Mock()
    selection.isEmpty.return_value = True
    selection_model = Mock()
    selection_model.selection.return_value = selection
    active_view = Mock()
    active_view.viewport.return_value = viewport
    active_view.selectionModel.return_value = selection_model
    find_proxy_index = Mock(return_value=proxy_index)
    context = SimpleNamespace(
        app_state=SimpleNamespace(focused_image_path=None),
        _is_syncing_selection=False,
        _get_active_file_view=lambda: active_view,
        _get_current_selected_image_path=lambda: current_path,
        _find_proxy_index_for_path=find_proxy_index,
    )
    return context, active_view, find_proxy_index


def test_viewer_focus_for_current_row_does_not_resync_selection():
    path = "/tmp/current.arw"
    context, active_view, find_proxy_index = _selection_sync_context(path)

    MainWindow._handle_focused_image_changed(context, 0, path)

    assert context.app_state.focused_image_path == path
    find_proxy_index.assert_not_called()
    active_view.setCurrentIndex.assert_not_called()
    assert not context._is_syncing_selection


def test_viewer_focus_sync_releases_guard_before_next_event_loop_turn():
    path = "/tmp/focused.arw"
    proxy_index = Mock()
    proxy_index.isValid.return_value = True
    context, active_view, find_proxy_index = _selection_sync_context(
        "/tmp/other.arw", proxy_index
    )

    MainWindow._handle_focused_image_changed(context, 0, path)

    find_proxy_index.assert_called_once_with(path)
    active_view.setCurrentIndex.assert_called_once_with(proxy_index)
    assert not context._is_syncing_selection


def test_cull_active_focus_preserves_existing_multiselection():
    path = "/tmp/focused.jpg"
    proxy_index = Mock()
    proxy_index.isValid.return_value = True
    selection_model = Mock()
    selection_model.selectedIndexes.return_value = [Mock(), Mock()]
    active_view = Mock()
    active_view.selectionModel.return_value = selection_model
    context = SimpleNamespace(
        app_state=SimpleNamespace(workflow_step="organize"),
        _is_syncing_selection=False,
        _get_active_file_view=Mock(return_value=active_view),
        _find_proxy_index_for_path=Mock(return_value=proxy_index),
    )

    assert MainWindow.focus_image(context, path)

    selection_model.select.assert_not_called()
    selection_model.setCurrentIndex.assert_called_once()
    active_view.scrollTo.assert_called_once()
    assert not context._is_syncing_selection


def test_multi_selection_uses_placeholders_and_one_background_request(tmp_path):
    paths = [str(tmp_path / "one.arw"), str(tmp_path / "two.arw")]
    for path in paths:
        open(path, "wb").close()
    pipeline = _cache_only_pipeline()
    viewer = Mock()
    preview_controller = Mock()
    status_bar = Mock()
    context = SimpleNamespace(
        image_pipeline=pipeline,
        advanced_image_viewer=viewer,
        preview_load_controller=preview_controller,
        app_state=SimpleNamespace(
            exif_disk_cache=SimpleNamespace(get=Mock(return_value=None)),
            embeddings_cache={},
        ),
        sidebar_visible=False,
        invalidate_last_displayed_preview=Mock(),
        _get_cached_metadata_for_selection=lambda _path: {"rating": 0},
        statusBar=lambda: status_bar,
    )
    context._get_cached_interactive_pixmap = lambda path: (
        MainWindow._get_cached_interactive_pixmap(context, path)
    )

    MainWindow._display_multi_selection_info(context, paths)

    images_data = viewer.set_images_data.call_args.args[0]
    assert [item["path"] for item in images_data] == paths
    assert [item["pixmap"] for item in images_data] == [None, None]
    preview_controller.request.assert_called_once_with(paths)
    pipeline.get_preview_qpixmap.assert_not_called()
    pipeline.get_thumbnail_qpixmap.assert_not_called()


def test_inactive_workflow_cannot_replace_active_inspection_session():
    active_viewer = Mock()
    hidden_viewer = Mock()
    inspection_controller = Mock()
    context = SimpleNamespace(
        image_inspection_controller=inspection_controller,
        _active_workflow_inspection_viewer=lambda: active_viewer,
    )

    MainWindow.activate_image_inspection(context, hidden_viewer, [Mock()])
    inspection_controller.activate.assert_not_called()

    specs = [Mock()]
    MainWindow.activate_image_inspection(context, active_viewer, specs)
    inspection_controller.activate.assert_called_once_with(
        active_viewer,
        specs,
        force_default_brightness=False,
    )


def test_rotation_comparison_never_decodes_on_ui_thread(tmp_path):
    path = str(tmp_path / "rotation.arw")
    open(path, "wb").close()
    pipeline = _cache_only_pipeline()
    viewer = Mock()
    context = SimpleNamespace(
        image_pipeline=pipeline,
        advanced_image_viewer=viewer,
        rotation_suggestions={path: 90},
        _pending_rotation_comparison_path=None,
        invalidate_last_displayed_preview=Mock(),
        _get_cached_metadata_for_selection=lambda _path: {"rating": 0},
        request_interactive_preview=Mock(),
    )
    context._get_cached_interactive_pixmap = lambda image_path: (
        MainWindow._get_cached_interactive_pixmap(context, image_path)
    )

    MainWindow._display_side_by_side_comparison(context, path)

    images_data = viewer.set_images_data.call_args.args[0]
    assert len(images_data) == 2
    assert all(item["path"] == path and item["pixmap"] is None for item in images_data)
    context.request_interactive_preview.assert_called_once_with(path)
    assert context._pending_rotation_comparison_path == path
    pipeline.get_preview_qpixmap.assert_not_called()
    pipeline.get_thumbnail_qpixmap.assert_not_called()


def test_rotation_completion_queues_cache_refresh_instead_of_decoding():
    path = "/tmp/rotation.arw"
    pipeline = _cache_only_pipeline()
    thumbnail_loader = Mock()
    status_bar = Mock()
    context = SimpleNamespace(
        image_pipeline=pipeline,
        thumbnail_loader=thumbnail_loader,
        _get_selected_file_paths_from_view=Mock(return_value=[path]),
        _handle_file_selection_changed=Mock(),
        request_interactive_preview=Mock(),
        statusBar=lambda: status_bar,
    )

    MainWindow._handle_successful_rotation(
        context, path, "clockwise", "Rotation complete", False
    )

    pipeline.invalidate_path.assert_called_once_with(path)
    thumbnail_loader.invalidate_paths.assert_called_once_with([path])
    context.request_interactive_preview.assert_called_once_with(
        path, force_default_brightness=True
    )
    pipeline.get_preview_qpixmap.assert_not_called()
    pipeline.get_thumbnail_qpixmap.assert_not_called()
