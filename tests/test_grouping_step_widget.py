from PyQt6.QtWidgets import QApplication, QMenu

from src.core.grouping import GroupingGroup, GroupingPlan
from src.ui.grouping_step_widget import GroupingStepWidget


_app = QApplication.instance() or QApplication([])


def test_grouping_step_widget_tracks_mode_and_busy_state():
    widget = GroupingStepWidget()
    assert widget.primary_button.text() == "Move files"


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


def test_grouping_step_widget_hides_leaf_only_context_actions(tmp_path):
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
    assert "Preview this file" in action_texts

    widget.set_source_folder("/tmp/demo")
    widget.set_current_mode("location")
    assert widget.current_mode() == "location"

    widget.set_preview_text("2 groups")
    assert widget.preview_label.text() == "2 groups"

    plan = GroupingPlan(
        mode="location",
        total_items=3,
        supported_items=3,
        groups=[GroupingGroup(group_id="1", group_label="Beach", source_paths=["a.jpg"])],
        unassigned_paths=["b.jpg"],
        skipped_paths=[],
    )
    widget.set_preview_plan(plan, "/tmp/demo/PhotoSort Groups/location")
    assert widget.preview_tree.topLevelItemCount() == 1
    assert widget.get_group_name_overrides()["1"] == "Beach"

    widget.set_busy(True)
    assert widget.primary_button.text() == "Grouping…"
    assert not widget.primary_button.isEnabled()

    widget.set_busy(False)
    assert widget.primary_button.isEnabled()
    assert widget.primary_button.text() == "Move files"


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
                GroupingGroup(group_id="1", group_label="Trips/Beach", source_paths=[first])
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
    assert widget.before_tree.currentItem() is widget._before_dir_items_by_relative_path["Beach"]

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


def test_grouping_step_widget_restores_selected_file_when_current_item_is_lost(tmp_path):
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


def test_grouping_step_widget_restores_selected_group_when_current_item_is_lost(tmp_path):
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


def test_grouping_step_widget_drag_drop_moves_file_into_folder(tmp_path):
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

    dragged_items = [widget._after_file_items_by_path[first]]
    target_item = widget._after_group_items_by_id["2"]

    assert widget._can_drop_preview_items(dragged_items, target_item)

    widget._handle_preview_tree_drop(dragged_items, target_item)

    plan = widget.get_effective_plan()
    trips_group = next(group for group in plan.groups if group.group_id == "2")
    assert all(group.group_id != "1" for group in plan.groups)
    assert trips_group.source_paths == [first, second]
    assert widget.preview_tree.currentItem() is widget._after_group_items_by_id["2"]
    assert widget.preview_pane_stack.currentWidget() is widget.folder_preview_page
    assert widget.folder_preview_grid.count() == 2


def test_grouping_step_widget_drag_drop_moves_folder_into_folder(tmp_path):
    source_root = tmp_path / "demo"
    source_root.mkdir()
    first = str(source_root / "Trips" / "Beach" / "a.jpg")
    second = str(source_root / "Trips" / "City" / "b.jpg")

    widget = GroupingStepWidget()
    widget.set_source_folder(str(source_root))
    widget.set_preview_plan(
        GroupingPlan(
            mode="location",
            total_items=2,
            supported_items=2,
            groups=[
                GroupingGroup(group_id="1", group_label="Trips/Beach", source_paths=[first]),
                GroupingGroup(group_id="2", group_label="Trips/City", source_paths=[second]),
            ],
            unassigned_paths=[],
            skipped_paths=[],
        ),
        str(source_root),
    )

    dragged_items = [widget._after_group_items_by_id["1"]]
    target_item = widget._after_group_items_by_id["2"]

    assert widget._can_drop_preview_items(dragged_items, target_item)

    widget._handle_preview_tree_drop(dragged_items, target_item)

    plan = widget.get_effective_plan()
    moved_group = next(group for group in plan.groups if group.group_id == "1")
    assert moved_group.group_label == "Trips/City/Beach"
    assert widget.preview_tree.currentItem() is widget._after_group_items_by_id["2"]
    assert widget.preview_pane_stack.currentWidget() is widget.folder_preview_page
    assert widget.folder_preview_grid.count() == 2
