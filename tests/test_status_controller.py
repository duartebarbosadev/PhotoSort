import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from types import SimpleNamespace
from unittest.mock import Mock

from ui.app_state import MediaSummary
from ui.controllers.status_controller import StatusController


def test_status_controller_uses_precomputed_media_summary():
    status_bar = Mock()
    left_panel = Mock()
    context = SimpleNamespace(
        app_state=SimpleNamespace(
            current_folder_path="/photos/trip",
            image_files_data=[object(), object(), object()],
            media_summary=Mock(
                return_value=MediaSummary(
                    total_items=3,
                    image_count=2,
                    video_count=1,
                    total_size_bytes=3 * 1024 * 1024,
                )
            ),
        ),
        image_pipeline=SimpleNamespace(
            preview_cache=SimpleNamespace(volume=Mock(return_value=1024 * 1024))
        ),
        menu_manager=SimpleNamespace(
            open_folder_action=SimpleNamespace(isEnabled=Mock(return_value=True))
        ),
        left_panel=left_panel,
        statusBar=Mock(return_value=status_bar),
    )

    StatusController(context).update()

    message = status_bar.showMessage.call_args.args[0]
    assert "Images: 2" in message
    assert "Videos: 1" in message
    context.app_state.media_summary.assert_called_once_with()
    left_panel.update_context.assert_called_once_with(
        "trip", 3, "2 images • 1 videos • 3.0 MB"
    )
