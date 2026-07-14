from types import SimpleNamespace
from unittest.mock import Mock

from core.app_settings import THUMBNAIL_PRELOAD_BATCH_SIZE
from ui.thumbnail_load_coordinator import ViewportThumbnailLoader


def _context(item_count=100):
    action = Mock()
    action.isChecked.return_value = True
    worker_manager = Mock()
    worker_manager.is_thumbnail_preload_running.return_value = False
    return SimpleNamespace(
        menu_manager=SimpleNamespace(toggle_thumbnails_action=action),
        worker_manager=worker_manager,
        app_state=SimpleNamespace(
            image_files_data=[
                {"path": f"image-{index}.jpg"} for index in range(item_count)
            ]
        ),
    )


def test_fallback_thumbnail_load_is_bounded_and_deduplicated():
    context = _context()
    loader = ViewportThumbnailLoader(context)
    loader._visible_paths = Mock(return_value=[])

    loader._load_visible_batch()
    loader._load_visible_batch()

    expected = [f"image-{index}.jpg" for index in range(THUMBNAIL_PRELOAD_BATCH_SIZE)]
    context.worker_manager.start_thumbnail_preload.assert_called_once_with(expected)


def test_thumbnail_loader_reset_allows_a_new_folder_to_request_paths_again():
    context = _context(item_count=1)
    loader = ViewportThumbnailLoader(context)
    loader._visible_paths = Mock(return_value=[])

    loader._load_visible_batch()
    loader.reset()
    loader._load_visible_batch()

    assert context.worker_manager.start_thumbnail_preload.call_count == 2
