import os
from types import SimpleNamespace
from unittest.mock import Mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from PyQt6.QtGui import QPixmap
from PyQt6.QtWidgets import QApplication, QMenu, QMessageBox

from src.core.grouping import GroupingGroup, GroupingPlan
from src.ui.grouping_step_widget import (
    DroppableGroupingTree,
    GroupingStepWidget,
    ITEM_GROUP,
    ROLE_KIND,
)


_app = QApplication.instance() or QApplication([])


class _CacheOnlyPipeline:
    def __init__(self, *, cached_preview: QPixmap | None = None):
        self.get_cached_thumbnail_qpixmap = Mock(return_value=None)
        self.get_cached_preview_qpixmap = Mock(return_value=cached_preview)
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
    assert widget.primary_button.text() == "Apply Changes"
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
    assert widget.primary_button.text() == "Apply Changes"
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


def test_grouping_step_widget_set_preview_plan_uses_cached_thumbnails_only(tmp_path):
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
    assert pipeline.get_cached_thumbnail_qpixmap.call_count >= 4


def test_grouping_step_widget_selected_preview_loads_immediately_on_cache_miss(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")

    preview = QPixmap(10, 10)
    preview.fill()
    pipeline = _CacheOnlyPipeline()
    pipeline.get_preview_qpixmap.return_value = preview
    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(image_pipeline=pipeline)

    widget._update_selected_preview(first)

    assert pipeline.get_preview_qpixmap.call_count == 1
    assert pipeline.get_thumbnail_qpixmap.call_count == 0
    assert widget.large_preview_name.text() == "a.jpg"
    assert widget.large_preview_view.current_pixmap() is not None


def test_grouping_step_widget_selected_preview_falls_back_to_thumbnail(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")

    thumbnail = QPixmap(12, 12)
    thumbnail.fill()
    pipeline = _CacheOnlyPipeline()
    pipeline.get_preview_qpixmap.return_value = None
    pipeline.get_thumbnail_qpixmap.return_value = thumbnail

    widget = GroupingStepWidget()
    widget._parent_window = SimpleNamespace(image_pipeline=pipeline)

    widget._update_selected_preview(first)

    assert pipeline.get_preview_qpixmap.call_count == 1
    assert pipeline.get_thumbnail_qpixmap.call_count == 1
    assert widget.large_preview_view.current_pixmap() is not None


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


def test_grouping_step_widget_filesystem_walk_filters_non_media(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    media_path = beach_dir / "a.jpg"
    text_path = beach_dir / "notes.txt"
    media_path.write_bytes(b"preview")
    text_path.write_text("ignore me")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))

    discovered = widget._filesystem_file_paths_under_root(str(source_root))

    assert discovered == [str(media_path)]


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
    assert "Delete file" in action_texts
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


def test_grouping_step_widget_can_mark_directory_for_deletion(tmp_path, monkeypatch):
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

    monkeypatch.setattr(
        "src.ui.grouping_step_widget.QMessageBox.question",
        lambda *args, **kwargs: QMessageBox.StandardButton.Yes,
    )
    widget._delete_item(widget._after_group_items_by_id["1"])

    assert widget.get_effective_plan().groups == []
    assert widget.get_effective_plan().deleted_paths == [str(beach_dir)]
    assert widget.pending_grouping_action_lines() == ["Delete folder Beach"]


def test_grouping_step_widget_can_delete_file_from_context(tmp_path, monkeypatch):
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

    deleted_paths = []
    monkeypatch.setattr(
        "src.ui.grouping_step_widget.QMessageBox.question",
        lambda *args, **kwargs: QMessageBox.StandardButton.Yes,
    )
    monkeypatch.setattr(
        "src.ui.grouping_step_widget.ImageFileOperations.move_to_trash",
        lambda path: (deleted_paths.append(path) or True, "Moved to trash."),
    )

    actions_by_text["Delete file"].trigger()

    assert deleted_paths == []
    assert widget.get_effective_plan().groups == []
    assert widget.get_effective_plan().deleted_paths == [first]
    assert widget.pending_grouping_action_lines() == [
        "Delete file Beach/a.jpg",
        "Remove empty folder Beach",
    ]


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


def test_grouping_step_widget_before_tree_includes_unmanaged_files(tmp_path):
    source_root = tmp_path / "demo"
    beach_dir = source_root / "Beach"
    beach_dir.mkdir(parents=True)
    first = str(beach_dir / "a.jpg")
    sidecar = str(beach_dir / "a.xmp")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")
    beach_dir.joinpath("a.xmp").write_text("sidecar", encoding="utf-8")

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
    assert sidecar in widget._before_file_items_by_path
    assert widget._before_file_items_by_path[sidecar].text(0) == "a.xmp"


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
    sidecar = str(beach_dir / "a.xmp")
    beach_dir.joinpath("a.jpg").write_bytes(b"preview")
    beach_dir.joinpath("a.xmp").write_text("sidecar", encoding="utf-8")

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
    assert sidecar in holiday_group.source_paths


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
