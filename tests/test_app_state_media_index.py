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


def test_bulk_deletion_marks_update_atomically():
    state = _state()
    state.marked_for_deletion = {"keep-marked.jpg", "remove-mark.jpg"}

    changed = state.set_deletion_marks(
        {
            "keep-marked.jpg": True,
            "remove-mark.jpg": False,
            "new-mark.jpg": True,
            "stay-clear.jpg": False,
        }
    )

    assert changed == 2
    assert state.marked_for_deletion == {"keep-marked.jpg", "new-mark.jpg"}


def test_assignment_removal_and_rename_keep_index_consistent():
    state = _state()
    state.image_files_data = [
        {"path": "old.jpg", "media_type": "image", "file_size": 42}
    ]
    state.mark_for_deletion("old.jpg")
    state.focused_image_path = "old.jpg"
    state.easy_delete_results = {
        "other.jpg": {"pair_path": "old.jpg"},
        "unrelated.jpg": {"pair_path": "keep.jpg"},
    }
    state.fix_rotation_results = {"old.jpg": 90}
    state.pick_best_results = {
        1: {
            "winner_path": "old.jpg",
            "all_paths": ["old.jpg", "other.jpg"],
            "ranked": [{"path": "old.jpg"}],
            "failed": [{"path": "old.jpg", "failure_reason": "unreadable"}],
            "unsupported_paths": ["old.jpg"],
            "_mark_state": {"old.jpg": True},
        }
    }
    state.pick_best_winners_by_path = {"old.jpg": True}

    state.update_path("old.jpg", "new.jpg")
    assert state.get_file_data_by_path("old.jpg") is None
    assert state.get_file_data_by_path("new.jpg")["path"] == "new.jpg"
    assert not state.is_marked_for_deletion("old.jpg")
    assert state.is_marked_for_deletion("new.jpg")
    assert state.focused_image_path == "new.jpg"
    assert state.easy_delete_results["other.jpg"]["pair_path"] == "new.jpg"
    assert state.fix_rotation_results == {"new.jpg": 90}
    assert state.pick_best_results[1]["winner_path"] == "new.jpg"
    assert state.pick_best_results[1]["all_paths"] == ["new.jpg", "other.jpg"]
    assert state.pick_best_results[1]["ranked"][0]["path"] == "new.jpg"
    assert state.pick_best_results[1]["failed"][0]["path"] == "new.jpg"
    assert state.pick_best_results[1]["unsupported_paths"] == ["new.jpg"]
    assert state.pick_best_results[1]["_mark_state"] == {"new.jpg": True}
    assert state.pick_best_winners_by_path == {"new.jpg": True}

    state.remove_data_for_path("new.jpg")
    assert state.get_file_data_by_path("new.jpg") is None
    assert not state.is_marked_for_deletion("new.jpg")
    assert state.focused_image_path is None
    assert state.easy_delete_results == {"unrelated.jpg": {"pair_path": "keep.jpg"}}
    assert state.fix_rotation_results == {}
    assert state.pick_best_results == {}
    assert state.pick_best_winners_by_path == {}
    assert state.media_summary() == MediaSummary()
