from types import SimpleNamespace

from src.ui.app_controller import AppController


class _DummyStatusBar:
    def __init__(self):
        self.messages = []

    def showMessage(self, message: str, timeout: int):
        self.messages.append((message, timeout))


def test_load_folder_blocks_while_grouping_workflow_is_running():
    status_bar = _DummyStatusBar()
    controller = SimpleNamespace(
        worker_manager=SimpleNamespace(
            is_grouping_workflow_running=lambda: True,
        ),
        main_window=SimpleNamespace(
            statusBar=lambda: status_bar,
        ),
    )

    AppController.load_folder(controller, "/tmp/demo")

    assert status_bar.messages == [
        (
            "Grouping is still moving files. Wait for it to finish before loading another folder.",
            4000,
        )
    ]
