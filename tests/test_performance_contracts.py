import json
import os
import subprocess
import sys
from pathlib import Path

from core.app_settings import (
    DISPLAY_MAX_RESOLUTION,
    FILE_SCAN_EMIT_BATCH_SIZE,
    HIGH_MEMORY_DECODE_MAX_WORKERS,
    IMAGE_PIPELINE_MAX_WORKERS,
    THUMBNAIL_PRELOAD_BATCH_SIZE,
)
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


def test_image_work_budgets_remain_bounded():
    assert 1 <= IMAGE_PIPELINE_MAX_WORKERS <= 4
    assert HIGH_MEMORY_DECODE_MAX_WORKERS == 1
    assert FILE_SCAN_EMIT_BATCH_SIZE >= 32
    assert THUMBNAIL_PRELOAD_BATCH_SIZE <= 32
    assert max(DISPLAY_MAX_RESOLUTION) <= 2560


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
