import os
from types import SimpleNamespace
from unittest.mock import Mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PyQt6.QtGui import QIcon, QPixmap
from PyQt6.QtWidgets import QApplication, QMenu

from src.core.grouping import GroupingGroup, GroupingPlan
from src.ui.grouping_step_widget import (
    DroppableGroupingTree,
    GroupingStepWidget,
    ITEM_GROUP,
    ROLE_KIND,
)


_app = QApplication.instance() or QApplication([])


class _CacheOnlyPipeline:
    def __init__(
        self,
        *,
        cached_preview: QPixmap | None = None,
        cached_thumbnail: QPixmap | None = None,
    ):
        self.get_cached_thumbnail_qpixmap = Mock(return_value=cached_thumbnail)
        self.get_cached_preview_qpixmap = Mock(return_value=cached_preview)
        self.get_immediate_review_qpixmap = Mock(
            return_value=(
                cached_preview if cached_preview is not None else cached_thumbnail,
                cached_preview is not None,
            )
        )
        self.get_thumbnail_qpixmap = Mock(return_value=None)
        self.get_preview_qpixmap = Mock(return_value=cached_preview)


def test_grouping_step_widget_tracks_mode_and_busy_state():
    source_root = "/tmp/demo"
    widget = GroupingStepWidget()
    widget.set_source_folder(source_root)
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="Beach",
                    source_paths=["a.jpg"],
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        "/tmp/demo/PhotoSort Groups/location",
    )

    assert widget.primary_button.isEnabled()
    assert widget.primary_button.text() == "Review, then Apply"
    assert widget.folder_button.isEnabled()
    assert all(btn.isEnabled() for btn in widget._mode_buttons.values())

    widget.set_busy(True)
    assert not widget.primary_button.isEnabled()
    assert widget.primary_button.text() == "Applying…"
    assert not widget.folder_button.isEnabled()
    assert not widget.back_button.isEnabled()
    assert not any(btn.isEnabled() for btn in widget._mode_buttons.values())

    widget.set_busy(False)
    assert widget.primary_button.isEnabled()
    assert widget.primary_button.text() == "Review, then Apply"
    assert widget.folder_button.isEnabled()
    assert widget.back_button.isEnabled()
    assert all(btn.isEnabled() for btn in widget._mode_buttons.values())


def test_grouping_step_widget_detects_unsaved_grouping_edits(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    assert not widget.has_unsaved_grouping_edits()

    after_item = widget._after_file_items_by_path[first]
    after_item.setText(0, "renamed.jpg")
    widget._handle_preview_item_changed(after_item, 0)

    assert widget.has_unsaved_grouping_edits()
    assert widget.pending_grouping_action_lines() == [
        "Move Beach/a.jpg -> Beach/renamed.jpg"
    ]


def test_grouping_apply_button_is_hidden_until_plan_has_real_changes(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    (beach_dir / "a.jpg").write_bytes(b"preview")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="current",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    assert widget.primary_button.isHidden()
    assert not widget.primary_button.isEnabled()

    marks = {first}
    apply_requests: list[bool] = []
    widget.apply_requested.connect(lambda: apply_requests.append(True))
    widget.set_has_any_marked_func(lambda: bool(marks))

    assert not widget.primary_button.isHidden()
    assert widget.primary_button.isEnabled()
    widget.primary_button.click()
    assert apply_requests == [True]

    marks.clear()
    widget.refresh_deletion_state()

    assert widget.primary_button.isHidden()
    assert not widget.primary_button.isEnabled()

    after_item = widget._after_file_items_by_path[first]
    after_item.setText(0, "renamed.jpg")
    widget._handle_preview_item_changed(after_item, 0)

    assert not widget.primary_button.isHidden()
    assert widget.primary_button.isEnabled()

    restored_item = widget._after_file_items_by_path[first]
    restored_item.setText(0, "a.jpg")
    widget._handle_preview_item_changed(restored_item, 0)

    assert widget.primary_button.isHidden()
    assert not widget.primary_button.isEnabled()


def test_unchanged_grouping_edit_check_does_not_build_filesystem_action_preview(
    tmp_path, monkeypatch
):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")
    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )
    monkeypatch.setattr(
        widget,
        "_build_action_lines",
        Mock(side_effect=AssertionError("edit check must stay in memory")),
    )

    assert not widget.has_unsaved_grouping_edits()


def test_grouping_step_widget_set_preview_plan_does_not_read_thumbnail_cache(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    second = str(beach_dir / "b.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")
    beach_dir.joinpath("b.jpg").write_bytes(b"preview")

    pipeline = _CacheOnlyPipeline()
    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(image_pipeline=pipeline)
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="Beach",
                    source_paths=[first, second],
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    assert pipeline.get_thumbnail_qpixmap.call_count == 0
    assert pipeline.get_preview_qpixmap.call_count == 0
    assert pipeline.get_cached_thumbnail_qpixmap.call_count == 0

    widget.refresh_cached_thumbnails([first])

    # One completion updates the matching item in each tree, without touching
    # the thousands of other rows a real folder may contain.
    assert pipeline.get_cached_thumbnail_qpixmap.call_count == 2


def test_organize_tree_reuses_icon_completed_before_tree_construction(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")
    pixmap = QPixmap(120, 80)
    pixmap.fill()
    cached_icon = QIcon(pixmap)
    pipeline = _CacheOnlyPipeline()
    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(
        image_pipeline=pipeline,
        get_cached_thumbnail_icon=Mock(return_value=cached_icon),
    )
    widget.set_source_folder(str(source_root))

    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    assert not widget._before_file_items_by_path[first].icon(0).isNull()
    assert not widget._after_file_items_by_path[first].icon(0).isNull()
    pipeline.get_cached_thumbnail_qpixmap.assert_not_called()


def test_cached_thumbnail_update_does_not_rebuild_organize_trees(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "a.jpg")
    source_root.joinpath("a.jpg").write_bytes(b"preview")
    thumbnail = QPixmap(16, 16)

    pipeline = _CacheOnlyPipeline(cached_thumbnail=thumbnail)
    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(image_pipeline=pipeline)
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="current",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="demo", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )
    widget._refresh_preview_trees = Mock()

    widget.refresh_cached_thumbnails([first])

    widget._refresh_preview_trees.assert_not_called()
    assert widget._file_name_overrides == {}
    assert not widget._after_file_items_by_path[first].icon(0).isNull()


def test_grouping_step_widget_selected_preview_uses_cached_preview_immediately(
    tmp_path,
):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")

    preview = QPixmap(10, 10)
    preview.fill()
    pipeline = _CacheOnlyPipeline(cached_preview=preview)
    request_preview = Mock()
    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(
        image_pipeline=pipeline,
        request_interactive_preview=request_preview,
    )

    widget._update_selected_preview(first)

    assert pipeline.get_immediate_review_qpixmap.call_count == 1
    assert pipeline.get_preview_qpixmap.call_count == 0
    assert pipeline.get_thumbnail_qpixmap.call_count == 0
    request_preview.assert_not_called()
    assert widget.large_preview_name.text() == "a.jpg"
    assert widget.large_preview_view.has_image()


def test_grouping_step_widget_selected_preview_queues_upgrade_from_cached_thumbnail(
    tmp_path,
):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")

    thumbnail = QPixmap(12, 12)
    thumbnail.fill()
    pipeline = _CacheOnlyPipeline(cached_thumbnail=thumbnail)
    request_preview = Mock()

    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(
        image_pipeline=pipeline,
        request_interactive_preview=request_preview,
    )

    widget._update_selected_preview(first)

    assert pipeline.get_immediate_review_qpixmap.call_count == 1
    assert pipeline.get_preview_qpixmap.call_count == 0
    assert pipeline.get_thumbnail_qpixmap.call_count == 0
    request_preview.assert_called_once_with(first)
    assert widget.large_preview_view.has_image()


def test_grouping_step_widget_applies_only_current_background_preview(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "a.jpg")
    second = str(source_root / "b.jpg")
    source_root.joinpath("a.jpg").write_bytes(b"preview")
    source_root.joinpath("b.jpg").write_bytes(b"preview")

    preview = QPixmap(20, 20)
    preview.fill()
    pipeline = _CacheOnlyPipeline()
    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(
        image_pipeline=pipeline,
        request_interactive_preview=Mock(),
    )
    widget._update_selected_preview(second)
    pipeline.get_cached_preview_qpixmap.return_value = preview

    widget.handle_preview_ready(first)
    assert not widget.large_preview_view.has_image()

    widget.handle_preview_ready(second)
    assert widget.large_preview_view.has_image()


def test_grouping_step_widget_folder_preview_uses_cached_thumbnails_only(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    second = str(beach_dir / "b.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")
    beach_dir.joinpath("b.jpg").write_bytes(b"preview")

    pipeline = _CacheOnlyPipeline()
    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(image_pipeline=pipeline)
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="Beach",
                    source_paths=[first, second],
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    pipeline.get_cached_thumbnail_qpixmap.reset_mock()
    widget._update_folder_preview(widget._after_group_items_by_id["1"])

    assert widget.folder_preview_grid.count() == 2
    assert pipeline.get_thumbnail_qpixmap.call_count == 0
    assert pipeline.get_preview_qpixmap.call_count == 0
    assert pipeline.get_cached_thumbnail_qpixmap.call_count == 2


def test_grouping_step_widget_hides_leaf_only_context_actions(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    menu = QMenu()
    leaf_item = widget._after_file_items_by_path[first]
    widget._populate_common_context_actions(
        menu, leaf_item, widget.preview_tree, is_after=True
    )

    action_texts = [action.text() for action in menu.actions() if action.text()]

    assert "Expand subtree" not in action_texts
    assert "Collapse subtree" not in action_texts
    assert "Expand all children" not in action_texts
    assert "Collapse all children" not in action_texts
    assert "Preview this file" not in action_texts
    assert "Mark for Trash" in action_texts
    assert "Move file to Trash now…" in action_texts
    assert "Move files now" not in action_texts
    assert "Send to Unassigned" not in action_texts


def test_grouping_step_widget_disables_no_op_subtree_actions(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")
    second = str(source_root / "Beach" / "b.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="Trips/Beach",
                    source_paths=[first, second],
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    menu = QMenu()
    folder_item = widget._after_dir_items_by_relative_path["Trips"]
    widget._populate_common_context_actions(
        menu, folder_item, widget.preview_tree, is_after=True
    )

    actions_by_text = {
        action.text(): action for action in menu.actions() if action.text()
    }

    assert "Expand subtree" in actions_by_text
    assert not actions_by_text["Expand subtree"].isEnabled()
    assert actions_by_text["Collapse subtree"].isEnabled()
    assert "Expand all children" not in actions_by_text
    assert "Collapse all children" not in actions_by_text


def test_grouping_step_widget_renders_all_group_files_in_after_tree(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    paths = [str(source_root / "Beach" / f"img_{idx}.jpg") for idx in range(13)]

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=13,
            supported_items=13,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=paths)
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    group_item = widget._after_group_items_by_id["1"]
    assert group_item.childCount() == 13
    assert all(not group_item.child(i).text(0).startswith("…") for i in range(13))


def test_grouping_step_widget_restore_returns_files_to_original_folder(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    widget._move_paths_to_unassigned([first])
    moved_plan = widget.get_effective_plan()
    assert moved_plan.groups == []
    assert moved_plan.unassigned_paths == [first]

    widget._restore_paths_to_original_location([first])
    restored_plan = widget.get_effective_plan()
    assert len(restored_plan.groups) == 1
    assert restored_plan.groups[0].group_label == "Beach"
    assert restored_plan.groups[0].source_paths == [first]


def test_grouping_step_widget_restore_keeps_root_level_files_at_root(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="current",
            total_items=1,
            supported_items=1,
            groups=[GroupingGroup(group_id="1", group_label="", source_paths=[first])],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    widget._move_paths_to_unassigned([first])
    widget._restore_paths_to_original_location([first])
    restored_plan = widget.get_effective_plan()

    assert len(restored_plan.groups) == 1
    assert restored_plan.groups[0].group_label == ""
    assert restored_plan.groups[0].source_paths == [first]


def test_grouping_step_widget_labels_root_level_group_as_root_files(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "a.jpg")
    source_root.joinpath("a.jpg").write_bytes(b"preview")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="current",
            total_items=1,
            supported_items=1,
            groups=[GroupingGroup(group_id="1", group_label="", source_paths=[first])],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    root_group_item = widget._after_group_items_by_id["1"]

    assert root_group_item.text(0) == "Root files"


def test_grouping_step_widget_can_create_parent_directory(tmp_path, monkeypatch):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    monkeypatch.setattr(
        "src.ui.grouping_step_widget.QInputDialog.getText",
        lambda *args, **kwargs: ("Trips", True),
    )

    widget._create_parent_directory_for_item(widget._after_group_items_by_id["1"])

    assert widget.get_effective_plan().groups[0].group_label == "Trips/Beach"


def test_grouping_step_widget_can_mark_directory_for_deletion(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    (beach_dir / "a.jpg").write_bytes(b"preview")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    marks: set[str] = set()
    widget.set_is_marked_func(marks.__contains__)
    widget.set_has_any_marked_func(lambda: bool(marks))
    widget._folder_validation_pool = SimpleNamespace(start=lambda task: task.run())
    widget.toggle_deletion_marks_requested.connect(marks.update)
    widget._delete_item(widget._after_group_items_by_id["1"])
    widget.refresh_deletion_state()

    assert widget.get_effective_plan().groups[0].source_paths == [first]
    assert widget.get_effective_plan().deleted_paths == []
    assert marks == {str(beach_dir), first}
    assert not widget.primary_button.isHidden()
    assert widget.primary_button.isEnabled()
    assert widget._after_file_items_by_path[first].text(0).endswith("(DELETED)")
    assert widget.pending_grouping_action_lines() == []


def test_grouping_step_widget_rejects_folder_mark_when_unshown_file_exists(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    (beach_dir / "a.jpg").write_bytes(b"preview")
    (beach_dir / ".hidden.xmp").write_text("sidecar", encoding="utf-8")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
            filesystem_inventory_complete=True,
        ),
        str(source_root),
    )

    requested: list[list[str]] = []
    widget._folder_validation_pool = SimpleNamespace(start=lambda task: task.run())
    widget.toggle_deletion_marks_requested.connect(requested.append)
    widget._delete_item(widget._after_group_items_by_id["1"])

    assert requested == []


def test_grouping_step_widget_can_mark_or_trash_file_from_context(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    menu = QMenu()
    file_item = widget._after_file_items_by_path[first]
    widget._populate_common_context_actions(
        menu, file_item, widget.preview_tree, is_after=True
    )
    actions_by_text = {
        action.text(): action for action in menu.actions() if action.text()
    }

    marks: set[str] = set()
    trash_requests: list[tuple[str, list[str]]] = []
    widget.set_is_marked_func(marks.__contains__)
    widget.toggle_deletion_marks_requested.connect(marks.update)
    widget.trash_requested.connect(
        lambda target, paths: trash_requests.append((target, paths))
    )

    actions_by_text["Mark for Trash"].trigger()

    assert marks == {first}
    assert widget.get_effective_plan().groups[0].source_paths == [first]
    assert widget.get_effective_plan().deleted_paths == []
    assert trash_requests == []

    actions_by_text["Move file to Trash now…"].trigger()

    assert trash_requests == [(first, [first])]


def test_grouping_step_widget_can_match_before_and_after_items(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    before_item = widget._before_file_items_by_path[first]
    after_item = widget._after_file_items_by_path[first]

    assert widget._find_matching_item(before_item, is_after=False) is after_item
    assert widget._find_matching_item(after_item, is_after=True) is before_item


def test_grouping_step_widget_ignores_deleted_items_during_selection_sync(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    stale_item = widget._after_file_items_by_path[first]
    widget.preview_tree.clear()

    widget._sync_selection_to_other_tree(stale_item, from_after=True)


def test_grouping_step_widget_syncs_selection_between_trees(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    after_item = widget._after_file_items_by_path[first]
    before_item = widget._before_file_items_by_path[first]

    widget._handle_after_item_changed(after_item, None)
    assert widget.before_tree.currentItem() is before_item

    widget._handle_before_item_changed(before_item, None)
    assert widget.preview_tree.currentItem() is after_item
    assert after_item.isSelected()


def test_grouping_active_focus_highlights_item_and_preserves_multiselection(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")
    second = str(source_root / "Beach" / "b.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="Beach",
                    source_paths=[first, second],
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )
    first_item = widget._after_file_items_by_path[first]
    second_item = widget._after_file_items_by_path[second]
    widget.preview_tree.clearSelection()
    first_item.setSelected(True)
    widget.large_preview_view.fit_in_view = Mock()

    assert widget.focus_image(second)

    assert widget.preview_tree.currentItem() is second_item
    assert first_item.isSelected()
    assert second_item.isSelected()
    widget.large_preview_view.fit_in_view.assert_called_once_with()


def test_grouping_step_widget_syncs_folder_selection_between_trees(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    before_dir = widget._before_dir_items_by_relative_path["Beach"]
    after_group = widget._after_group_items_by_label["Beach"]

    widget._handle_before_item_changed(before_dir, None)
    assert widget.preview_tree.currentItem() is after_group
    assert after_group.isSelected()

    widget._handle_after_item_changed(after_group, None)
    assert widget.before_tree.currentItem() is before_dir


def test_grouping_step_widget_builds_confirmation_action_list(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(source_root / "Beach" / "a.jpg")
    (beach_dir / "a.jpg").write_bytes(b"preview")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(
                    group_id="1", group_label="Trips/Beach", source_paths=[first]
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    action_lines = widget._build_action_lines(widget.get_effective_plan())

    assert action_lines == [
        "Rename folder Beach -> Trips/Beach",
    ]


def test_grouping_step_widget_shows_folder_preview_grid(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")
    second = str(source_root / "Beach" / "b.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="Beach",
                    source_paths=[first, second],
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    before_dir = widget._before_dir_items_by_relative_path["Beach"]
    widget._handle_before_item_changed(before_dir, None)

    assert widget.preview_pane_stack.currentWidget() is widget.folder_preview_page
    assert widget.folder_preview_grid.count() == 2
    assert widget.folder_preview_title.text() == "Beach"
    assert widget.visible_thumbnail_paths()[:2] == [first, second]


def test_grouping_folder_preview_requests_thumbnail_loading(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")
    schedule = Mock()

    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(
        image_pipeline=_CacheOnlyPipeline(),
        schedule_visible_thumbnail_load=schedule,
    )
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="Beach",
                    source_paths=[first],
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    schedule.reset_mock()
    widget._handle_before_item_changed(
        widget._before_dir_items_by_relative_path["Beach"],
        None,
    )

    schedule.assert_called_once()


def test_grouping_step_widget_before_tree_includes_unmanaged_files(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    extra = str(beach_dir / "a.json")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")
    beach_dir.joinpath("a.json").write_text("{}", encoding="utf-8")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    assert first in widget._before_file_items_by_path
    assert extra in widget._before_file_items_by_path
    assert widget._before_file_items_by_path[extra].text(0) == "a.json"


def test_grouping_step_widget_folder_preview_includes_unmanaged_files(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")
    beach_dir.joinpath("a.json").write_text("{}", encoding="utf-8")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    before_dir = widget._before_dir_items_by_relative_path["Beach"]
    widget._handle_before_item_changed(before_dir, None)

    names = {
        widget.folder_preview_grid.item(index).text()
        for index in range(widget.folder_preview_grid.count())
    }

    assert names == {"a.jpg", "a.json"}


def test_grouping_step_widget_after_tree_includes_unmanaged_files(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    sidecar = str(beach_dir / "notes.sdaldjsa")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")
    beach_dir.joinpath("notes.sdaldjsa").write_text("meta", encoding="utf-8")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Trips", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    assert sidecar in widget._after_file_items_by_path
    assert widget._after_file_items_by_path[sidecar].text(0) == "notes.sdaldjsa"
    sidecar_group = widget._find_group(
        widget._item_group_id(widget._after_file_items_by_path[sidecar])
    )
    assert sidecar_group is not None
    assert sidecar_group.group_label == "Beach"


def test_grouping_step_widget_group_rename_moves_unmanaged_files_too(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    extra = str(beach_dir / "a.json")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")
    beach_dir.joinpath("a.json").write_text("{}", encoding="utf-8")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="current",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    group_item = widget._after_group_items_by_id["1"]
    group_item.setText(0, "Holiday")
    widget._handle_preview_item_changed(group_item, 0)

    effective_plan = widget.get_effective_plan()
    holiday_group = next(
        group for group in effective_plan.groups if group.group_label == "Holiday"
    )

    assert first in holiday_group.source_paths
    assert extra in holiday_group.source_paths


def test_grouping_step_widget_supports_file_rename_in_preview_plan(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    after_item = widget._after_file_items_by_path[first]
    after_item.setText(0, "renamed.jpg")
    widget._handle_preview_item_changed(after_item, 0)

    effective_plan = widget.get_effective_plan()

    assert effective_plan.file_name_overrides[first] == "renamed.jpg"
    assert widget._after_file_items_by_path[first].text(0) == "renamed.jpg"
    assert widget._projected_path_for_source(first).endswith("Beach/renamed.jpg")


def test_grouping_step_widget_builds_collision_aware_action_list(tmp_path):
    source_root = tmp_path / "demo"
    first_dir = source_root / "A"
    second_dir = source_root / "B"
    first_dir.mkdir(parents=True)
    second_dir.mkdir(parents=True)
    (source_root / "Untouched empty folder").mkdir()
    first = str(first_dir / "same.jpg")
    second = str(second_dir / "same.jpg")
    (first_dir / "same.jpg").write_bytes(b"a")
    (second_dir / "same.jpg").write_bytes(b"b")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="Merged",
                    source_paths=[first, second],
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    action_lines = widget._build_action_lines(widget.get_effective_plan())

    assert action_lines[:2] == [
        "Move A/same.jpg -> Merged/same.jpg",
        "Move B/same.jpg -> Merged/same_1.jpg",
    ]
    assert set(action_lines[2:]) == {
        "Remove empty folder A",
        "Remove empty folder B",
    }


def test_grouping_apply_ignores_preexisting_empty_folders_without_changes(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    untouched_empty_folder = source_root / "Folder"
    untouched_empty_folder.mkdir()
    photo = source_root / "photo.jpg"
    photo.write_bytes(b"photo")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="current",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="",
                    source_paths=[str(photo)],
                )
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    action_lines = widget._build_action_lines(widget.get_effective_plan())

    assert action_lines == []
    assert untouched_empty_folder.is_dir()


def test_grouping_step_widget_selects_original_items_for_renamed_entries(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    renamed_group = widget._after_group_items_by_id["1"]
    renamed_group.setText(0, "Holiday")
    widget._handle_preview_item_changed(renamed_group, 0)

    renamed_file = widget._after_file_items_by_path[first]
    renamed_file.setText(0, "sunset.jpg")
    widget._handle_preview_item_changed(renamed_file, 0)

    widget._handle_after_item_changed(widget._after_group_items_by_id["1"], None)
    assert (
        widget.before_tree.currentItem()
        is widget._before_dir_items_by_relative_path["Beach"]
    )

    widget._handle_after_item_changed(widget._after_file_items_by_path[first], None)
    assert widget.before_tree.currentItem() is widget._before_file_items_by_path[first]


def test_grouping_step_widget_keeps_file_preview_visible_after_rename(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    after_item = widget._after_file_items_by_path[first]
    widget.preview_tree.setCurrentItem(after_item)
    after_item.setSelected(True)
    widget._handle_after_item_changed(after_item, None)

    after_item.setText(0, "renamed.jpg")
    widget._handle_preview_item_changed(after_item, 0)

    assert widget.preview_tree.currentItem() is widget._after_file_items_by_path[first]
    assert widget.preview_pane_stack.currentWidget() is not widget.preview_hint_label
    assert widget.large_preview_name.text() == "a.jpg"


def test_grouping_step_widget_keeps_folder_preview_visible_after_rename(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    group_item = widget._after_group_items_by_id["1"]
    widget.preview_tree.setCurrentItem(group_item)
    group_item.setSelected(True)
    widget._handle_after_item_changed(group_item, None)

    group_item.setText(0, "Holiday")
    widget._handle_preview_item_changed(group_item, 0)

    assert widget.preview_tree.currentItem() is widget._after_group_items_by_id["1"]
    assert widget.preview_pane_stack.currentWidget() is widget.folder_preview_page
    assert widget.folder_preview_grid.count() == 1


def test_grouping_step_widget_restores_selected_file_when_current_item_is_lost(
    tmp_path,
):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    after_item = widget._after_file_items_by_path[first]
    after_item.setSelected(True)

    widget._restore_selection_state(
        {
            "selected_paths": [first],
            "selected_group_ids": [],
            "selected_match_relative_paths": [],
            "current_path": None,
            "current_group_id": None,
            "current_match_relative_path": None,
        }
    )

    assert widget.preview_tree.currentItem() is widget._after_file_items_by_path[first]
    assert widget.preview_pane_stack.currentWidget() is not widget.preview_hint_label


def test_grouping_step_widget_restores_selected_group_when_current_item_is_lost(
    tmp_path,
):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first])
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    widget._restore_selection_state(
        {
            "selected_paths": [],
            "selected_group_ids": ["1"],
            "selected_match_relative_paths": ["Beach"],
            "current_path": None,
            "current_group_id": None,
            "current_match_relative_path": None,
        }
    )

    assert widget.preview_tree.currentItem() is widget._after_group_items_by_id["1"]
    assert widget.preview_pane_stack.currentWidget() is widget.folder_preview_page


def test_nest_group_under_target(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")
    second = str(source_root / "Trips" / "b.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first]),
                GroupingGroup(group_id="2", group_label="Trips", source_paths=[second]),
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    widget._nest_group_under_target("1", "2")
    plan = widget.get_effective_plan()

    labels = {g.group_label for g in plan.groups}
    assert "Trips/Beach" in labels
    assert "Trips" in labels


def test_nest_group_prevents_self_nesting(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first]),
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    widget._nest_group_under_target("1", "1")
    plan = widget.get_effective_plan()

    assert plan.groups[0].group_label == "Beach"


def test_nest_group_prevents_circular_nesting(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "a.jpg")
    second = str(source_root / "b.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(
                    group_id="1", group_label="Trips/Beach", source_paths=[first]
                ),
                GroupingGroup(group_id="2", group_label="Trips", source_paths=[second]),
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    widget._nest_group_under_target("2", "1")
    plan = widget.get_effective_plan()

    labels = {g.group_label for g in plan.groups}
    assert "Trips" in labels
    assert "Trips/Beach" in labels


def test_nest_group_under_directory_prevents_circular_nesting(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "a.jpg")
    second = str(source_root / "b.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(group_id="1", group_label="Trips", source_paths=[first]),
                GroupingGroup(
                    group_id="2", group_label="Trips/Beach", source_paths=[second]
                ),
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    widget._nest_group_under_directory("1", "Trips/Beach")
    plan = widget.get_effective_plan()

    labels = {g.group_label for g in plan.groups}
    assert "Trips" in labels
    assert "Trips/Beach" in labels


def test_unnest_group_to_root(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(
                    group_id="1", group_label="Trips/Beach", source_paths=[first]
                ),
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    widget._unnest_group_to_root("1")
    plan = widget.get_effective_plan()

    assert plan.groups[0].group_label == "Beach"


def test_move_files_to_directory(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first]),
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    widget._move_files_to_directory([first], "Vacation")
    plan = widget.get_effective_plan()

    labels = {g.group_label for g in plan.groups}
    assert "Vacation" in labels
    vacation_group = next(g for g in plan.groups if g.group_label == "Vacation")
    assert first in vacation_group.source_paths


def test_is_descendant_check():
    from PyQt6.QtWidgets import QTreeWidgetItem

    grandparent = QTreeWidgetItem(["grandparent"])
    parent = QTreeWidgetItem(["parent"])
    child = QTreeWidgetItem(["child"])
    unrelated = QTreeWidgetItem(["unrelated"])

    grandparent.addChild(parent)
    parent.addChild(child)

    assert DroppableGroupingTree._is_descendant(child, grandparent)
    assert DroppableGroupingTree._is_descendant(child, parent)
    assert not DroppableGroupingTree._is_descendant(child, unrelated)
    assert not DroppableGroupingTree._is_descendant(grandparent, child)


def test_droppable_tree_has_drag_enabled():
    widget = GroupingStepWidget()
    tree = widget.preview_tree

    assert isinstance(tree, DroppableGroupingTree)
    assert tree.dragEnabled()


def test_group_items_have_drag_flag(tmp_path):
    from PyQt6.QtCore import Qt

    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=1,
            supported_items=1,
            groups=[
                GroupingGroup(group_id="1", group_label="Beach", source_paths=[first]),
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    group_item = widget._after_group_items_by_id["1"]
    file_item = widget._after_file_items_by_path[first]

    assert group_item.flags() & Qt.ItemFlag.ItemIsDragEnabled
    assert file_item.flags() & Qt.ItemFlag.ItemIsDragEnabled
    assert group_item.data(0, ROLE_KIND) == ITEM_GROUP


def test_grouping_step_widget_keyboard_navigation_skips_deleted(tmp_path):
    from PyQt6.QtCore import QEvent, Qt
    from PyQt6.QtGui import QKeyEvent
    import sys

    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Beach" / "a.jpg")
    second = str(source_root / "Beach" / "b.jpg")
    third = str(source_root / "Beach" / "c.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=3,
            supported_items=3,
            groups=[
                GroupingGroup(
                    group_id="1",
                    group_label="Beach",
                    source_paths=[first, second, third],
                ),
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    # Mark the second file for deletion
    marks = {second}
    widget.set_is_marked_func(marks.__contains__)

    first_item = widget._after_file_items_by_path[first]
    second_item = widget._after_file_items_by_path[second]
    third_item = widget._after_file_items_by_path[third]

    # Focus on first item
    widget.preview_tree.setCurrentItem(first_item)

    # 1. Press Down without override modifier -> should skip second and select third
    event_down = QKeyEvent(
        QEvent.Type.KeyPress, Qt.Key.Key_Down, Qt.KeyboardModifier.NoModifier
    )
    handled = widget.eventFilter(widget.preview_tree, event_down)
    assert handled is True
    assert widget.preview_tree.currentItem() is third_item

    # 2. Press Up without override modifier -> should skip second and select first
    event_up = QKeyEvent(
        QEvent.Type.KeyPress, Qt.Key.Key_Up, Qt.KeyboardModifier.NoModifier
    )
    handled = widget.eventFilter(widget.preview_tree, event_up)
    assert handled is True
    assert widget.preview_tree.currentItem() is first_item

    # 3. Press Down with override modifier (Ctrl on Win/Linux, Cmd on macOS)
    modifier = (
        Qt.KeyboardModifier.MetaModifier
        if sys.platform == "darwin"
        else Qt.KeyboardModifier.ControlModifier
    )
    event_down_override = QKeyEvent(QEvent.Type.KeyPress, Qt.Key.Key_Down, modifier)
    handled_override = widget.eventFilter(widget.preview_tree, event_down_override)
    assert handled_override is True
    assert widget.preview_tree.currentItem() is second_item
