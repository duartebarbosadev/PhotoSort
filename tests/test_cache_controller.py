import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from types import SimpleNamespace
from unittest.mock import Mock

from ui.controllers.cache_controller import CacheController


def _action():
    return SimpleNamespace(setChecked=Mock(), setEnabled=Mock())


def test_clear_analysis_cache_resets_dependent_ui_state():
    menu = SimpleNamespace(
        group_by_similarity_action=_action(),
        analyze_best_shots_action=_action(),
        stop_best_shots_action=_action(),
        analyze_best_shots_selected_action=_action(),
        analyze_similarity_action=_action(),
        set_cluster_sort_menu_visible=Mock(),
        set_cluster_sort_menu_enabled=Mock(),
    )
    app_state = SimpleNamespace(
        analysis_cache=SimpleNamespace(clear_all=Mock()),
        cluster_results={"a.jpg": 1},
        clear_best_shot_results=Mock(),
        image_files_data=[{"path": "a.jpg"}],
    )
    context = SimpleNamespace(
        app_state=app_state,
        menu_manager=menu,
        group_by_similarity_mode=True,
        cluster_filter_combo=SimpleNamespace(
            clear=Mock(), addItem=Mock(), setEnabled=Mock()
        ),
        cluster_sort_combo=SimpleNamespace(setEnabled=Mock()),
        status_message=Mock(),
        refresh_navigation_shortcut_actions=Mock(),
        _rebuild_model_view=Mock(),
    )
    controller = CacheController(context)
    controller.update_labels = Mock()

    controller.clear_analysis_cache()

    app_state.analysis_cache.clear_all.assert_called_once_with()
    assert app_state.cluster_results == {}
    assert context.group_by_similarity_mode is False
    app_state.clear_best_shot_results.assert_called_once_with()
    context._rebuild_model_view.assert_called_once_with()
    controller.update_labels.assert_called_once_with()
