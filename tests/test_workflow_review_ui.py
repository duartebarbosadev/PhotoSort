import os

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from types import SimpleNamespace

from PyQt6.QtGui import QAction, QKeySequence
from PyQt6.QtWidgets import QApplication, QWidget

from ui.easy_delete_step_widget import EasyDeleteStepWidget
from ui.fix_rotation_step_widget import FixRotationStepWidget
from ui.grouping_step_widget import GroupingStepWidget
from ui.pick_best_step_widget import PickBestStepWidget
from ui.main_window import MainWindow
from ui.workflow_review_components import (
    EASY_DELETE_SHORTCUTS,
    FIX_ROTATION_SHORTCUTS,
    ORGANIZE_SHORTCUTS,
    PICK_BEST_SHORTCUTS,
)


_app = QApplication.instance() or QApplication([])


def test_easy_delete_uses_explicit_staged_trash_state():
    marks: set[str] = set()
    delete_path = "/tmp/delete.jpg"
    keep_path = "/tmp/keep.jpg"
    widget = EasyDeleteStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda paths: marks.update(paths))
    widget.unmark_for_deletion_requested.connect(
        lambda paths: marks.difference_update(paths)
    )

    widget.show_results(
        {
            delete_path: {
                "type": "duplicate",
                "pair_path": keep_path,
                "suggest_delete": True,
                "reason": "Lower sharpness",
            }
        }
    )

    assert widget._state_banner.title_label.text() == "Keeping this photo"
    assert "KEEP" in widget._items_list.item(0).text()
    assert "delete suggested" in widget._pair_left_hdr.text()

    widget._set_current_marked(True)
    _app.processEvents()

    assert delete_path in marks
    assert widget._state_banner.title_label.text() == "Marked for Trash"
    assert "has not been moved or deleted" in widget._state_banner.detail_label.text()
    assert "MARKED" in widget._items_list.item(0).text()
    assert "staged only" in widget._pair_left_hdr.text()


def test_fix_rotation_distinguishes_preview_queue_and_applied_state():
    first = "/tmp/first.jpg"
    second = "/tmp/second.jpg"
    emitted: list[dict] = []
    widget = FixRotationStepWidget()
    widget.apply_rotations_requested.connect(emitted.append)
    widget.show_results({first: 90, second: -90})

    assert widget._state_banner.title_label.text().startswith("Queued: rotate")
    assert "only previewed" in widget._state_banner.detail_label.text()
    assert widget._mark_btn.isChecked()
    assert "QUEUED" in widget._items_list.item(0).text()

    widget._set_current_marked(False)
    _app.processEvents()
    assert widget._keep_btn.isChecked()
    assert widget._preview_hdr.text() == "LEAVE AS-IS · no change"

    widget._on_mark_all()
    widget._on_apply()
    assert emitted == [{first: 90, second: -90}]
    widget.record_apply_result(first, True)
    widget.record_apply_result(second, True)
    widget.show_apply_complete(2, 0)

    assert widget._content_stack.currentIndex() == 2
    assert widget._empty_title.text() == "Rotations applied"
    assert not widget._ordered_paths


def test_pick_best_stages_initial_recommendations_in_shared_state():
    marks: set[str] = set()
    challenger = "/tmp/challenger.jpg"
    winner = "/tmp/winner.jpg"
    widget = PickBestStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda paths: marks.update(paths))
    widget.unmark_for_deletion_requested.connect(
        lambda paths: marks.difference_update(paths)
    )

    widget.show_results(
        {
            1: {
                "winner_path": winner,
                "ranked": [
                    {"path": winner, "final_score": 0.9},
                    {"path": challenger, "final_score": 0.7},
                ],
                "failed": [],
                "all_paths": [challenger, winner],
            }
        }
    )

    assert challenger in marks
    assert winner not in marks
    assert widget._compare_cards[0]._state_label.text() == "MARKED FOR TRASH · staged"
    assert widget._compare_cards[1]._state_label.text() == "AI PICK · KEEP"
    assert "marked for Trash" in widget._review_header.summary_label.text()
    assert (
        "no files move"
        in widget._review_header.findChild(
            type(widget._review_header.summary_label), "workflowReviewDescription"
        ).text()
    )


def test_visible_shortcut_specs_are_the_installed_source_of_truth():
    widgets_and_specs = (
        (GroupingStepWidget(), ORGANIZE_SHORTCUTS),
        (EasyDeleteStepWidget(), EASY_DELETE_SHORTCUTS),
        (FixRotationStepWidget(), FIX_ROTATION_SHORTCUTS),
        (PickBestStepWidget(), PICK_BEST_SHORTCUTS),
    )

    for widget, specs in widgets_and_specs:
        expected = sum(len(spec.sequences) for spec in specs)
        assert len(widget._shortcuts) == expected


def test_guided_workflows_suspend_and_restore_cull_shortcuts():
    owner = QWidget()
    action_names = (
        "find_action",
        "rotate_clockwise_action",
        "rotate_counterclockwise_action",
        "rotate_180_action",
        "mark_for_delete_action",
        "commit_deletions_action",
        "clear_marked_deletions_action",
        "actual_size_action",
        "fit_to_view_action",
        "zoom_in_action",
        "zoom_out_action",
        "single_view_action",
        "side_by_side_view_action",
        "sync_pan_zoom_action",
        "view_list_action",
        "view_icons_action",
        "view_grid_action",
        "view_rotation_action",
        "toggle_folder_view_action",
        "group_by_similarity_action",
        "toggle_thumbnails_action",
        "detect_blur_action",
        "toggle_metadata_sidebar_action",
    )
    actions = {}
    for index, name in enumerate(action_names):
        action = QAction(owner)
        action.setShortcut(QKeySequence(f"Ctrl+F{(index % 10) + 1}"))
        actions[name] = action
    context = SimpleNamespace(menu_manager=SimpleNamespace(**actions))

    MainWindow._set_cull_shortcuts_active(context, False)
    assert all(action.shortcut().isEmpty() for action in actions.values())

    MainWindow._set_cull_shortcuts_active(context, True)
    assert all(not action.shortcut().isEmpty() for action in actions.values())
