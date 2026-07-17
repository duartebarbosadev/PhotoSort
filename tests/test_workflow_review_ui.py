import os
from pathlib import Path

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from types import SimpleNamespace

from PyQt6.QtGui import QAction, QKeySequence
from PyQt6.QtCore import Qt
from PyQt6.QtTest import QTest
from PyQt6.QtWidgets import QApplication, QFrame, QPushButton, QVBoxLayout, QWidget

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
    WorkflowDecisionCard,
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
    assert widget._items_list.item(0).text() == "Similar  ·  delete.jpg  ↔  keep.jpg"
    assert "SELECTED FOR TRASH" in widget._pair_left_hdr.text()
    assert "not confirmed" in widget._pair_left_hdr.text()
    assert widget._pair_left_card._name_label.text() == "delete.jpg"
    assert widget._pair_right_card._name_label.text() == "keep.jpg"
    assert isinstance(widget._pair_left_card, WorkflowDecisionCard)
    assert not hasattr(widget, "_keep_btn")
    assert not hasattr(widget, "_mark_btn")
    assert "border" not in widget._pair_left_img.styleSheet()
    assert "border" not in widget._pair_right_img.styleSheet()
    assert all(
        label.isHidden() for row in widget._pair_left_card._detail_rows for label in row
    )
    assert not marks

    shortcuts = {shortcut.key().toString(): shortcut for shortcut in widget._shortcuts}
    shortcuts["I"].activated.emit()
    _app.processEvents()

    detail_values = [
        value.text() for _key, value in widget._pair_left_card._detail_rows
    ]
    assert delete_path in detail_values
    assert "Lower sharpness" in detail_values
    assert all(
        not label.isHidden()
        for row in widget._pair_left_card._detail_rows[:3]
        for label in row
    )

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
    assert (
        "no file has been moved or deleted" in widget._state_banner.detail_label.text()
    )
    assert "Confirmed" in widget._items_list.item(0).text()
    assert "MARKED FOR TRASH" in widget._pair_right_hdr.text()


def test_easy_delete_enter_cancels_a_confirmed_pick_and_restores_prior_marks():
    marks = {"/tmp/already-marked.jpg"}
    review_path = "/tmp/review.jpg"
    widget = EasyDeleteStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda paths: marks.update(paths))
    widget.unmark_for_deletion_requested.connect(
        lambda paths: marks.difference_update(paths)
    )
    widget.show_results(
        {
            review_path: {
                "type": "duplicate",
                "pair_path": "/tmp/already-marked.jpg",
                "suggest_delete": True,
                "reason": "Suggested choice",
            }
        }
    )
    shortcuts = {shortcut.key().toString(): shortcut for shortcut in widget._shortcuts}

    shortcuts["1"].activated.emit()
    shortcuts["Return"].activated.emit()
    _app.processEvents()

    assert marks == {review_path}
    assert widget._confirmed_reviews == {review_path}
    assert widget._confirm_btn.text() == "Cancel confirmation"

    shortcuts["Return"].activated.emit()
    _app.processEvents()

    assert marks == {"/tmp/already-marked.jpg"}
    assert not widget._confirmed_reviews
    assert widget._state_banner.title_label.text() == "Choose, then confirm"
    assert widget._confirm_btn.text() == "Confirm  →"


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
    assert "SELECTED FOR TRASH" in widget._single_hdr.text()
    assert "border" not in widget._single_img.styleSheet()

    widget._on_apply_all()
    _app.processEvents()

    assert marks == {first, second}
    assert widget._confirmed_reviews == {first, second}


def test_easy_delete_apply_all_only_uses_visible_categories():
    duplicate = "/tmp/duplicate.jpg"
    duplicate_keep = "/tmp/duplicate-keep.jpg"
    blurry = "/tmp/blurry.jpg"
    marks: set[str] = set()
    widget = EasyDeleteStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda paths: marks.update(paths))
    widget.show_results(
        {
            duplicate: {
                "type": "duplicate",
                "pair_path": duplicate_keep,
                "suggest_delete": True,
            },
            blurry: {
                "type": "blur",
                "pair_path": None,
                "suggest_delete": True,
            },
        }
    )

    widget._category_checkboxes["blur"].setChecked(False)
    widget._on_apply_all()

    assert widget._apply_all_btn.text() == "Confirm visible"
    assert "currently visible categories" in widget._apply_all_btn.toolTip()
    assert "review or revise" in widget._apply_all_btn.toolTip()
    assert widget._apply_all_btn.parentWidget() is not widget._confirm_btn.parentWidget()
    assert widget._action_layout.indexOf(widget._confirm_btn) == 3
    assert marks == {duplicate}
    assert widget._confirmed_reviews == {duplicate}


def test_easy_delete_apply_requests_resolution_without_naming_next_step():
    widget = EasyDeleteStepWidget()
    requests: list[bool] = []
    widget.apply_requested.connect(lambda: requests.append(True))

    widget._apply_btn.click()

    assert widget._apply_btn.text() == "Apply"
    assert requests == [True]


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


def test_easy_delete_shortcuts_survive_focus_leaving_workflow_contents():
    host = QWidget()
    layout = QVBoxLayout(host)
    widget = EasyDeleteStepWidget()
    outside_button = QPushButton("Outside workflow contents")
    layout.addWidget(widget)
    layout.addWidget(outside_button)
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results(
        {
            "/tmp/first.jpg": {
                "type": "blur",
                "pair_path": None,
                "suggest_delete": True,
                "reason": "Blurry image",
            },
            "/tmp/second.jpg": {
                "type": "dark",
                "pair_path": None,
                "suggest_delete": True,
                "reason": "Dark image",
            },
        }
    )
    host.show()
    widget.setFocus()
    _app.processEvents()

    QTest.keyClick(widget, Qt.Key.Key_Down)
    assert widget._current_index == 1

    outside_button.setFocus()
    QTest.keyClick(outside_button, Qt.Key.Key_Up)
    assert widget._current_index == 0

    QTest.keyClick(outside_button, Qt.Key.Key_Return)
    assert widget._confirmed_reviews == {"/tmp/first.jpg"}


def test_review_pages_do_not_render_redundant_headers():
    organize = GroupingStepWidget()
    easy_delete = EasyDeleteStepWidget()
    fix_rotation = FixRotationStepWidget()
    pick_best = PickBestStepWidget()

    assert not hasattr(organize, "shortcut_strip")
    assert not hasattr(easy_delete, "_review_header")
    assert not hasattr(fix_rotation, "_review_header")
    assert not hasattr(pick_best, "_review_header")


def test_review_workflows_share_compact_review_list_panel():
    easy_delete = EasyDeleteStepWidget()
    fix_rotation = FixRotationStepWidget()
    pick_best = PickBestStepWidget()

    for widget in (easy_delete, fix_rotation, pick_best):
        panel = widget._review_list_panel
        assert panel.objectName() == "workflowReviewListPanel"
        assert panel.frameShape() == QFrame.Shape.NoFrame
        assert panel.minimumWidth() == 220
        assert panel.maximumWidth() == 310
        assert widget._items_list is panel.list_widget

    easy_delete.show_results(
        {
            "/tmp/blur.jpg": {
                "type": "blur",
                "suggest_delete": True,
                "reason": "Blurry image",
            }
        }
    )
    fix_rotation.show_results({"/tmp/rotated.jpg": 90})

    assert easy_delete._review_list_panel.count_label.text() == "1 item"
    assert fix_rotation._review_list_panel.count_label.text() == "1 item"
    assert easy_delete._review_list_panel.filters.isVisibleTo(easy_delete)
    assert not fix_rotation._review_list_panel.filters.isVisible()
    assert not pick_best._review_list_panel.filters.isVisible()


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

    assert widget._state_banner.title_label.text() == "Choose, then confirm"
    assert "Nothing is queued" in widget._state_banner.detail_label.text()
    assert widget.pending_rotations() == {}
    assert not widget._apply_btn.isEnabled()
    assert not hasattr(widget, "_mark_btn")
    assert not hasattr(widget, "_keep_btn")
    assert widget._items_list.item(0).text() == "first.jpg  ·  90° CW"
    assert widget._confirm_all_btn.parentWidget() is not widget._confirm_btn.parentWidget()
    assert widget._action_layout.indexOf(widget._confirm_btn) == 3
    assert all(
        child.text() != "Continue without applying  →"
        for child in widget.findChildren(type(widget._confirm_btn))
    )

    widget._on_confirm()
    assert widget.pending_rotations() == {first: 90}
    assert "Confirmed" in widget._items_list.item(0).text()
    assert "QUEUED" not in widget._items_list.item(0).text()
    assert widget._current_index == 1

    widget._current_img.clicked.emit()
    _app.processEvents()
    assert second not in widget._confirmed
    assert widget._current_hdr.text() == "ORIGINAL · SELECTED"
    widget._on_confirm()
    assert widget.pending_rotations() == {first: 90}

    widget._on_confirm_all()
    widget._on_apply()
    assert emitted == [{first: 90, second: -90}]
    widget.record_apply_result(first, True)
    widget.record_apply_result(second, True)
    widget.show_apply_complete(2, 0)

    assert widget._content_stack.currentIndex() == 2
    assert widget._empty_title.text() == "Rotations applied"
    assert not widget._ordered_paths


def _pick_best_payload(paths: list[str], scores: dict[str, float] | None = None):
    scores = scores or {}
    winner = max(paths, key=lambda path: scores.get(path, 0.0))
    return {
        "winner_path": winner,
        "ranked": [
            {"path": path, "final_score": scores.get(path)} for path in paths
        ],
        "failed": [],
        "all_paths": paths,
    }


def _pick_best_item(widget: PickBestStepWidget, path: str):
    return next(
        widget._items_list.item(index)
        for index in range(widget._items_list.count())
        if widget._items_list.item(index).data(Qt.ItemDataRole.UserRole) == path
    )


def _pick_best_state(widget: PickBestStepWidget, path: str) -> str:
    return _pick_best_item(widget, path).text().splitlines()[-1].strip()


def _pick_best_cluster_items(widget: PickBestStepWidget):
    return [
        widget._items_list.item(index)
        for index in range(widget._items_list.count())
        if widget._items_list.item(index).text().startswith("Cluster ")
    ]


def test_pick_best_publishes_trash_mark_as_soon_as_comparison_is_confirmed():
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
            1: _pick_best_payload(
                [challenger, winner], {challenger: 0.7, winner: 0.9}
            )
        }
    )

    assert not marks
    assert isinstance(widget._compare_cards[0], WorkflowDecisionCard)
    assert widget._compare_cards[0]._state_label.text() == "TRASH · not confirmed"
    assert widget._compare_cards[1]._state_label.text() == "KEEP · not confirmed"
    assert widget._review_list_panel.count_label.text() == "0/1 done"
    assert _pick_best_state(widget, challenger) == "Current"
    assert _pick_best_state(widget, winner) == "Current"
    assert not widget._done_btn.isEnabled()
    assert "Cluster 1 of 1" in widget._cluster_info_label.text()
    assert len(widget._subset_paths) == 2

    widget._on_confirm()

    assert marks == {challenger}
    assert widget._current_tournament().final_winner == winner
    assert _pick_best_state(widget, challenger) == "Trash"
    assert _pick_best_state(widget, winner) == "Winner"
    assert widget._compare_cards[0]._state_label.text() == "TRASH · confirmed"
    assert widget._compare_cards[1]._state_label.text() == "KEPT · confirmed"
    assert widget._done_btn.isEnabled()


def test_pick_best_revising_final_choice_restores_prior_marks_until_reconfirmed():
    marks: set[str] = set()
    challenger = "/tmp/challenger.jpg"
    winner = "/tmp/winner.jpg"
    widget = PickBestStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda paths: marks.update(paths))
    widget.unmark_for_deletion_requested.connect(
        lambda paths: marks.difference_update(paths)
    )
    widget.show_results({1: _pick_best_payload([challenger, winner], {winner: 0.9})})
    widget._on_confirm()
    assert marks == {challenger}

    widget._select_path(challenger)

    assert not marks
    assert widget._current_tournament().final_winner is None
    assert "not confirmed" in widget._compare_cards[0]._state_label.text()
    assert not widget._done_btn.isEnabled()

    widget._on_confirm()

    assert marks == {winner}
    assert widget._current_tournament().final_winner == challenger


def test_pick_best_33_photo_cluster_uses_rolling_pairwise_comparisons():
    paths = [f"/tmp/photo-{index:02}.jpg" for index in range(33)]
    scores = {path: float(33 - index) for index, path in enumerate(paths)}
    marks: set[str] = set()
    widget = PickBestStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda selected: marks.update(selected))
    widget.unmark_for_deletion_requested.connect(
        lambda selected: marks.difference_update(selected)
    )
    widget.show_results({7: _pick_best_payload(paths, scores)})
    tournament = widget._current_tournament()

    assert tournament.rounds[0].groups[0].paths == paths[:2]
    assert len(widget._subset_paths) == 2

    for comparison in range(31):
        widget._on_confirm()
        assert marks == set(paths[1 : comparison + 2])
        assert tournament.rounds[tournament.current_round].groups[0].paths == [
            paths[0],
            paths[comparison + 2],
        ]

    widget._on_confirm()

    assert tournament.final_winner == paths[0]
    assert tournament.finalized
    assert len(tournament.rounds) == 32
    assert len(marks) == 32
    assert paths[0] not in marks
    assert widget._done_btn.isEnabled()


def test_pick_best_total_comparisons_are_one_less_than_cluster_size():
    assert PickBestStepWidget._total_round_count(2) == 1
    assert PickBestStepWidget._total_round_count(3) == 2
    assert PickBestStepWidget._total_round_count(7) == 6
    assert PickBestStepWidget._total_round_count(33) == 32


def test_pick_best_missing_scores_preselects_first_photo_without_reordering():
    paths = ["/tmp/third.jpg", "/tmp/first.jpg", "/tmp/second.jpg"]
    widget = PickBestStepWidget()
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results({1: _pick_best_payload(paths)})

    group = widget._current_group()
    assert group.paths == paths[:2]
    assert group.selected_path == paths[0]

    widget._on_confirm()

    assert widget._current_group().paths == [paths[0], paths[2]]


def test_pick_best_revising_earlier_round_restores_marks_and_rebuilds_dependents():
    paths = [f"/tmp/photo-{index}.jpg" for index in range(7)]
    scores = {path: float(7 - index) for index, path in enumerate(paths)}
    marks: set[str] = set()
    widget = PickBestStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda selected: marks.update(selected))
    widget.unmark_for_deletion_requested.connect(
        lambda selected: marks.difference_update(selected)
    )
    widget.show_results({1: _pick_best_payload(paths, scores)})
    tournament = widget._current_tournament()

    while tournament.final_winner is None:
        widget._on_confirm()

    assert len(marks) == 6
    assert len(tournament.rounds) == 6

    widget._prev_round()
    original = widget._current_group().selected_path
    replacement = next(path for path in widget._current_group().paths if path != original)
    widget._select_path(replacement)

    assert tournament.final_winner is None
    assert len(tournament.rounds) == 5
    assert not widget._current_group().confirmed
    assert marks == set(paths[1:5])

    widget._on_confirm()

    assert len(tournament.rounds) == 6
    assert tournament.current_round == 5


def test_pick_best_up_and_down_shortcuts_navigate_comparison_history():
    paths = [f"/tmp/photo-{index}.jpg" for index in range(7)]
    widget = PickBestStepWidget()
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results({1: _pick_best_payload(paths)})
    widget.resize(1000, 700)
    widget.show()
    widget.setFocus()
    _app.processEvents()

    widget._on_confirm()
    widget._on_confirm()
    assert widget._current_tournament().current_round == 2

    QTest.keyClick(widget, Qt.Key.Key_Up)
    assert widget._current_tournament().current_round == 1

    QTest.keyClick(widget, Qt.Key.Key_Down)
    assert widget._current_tournament().current_round == 2


def test_pick_best_left_panel_keeps_every_photo_and_uses_simple_states():
    paths = [f"/tmp/photo-{index}.jpg" for index in range(7)]
    scores = {path: float(7 - index) for index, path in enumerate(paths)}
    widget = PickBestStepWidget()
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results({1: _pick_best_payload(paths, scores)})
    original_items = {path: _pick_best_item(widget, path) for path in paths}

    assert widget._items_list.item(0).text().startswith("Cluster 1 · 7 photos")
    assert [
        widget._items_list.item(index).data(Qt.ItemDataRole.UserRole)
        for index in range(1, 8)
    ] == paths
    assert all(
        not (_pick_best_item(widget, path).flags() & Qt.ItemFlag.ItemIsSelectable)
        for path in paths
    )
    assert widget._review_list_panel.count_label.text() == "0/1 done"

    for _ in range(3):
        widget._on_confirm()

    assert widget._current_tournament().current_round == 3
    assert all(
        _pick_best_item(widget, path) is original_items[path] for path in paths
    )
    for survivor_index in (0, 4):
        assert _pick_best_state(widget, paths[survivor_index]) == "Current"
    for eliminated_index in (1, 2, 3):
        assert _pick_best_state(widget, paths[eliminated_index]) == "Trash"
    assert widget._review_list_panel.count_label.text() == "0/1 done"

    subset_before = list(widget._subset_paths)
    widget._on_photo_item_clicked(_pick_best_item(widget, paths[2]))

    tournament = widget._current_tournament()
    assert tournament.current_round == 3
    assert tournament.current_group == 0
    assert widget._subset_paths == subset_before


def test_pick_best_left_panel_shows_every_cluster_and_switches_from_summary():
    first_paths = ["/tmp/first-a.jpg", "/tmp/first-b.jpg"]
    second_paths = [
        "/tmp/second-a.jpg",
        "/tmp/second-b.jpg",
        "/tmp/second-c.jpg",
    ]
    widget = PickBestStepWidget()
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results(
        {
            10: _pick_best_payload(first_paths),
            20: _pick_best_payload(second_paths),
        }
    )

    cluster_items = _pick_best_cluster_items(widget)
    assert len(cluster_items) == 2
    assert cluster_items[0].text() == "Cluster 1 · 2 photos\nCurrent · 2 active"
    assert cluster_items[1].text() == "Cluster 2 · 3 photos\nNot started"
    assert widget._review_list_panel.count_label.text() == "0/2 done"
    assert _pick_best_item(widget, first_paths[0]) is not None
    first_cluster_row = widget._items_list.row(cluster_items[0])
    assert (
        widget._items_list.item(first_cluster_row + 1).data(Qt.ItemDataRole.UserRole)
        == first_paths[0]
    )

    widget._on_photo_item_clicked(cluster_items[1])

    assert widget._cluster_index == 1
    assert _pick_best_item(widget, second_paths[0]) is not None
    assert all(
        widget._items_list.item(index).data(Qt.ItemDataRole.UserRole)
        not in first_paths
        for index in range(widget._items_list.count())
    )
    second_cluster_row = widget._items_list.row(_pick_best_cluster_items(widget)[1])
    assert (
        widget._items_list.item(second_cluster_row + 1).data(Qt.ItemDataRole.UserRole)
        == second_paths[0]
    )

    widget._on_keep_all()
    widget._on_keep_all()

    cluster_items = _pick_best_cluster_items(widget)
    assert cluster_items[1].text() == "Cluster 2 · 3 photos\nComplete · 3 kept"
    assert widget._review_list_panel.count_label.text() == "1/2 done"

    widget._on_photo_item_clicked(cluster_items[0])

    assert widget._cluster_index == 0
    assert _pick_best_item(widget, first_paths[0]) is not None


def test_pick_best_keep_all_protects_group_and_completes_without_forced_winner():
    paths = ["/tmp/left.jpg", "/tmp/right.jpg"]
    marks = {paths[1]}
    widget = PickBestStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda selected: marks.update(selected))
    widget.unmark_for_deletion_requested.connect(
        lambda selected: marks.difference_update(selected)
    )
    widget.show_results({1: _pick_best_payload(paths)})

    widget._on_keep_all()

    tournament = widget._current_tournament()
    assert tournament.finalized
    assert tournament.final_winner == paths[0]
    assert not marks
    assert widget._done_btn.isEnabled()
    assert widget._review_list_panel.count_label.text() == "1/1 done"
    assert _pick_best_state(widget, paths[0]) == "Winner"
    assert _pick_best_state(widget, paths[1]) == "Kept"
    assert all(
        card._state_label.text() == "KEPT · confirmed"
        for card in widget._compare_cards[:2]
    )


def test_pick_best_keep_all_can_mix_with_a_winner_in_the_same_round():
    paths = [f"/tmp/photo-{index}.jpg" for index in range(7)]
    scores = {path: float(7 - index) for index, path in enumerate(paths)}
    marks: set[str] = set()
    widget = PickBestStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda selected: marks.update(selected))
    widget.unmark_for_deletion_requested.connect(
        lambda selected: marks.difference_update(selected)
    )
    widget.show_results({1: _pick_best_payload(paths, scores)})

    widget._on_keep_all()
    widget._on_confirm()
    widget._on_keep_all()
    widget._on_confirm()
    widget._on_confirm()
    widget._on_confirm()

    tournament = widget._current_tournament()
    assert tournament.finalized
    assert tournament.final_winner == paths[0]
    assert marks == {paths[2], paths[4], paths[5], paths[6]}
    assert PickBestStepWidget._kept_paths(tournament) == {
        paths[0],
        paths[1],
        paths[3],
    }


def test_pick_best_kept_incumbent_can_be_replaced_by_next_challenger():
    paths = ["/tmp/incumbent.jpg", "/tmp/kept.jpg", "/tmp/challenger.jpg"]
    scores = {paths[0]: 0.8, paths[1]: 0.7, paths[2]: 0.95}
    marks: set[str] = set()
    widget = PickBestStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda selected: marks.update(selected))
    widget.unmark_for_deletion_requested.connect(
        lambda selected: marks.difference_update(selected)
    )
    widget.show_results({1: _pick_best_payload(paths, scores)})

    widget._on_keep_all()

    assert widget._current_group().paths == [paths[0], paths[2]]
    assert widget._current_group().selected_path == paths[2]
    assert not marks
    assert _pick_best_state(widget, paths[0]) == "Current"
    assert "Current · 2 active" in _pick_best_cluster_items(widget)[0].text()

    widget._on_confirm()

    tournament = widget._current_tournament()
    assert tournament.finalized
    assert tournament.final_winner == paths[2]
    assert marks == {paths[0]}
    assert PickBestStepWidget._kept_paths(tournament) == {paths[1], paths[2]}
    assert _pick_best_state(widget, paths[0]) == "Trash"
    assert _pick_best_state(widget, paths[1]) == "Kept"
    assert _pick_best_state(widget, paths[2]) == "Winner"


def test_pick_best_revising_kept_group_restores_marks_until_reconfirmed():
    paths = ["/tmp/left.jpg", "/tmp/right.jpg"]
    initial_marks = {paths[1]}
    marks = set(initial_marks)
    widget = PickBestStepWidget()
    widget.set_is_marked_func(marks.__contains__)
    widget.mark_for_deletion_requested.connect(lambda selected: marks.update(selected))
    widget.unmark_for_deletion_requested.connect(
        lambda selected: marks.difference_update(selected)
    )
    widget.show_results({1: _pick_best_payload(paths)})
    widget._on_keep_all()

    widget._select_path(paths[0])

    tournament = widget._current_tournament()
    assert not tournament.finalized
    assert not tournament.rounds[0].groups[0].keep_all
    assert marks == initial_marks
    assert not widget._done_btn.isEnabled()

    widget._on_confirm()

    assert tournament.finalized
    assert tournament.final_winner == paths[0]
    assert marks == {paths[1]}


def test_pick_best_left_right_and_enter_shortcuts_control_tournament():
    widget = PickBestStepWidget()
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results(
        {
            index: _pick_best_payload(
                [f"/tmp/challenger-{index}.jpg", f"/tmp/winner-{index}.jpg"],
                {f"/tmp/winner-{index}.jpg": 0.9},
            )
            for index in (1, 2)
        }
    )
    widget.resize(1000, 700)
    widget.show()
    widget.setFocus()
    _app.processEvents()

    QTest.keyClick(widget, Qt.Key.Key_Right)
    assert widget._cluster_index == 1

    QTest.keyClick(widget, Qt.Key.Key_Left)
    assert widget._cluster_index == 0

    QTest.keyClick(widget, Qt.Key.Key_Return)
    assert widget._tournaments[0].final_winner == "/tmp/winner-1.jpg"
    assert widget._cluster_index == 1


def test_pick_best_requests_previews_only_for_current_pair():
    class PreviewHost(QWidget):
        def __init__(self):
            super().__init__()
            self.requests: list[list[str]] = []
            self.image_pipeline = None

        def request_interactive_previews(self, paths):
            self.requests.append(list(paths))

    paths = [f"/tmp/photo-{index}.jpg" for index in range(7)]
    host = PreviewHost()
    layout = QVBoxLayout(host)
    widget = PickBestStepWidget()
    layout.addWidget(widget)
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results({1: _pick_best_payload(paths)})

    assert host.requests[-1] == paths[:2]

    widget._on_confirm()

    assert host.requests[-1] == [paths[0], paths[2]]
    assert all(len(request) <= 2 for request in host.requests)
    assert len(widget._sync_viewer.image_viewers) == 2
    assert all(not viewer.isHidden() for viewer in widget._sync_viewer.image_viewers)


def test_easy_delete_focuses_exact_duplicate_without_changing_decision():
    left = "/tmp/left.jpg"
    right = "/tmp/right.jpg"
    widget = EasyDeleteStepWidget()
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results(
        {
            left: {
                "type": "duplicate",
                "pair_path": right,
                "suggest_delete": True,
                "reason": "Similar",
            }
        }
    )
    pending_before = dict(widget._pending_delete_by_review)

    assert widget.focus_image(right)

    assert widget._focused_path == right
    assert widget._pair_right_card._focused
    assert widget._pending_delete_by_review == pending_before


def test_easy_delete_reentry_keeps_position_when_active_image_is_not_flagged():
    first = "/tmp/first.jpg"
    second = "/tmp/second.jpg"
    results = {
        first: {
            "type": "blur",
            "pair_path": None,
            "suggest_delete": True,
            "reason": "Blurry",
        },
        second: {
            "type": "dark",
            "pair_path": None,
            "suggest_delete": True,
            "reason": "Dark",
        },
    }
    widget = EasyDeleteStepWidget()
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results(results)
    assert widget.focus_image(second)

    widget.show_results(results)
    assert not widget.focus_image("/tmp/not-flagged.jpg")

    assert widget._flagged_paths[widget._current_index] == second


def test_fix_rotation_focus_does_not_change_queued_state():
    first = "/tmp/first.jpg"
    second = "/tmp/second.jpg"
    widget = FixRotationStepWidget()
    widget.show_results({first: 90, second: -90})
    marked_before = dict(widget._marked)

    assert widget.focus_image(second)

    assert widget._ordered_paths[widget._current_index] == second
    assert widget._marked == marked_before


def test_pick_best_focus_finds_photo_group_without_changing_selection():
    challengers = [f"/tmp/challenger-{index}.jpg" for index in range(3)]
    winner = "/tmp/winner.jpg"
    widget = PickBestStepWidget()
    widget.set_is_marked_func(lambda _path: False)
    widget.show_results(
        {
            1: {
                "winner_path": winner,
                "ranked": [
                    {"path": path, "final_score": 0.8 - index * 0.1}
                    for index, path in enumerate([winner, *challengers])
                ],
                "failed": [],
                "all_paths": [*challengers, winner],
            }
        }
    )
    tournament = widget._current_tournament()
    selections_before = [
        group.selected_path for group in tournament.rounds[0].groups
    ]

    assert widget.focus_image(challengers[1])

    assert challengers[1] in widget._subset_paths
    assert widget._subset_paths[widget._focused_slot_index] == challengers[1]
    assert [group.selected_path for group in tournament.rounds[0].groups] == selections_before


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
