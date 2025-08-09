import os
import sys
import pytest

# Insert project root before importing src modules
PROJECT_ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
if PROJECT_ROOT not in sys.path:  # pragma: no cover
    sys.path.insert(0, PROJECT_ROOT)

from PyQt6.QtWidgets import QApplication
from PyQt6.QtCore import Qt

# Attempt heavy UI import; skip module if dependencies (onnxruntime, torch, etc.) fail.
try:  # pragma: no cover - environment dependent
    from src.ui.main_window import MainWindow  # noqa: F401
except Exception as e:  # Broad except to catch DLL load issues
    import pytest as _pytest

    _pytest.skip(
        f"Skipping rotation acceptance integration tests (dependency load failure: {e.__class__.__name__}: {e})",
        allow_module_level=True,
    )

from src.ui.selection_utils import select_next_surviving_path  # sanity reference


@pytest.fixture(scope="module")
def qapp():
    # Reuse a single QApplication for speed; PyQt requires exactly one instance.
    app = QApplication.instance()
    if app is None:
        app = QApplication([])
    yield app


def _collect_visible_paths(window: MainWindow):
    # Use provided helper that respects filtering & current view
    return window._get_all_visible_image_paths()


@pytest.mark.skipif(not os.path.isdir("sample"), reason="sample folder missing for integration test")
def test_accept_single_rotation_advances_selection(qapp):
    # Launch window pointing at sample folder if present
    folder = os.path.abspath("sample")
    window = MainWindow(initial_folder=folder)
    window.show()  # Needed for some selection behaviors

    # Switch to rotation view if not already (depends on UI logic)
    window.left_panel.current_view_mode = "rotation"

    # Simulate presence of a rotation suggestion if none exist.
    # We approximate by faking an entry in rotation_suggestions for first image.
    visible_paths = _collect_visible_paths(window)
    if not visible_paths:
        pytest.skip("No images available in sample folder for integration test")

    first = visible_paths[0]
    if first not in window.rotation_suggestions:
        window.rotation_suggestions[first] = {"direction": "clockwise"}

    # Select first
    window._select_items_in_current_view([first])
    assert first in window._get_selected_file_paths_from_view()

    # Accept single rotation via internal helper (bypassing actual keyboard event)
    window._accept_single_rotation_and_move_to_next()

    # After acceptance, the suggestion should be gone and a new selection chosen
    assert first not in window.rotation_suggestions
    new_selection = window._get_selected_file_paths_from_view()
    if new_selection:
        # Newly selected item must differ from the one we accepted
        assert first not in new_selection

    window.close()


@pytest.mark.skipif(not os.path.isdir("sample"), reason="sample folder missing for integration test")
def test_accept_multi_rotation_advances_selection(qapp):
    folder = os.path.abspath("sample")
    window = MainWindow(initial_folder=folder)
    window.show()
    window.left_panel.current_view_mode = "rotation"

    visible_paths = _collect_visible_paths(window)
    if len(visible_paths) < 3:
        pytest.skip("Need at least 3 images for multi-selection test")

    targets = visible_paths[:3]
    for p in targets:
        window.rotation_suggestions[p] = {"direction": "clockwise"}

    window._select_items_in_current_view(targets)
    before_set = set(window._get_selected_file_paths_from_view())
    assert before_set == set(targets)

    # Accept current rotation (multi) -> should remove all and select next surviving
    window._accept_current_rotation()

    for p in targets:
        assert p not in window.rotation_suggestions

    post_selection = window._get_selected_file_paths_from_view()
    # Selection may be empty if no more items; if not empty, ensure it's not one of the removed targets.
    if post_selection:
        assert not (set(post_selection) & set(targets))

    window.close()
