from types import SimpleNamespace
from unittest.mock import Mock

from ui.thumbnail_load_coordinator import ViewportThumbnailLoader


def _context(item_count=100):
    action = Mock()
    action.isChecked.return_value = True
    worker_manager = Mock()
    worker_manager.is_thumbnail_preload_running.return_value = False
    worker_manager.start_thumbnail_session.return_value = True
    worker_manager.prioritize_thumbnail_paths.return_value = True
    return SimpleNamespace(
        menu_manager=SimpleNamespace(toggle_thumbnails_action=action),
        worker_manager=worker_manager,
        app_state=SimpleNamespace(
            image_files_data=[
                {"path": f"image-{index}.jpg"} for index in range(item_count)
            ]
        ),
        set_thumbnail_progress=Mock(),
        hide_thumbnail_progress=Mock(),
        _update_thumbnails_from_cache=Mock(),
        get_cached_thumbnail_icon=Mock(return_value=Mock()),
        remove_cached_thumbnail_icons=Mock(),
        image_pipeline=Mock(),
        grouping_step_widget=Mock(),
        _get_active_file_view=Mock(return_value=None),
    )


def test_folder_session_prioritizes_visible_and_warms_every_path_once():
    context = _context(item_count=3)
    loader = ViewportThumbnailLoader(context)
    loader._visible_paths = Mock(return_value=["image-2.jpg", "image-2.jpg"])

    loader.start_folder(["image-0.jpg", "image-1.jpg", "image-2.jpg", "image-1.jpg"])

    args = context.worker_manager.start_thumbnail_session.call_args.args
    assert args[1] == ["image-0.jpg", "image-1.jpg", "image-2.jpg"]
    assert args[2] == ["image-2.jpg"]
    context.set_thumbnail_progress.assert_called_once_with(0, 3, 0, False)


def test_scroll_request_is_reprioritized_while_session_is_active():
    context = _context(item_count=2)
    loader = ViewportThumbnailLoader(context)
    loader._session_id = "folder-session"
    loader._all_paths = ["image-0.jpg", "image-1.jpg"]
    loader._all_path_set = set(loader._all_paths)
    loader._visible_paths = Mock(return_value=["image-1.jpg"])

    loader._load_visible_batch()

    context.worker_manager.prioritize_thumbnail_paths.assert_called_once_with(
        "folder-session", ["image-1.jpg"]
    )


def test_visible_inventory_files_outside_media_session_are_not_requested():
    context = _context(item_count=1)
    loader = ViewportThumbnailLoader(context)
    loader._session_id = "folder-session"
    loader._all_paths = ["image-0.jpg"]
    loader._all_path_set = {"image-0.jpg"}
    loader._visible_paths = Mock(return_value=[".DS_Store", "image-0.jpg"])

    loader._load_visible_batch()

    context.worker_manager.prioritize_thumbnail_paths.assert_called_once_with(
        "folder-session", ["image-0.jpg"]
    )


def test_reset_invalidates_session_and_hides_progress():
    context = _context(item_count=1)
    loader = ViewportThumbnailLoader(context)
    loader._session_id = "old-session"
    loader._all_paths = ["image-0.jpg"]

    loader.reset()

    assert loader._session_id == ""
    assert loader._all_paths == []
    context.hide_thumbnail_progress.assert_called_once()


def test_repeated_schedules_restart_idle_timer_for_active_session(monkeypatch):
    context = _context(item_count=1)
    loader = ViewportThumbnailLoader(context)
    loader._session_id = "folder-session"
    starts = []
    monkeypatch.setattr(loader._load_timer, "start", lambda *args: starts.append(args))

    loader.schedule()
    loader.schedule()
    loader.schedule()

    assert starts == [(), (), ()]


def test_thumbnail_loader_uses_visible_paths_from_active_workflow():
    context = _context()
    context.get_workflow_visible_thumbnail_paths = Mock(
        return_value=["organize-visible.jpg"]
    )
    context._get_active_file_view = Mock(
        side_effect=AssertionError("hidden Cull view must not be inspected")
    )
    loader = ViewportThumbnailLoader(context)

    assert loader._visible_paths() == ["organize-visible.jpg"]


def test_stale_progress_is_discarded():
    context = _context()
    loader = ViewportThumbnailLoader(context)
    loader._session_id = "current"

    loader._handle_progress("old", 5, 10, 0, False)

    context.set_thumbnail_progress.assert_not_called()


def test_foreground_result_is_applied_even_if_viewport_changes_before_callback():
    context = _context()
    loader = ViewportThumbnailLoader(context)
    loader._session_id = "current"
    loader._visible_paths = Mock(return_value=["different-image.jpg"])
    context.image_pipeline.get_cached_thumbnail_qpixmap.return_value = Mock()

    loader._handle_batch_ready("current", ["requested-image.jpg"])

    context._update_thumbnails_from_cache.assert_called_once_with(
        ["requested-image.jpg"]
    )
    context.grouping_step_widget.refresh_cached_thumbnails.assert_called_once_with(
        ["requested-image.jpg"]
    )
