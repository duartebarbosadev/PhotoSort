import json
import os
import subprocess
import sys
from pathlib import Path

from core import app_settings
from core.packaging_smoke import REQUIRED_PACKAGED_MODULES

PROJECT_ROOT = Path(__file__).resolve().parents[1]


def test_main_window_import_does_not_eagerly_load_heavy_or_hidden_modes():
    code = """
import json
import sys
import ui.main_window
print(json.dumps({
    'sklearn': 'sklearn' in sys.modules,
    'easy_delete': 'ui.easy_delete_step_widget' in sys.modules,
    'fix_rotation': 'ui.fix_rotation_step_widget' in sys.modules,
    'pick_best': 'ui.pick_best_step_widget' in sys.modules,
    'metadata_sidebar': 'ui.metadata_sidebar' in sys.modules,
}))
"""
    env = os.environ.copy()
    env["PYTHONPATH"] = str(PROJECT_ROOT / "src")
    env["QT_QPA_PLATFORM"] = "offscreen"
    process = subprocess.run(
        [sys.executable, "-c", code],
        cwd=PROJECT_ROOT,
        env=env,
        text=True,
        capture_output=True,
        check=True,
        timeout=20,
    )
    state = json.loads(process.stdout.strip().splitlines()[-1])
    assert state == {
        "sklearn": False,
        "easy_delete": False,
        "fix_rotation": False,
        "pick_best": False,
        "metadata_sidebar": False,
    }


def test_image_work_budgets_follow_performance_mode(monkeypatch):
    monkeypatch.setattr(app_settings, "get_available_cpu_count", lambda: 14)
    monkeypatch.setattr(
        app_settings,
        "get_performance_mode",
        lambda: app_settings.PerformanceMode.PERFORMANCE,
    )
    monkeypatch.setattr(
        app_settings,
        "get_usable_memory_bytes",
        lambda: 36 * 1024**3,
    )

    assert app_settings.calculate_thumbnail_workers() == 14
    assert app_settings.calculate_high_memory_decode_workers() == 4
    assert app_settings.FILE_SCAN_EMIT_BATCH_SIZE >= 32
    assert app_settings.THUMBNAIL_PRELOAD_BATCH_SIZE <= 32
    assert max(app_settings.DISPLAY_MAX_RESOLUTION) <= 2560


def test_packaging_contract_covers_every_lazy_workflow():
    required = set(REQUIRED_PACKAGED_MODULES)
    assert {
        "ui.easy_delete_step_widget",
        "ui.fix_rotation_step_widget",
        "ui.metadata_sidebar",
        "ui.pick_best_step_widget",
        "workers.best_shot_worker",
        "workers.grouping_worker",
    } <= required


def test_packaging_excludes_unused_native_sentencepiece_extension():
    spec = (PROJECT_ROOT / "PhotoSort.spec").read_text(encoding="utf-8")
    assert '"sentencepiece"' in spec
