from PyQt6.QtWidgets import QApplication

from ui.easy_delete_step_widget import EasyDeleteStepWidget
from ui.fix_rotation_step_widget import FixRotationStepWidget
from ui.pick_best_step_widget import PickBestStepWidget
from ui.ui_components import LoadingOverlay
from ui.workflow_review_components import WorkflowProgressView


_app = QApplication.instance() or QApplication([])


def test_workflow_pages_share_the_same_progress_view():
    widgets = (
        EasyDeleteStepWidget(),
        FixRotationStepWidget(),
        PickBestStepWidget(),
    )

    assert all(
        isinstance(widget._progress_view, WorkflowProgressView) for widget in widgets
    )
    assert len({type(widget._progress_view) for widget in widgets}) == 1
    assert all(
        widget._loading_label is widget._progress_view.message_label
        and widget._progress_bar is widget._progress_view.progress_bar
        for widget in widgets
    )


def test_global_loading_overlay_reuses_shared_progress_presentation():
    overlay = LoadingOverlay()

    overlay.setText("Similarity: Generating embeddings… (42%)")

    assert isinstance(overlay.progress_view, WorkflowProgressView)
    assert overlay.text_label is overlay.progress_view.message_label
    assert overlay.progress_view.percent_label.text() == "42%"


def test_progress_view_estimates_remaining_time_and_extracts_counts():
    now = [100.0]
    view = WorkflowProgressView(
        "Easy Delete",
        default_message="Preparing…",
        clock=lambda: now[0],
    )

    view.update_progress("Checking first.jpg… (10/100)", 10)
    assert view.remaining_label.text() == "Estimating…"
    assert view.elapsed_label.text() == "0s"
    assert view.count_label.text() == "10 of 100"

    now[0] += 10
    view.update_progress("Checking next.jpg… (50/100)", 50)

    assert view.remaining_label.text() == "About 12s"
    assert view.elapsed_label.text() == "10s"
    assert view.percent_label.text() == "50%"

    view.update_progress("Scoring cluster 6/12: IMG_1234.jpg", 50)
    assert view.count_label.text() == "6 of 12"


def test_progress_view_resets_eta_when_a_new_phase_restarts_percentage():
    now = [0.0]
    view = WorkflowProgressView(
        "Pick Best Photos",
        default_message="Preparing…",
        clock=lambda: now[0],
    )
    view.update_progress("Step 1/2: Embeddings (80/100)", 80)
    now[0] = 8.0
    view.update_progress("Step 2/2: Scoring clusters", 0)
    now[0] = 10.0
    view.update_progress("Scoring cluster 1/10", 20)

    assert view.elapsed_label.text() == "10s"
    assert view.remaining_label.text() == "About 8s"


def test_progress_view_has_consistent_indeterminate_and_error_states():
    view = WorkflowProgressView("Fix Rotation", default_message="Preparing…")

    view.update_progress("Loading model…", -1)
    assert view.progress_bar.minimum() == 0
    assert view.progress_bar.maximum() == 0
    assert view.percent_label.text() == "•••"

    view.show_error("The model could not be loaded.")
    assert view.property("state") == "error"
    assert view.message_label.text() == "The model could not be loaded."
    assert view.remaining_label.text() == "Needs attention"
    assert view.count_label.text() == "Not completed"
