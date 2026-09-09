"""Shared consent policy for workflows that need a downloadable model.

Similarity, Cull and Pick Best all face the same two questions before they can
start: are the model weights present, and is hardware acceleration available?
Keeping the answer here means every workflow asks in the same words, remembers
the same approvals for the session, and cannot drift apart.
"""

from __future__ import annotations

from collections.abc import Sequence
from dataclasses import dataclass, field
from enum import Enum
from typing import Protocol


class PrerequisiteDecline(Enum):
    """Why a workflow may not start."""

    DOWNLOAD = "download"
    ACCELERATION = "acceleration"
    #: A consent prompt is already on screen. The modal dialog runs a nested Qt
    #: event loop, so a queued signal can re-enter a workflow start method while
    #: the user is still deciding. Such a start is a duplicate of the one already
    #: waiting, and must abort silently rather than stack a second dialog.
    BUSY = "busy"


@dataclass(frozen=True)
class PrerequisiteOutcome:
    """The verdict of the prerequisite check.

    ``allow_download`` is only meaningful when ``declined`` is ``None``.
    """

    allow_download: bool = False
    declined: PrerequisiteDecline | None = None

    @property
    def approved(self) -> bool:
        return self.declined is None


class ModelConsentDialogs(Protocol):
    """The dialog surface the policy needs (satisfied by ``DialogManager``)."""

    def confirm_model_download(
        self, model_keys: list[str], *, feature: str, fallback: str = ""
    ) -> bool: ...

    def confirm_slow_cpu_processing(self, feature: str) -> bool: ...


@dataclass
class ModelConsentState:
    """Session-scoped memory of what the user already agreed to."""

    approved_downloads: set[str] = field(default_factory=set)
    cpu_accepted: bool = False

    def reset_downloads(self, model_keys: Sequence[str]) -> None:
        """Forget approvals once the download has actually succeeded."""

        self.approved_downloads.difference_update(model_keys)

    def forget_downloads(self) -> None:
        """Forget every approval so the next start asks again.

        An explicit retry means the previous attempt did not work, so the user
        should be shown the download prompt rather than silently reusing an
        approval that never produced a usable model.
        """

        self.approved_downloads.clear()


def confirm_model_prerequisites(
    dialogs: ModelConsentDialogs,
    state: ModelConsentState,
    *,
    required_keys: Sequence[str],
    missing_keys: Sequence[str],
    torch_device: str,
    feature: str,
    fallback: str = "",
) -> PrerequisiteOutcome:
    """Ask for whatever consent is still outstanding for ``required_keys``."""

    outstanding = [key for key in required_keys if key in set(missing_keys)]
    allow_download = False
    if outstanding:
        if state.approved_downloads.issuperset(outstanding):
            allow_download = True
        elif dialogs.confirm_model_download(
            list(outstanding),
            feature=feature,
            fallback=fallback,
        ):
            state.approved_downloads.update(outstanding)
            allow_download = True
        else:
            return PrerequisiteOutcome(declined=PrerequisiteDecline.DOWNLOAD)

    if torch_device == "cpu" and not state.cpu_accepted:
        if not dialogs.confirm_slow_cpu_processing(feature):
            return PrerequisiteOutcome(declined=PrerequisiteDecline.ACCELERATION)
        state.cpu_accepted = True

    return PrerequisiteOutcome(allow_download=allow_download)


class DeferredModelStarts:
    """Workflow starts waiting on the one-shot model-environment probe.

    Resolving what is installed takes seconds, so every model-backed workflow
    defers its start behind the same probe and resumes from a single callback.
    Each workflow used to carry its own hand-managed latch that had to be reset
    in four separate places - construction, folder load, cancellation and
    resume - and missing any one of them leaves that workflow permanently unable
    to start again. Holding the latches together keeps those resets in one place.

    A latch may carry a payload (the arguments the deferred start needs), so
    ``take`` returns it and clears the latch in one step.
    """

    def __init__(self) -> None:
        self._pending: dict[str, object] = {}

    def arm(self, name: str, payload: object = True) -> None:
        self._pending[name] = payload

    def is_armed(self, name: str) -> bool:
        return name in self._pending

    def take(self, name: str) -> object | None:
        """Return and clear the latch, or ``None`` when it was not armed."""

        return self._pending.pop(name, None)

    def disarm(self, name: str) -> None:
        self._pending.pop(name, None)

    def clear(self) -> None:
        self._pending.clear()

    def __bool__(self) -> bool:
        return bool(self._pending)
