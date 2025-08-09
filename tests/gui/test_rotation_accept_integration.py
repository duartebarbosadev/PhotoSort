import os
import pytest
from PyQt6.QtWidgets import QApplication  # type: ignore  # noqa: F401
from src.ui.main_window import MainWindow


def _collect_visible_paths(window: MainWindow):
    return window._get_all_visible_image_paths()


@pytest.mark.skipif(not os.path.isdir("sample"), reason="sample folder missing for integration test")
def test_accept_single_rotation_advances_selection(qapp):
    folder = os.path.abspath("sample")
    window = MainWindow(initial_folder=folder)
    window.show()
    window.left_panel.current_view_mode = "rotation"

    visible_paths = _collect_visible_paths(window)
    if not visible_paths:
        pytest.skip("No images available in sample folder for integration test")

    first = visible_paths[0]
    if first not in window.rotation_suggestions:
        window.rotation_suggestions[first] = {"direction": "clockwise"}

    window._select_items_in_current_view([first])
    assert first in window._get_selected_file_paths_from_view()

    window._accept_single_rotation_and_move_to_next()

    assert first not in window.rotation_suggestions
    new_selection = window._get_selected_file_paths_from_view()
    if new_selection:
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
    assert set(window._get_selected_file_paths_from_view()) == set(targets)

    window._accept_current_rotation()

    for p in targets:
        assert p not in window.rotation_suggestions

    post_selection = window._get_selected_file_paths_from_view()
    if post_selection:
        assert not (set(post_selection) & set(targets))
    window.close()
