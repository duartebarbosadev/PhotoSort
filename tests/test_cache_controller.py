import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

from types import SimpleNamespace
from unittest.mock import Mock

from ui.controllers.cache_controller import CacheController


def _action():
    return SimpleNamespace(setChecked=Mock(), setEnabled=Mock())


def test_clear_analysis_cache_resets_dependent_ui_state(monkeypatch):
    clear_similarity_cache = Mock()
    monkeypatch.setattr(
        "core.similarity_engine.SimilarityEngine.clear_embedding_cache",
        clear_similarity_cache,
    )
    menu = SimpleNamespace(
        group_by_similarity_action=_action(),
        analyze_similarity_action=_action(),
        set_cluster_sort_menu_visible=Mock(),
        set_cluster_sort_menu_enabled=Mock(),
    )
    app_state = SimpleNamespace(
        analysis_cache=SimpleNamespace(clear_all=Mock()),
        cluster_results={"a.jpg": 1},
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
    clear_similarity_cache.assert_called_once_with()
    assert app_state.cluster_results == {}
    assert context.group_by_similarity_mode is False
    context._rebuild_model_view.assert_called_once_with()
    controller.update_labels.assert_called_once_with()


def test_clear_downloaded_models_reports_and_refreshes(monkeypatch):
    clear_models = Mock(return_value=("models--facebook--dinov2-small",))
    monkeypatch.setattr("core.model_provisioning.clear_model_caches", clear_models)
    monkeypatch.setattr(
        "core.model_provisioning.model_cache_usage_bytes",
        Mock(return_value=168 * 1024 * 1024),
    )
    context = SimpleNamespace(status_message=Mock())
    controller = CacheController(context)
    controller.update_labels = Mock()

    controller.clear_downloaded_models()

    clear_models.assert_called_once_with()
    message = context.status_message.call_args[0][0]
    assert "1 model" in message
    assert "168 MB" in message
    controller.update_labels.assert_called_once_with()


def test_clear_downloaded_models_survives_failure(monkeypatch):
    monkeypatch.setattr(
        "core.model_provisioning.clear_model_caches",
        Mock(side_effect=OSError("locked")),
    )
    monkeypatch.setattr(
        "core.model_provisioning.model_cache_usage_bytes", Mock(return_value=0)
    )
    context = SimpleNamespace(status_message=Mock())
    controller = CacheController(context)
    controller.update_labels = Mock()

    controller.clear_downloaded_models()

    assert "Failed" in context.status_message.call_args[0][0]
    controller.update_labels.assert_not_called()


def test_update_labels_reports_model_disk_usage(monkeypatch):
    monkeypatch.setattr(
        "core.model_provisioning.model_cache_usage_bytes",
        Mock(return_value=5 * 1024 * 1024),
    )
    label = SimpleNamespace(setText=Mock())
    context = SimpleNamespace(
        image_pipeline=SimpleNamespace(
            thumbnail_cache=SimpleNamespace(volume=Mock(return_value=0)),
            preview_cache=SimpleNamespace(volume=Mock(return_value=0)),
        ),
        app_state=SimpleNamespace(exif_disk_cache=None, analysis_cache=None),
        thumb_cache_usage_label=SimpleNamespace(setText=Mock()),
        preview_cache_configured_limit_label=SimpleNamespace(setText=Mock()),
        preview_cache_usage_label=SimpleNamespace(setText=Mock()),
        exif_cache_configured_limit_label=SimpleNamespace(setText=Mock()),
        exif_cache_usage_label=SimpleNamespace(setText=Mock()),
        model_cache_usage_label=label,
    )

    CacheController(context).update_labels()

    label.setText.assert_called_once_with("5.00 MB")
