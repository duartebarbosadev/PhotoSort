import pyexiv2  # noqa: F401  # Must be first to avoid Windows crashes

import unicodedata
from types import SimpleNamespace
from unittest.mock import Mock

from core import app_settings
from core.caching.exif_cache import ExifCache
from ui.app_controller import AppController
from workers.rating_loader_worker import RatingLoaderWorker


def test_exif_cache_defaults_to_two_gb_and_caps_selector_at_five_gb():
    assert app_settings.DEFAULT_EXIF_CACHE_SIZE_MB == 2 * 1024
    assert app_settings.MAX_EXIF_CACHE_SIZE_MB == 5 * 1024


def test_dataset_residency_uses_unique_canonical_paths():
    cache = ExifCache.__new__(ExifCache)
    decomposed = "photos/Cafe\N{COMBINING ACUTE ACCENT}.ARW"
    canonical = unicodedata.normalize("NFC", decomposed)
    cache._cache = {canonical: {"rating": 0}}

    assert cache.dataset_residency([decomposed, canonical, "missing.ARW"]) == (1, 2)


def test_cache_near_capacity_uses_configured_limit():
    cache = ExifCache.__new__(ExifCache)
    cache._size_limit_bytes = 100
    cache.volume = Mock(return_value=95)

    assert cache.is_near_capacity() is True

    cache.volume.return_value = 94
    assert cache.is_near_capacity() is False


def test_rating_loader_reports_dataset_eviction(tmp_path, monkeypatch):
    first = tmp_path / "one.jpg"
    second = tmp_path / "two.jpg"
    first.touch()
    second.touch()
    paths = [str(first), str(second)]
    exif_cache = SimpleNamespace(
        dataset_residency=Mock(return_value=(1, 2)),
        get_current_size_limit_bytes=Mock(return_value=2 * 1024**3),
        is_near_capacity=Mock(return_value=True),
    )
    state = SimpleNamespace(exif_disk_cache=exif_cache, rating_cache={}, date_cache={})
    worker = RatingLoaderWorker(
        [{"path": path} for path in paths],
        rating_disk_cache=Mock(),
        app_state=state,
    )
    monkeypatch.setattr(
        "workers.rating_loader_worker.MetadataProcessor.get_batch_display_metadata",
        lambda *_args: {path: {"rating": 0, "date": None} for path in paths},
    )
    warnings = []
    worker.cache_capacity_warning.connect(lambda *args: warnings.append(args))

    worker.run_load()

    assert warnings == [(2, 1, 2 * 1024**3)]


def test_capacity_dialog_is_deferred_until_metadata_load_finishes(monkeypatch):
    dialog_manager = SimpleNamespace(show_exif_cache_capacity_warning=Mock())
    status_bar = SimpleNamespace(showMessage=Mock())
    main_window = SimpleNamespace(
        dialog_manager=dialog_manager,
        statusBar=lambda: status_bar,
        hide_loading_overlay=Mock(),
        hide_exif_progress=Mock(),
    )
    controller = AppController(main_window, Mock(), Mock())
    monkeypatch.setattr(
        "ui.app_controller.QTimer.singleShot",
        lambda _delay, callback: callback(),
    )

    controller.handle_exif_cache_capacity_warning(4_482, 3_792, 1024**3)
    assert dialog_manager.show_exif_cache_capacity_warning.call_count == 0

    controller.handle_rating_load_finished()

    dialog_manager.show_exif_cache_capacity_warning.assert_called_once_with(
        4_482,
        3_792,
        1024**3,
    )
