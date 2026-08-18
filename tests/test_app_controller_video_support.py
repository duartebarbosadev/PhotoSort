from types import SimpleNamespace
from unittest.mock import Mock, call, patch

from src.ui.app_controller import AppController


class _DummyAction:
    def __init__(self):
        self.enabled = None

    def setEnabled(self, value):
        self.enabled = value


class _DummyStatusBar:
    def __init__(self):
        self.messages = []

    def showMessage(self, message, timeout=0):
        self.messages.append((message, timeout))


class _DummyMenuManager:
    def __init__(self):
        self.open_folder_action = _DummyAction()
        self.analyze_similarity_action = _DummyAction()
        self.group_by_similarity_action = _DummyAction()
        self.ai_rate_images_action = _DummyAction()


class _DummyMainWindow:
    def __init__(self):
        self.menu_manager = _DummyMenuManager()
        self._status_bar = _DummyStatusBar()
        self._loading_updates = []
        self.rebuild_count = 0
        self.info_label_updates = 0
        self.overlay_hidden = False
        self.schedule_visible_thumbnail_load = Mock()
        self.start_thumbnail_warming = Mock()
        self.start_thumbnail_warming.return_value = "folder-assets"
        self.set_exif_progress = Mock()
        self.hide_exif_progress = Mock()
        self.reset_thumbnail_requests = Mock()
        self.dialog_manager = Mock()

    def update_loading_text(self, text):
        self._loading_updates.append(text)

    def _rebuild_model_view(self):
        self.rebuild_count += 1

    def _update_image_info_label(self):
        self.info_label_updates += 1

    def hide_loading_overlay(self):
        self.overlay_hidden = True

    def show_loading_overlay(self, text):
        self._loading_updates.append(text)

    def statusBar(self):
        return self._status_bar


class _DummyWorkerManager:
    def __init__(self):
        self.start_thumbnail_preload = Mock()
        self.start_rating_load = Mock()
        self.start_rating_writer = Mock()
        self.resolve_thumbnail_capacity_request = Mock(return_value=True)


class _DummyAppState:
    def __init__(self, image_files_data):
        self.image_files_data = image_files_data
        self.rating_disk_cache = Mock()
        self.exif_disk_cache = Mock()
        self.cluster_results = {}
        self.current_folder_path = None
        self.analysis_cache = Mock()
        self.clear_all_file_specific_data = Mock()


def _make_controller(image_files_data):
    main_window = _DummyMainWindow()
    app_state = _DummyAppState(image_files_data)
    worker_manager = _DummyWorkerManager()
    controller = AppController(main_window, app_state, worker_manager)
    controller._restore_analysis_state = Mock()
    return controller, main_window, app_state, worker_manager


def test_handle_scan_finished_preloads_thumbnails_and_metadata_for_videos_too():
    image_path = "/tmp/a.jpg"
    video_path = "/tmp/b.mp4"
    controller, main_window, _, worker_manager = _make_controller(
        [
            {"path": image_path, "media_type": "image", "is_blurred": None},
            {"path": video_path, "media_type": "video", "is_blurred": None},
        ]
    )

    controller.handle_scan_finished()

    worker_manager.start_thumbnail_preload.assert_not_called()
    main_window.start_thumbnail_warming.assert_called_once_with(
        [image_path, video_path]
    )
    args, _ = worker_manager.start_rating_load.call_args
    loaded_data = args[0]
    assert len(loaded_data) == 2
    assert {entry["media_type"] for entry in loaded_data} == {"image", "video"}
    assert not main_window.overlay_hidden

    controller.handle_review_asset_finished("folder-assets", 2, 0)
    assert main_window.overlay_hidden


def test_rating_completion_does_not_trigger_global_preview_preload():
    controller, main_window, _, worker_manager = _make_controller(
        [{"path": "/tmp/a.jpg", "media_type": "image", "is_blurred": None}]
    )
    worker_manager.start_preview_preload = Mock()

    controller.handle_rating_load_finished()

    worker_manager.start_preview_preload.assert_not_called()
    assert main_window.overlay_hidden
    main_window.hide_exif_progress.assert_called_once_with()


def test_rating_progress_updates_the_exif_footer_progress():
    controller, main_window, _, _ = _make_controller([])

    controller.handle_rating_load_progress(0, 4_482, "")
    controller.handle_rating_load_progress(250, 4_482, "photo.arw")

    assert main_window.set_exif_progress.call_args_list == [
        call(0, 4_482),
        call(250, 4_482),
    ]


def test_review_cache_capacity_increase_is_required_before_preparation(tmp_path):
    controller, main_window, _, _ = _make_controller([])
    preview_cache = Mock(
        _cache_dir=str(tmp_path),
        volume=Mock(return_value=0),
    )
    pipeline = Mock(preview_cache=preview_cache)
    pipeline.estimate_active_review_cache_bytes.return_value = 3 * 1024**3
    main_window.image_pipeline = pipeline
    main_window.dialog_manager.confirm_preview_cache_capacity_increase.return_value = (
        True
    )

    with (
        patch("src.ui.app_controller.get_preview_cache_size_bytes", return_value=2**30),
        patch("src.ui.app_controller.set_preview_cache_size_gb") as set_limit,
        patch(
            "src.ui.app_controller.shutil.disk_usage",
            return_value=SimpleNamespace(free=8 * 1024**3),
        ),
    ):
        assert controller._prepare_review_cache_capacity(["photo.arw"])

    set_limit.assert_called_once_with(3.0)
    preview_cache.reinitialize_from_settings.assert_called_once_with()
    pipeline.begin_active_review_working_set.assert_called_once_with(["photo.arw"])
    preview_cache.trim_to_limit.assert_called_once_with()


def test_declining_required_review_cache_capacity_cancels_folder(tmp_path):
    controller, main_window, app_state, _ = _make_controller([])
    preview_cache = Mock(
        _cache_dir=str(tmp_path),
        volume=Mock(return_value=0),
    )
    pipeline = Mock(preview_cache=preview_cache)
    pipeline.estimate_active_review_cache_bytes.return_value = 3 * 1024**3
    main_window.image_pipeline = pipeline
    main_window.dialog_manager.confirm_preview_cache_capacity_increase.return_value = (
        False
    )

    with (
        patch("src.ui.app_controller.get_preview_cache_size_bytes", return_value=2**30),
        patch(
            "src.ui.app_controller.shutil.disk_usage",
            return_value=SimpleNamespace(free=8 * 1024**3),
        ),
    ):
        assert not controller._prepare_review_cache_capacity(["photo.arw"])

    app_state.clear_all_file_specific_data.assert_called_once_with()
    pipeline.begin_active_review_working_set.assert_not_called()
    assert main_window.overlay_hidden


def test_actual_cache_overrun_pauses_then_resumes_with_raised_live_limit(tmp_path):
    controller, main_window, _, worker_manager = _make_controller([])
    current_limit = 1024**3
    required = current_limit + 1
    preview_cache = Mock(
        _cache_dir=str(tmp_path),
        _size_limit_bytes=current_limit,
        volume=Mock(return_value=0),
    )
    main_window.image_pipeline = Mock(preview_cache=preview_cache)
    main_window.dialog_manager.confirm_preview_cache_capacity_increase.return_value = (
        True
    )
    controller._folder_asset_session_id = "folder-assets"

    with (
        patch("src.ui.app_controller.set_preview_cache_size_gb") as set_limit,
        patch(
            "src.ui.app_controller.shutil.disk_usage",
            return_value=SimpleNamespace(free=8 * 1024**3),
        ),
    ):
        controller.handle_review_asset_capacity_required("folder-assets", required)

    set_limit.assert_called_once_with(1.25)
    preview_cache.increase_size_limit.assert_called_once_with(int(1.25 * 1024**3))
    worker_manager.resolve_thumbnail_capacity_request.assert_called_once_with(
        "folder-assets", True
    )


def test_apply_rating_to_selection_skips_videos_and_writes_images_only():
    controller, main_window, app_state, worker_manager = _make_controller([])
    selected_paths = ["/tmp/a.jpg", "/tmp/b.mp4", "/tmp/c.png"]

    controller.apply_rating_to_selection(4, selected_paths)

    worker_manager.start_rating_writer.assert_called_once()
    kwargs = worker_manager.start_rating_writer.call_args.kwargs
    assert kwargs["rating_operations"] == [("/tmp/a.jpg", 4), ("/tmp/c.png", 4)]
    assert kwargs["rating_disk_cache"] is app_state.rating_disk_cache
    assert kwargs["exif_disk_cache"] is app_state.exif_disk_cache
    assert "Skipping 1 video(s)" in main_window.statusBar().messages[-1][0]


def test_apply_rating_to_selection_video_only_does_not_start_writer():
    controller, main_window, _, worker_manager = _make_controller([])

    controller.apply_rating_to_selection(3, ["/tmp/clip.mp4"])

    worker_manager.start_rating_writer.assert_not_called()
    assert (
        main_window.statusBar().messages[-1][0]
        == "Ratings are currently supported for images only."
    )
