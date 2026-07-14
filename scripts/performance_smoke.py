#!/usr/bin/env python3
"""Run a repeatable PhotoSort startup/folder-load smoke test and emit JSON."""

from __future__ import annotations

import argparse
import json
import os
import resource
import sys
import tempfile
import time
from pathlib import Path

PROJECT_ROOT = Path(__file__).resolve().parents[1]
SRC_ROOT = PROJECT_ROOT / "src"
sys.path.insert(0, str(PROJECT_ROOT))
sys.path.insert(0, str(SRC_ROOT))
os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")


def _max_rss_mb() -> float:
    value = resource.getrusage(resource.RUSAGE_SELF).ru_maxrss
    if sys.platform == "darwin":
        return value / (1024 * 1024)
    return value / 1024


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--folder", type=Path, help="Optional media folder to scan")
    parser.add_argument(
        "--duration",
        type=float,
        default=1.0,
        help="Seconds to process events before reporting (default: 1)",
    )
    parser.add_argument(
        "--cold-cache",
        action="store_true",
        help="Use a temporary empty cache for a reproducible cold run",
    )
    args = parser.parse_args()
    if args.folder and not args.folder.is_dir():
        parser.error(f"Folder does not exist: {args.folder}")

    temporary_cache = None
    if args.cold_cache:
        temporary_cache = tempfile.TemporaryDirectory(prefix="photosort-perf-")
        os.environ["PHOTOSORT_CACHE_ROOT"] = temporary_cache.name

    process_started = time.perf_counter()
    import pyexiv2  # noqa: F401  # Must precede Qt on Windows

    from PyQt6.QtCore import QTimer
    from PyQt6.QtWidgets import QApplication
    from pillow_heif import register_heif_opener

    from ui.main_window import MainWindow

    register_heif_opener()
    imported_at = time.perf_counter()
    app = QApplication.instance() or QApplication([])
    window = MainWindow(initial_folder=str(args.folder) if args.folder else None)
    constructed_at = time.perf_counter()
    scan_finished_at: list[float] = []
    window.worker_manager.file_scan_finished.connect(
        lambda: scan_finished_at.append(time.perf_counter())
    )
    window.show()

    QTimer.singleShot(max(1, int(args.duration * 1000)), app.quit)
    app.exec()
    measured_at = time.perf_counter()

    result = {
        "folder": str(args.folder.resolve()) if args.folder else None,
        "import_seconds": round(imported_at - process_started, 4),
        "window_construct_seconds": round(constructed_at - imported_at, 4),
        "measurement_seconds": round(measured_at - process_started, 4),
        "folder_usable_seconds": (
            round(scan_finished_at[0] - process_started, 4)
            if scan_finished_at
            else None
        ),
        "media_count": len(window.app_state.image_files_data),
        "max_rss_mb": round(_max_rss_mb(), 1),
        "thumbnail_cache_mb": round(
            window.image_pipeline.thumbnail_cache.volume() / (1024 * 1024), 1
        ),
        "preview_cache_mb": round(
            window.image_pipeline.preview_cache.volume() / (1024 * 1024), 1
        ),
        "sklearn_loaded": "sklearn" in sys.modules,
        "lazy_workflows_created": {
            "easy_delete": window.easy_delete_step_widget is not None,
            "fix_rotation": window.fix_rotation_step_widget is not None,
            "pick_best": window.pick_best_step_widget is not None,
            "metadata_sidebar": window.metadata_sidebar is not None,
        },
    }
    print(json.dumps(result, indent=2, sort_keys=True))

    window.worker_manager.stop_all_workers()
    window.close()
    if temporary_cache is not None:
        temporary_cache.cleanup()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
