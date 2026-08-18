import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from ui.controllers.model_prerequisites import (
    ModelConsentState,
    PrerequisiteDecline,
    confirm_model_prerequisites,
)


class _Dialogs:
    def __init__(self, *, approve_download=True, approve_cpu=True):
        self.approve_download = approve_download
        self.approve_cpu = approve_cpu
        self.download_calls = []
        self.cpu_calls = []

    def confirm_model_download(self, model_keys, *, feature, fallback=""):
        self.download_calls.append((list(model_keys), feature, fallback))
        return self.approve_download

    def confirm_slow_cpu_processing(self, feature):
        self.cpu_calls.append(feature)
        return self.approve_cpu


def _confirm(dialogs, state, **overrides):
    kwargs = {
        "required_keys": ["embedding"],
        "missing_keys": ["embedding"],
        "torch_device": "mps",
        "feature": "similarity grouping",
    }
    kwargs.update(overrides)
    return confirm_model_prerequisites(dialogs, state, **kwargs)


def test_installed_model_on_accelerated_device_asks_nothing():
    dialogs = _Dialogs()
    outcome = _confirm(dialogs, ModelConsentState(), missing_keys=[])

    assert outcome.approved is True
    assert outcome.allow_download is False
    assert dialogs.download_calls == []
    assert dialogs.cpu_calls == []


def test_only_missing_required_models_are_offered():
    dialogs = _Dialogs()
    outcome = _confirm(
        dialogs,
        ModelConsentState(),
        required_keys=["embedding"],
        missing_keys=["embedding", "aesthetic"],
    )

    assert outcome.allow_download is True
    assert dialogs.download_calls[0][0] == ["embedding"]


def test_declined_download_reports_the_reason():
    dialogs = _Dialogs(approve_download=False)
    state = ModelConsentState()
    outcome = _confirm(dialogs, state)

    assert outcome.approved is False
    assert outcome.declined is PrerequisiteDecline.DOWNLOAD
    assert state.approved_downloads == set()
    # A refused download must not also trigger the CPU question.
    assert dialogs.cpu_calls == []


def test_approval_is_remembered_for_the_session():
    dialogs = _Dialogs()
    state = ModelConsentState()

    assert _confirm(dialogs, state).allow_download is True
    assert _confirm(dialogs, state).allow_download is True

    assert len(dialogs.download_calls) == 1


def test_successful_download_clears_the_remembered_approval():
    state = ModelConsentState()
    state.approved_downloads.add("embedding")

    state.reset_downloads(["embedding"])

    assert state.approved_downloads == set()


def test_cpu_warning_is_shared_across_features_and_asked_once():
    dialogs = _Dialogs()
    state = ModelConsentState()

    _confirm(dialogs, state, missing_keys=[], torch_device="cpu")
    _confirm(
        dialogs,
        state,
        required_keys=["aesthetic"],
        missing_keys=[],
        torch_device="cpu",
        feature="Pick Best scoring",
    )

    assert dialogs.cpu_calls == ["similarity grouping"]


def test_declined_cpu_warning_reports_the_reason():
    dialogs = _Dialogs(approve_cpu=False)
    state = ModelConsentState()
    outcome = _confirm(dialogs, state, missing_keys=[], torch_device="cpu")

    assert outcome.declined is PrerequisiteDecline.ACCELERATION
    assert state.cpu_accepted is False


def test_an_explicit_retry_forgets_approvals_so_the_prompt_returns():
    """Retry must re-ask, because the previous approval produced no model.

    ``reset_downloads`` is for the opposite case - a run that proved the weights
    are on disk. When the user retries after a failure we cannot trust the
    remembered approval, so it is dropped and the dialog is shown again.
    """

    dialogs = _Dialogs()
    state = ModelConsentState(approved_downloads={"embedding"})

    _confirm(dialogs, state)
    assert dialogs.download_calls == [], "a remembered approval should not re-ask"

    state.forget_downloads()
    outcome = _confirm(dialogs, state)

    assert outcome.approved is True
    assert outcome.allow_download is True
    assert len(dialogs.download_calls) == 1


def test_retry_entry_points_reprobe_and_restart_their_workflow():
    """Retry must also drop the cached probe result, not just the approval.

    ``_model_environment`` is resolved once per process. If a download was
    cancelled, or a snapshot was deleted, that cached answer is stale and a retry
    would start the workflow with ``allow_download=False`` all over again.
    """

    from ui.app_controller import AppController

    class _Controller:
        _model_environment = ((), "cpu")
        _reset_model_environment = AppController._reset_model_environment
        retry_pick_best_workflow = AppController.retry_pick_best_workflow
        retry_easy_delete_workflow = AppController.retry_easy_delete_workflow

        _pick_best_pending_after_subject_grouping = True
        _easy_delete_pending_after_similarity = True

        def __init__(self):
            self._model_consent = ModelConsentState(approved_downloads={"aesthetic"})
            self.started = []

        def start_pick_best_workflow(self):
            self.started.append("pick_best")

        def start_easy_delete_workflow(self):
            self.started.append("easy_delete")

    latches = {
        "pick_best": "_pick_best_pending_after_subject_grouping",
        "easy_delete": "_easy_delete_pending_after_similarity",
    }
    for method, expected in (
        ("retry_pick_best_workflow", "pick_best"),
        ("retry_easy_delete_workflow", "easy_delete"),
    ):
        controller = _Controller()
        getattr(controller, method)()

        assert controller.started == [expected]
        assert controller._model_environment is None, "the stale probe was reused"
        assert controller._model_consent.approved_downloads == set()
        # A stale "waiting for the shared grouping run" latch would make the
        # restart a silent no-op, so retry must clear it too.
        assert getattr(controller, latches[expected]) is False


def test_a_second_prompt_cannot_open_while_one_is_already_on_screen():
    """The modal prompt runs a nested event loop, so starts can re-enter.

    While the user looks at the consent dialog, a queued Qt signal can call a
    workflow start method again. Without a guard that would stack a second
    dialog and start the same workflow twice, so the re-entrant call is refused
    with ``BUSY`` and the caller aborts silently.
    """

    from ui.app_controller import AppController

    class _Controller:
        _model_environment = (("embedding",), "mps")
        _confirm_model_prerequisites = AppController._confirm_model_prerequisites

        def __init__(self):
            self._consent_prompt_active = False
            self._model_consent = ModelConsentState()
            self.reentrant_outcome = None

            controller = self

            class _Reentrant(_Dialogs):
                def confirm_model_download(self, model_keys, *, feature, fallback=""):
                    # Simulate a queued signal firing inside the nested loop.
                    controller.reentrant_outcome = (
                        controller._confirm_model_prerequisites(
                            ["embedding"], feature="similarity grouping"
                        )
                    )
                    return super().confirm_model_download(
                        model_keys, feature=feature, fallback=fallback
                    )

            self.dialogs = _Reentrant()
            self.main_window = type("_MW", (), {"dialog_manager": self.dialogs})()

    controller = _Controller()
    outcome = controller._confirm_model_prerequisites(
        ["embedding"], feature="similarity grouping"
    )

    assert outcome.approved is True
    assert controller.reentrant_outcome.declined is PrerequisiteDecline.BUSY
    assert len(controller.dialogs.download_calls) == 1, "a second dialog was opened"
    # The guard must not latch: a later, legitimate prompt still works.
    assert controller._consent_prompt_active is False


def test_deferred_starts_are_cleared_together():
    """One reset must clear every workflow waiting on the probe.

    Each workflow previously owned a hand-managed latch that had to be reset in
    four places. Missing one left that workflow believing a start was still
    queued, so it would refuse to start again.
    """

    from ui.controllers.model_prerequisites import DeferredModelStarts

    starts = DeferredModelStarts()
    starts.arm("similarity")
    starts.arm("grouping_workflow", ("similarity", {}, None))

    assert bool(starts) is True
    assert starts.is_armed("similarity") is True
    assert starts.is_armed("pick_best_scoring") is False

    starts.clear()

    assert bool(starts) is False
    assert starts.is_armed("similarity") is False
    assert starts.take("grouping_workflow") is None


def test_taking_a_deferred_start_returns_its_payload_and_disarms_it():
    """Resuming must be a single step, so a start cannot be run twice."""

    from ui.controllers.model_prerequisites import DeferredModelStarts

    starts = DeferredModelStarts()
    payload = ("similarity", {"a": "b"}, None)
    starts.arm("grouping_workflow", payload)

    assert starts.take("grouping_workflow") == payload
    assert starts.take("grouping_workflow") is None
    assert starts.is_armed("grouping_workflow") is False
