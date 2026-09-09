import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from types import SimpleNamespace
from unittest.mock import Mock

from PyQt6.QtCore import QItemSelectionModel
from PyQt6.QtWidgets import QApplication

from src.ui.main_window import MainWindow
from src.ui.selection_utils import resolve_anchor_index_after_rebuild
from ui.easy_delete_step_widget import EasyDeleteStepWidget
from ui.fix_rotation_step_widget import FixRotationStepWidget
from ui.pick_best_step_widget import PickBestStepWidget


_app = QApplication.instance() or QApplication([])


# ---------------------------------------------------------------------------
# Shared helper
# ---------------------------------------------------------------------------


def test_anchor_index_keeps_surviving_anchor():
    before = ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
    after = ["a.jpg", "c.jpg", "d.jpg"]

    assert resolve_anchor_index_after_rebuild(before, after, "c.jpg") == 1


def test_anchor_index_falls_back_to_nearest_survivor():
    before = ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]
    after = ["a.jpg", "d.jpg"]

    # 'b.jpg' is gone; the next surviving neighbour is 'd.jpg', not the first row.
    assert resolve_anchor_index_after_rebuild(before, after, "b.jpg") == 1


def test_anchor_index_reports_empty_queue():
    assert resolve_anchor_index_after_rebuild(["a.jpg"], [], "a.jpg") == -1


# ---------------------------------------------------------------------------
# Cull (main file view) commit
# ---------------------------------------------------------------------------


def _commit_context(visible_after, selected_index_valid=True, already_selected=False):
    proxy_index = Mock()
    proxy_index.isValid.return_value = selected_index_valid
    selection_model = Mock()
    selection_model.isSelected.return_value = already_selected
    active_view = Mock()
    active_view.selectionModel.return_value = selection_model

    context = SimpleNamespace(
        _get_active_file_view=lambda: active_view,
        _get_all_visible_image_paths=lambda: visible_after,
        _find_proxy_index_for_path=Mock(return_value=proxy_index),
        _update_image_info_label=Mock(),
        _handle_file_selection_changed=Mock(),
        statusBar=Mock(return_value=Mock()),
        advanced_image_viewer=Mock(),
        app_controller=SimpleNamespace(resume_folder_load_after_deletion=None),
        _close_after_deletion=False,
        _pending_workflow_transition=None,
    )
    return context, active_view, selection_model


def test_commit_keeps_current_image_when_it_survives():
    visible_before = ["a.jpg", "b.jpg", "current.jpg", "d.jpg"]
    deleted = ["a.jpg", "d.jpg"]
    visible_after = ["b.jpg", "current.jpg"]
    context, active_view, selection_model = _commit_context(
        visible_after, already_selected=True
    )

    MainWindow._finish_marked_deletion_batch(
        context,
        visible_before,
        "current.jpg",
        deleted,
        deleted,
        deleted,
        {},
        set(deleted),
    )

    context._find_proxy_index_for_path.assert_called_once_with("current.jpg")
    # The surviving current image stays selected: no ClearAndSelect, no reload.
    selection_model.select.assert_not_called()
    active_view.setCurrentIndex.assert_not_called()
    selection_model.setCurrentIndex.assert_called_once()
    assert (
        selection_model.setCurrentIndex.call_args[0][1]
        == QItemSelectionModel.SelectionFlag.NoUpdate
    )
    context._handle_file_selection_changed.assert_not_called()


def test_commit_reselects_when_current_image_was_deleted():
    visible_before = ["a.jpg", "b.jpg", "current.jpg", "d.jpg"]
    deleted = ["current.jpg"]
    visible_after = ["a.jpg", "b.jpg", "d.jpg"]
    context, active_view, selection_model = _commit_context(visible_after)

    MainWindow._finish_marked_deletion_batch(
        context,
        visible_before,
        "current.jpg",
        deleted,
        deleted,
        deleted,
        {},
        set(deleted),
    )

    # Nearest survivor after the deleted anchor, not the top of the list.
    context._find_proxy_index_for_path.assert_called_once_with("d.jpg")
    active_view.setCurrentIndex.assert_called_once()
    selection_model.select.assert_called_once()


# ---------------------------------------------------------------------------
# Easy Delete review queue
# ---------------------------------------------------------------------------


def _easy_delete_results(paths):
    return {
        path: {"type": "blur", "suggest_delete": True, "pair_path": None}
        for path in paths
    }


def test_easy_delete_keeps_current_review_after_file_mutation():
    widget = EasyDeleteStepWidget()
    paths = [f"/photos/{name}.jpg" for name in ("a", "b", "c", "d")]
    widget.show_results(_easy_delete_results(paths))
    widget._navigate_to(2)
    assert widget._flagged_paths[widget._current_index] == "/photos/c.jpg"

    surviving = [p for p in paths if p != "/photos/a.jpg"]
    widget.sync_results_after_file_mutation(_easy_delete_results(surviving))

    assert widget._flagged_paths[widget._current_index] == "/photos/c.jpg"


def test_easy_delete_falls_back_to_neighbour_when_review_is_deleted():
    widget = EasyDeleteStepWidget()
    paths = [f"/photos/{name}.jpg" for name in ("a", "b", "c", "d")]
    widget.show_results(_easy_delete_results(paths))
    widget._navigate_to(2)

    surviving = [p for p in paths if p != "/photos/c.jpg"]
    widget.sync_results_after_file_mutation(_easy_delete_results(surviving))

    assert widget._flagged_paths[widget._current_index] == "/photos/d.jpg"


# ---------------------------------------------------------------------------
# Fix Rotation review queue
# ---------------------------------------------------------------------------


def test_fix_rotation_keeps_current_review_after_file_mutation():
    widget = FixRotationStepWidget()
    paths = [f"/photos/{name}.jpg" for name in ("a", "b", "c", "d")]
    widget.show_results(dict.fromkeys(paths, 90))
    widget._navigate_to(2)
    assert widget._ordered_paths[widget._current_index] == "/photos/c.jpg"

    surviving = {p: 90 for p in paths if p != "/photos/a.jpg"}
    widget.sync_results_after_file_mutation(surviving)

    assert widget._ordered_paths[widget._current_index] == "/photos/c.jpg"


# ---------------------------------------------------------------------------
# Pick Best tournaments
# ---------------------------------------------------------------------------


def _pick_best_cluster(paths):
    return {
        "winner_path": paths[0],
        "ranked": [{"path": path, "final_score": 1.0} for path in paths],
        "failed": [],
        "all_paths": list(paths),
        "unsupported_paths": [],
    }


def _pick_best_results(cluster_paths_by_key):
    return {
        key: _pick_best_cluster(paths)
        for key, paths in sorted(cluster_paths_by_key.items())
    }


def test_pick_best_keeps_current_cluster_after_file_mutation():
    widget = PickBestStepWidget()
    results = _pick_best_results(
        {
            1: ["/photos/c1a.jpg", "/photos/c1b.jpg"],
            2: ["/photos/c2a.jpg", "/photos/c2b.jpg"],
            3: ["/photos/c3a.jpg", "/photos/c3b.jpg"],
        }
    )
    widget.show_results(results)
    widget._load_cluster(2)
    assert widget._cluster_index == 2

    # Deleting inside cluster 1 invalidates only that cluster.
    surviving = _pick_best_results(
        {
            2: ["/photos/c2a.jpg", "/photos/c2b.jpg"],
            3: ["/photos/c3a.jpg", "/photos/c3b.jpg"],
        }
    )
    widget.sync_results_after_file_mutation(surviving)

    assert widget._cluster_keys[widget._cluster_index] == 3


def test_pick_best_falls_back_to_neighbour_cluster_when_its_own_is_removed():
    widget = PickBestStepWidget()
    results = _pick_best_results(
        {
            1: ["/photos/c1a.jpg", "/photos/c1b.jpg"],
            2: ["/photos/c2a.jpg", "/photos/c2b.jpg"],
            3: ["/photos/c3a.jpg", "/photos/c3b.jpg"],
        }
    )
    widget.show_results(results)
    widget._load_cluster(1)

    surviving = _pick_best_results(
        {
            1: ["/photos/c1a.jpg", "/photos/c1b.jpg"],
            3: ["/photos/c3a.jpg", "/photos/c3b.jpg"],
        }
    )
    widget.sync_results_after_file_mutation(surviving)

    assert widget._cluster_keys[widget._cluster_index] == 3
