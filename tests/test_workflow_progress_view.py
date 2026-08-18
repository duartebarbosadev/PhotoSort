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


def test_progress_view_exposes_cancellation_only_when_enabled():
    view = WorkflowProgressView("Cull", default_message="Preparing…")
    requests = []
    view.cancel_requested.connect(lambda: requests.append(True))

    assert view.cancel_button.isHidden()
    view.set_cancel_visible(True)
    view.cancel_button.click()

    assert not view.cancel_button.isHidden()
    assert requests == [True]

    view.mark_cancelled("Cull cancelled")
    assert view.message_label.text() == "Cull cancelled"
    assert not view.cancel_button.isEnabled()


def test_terminal_states_offer_an_escape_from_the_progress_page():
    """A failed run must never leave a workflow page without a way forward."""

    view = WorkflowProgressView(
        "Cull",
        default_message="Preparing…",
        dismiss_label="Continue without grouping",
    )
    dismissals = []
    view.dismiss_requested.connect(lambda: dismissals.append(True))

    view.update_progress("Working…", 10)
    assert view.dismiss_button.isHidden()

    view.show_error("The model could not be loaded.")
    assert not view.dismiss_button.isHidden()
    view.dismiss_button.click()
    assert dismissals == [True]

    view.update_progress("Retrying…", 5)
    assert view.dismiss_button.isHidden()


def test_retry_is_only_offered_to_workflows_that_can_restart_themselves():
    plain = WorkflowProgressView("Similarity", default_message="Working…")
    restartable = WorkflowProgressView(
        "Cull",
        default_message="Working…",
        retry_label="Enable same-subject grouping",
    )

    plain.show_error("Failed")
    restartable.show_error("Failed")

    assert plain.retry_button.isVisibleTo(plain) is False
    assert restartable.retry_button.isVisibleTo(restartable) is True
    assert restartable.retry_button.text() == "Enable same-subject grouping"


def test_retry_is_hidden_once_the_workflow_runs_again():
    view = WorkflowProgressView(
        "Cull", default_message="Working…", retry_label="Retry now"
    )

    view.mark_cancelled("Cancelled")
    assert view.retry_button.isVisibleTo(view) is True

    view.update_progress("Working…", 10)
    assert view.retry_button.isVisibleTo(view) is False

    view.show_error("Failed again")
    assert view.retry_button.isVisibleTo(view) is True

    view.mark_finished()
    assert view.retry_button.isVisibleTo(view) is False


def test_retry_button_emits_its_request():
    view = WorkflowProgressView(
        "Cull", default_message="Working…", retry_label="Retry now"
    )
    requests = []
    view.retry_requested.connect(lambda: requests.append(True))

    view.show_error("Failed")
    view.retry_button.click()

    assert requests == [True]


def test_dismiss_is_only_offered_when_a_workflow_acts_on_it():
    """A visible-but-unconnected escape hatch would silently do nothing."""
    plain = WorkflowProgressView("Pick Best Photos", default_message="Working…")
    dismissible = WorkflowProgressView(
        "Cull",
        default_message="Working…",
        dismiss_label="Continue without grouping",
    )

    plain.show_error("Failed")
    dismissible.show_error("Failed")

    assert plain.dismiss_button.isVisibleTo(plain) is False
    assert dismissible.dismiss_button.isVisibleTo(dismissible) is True
    assert dismissible.dismiss_button.text() == "Continue without grouping"


def test_step_pages_do_not_duplicate_the_footer_navigation():
    """Moving on is the footer's job, so the error page must not repeat it.

    An in-page "skip" button competed with the workflow footer for the same
    action. The page keeps only the action the footer cannot offer - retrying
    the run - and leaves navigation to the footer.
    """

    for widget in (
        EasyDeleteStepWidget(),
        FixRotationStepWidget(),
        PickBestStepWidget(),
    ):
        view = widget._progress_view
        widget.show_error("Model download was not approved.")

        assert view.dismiss_button.isVisibleTo(view) is False


def test_model_backed_pages_offer_a_retry_that_can_reopen_the_download_prompt():
    """A failed run must not dead-end into "skip"; the user can try again.

    Cancelling a model download leaves the page in an error state. Without a
    retry the user has no way to reopen the download prompt, so every
    model-backed page exposes one. Moving on stays with the footer navigation.
    """

    # Fix Rotation is excluded on purpose: its model is a manual download and its
    # missing-model panel already owns a "Check Again" button, so a second retry
    # on the progress card would be a duplicate control for the same action.
    pages = {
        "easy_delete": EasyDeleteStepWidget(),
        "pick_best": PickBestStepWidget(),
    }

    for name, widget in pages.items():
        view = widget._progress_view
        view.show_error(
            "The Pick Best aesthetic scoring model has not been downloaded yet."
        )

        assert view.retry_button.isVisibleTo(view), f"{name} offers no retry"

        seen = []
        widget.retry_requested.connect(lambda n=name: seen.append(n))
        view.retry_button.click()

        assert seen == [name], f"{name} retry is not wired to the page signal"


def test_a_long_error_message_does_not_collide_with_the_title():
    """Wrapped text must push the layout down rather than overlap the title.

    Centring the card with an alignment flag makes the layout ignore
    height-for-width, so a two-line message is laid out as if it were one line
    and paints over the title. This asserts the geometry instead of the styling.
    """

    view = WorkflowProgressView("Pick Best Photos", default_message="Starting…")
    view.resize(1000, 460)
    view.show_error(
        "Pick Best stopped because cluster 18 scoring failed. "
        "The Pick Best aesthetic scoring model has not been downloaded yet."
    )
    view.show()
    _app.processEvents()

    message = view.message_label
    needed = message.heightForWidth(message.width())

    assert needed > message.fontMetrics().height(), (
        "the fixture message no longer wraps, so it cannot detect the overlap"
    )
    assert message.height() >= needed, (
        "the wrapped error message was laid out as a single line, so it paints "
        "over the title instead of pushing the card taller"
    )

    view.hide()
