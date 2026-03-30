from types import SimpleNamespace

from src.ui.main_window import MainWindow


class _DummyEvent:
    def __init__(self):
        self.accepted = False
        self.ignored = False

    def accept(self):
        self.accepted = True

    def ignore(self):
        self.ignored = True


class _DummyStatusBar:
    def __init__(self):
        self.messages = []

    def showMessage(self, message: str, timeout: int):
        self.messages.append((message, timeout))


def test_close_event_blocks_while_grouping_workflow_is_running():
    status_bar = _DummyStatusBar()
    window = SimpleNamespace(
        worker_manager=SimpleNamespace(
            is_grouping_workflow_running=lambda: True,
        ),
        grouping_step_widget=SimpleNamespace(
            pending_grouping_action_lines=lambda: [],
            has_unsaved_grouping_edits=lambda: False,
        ),
        statusBar=lambda: status_bar,
    )
    event = _DummyEvent()

    MainWindow.closeEvent(window, event)

    assert event.ignored
    assert not event.accepted
    assert status_bar.messages == [
        (
            "Grouping is still moving files. Wait for it to finish before closing.",
            4000,
        )
    ]
