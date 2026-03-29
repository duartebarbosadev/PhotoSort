# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Development Commands

```bash
# Run the application
python -m src.main [--folder FOLDER_PATH] [--clear-cache]

# Run tests with coverage
pytest -q --cov=src --cov-report=xml:coverage.xml --cov-report=term

# Run a specific test file
pytest tests/test_file_deletion_controller.py

# Linting and formatting
ruff check .
ruff format .

# Headless smoke test (CI)
QT_QPA_PLATFORM=offscreen timeout 20s python src/main.py --smoke-test --smoke-delay-ms 1500
```

## Architecture Overview

PhotoSort is a PyQt6 desktop photo culling application following MVC architecture with a controller layer for behavior encapsulation.

### Package Structure

- **`src/core/`**: Business logic, independent of UI
  - `app_settings.py`: Centralized constants and QSettings-based configuration
  - `image_pipeline.py`: Orchestrates image loading and caching
  - `image_file_ops.py`: Single source of truth for all file operations (move/rename/delete)
  - `pyexiv2_wrapper.py`: Abstraction layer for all EXIF/XMP operations
  - `similarity_engine.py`: ML-based image similarity and clustering
  - `caching/`: Thumbnail, preview, EXIF, and rating caches

- **`src/ui/`**: UI layer
  - `main_window.py`: Thin view layer, minimal logic
  - `app_controller.py`: Central mediator between UI and core
  - `app_state.py`: Application runtime state
  - `worker_manager.py`: Background thread orchestration
  - `controllers/`: Specialized sub-controllers (navigation, hotkeys, similarity, deletion, etc.)

- **`src/workers/`**: Background worker threads (rating loader, rotation, AI rating)

### Key Patterns

**Controller Extraction**: Non-trivial behaviors are extracted from MainWindow into focused controllers under `src/ui/controllers/`. Each controller defines a minimal Protocol for type safety and loose coupling.

**Lazy Loading**: Heavy dependencies (torch, onnxruntime) are imported only on first use via `_ensure_session_loaded()` patterns. Never use environment flags to skip imports.

**Worker Pattern**: Long-running tasks use `WorkerManager` with Qt signals. Workers inherit from QObject. Use `calculate_max_workers()` from app_settings for parallel processing.

## Critical Conventions

### PyExiv2 Usage
All PyExiv2 operations MUST go through `src/core/pyexiv2_wrapper.py`:
```python
# Correct
from core.pyexiv2_wrapper import PyExiv2Operations, safe_pyexiv2_image

# Never do this
import pyexiv2  # Causes Windows DLL conflicts
```

### Test Files Must Import PyExiv2 First
Every test file must start with:
```python
import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash
```

### File Operations
All file system operations (move, rename, delete) MUST use `ImageFileOperations` class in `src/core/image_file_ops.py`.

### Configuration
All hardcoded values, thresholds, cache sizes, AI parameters go in `src/core/app_settings.py`. Never use magic numbers in code.

### Logging
Use `logging` module exclusively. Never use `print()`.

### RAW Processing
RAW files are auto-detected by extension via `is_raw_extension()` and receive automatic brightness adjustment (`RAW_AUTO_EDIT_BRIGHTNESS_STANDARD = 1.15`).

## Testing Notes

- Coverage threshold: 8% minimum (enforced in pytest.ini)
- Use `block=False` for dialogs in tests to avoid modal blocking
- Skip tests gracefully when assets/GPU libs are missing
- Test selection helpers with synthetic path lists, not GUI integration
