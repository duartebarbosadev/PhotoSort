from types import SimpleNamespace
from unittest.mock import Mock

from core.app_settings import LARGE_FOLDER_THRESHOLD, UI_POPULATION_CHUNK_SIZE
import ui.helpers.ui_yield as ui_yield


def test_cooperative_ui_yield_processes_large_collections_by_chunk(monkeypatch):
    process_events = Mock()
    monkeypatch.setattr(
        ui_yield,
        "QCoreApplication",
        SimpleNamespace(instance=lambda: object(), processEvents=process_events),
    )
    progress = Mock()

    ui_yield.cooperative_ui_yield(
        UI_POPULATION_CHUNK_SIZE,
        LARGE_FOLDER_THRESHOLD + 1,
        progress_callback=progress,
    )

    progress.assert_called_once_with(
        UI_POPULATION_CHUNK_SIZE,
        LARGE_FOLDER_THRESHOLD + 1,
    )
    process_events.assert_called_once()


def test_cooperative_ui_yield_skips_small_collections(monkeypatch):
    process_events = Mock()
    monkeypatch.setattr(
        ui_yield,
        "QCoreApplication",
        SimpleNamespace(instance=lambda: object(), processEvents=process_events),
    )

    ui_yield.cooperative_ui_yield(1, LARGE_FOLDER_THRESHOLD)

    process_events.assert_not_called()
