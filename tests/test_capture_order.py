import pyexiv2  # noqa: F401  # Must be imported first to avoid Windows crashes


import os
import shutil
import struct
from datetime import UTC, datetime
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock

from PyQt6.QtGui import QStandardItem
from PyQt6.QtWidgets import QApplication

from core.capture_order import capture_order_key
from core.metadata_processor import MetadataProcessor
from ui.app_state import AppState
from ui.easy_delete_step_widget import EasyDeleteStepWidget
from ui.fix_rotation_step_widget import FixRotationStepWidget
from ui.main_window import MainWindow
from ui.pick_best_step_widget import PickBestStepWidget

_app = QApplication.instance() or QApplication([])

SAMPLES = Path(__file__).parent / "samples"
# Chronological by EXIF capture date; filename order puts GAB00915 before
# jpg_sample even though it was taken more than a year later.
SAMPLES_BY_CAPTURE_DATE = [
    "20230428_034640_IMG_4658.JPG",
    "20230428_035435_IMG_4666.JPG",
    "20230428_042445_IMG_4667.JPG",
    "jpg_sample.jpg",
    "GAB00915.jpg",
]


def _write_movie(path: Path, recorded_at: datetime) -> None:
    def atom(kind: bytes, body: bytes) -> bytes:
        return struct.pack(">I4s", len(body) + 8, kind) + body

    seconds = int((recorded_at - datetime(1904, 1, 1, tzinfo=UTC)).total_seconds())
    mvhd = bytes(4) + struct.pack(">II", seconds, seconds) + bytes(80)
    path.write_bytes(
        atom(b"ftyp", b"isom")
        + atom(b"mdat", bytes(32))
        + atom(b"moov", atom(b"mvhd", mvhd))
    )


def _load_state(paths: list[str]) -> AppState:
    """Load media the way the app does: scan records plus background metadata."""
    state = AppState()
    state.image_files_data = [
        {"path": path, "mtime_ns": os.stat(path).st_mtime_ns} for path in paths
    ]
    metadata = MetadataProcessor.get_batch_display_metadata(
        paths, rating_disk_cache=None, exif_disk_cache=None
    )
    for path in paths:
        state.date_cache[path] = metadata[path]["date"]
    return state


def test_photos_and_videos_are_ordered_by_capture_date(tmp_path):
    for name in SAMPLES_BY_CAPTURE_DATE:
        shutil.copy2(SAMPLES / name, tmp_path / name)
    # The clip's file was written just now, but it was recorded before any photo.
    clip = tmp_path / "AAA_clip.mp4"
    _write_movie(clip, datetime(2020, 6, 1, 12, 0, tzinfo=UTC))
    paths = [str(path) for path in sorted(tmp_path.iterdir())]

    state = _load_state(paths)

    ordered = [os.path.basename(p) for p in state.sort_paths_by_capture_date(paths)]
    assert ordered == ["AAA_clip.mp4", *SAMPLES_BY_CAPTURE_DATE]

    # Video payloads cached by older releases lack the recording date and are
    # rebuilt rather than reused.
    legacy = {"file_path": str(clip), "media_type": "video", "file_size": 1}
    exif_cache = Mock(get=Mock(return_value=legacy))
    metadata = MetadataProcessor.get_batch_display_metadata(
        [str(clip)], rating_disk_cache=None, exif_disk_cache=exif_cache
    )
    assert metadata[str(clip)]["date"].year == 2020
    exif_cache.set.assert_called_once()


def test_file_times_are_used_until_metadata_dates_are_known():
    def ns(year: int) -> int:
        return int(datetime(year, 1, 1).timestamp() * 1_000_000_000)

    records = {
        "/p/edited.jpg": {"birthtime_ns": ns(2020), "mtime_ns": ns(2025)},
        "/p/copied.jpg": {"mtime_ns": ns(2022)},
        "/p/tagged.jpg": {"mtime_ns": ns(2026)},
        "/p/b-undated.jpg": {},
        "/p/a-undated.jpg": {},
    }
    date_cache = {"/p/tagged.jpg": datetime(2019, 1, 1)}

    ordered = sorted(
        records, key=lambda path: capture_order_key(path, date_cache, records[path])
    )

    assert ordered == [
        "/p/tagged.jpg",
        "/p/edited.jpg",
        "/p/copied.jpg",
        "/p/a-undated.jpg",
        "/p/b-undated.jpg",
    ]


def test_every_review_workflow_uses_and_refreshes_the_shared_capture_order():
    state = AppState()
    state.date_cache.update(
        {
            "/p/late-a.jpg": datetime(2023, 1, 1),
            "/p/late-b.jpg": datetime(2023, 1, 2),
            "/p/early-a.jpg": datetime(2020, 1, 1),
            "/p/early-b.jpg": datetime(2020, 1, 2),
        }
    )

    rotation = FixRotationStepWidget()
    rotation.set_capture_order_key(state.capture_order_key)
    rotation.show_results({"/p/late-a.jpg": 90, "/p/early-a.jpg": 180})
    assert rotation._ordered_paths == ["/p/early-a.jpg", "/p/late-a.jpg"]

    easy_delete = EasyDeleteStepWidget()
    easy_delete.set_is_marked_func(lambda _path: False)
    easy_delete.set_capture_order_key(state.capture_order_key)
    blur = {"type": "blur", "pair_path": None, "suggest_delete": True}
    easy_delete.show_results({"/p/late-a.jpg": blur, "/p/early-a.jpg": dict(blur)})
    assert easy_delete._flagged_paths == ["/p/early-a.jpg", "/p/late-a.jpg"]

    def cluster(paths):
        return {"winner_path": paths[0], "ranked": [], "failed": [], "all_paths": paths}

    pick_best = PickBestStepWidget()
    pick_best.set_is_marked_func(lambda _path: False)
    pick_best.set_capture_order_key(state.capture_order_key)
    pick_best.show_results(
        {
            1: cluster(["/p/late-a.jpg", "/p/late-b.jpg"]),
            2: cluster(["/p/early-a.jpg", "/p/early-b.jpg"]),
        }
    )
    assert pick_best._cluster_keys == [2, 1]

    # Metadata later reveals the "late" photos were taken first. Every workflow
    # re-sorts while keeping the reviewer on the same photo or cluster.
    easy_delete._navigate_to(1)
    rotation._navigate_to(1)
    pick_best._load_cluster(1)
    state.date_cache["/p/late-a.jpg"] = datetime(2019, 1, 1)
    for widget in (rotation, easy_delete, pick_best):
        widget.refresh_capture_order()

    assert rotation._ordered_paths == ["/p/late-a.jpg", "/p/early-a.jpg"]
    assert rotation._ordered_paths[rotation._current_index] == "/p/late-a.jpg"
    assert easy_delete._flagged_paths == ["/p/late-a.jpg", "/p/early-a.jpg"]
    assert easy_delete._flagged_paths[easy_delete._current_index] == "/p/late-a.jpg"
    assert pick_best._cluster_keys == [1, 2]
    assert pick_best._cluster_keys[pick_best._cluster_index] == 1


def test_cull_list_is_built_and_resorted_in_capture_order():
    state = AppState()
    state.workflow_step = "cull"
    state.image_files_data = [{"path": p} for p in ("/p/a.jpg", "/p/b.jpg")]
    state.date_cache.update(
        {"/p/a.jpg": datetime(2023, 1, 1), "/p/b.jpg": datetime(2021, 1, 1)}
    )
    window = SimpleNamespace(
        app_state=state,
        show_folders_mode=False,
        group_by_similarity_mode=False,
        _cull_model_dirty=False,
        _note_model_item_populated=lambda: None,
        _create_standard_item=lambda record: QStandardItem(record["path"]),
        _ensure_cull_model_ready=Mock(),
        grouping_step_widget=Mock(),
        easy_delete_step_widget=None,
        fix_rotation_step_widget=None,
        pick_best_step_widget=None,
    )
    window._capture_order_key_for_record = lambda record: (
        MainWindow._capture_order_key_for_record(window, record)
    )
    window._current_capture_order = lambda: MainWindow._current_capture_order(window)
    window.mark_cull_model_dirty = lambda: setattr(window, "_cull_model_dirty", True)

    root = QStandardItem("root")
    MainWindow._populate_model_standard(window, root, state.image_files_data)
    assert [root.child(row).text() for row in range(2)] == ["/p/b.jpg", "/p/a.jpg"]

    window._populated_capture_order = window._current_capture_order()
    window._capture_order_signature = window._populated_capture_order
    MainWindow.refresh_capture_order(window)
    window._ensure_cull_model_ready.assert_not_called()
    window.grouping_step_widget.refresh_capture_order.assert_not_called()

    # A date change that keeps the order still regroups the Date view.
    state.date_cache["/p/b.jpg"] = datetime(2020, 1, 1)
    MainWindow.refresh_capture_order(window)
    window._ensure_cull_model_ready.assert_called_once()
    window.grouping_step_widget.refresh_capture_order.assert_called_once()

    # A metadata date arriving later changes the order, so the list is rebuilt.
    window._populated_capture_order = window._current_capture_order()
    state.date_cache["/p/a.jpg"] = datetime(2019, 1, 1)
    MainWindow.refresh_capture_order(window)
    assert window._ensure_cull_model_ready.call_count == 2
