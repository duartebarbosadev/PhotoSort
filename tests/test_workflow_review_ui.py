import os
from pathlib import Path

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
    WORKFLOW_SHORTCUTS,
    WorkflowShortcutStrip,
)


_app = QApplication.instance() or QApplication([])


def test_easy_delete_requires_confirmation_before_staging_trash():
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

    assert widget._state_banner.title_label.text() == "Choose, then confirm"
    assert "REVIEW" in widget._items_list.item(0).text()
    assert "SELECTED FOR TRASH" in widget._pair_left_hdr.text()
    assert "not confirmed" in widget._pair_left_hdr.text()
    assert not marks

    widget._pair_right_img.clicked.emit()
    _app.processEvents()

    assert not marks
    assert "SELECTED TO KEEP" in widget._pair_left_hdr.text()
    assert "SELECTED FOR TRASH" in widget._pair_right_hdr.text()

    widget._on_confirm()
    _app.processEvents()

    assert keep_path in marks
    assert delete_path not in marks
    assert widget._state_banner.title_label.text() == "Decision confirmed"
    assert "no file has been moved or deleted" in widget._state_banner.detail_label.text()
    assert "CONFIRMED" in widget._items_list.item(0).text()
    assert "MARKED FOR TRASH" in widget._pair_right_hdr.text()


def test_easy_delete_confirm_advances_and_confirm_all_uses_suggestions():
    marks: set[str] = set()
    first = "/tmp/first.jpg"
    first_keep = "/tmp/first-keep.jpg"
    second = "/tmp/second.jpg"
    widget = EasyDeleteStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda paths: marks.update(paths))
    widget.unmark_for_deletion_requested.connect(
        lambda paths: marks.difference_update(paths)
    )
    widget.show_results(
        {
            first: {
                "type": "duplicate",
                "pair_path": first_keep,
                "suggest_delete": True,
                "duplicate_kind": "exact",
                "reason": "The files are byte-for-byte identical",
            },
            second: {
                "type": "blur",
                "pair_path": None,
                "suggest_delete": True,
                "reason": "Blurry image",
            },
        }
    )

    assert "Exact duplicate" in widget._issue_label.text()
    assert widget._suggestion_label.isHidden()
    widget._on_confirm()
    _app.processEvents()

    assert first in marks
    assert widget._current_index == 1

    widget._on_apply_all()
    _app.processEvents()

    assert marks == {first, second}
    assert widget._confirmed_reviews == {first, second}


def test_easy_delete_arrow_shortcuts_separate_choice_from_navigation():
    first = "/tmp/first.jpg"
    first_keep = "/tmp/first-keep.jpg"
    second = "/tmp/second.jpg"
    widget = EasyDeleteStepWidget()
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results(
        {
            first: {
                "type": "duplicate",
                "pair_path": first_keep,
                "suggest_delete": True,
                "duplicate_kind": "near",
                "reason": "Suggested choice",
            },
            second: {
                "type": "blur",
                "pair_path": None,
                "suggest_delete": True,
                "reason": "Blurry image",
            },
        }
    )
    shortcuts = {shortcut.key().toString(): shortcut for shortcut in widget._shortcuts}

    shortcuts["Right"].activated.emit()

    assert widget._current_index == 0
    assert "SELECTED FOR TRASH" in widget._pair_right_hdr.text()

    shortcuts["Down"].activated.emit()

    assert widget._current_index == 1


def test_page_headers_no_longer_duplicate_the_footer_shortcuts():
    organize = GroupingStepWidget()
    easy_delete = EasyDeleteStepWidget()

    assert not hasattr(organize, "shortcut_strip")
    assert not hasattr(easy_delete._review_header, "shortcut_strip")


def test_footer_shortcuts_use_the_most_columns_that_fit():
    strip = WorkflowShortcutStrip(ORGANIZE_SHORTCUTS)
    stylesheet = Path("src/ui/dark_theme.qss").read_text(encoding="utf-8")
    strip.setStyleSheet(stylesheet)
    strip.resize(1200, 100)
    strip.show()
    _app.processEvents()

    assert strip._current_columns == len(ORGANIZE_SHORTCUTS)

    strip.resize(320, 100)
    _app.processEvents()

    assert 1 < strip._current_columns < len(ORGANIZE_SHORTCUTS)


def test_organize_top_bar_returns_to_a_single_control_row():
    organize = GroupingStepWidget()
    stylesheet = Path("src/ui/dark_theme.qss").read_text(encoding="utf-8")
    organize.setStyleSheet(stylesheet)
    organize.resize(1800, 900)
    organize.show()
    _app.processEvents()

    assert organize.top_bar.height() == 52
    assert not hasattr(organize, "stats_label")
    assert organize.primary_button.parentWidget() is organize.bottom_bar
    assert not hasattr(organize, "skip_button")


def test_workflow_footer_navigation_is_centered_in_the_window(monkeypatch):
    monkeypatch.setattr("ui.main_window.get_show_workflow_shortcuts", lambda: True)
    window = MainWindow()
    stylesheet = Path("src/ui/dark_theme.qss").read_text(encoding="utf-8")
    window.setStyleSheet(stylesheet)
    window.resize(1600, 900)
    window.show()
    _app.processEvents()

    buttons = (
        window.step_organize_button,
        window.step_easy_delete_button,
        window.step_fix_rotation_button,
        window.step_pick_best_button,
        window.step_cull_button,
    )
    status_bar = window.statusBar()
    left = buttons[0].mapTo(status_bar, buttons[0].rect().topLeft()).x()
    right = buttons[-1].mapTo(status_bar, buttons[-1].rect().bottomRight()).x()
    button_center = (left + right) / 2

    assert window.workflow_nav_host.width() >= status_bar.width() * 0.9
    assert window.workflow_nav.width() < window.workflow_nav_host.width() * 0.5
    nav_left = window.workflow_nav.mapTo(
        status_bar, window.workflow_nav.rect().topLeft()
    ).x()
    nav_right = window.workflow_nav.mapTo(
        status_bar, window.workflow_nav.rect().bottomRight()
    ).x()
    assert nav_left <= left
    assert nav_right >= right
    assert left - nav_left <= 12
    assert nav_right - right <= 12
    assert abs(button_center - status_bar.width() / 2) <= 16

    status_bar.showMessage("Status text remains visible")
    _app.processEvents()
    assert window.workflow_status_label.text() == "Status text remains visible"

    for workflow_step, specs in WORKFLOW_SHORTCUTS.items():
        window._set_workflow_step(workflow_step)
        strip = window.workflow_shortcut_strips[workflow_step]
        assert window.workflow_shortcut_stack.currentWidget() is strip
        assert strip.shortcut_specs == specs

    window.set_workflow_shortcuts_visible(False)
    assert window.workflow_shortcut_stack.isHidden()
    window.set_workflow_shortcuts_visible(True)
    assert window.workflow_shortcut_stack.isVisible()
    window.close()


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
