from types import SimpleNamespace
from unittest.mock import Mock

from src.ui.controllers.active_image_controller import ActiveImageController


class _Context:
    def __init__(self):
        self.app_state = SimpleNamespace(focused_image_path=None)
        self.adapters = {}

    def get_active_image_adapter(self, workflow_step):
        return self.adapters.get(workflow_step)


def test_publish_updates_state_and_synchronizes_other_workflows_once():
    context = _Context()
    organize = SimpleNamespace(focus_image=Mock(return_value=True))
    easy_delete = SimpleNamespace(focus_image=Mock(return_value=True))
    context.adapters = {"organize": organize, "easy_delete": easy_delete}
    controller = ActiveImageController(context)

    assert controller.publish("/photos/a.jpg", source="organize")

    assert context.app_state.focused_image_path == "/photos/a.jpg"
    organize.focus_image.assert_not_called()
    easy_delete.focus_image.assert_called_once_with("/photos/a.jpg")


def test_programmatic_adapter_feedback_is_ignored():
    context = _Context()
    controller = ActiveImageController(context)
    feedback = Mock(
        side_effect=lambda _path: controller.publish(
            "/photos/unwanted.jpg", source="easy_delete"
        )
    )
    context.adapters = {
        "easy_delete": SimpleNamespace(focus_image=feedback),
    }

    controller.publish("/photos/a.jpg", source="organize")

    assert context.app_state.focused_image_path == "/photos/a.jpg"
    feedback.assert_called_once_with("/photos/a.jpg")


def test_unrepresented_path_does_not_replace_active_path_or_force_fallback():
    context = _Context()
    adapter = SimpleNamespace(focus_image=Mock(return_value=False))
    context.adapters = {"easy_delete": adapter}
    context.app_state.focused_image_path = "/photos/a.jpg"
    controller = ActiveImageController(context)

    assert not controller.sync_workflow("easy_delete")
    assert context.app_state.focused_image_path == "/photos/a.jpg"
    adapter.focus_image.assert_called_once_with("/photos/a.jpg")


def test_active_path_can_be_cleared_and_updated_centrally():
    context = _Context()
    adapter = SimpleNamespace(focus_image=Mock(return_value=True))
    context.adapters = {"organize": adapter}
    context.app_state.focused_image_path = "/photos/old.jpg"
    controller = ActiveImageController(context)

    controller.path_updated("/photos/old.jpg", "/photos/new.jpg")

    assert context.app_state.focused_image_path == "/photos/new.jpg"
    adapter.focus_image.assert_called_once_with("/photos/new.jpg")
    assert controller.clear_if_active("/photos/new.jpg")
    assert context.app_state.focused_image_path is None
