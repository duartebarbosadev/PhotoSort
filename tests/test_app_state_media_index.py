import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from unittest.mock import patch

from ui.app_state import AppState, MediaSummary


def _state() -> AppState:
    with (
        patch("ui.app_state.RatingCache"),
        patch("ui.app_state.ExifCache"),
        patch("ui.app_state.AnalysisCache"),
    ):
        return AppState()


def test_scan_batches_maintain_media_summary_and_path_index():
    state = _state()
    state.extend_file_data(
        [
            {"path": "a.jpg", "media_type": "image", "file_size": 100},
            {"path": "b.mov", "media_type": "video", "file_size": 250},
        ]
    )

    assert state.media_summary() == MediaSummary(
        total_items=2,
        image_count=1,
        video_count=1,
        total_size_bytes=350,
    )
    assert state.get_file_data_by_path("b.mov")["file_size"] == 250

    state.update_blur_status("a.jpg", True)
    assert state.get_file_data_by_path("a.jpg")["is_blurred"] is True


def test_assignment_removal_and_rename_keep_index_consistent():
    state = _state()
    state.image_files_data = [
        {"path": "old.jpg", "media_type": "image", "file_size": 42}
    ]

    state.update_path("old.jpg", "new.jpg")
    assert state.get_file_data_by_path("old.jpg") is None
    assert state.get_file_data_by_path("new.jpg")["path"] == "new.jpg"

    state.remove_data_for_path("new.jpg")
    assert state.get_file_data_by_path("new.jpg") is None
    assert state.media_summary() == MediaSummary()
