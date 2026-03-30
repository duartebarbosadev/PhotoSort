from PyQt6.QtWidgets import QApplication

from src.core.grouping import GroupingGroup, GroupingPlan
from src.ui.grouping_step_widget import GroupingStepWidget


_app = QApplication.instance() or QApplication([])


def test_grouping_step_widget_tracks_mode_and_busy_state():
    widget = GroupingStepWidget()

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
