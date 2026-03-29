from unittest.mock import Mock

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
        self.analyze_best_shots_action = _DummyAction()
        self.analyze_similarity_action = _DummyAction()
        self.analyze_best_shots_selected_action = _DummyAction()
        self.stop_best_shots_action = _DummyAction()
        self.detect_blur_action = _DummyAction()
        self.auto_rotate_action = _DummyAction()
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
        self.start_best_shot_analysis = Mock()

    def is_best_shot_worker_running(self):
        return False


class _DummyAppState:
    def __init__(self, image_files_data):
        self.image_files_data = image_files_data
        self.rating_disk_cache = Mock()
        self.exif_disk_cache = Mock()
        self.cluster_results = {}
        self.best_shot_rankings = {}
        self.current_folder_path = None
        self.analysis_cache = Mock()


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
    controller, _, _, worker_manager = _make_controller(
        [
            {"path": image_path, "media_type": "image", "is_blurred": None},
            {"path": video_path, "media_type": "video", "is_blurred": None},
        ]
    )

    controller.handle_scan_finished()

    worker_manager.start_thumbnail_preload.assert_called_once_with(
        [image_path, video_path]
    )
    args, _ = worker_manager.start_rating_load.call_args
    loaded_data = args[0]
    assert len(loaded_data) == 2
    assert {entry["media_type"] for entry in loaded_data} == {"image", "video"}


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


def test_start_best_shot_analysis_excludes_video_paths_from_cluster_map():
    controller, _, app_state, worker_manager = _make_controller(
        [
            {"path": "/tmp/a.jpg", "media_type": "image", "is_blurred": None},
            {"path": "/tmp/b.mp4", "media_type": "video", "is_blurred": None},
        ]
    )
    app_state.cluster_results = {"/tmp/a.jpg": 1, "/tmp/b.mp4": 1}
    app_state.best_shot_rankings = {}
    app_state.analysis_cache = Mock()

    controller.start_best_shot_analysis()

    worker_manager.start_best_shot_analysis.assert_called_once()
    cluster_map = worker_manager.start_best_shot_analysis.call_args.args[0]
    assert cluster_map == {1: ["/tmp/a.jpg"]}
