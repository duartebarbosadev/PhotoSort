# PhotoSort Developer Guide

This guide provides a high-level overview of the PhotoSort application architecture, development workflow, and coding conventions.

## 1. Project Structure

The application is structured into two main packages: `core` and `ui`.

- **`src/`**: The root source directory.
  - **`main.py`**: The application entry point. Handles application setup, command-line argument parsing, and instantiates the `MainWindow`.
  - **`core/`**: Contains the application's business logic, independent of the UI.
    - **`app_settings.py`**: Manages persistent application settings using `QSettings` and centralizes all configurable constants. All settings-related logic (getting, setting, defaults) and hardcoded values should be here.
    - **`caching/`**: Caching mechanisms for thumbnails, previews, ratings, and EXIF data. To add a new cache, create a new class in this directory following the existing examples. The rating cache is cleared alongside the EXIF cache.
    - **`image_features/`**: Image analysis features like blur detection. New features that analyze image properties should be added here.
  - **`model_rotation_detector.py`**: Lazy-loading ONNX orientation detector. Heavy dependencies (onnxruntime / torchvision / Pillow) are imported only on first prediction request. Never gate imports with environment variables—extend the lazy loader if further deferral is needed.
  - **`rotation_detector.py`**: Orchestrates batch rotation detection using the model rotation detector (instantiated lazily on demand).
    - **`image_processing/`**: Low-level image manipulation, such as RAW processing and rotation.
      - **`image_orientation_handler.py`**: Handles EXIF-based image orientation correction and composite rotation calculations.
    - **`file_scanner.py`**: Scans directories for image files.
    - **`image_file_ops.py`**: Handles all file system operations, such as moving, renaming, and deleting files. This is the single source of truth for file manipulations.
    - **`image_pipeline.py`**: Orchestrates image processing, caching, and retrieval.
    - **`metadata_processor.py`**: Handles reading and writing image metadata.
    - **`rating_loader_worker.py`**: A worker dedicated to loading image ratings and metadata.
    - **`similarity_engine.py`**: Handles image feature extraction and clustering.
  - **`ui/`**: Contains all UI-related components, following the Model-View-Controller (MVC) pattern.
    - **`main_window.py`**: The main application window (the "View"). It should contain minimal business logic and delegate user actions to the `AppController`.
    - **`app_controller.py`**: The controller that mediates between the UI and the `core` logic. It handles user actions, calls the appropriate `core` services, and updates the UI.
    - **`app_state.py`**: Holds the application's runtime state, including caches and loaded data. This object is shared across the application.
    - **`worker_manager.py`**: Manages all background threads and workers, decoupling the UI from long-running tasks.
    - **Controller Layer (Encapsulation Refactor)**: Non-trivial UI behaviors previously embedded in `MainWindow` have been extracted into focused controllers under `src/ui/controllers/`:
        - `navigation_controller.py`: Linear & group-aware navigation (honors skip-deleted logic, smart up/down traversal). Consumes a minimal protocol for selection & model interrogation.
        - `hotkey_controller.py`: Central mapping of key events to actions (allows headless tests to exercise hotkey dispatch logic).
        - `similarity_controller.py`: Orchestrates similarity analysis workflow (start, embeddings, clustering) AND (post-refactor) prepares cluster structures via `prepare_clusters()` returning pure data used by the view. Sorting strategies (Default / Time / Similarity then Time) live here; PCA fallback rationale documented inline. Uses only a protocol subset of `AppState` (see `AppStateSimilarityView`) for loose coupling.
        - `preview_controller.py`: Handles preview image loading / refresh separation from navigation triggers.
        - `metadata_controller.py`: Updates metadata sidebar without polluting MainWindow event handlers.
        - `filter_controller.py`: Applies user-entered filter text / rating filters to proxy model.
        - `selection_controller.py`: Shared selection operations & multi-select semantics.
        - `deletion_mark_controller.py`: Non-destructive mark/unmark & presentation (text color/blur). Distinguished from actual deletion for clarity & test isolation.
        - `file_deletion_controller.py`: Destructive operations (move to trash), reverse-order removal, prunes empty headers, restores deterministic selection.

      Extension Pattern:
        1. Identify a cohesive behavior cluster in `MainWindow` (heuristics, branching logic, side-effect orchestration).
        2. Define a minimal Protocol capturing only what the controller needs (attributes + methods). Avoid passing full MainWindow if possible.
        3. Implement controller with pure helpers; return data structures instead of mutating widgets directly where feasible.
        4. Add targeted tests for new controller focusing on logic not easily covered via GUI.
        5. Replace in-place logic with controller delegation. Remove obsolete helpers after tests pass.
        6. Document rationale (fallbacks, sentinels like `date_obj.max`) inline for future maintainers.

      Benefits:
        - Smaller `main_window.py` surface area.
        - Faster unit tests (controllers testable without QApplication event loop).
        - Clear separation of destructive vs non-destructive operations (mark vs delete).
        - Easier future feature toggles or re-use in alternate front-ends.
  - **`dialog_manager.py`**: Manages the creation and display of all dialog boxes. `show_about_dialog(block: bool = True)` supports a non-blocking mode (`block=False`) used by tests; prefer blocking mode in production UI code.
    - **`menu_manager.py`**: Manages the main menu bar and its actions.
    - **`left_panel.py`**, **`metadata_sidebar.py`**, **`advanced_image_viewer.py`**: Reusable UI components.
  - **`selection_utils.py`**: Pure selection/navigation heuristics (e.g., `select_next_surviving_path`) used after rotations, deletions, or filtering to deterministically choose the next image without embedding logic in widgets. Fully unit-tested; extend here when changing advancement behavior.

## 2. Development Workflow

### Adding a New Feature

1. **Identify the right place:**

   - If the feature is a core logic change (e.g., a new image analysis technique), it should be implemented in the `src/core` directory.
   - If it's a new UI component, it should be in `src/ui`.
   - If it's a new background task, a new worker should be added and managed by `src/ui/worker_manager.py`.
2. **Create new files when necessary:**

   - Create a new file for a new class or a distinct set of functionalities. For example, a new image analysis feature like "sharpness detection" would warrant a new file `src/core/image_features/sharpness_detector.py`.
   - For smaller, related functions, you can add them to an existing relevant file.
3. **Integrating the feature:**

   - The `AppController` is the primary point of integration. User actions from the UI (e.g., a button click in `MainWindow`) should call a method in the `AppController`.
   - The `AppController` then calls the relevant service in the `core` package. For any file system operations, the `AppController` must call the appropriate method in `ImageFileOperations`.
   - If the feature involves a long-running task, the `AppController` should use the `WorkerManager` to run it in the background. The `WorkerManager` will then emit signals with the results, which the `AppController` will catch to update the `AppState` and the UI.
4. At the end, update this document if necessary.
5. **Shortcuts**: All user-facing features must have accessible keyboard shortcuts. When introducing a new feature or command, add a QAction/QShortcut with ApplicationShortcut context, and document it in the Keyboard Shortcuts section of the [README](README.md).
6. **Lazy Loading Over Env Flags**: If a feature adds a heavyweight dependency, implement lazy initialization (move imports into an `_ensure_loaded()` or similar) instead of adding environment variable conditionals.


### Example: Adding a "Detect Duplicates" Feature

1. **Core Logic**: Create a `src/core/image_features/duplicate_detector.py` file. This file would contain a `DuplicateDetector` class with a method like `find_duplicates(image_paths)`. If this feature needs to move or delete files, it should call the methods in `ImageFileOperations`.
2. **Worker**: In `src/ui/worker_manager.py`, create a new worker `DuplicateDetectionWorker` that calls `DuplicateDetector.find_duplicates()` in a separate thread.
3. **Integration**:
   - Add a "Detect Duplicates" `QAction` in `src/ui/menu_manager.py`.
   - Connect this action to a new slot in `AppController`, e.g., `start_duplicate_detection`.
   - The `start_duplicate_detection` method in `AppController` would call a new method in `WorkerManager`, e.g., `worker_manager.start_duplicate_detection(...)`.
   - The `WorkerManager` would emit signals with the results (e.g., `duplicate_detection_finished(duplicate_sets)`).
   - The `AppController` would have a slot `handle_duplicate_detection_finished(duplicate_sets)` to receive the results.
   - Finally, the `AppController` would update the `AppState` and call a method in `MainWindow` to display the results to the user.

## 3. Coding Conventions

- **Configuration Constants**: All hardcoded values, thresholds, and configurable parameters must be centralized in `src/core/app_settings.py`. This includes UI dimensions, processing thresholds, cache sizes, AI/ML parameters, and other constants. Never use hardcoded values directly in the code - always import from `app_settings.py`.

- **Logging**: Use Python's `logging` module for all logging. **Do not use `print()`**.

  - **Style**: Keep messages concise, human-readable, and provide context.
    - **Good**: `logger.info(f"Initializing EXIF cache: {cache_dir} (Size Limit: {self._size_limit_mb} MB)")`
    - **Bad**: `ExifCache.__init__ - Start, dir: {cache_dir}, configured size_limit: {self._size_limit_mb} MB`

  - **Log Levels**:
    - `debug()`: For detailed debugging (variable states, function calls).
    - `info()`: For major application events (startup, folder loaded).
    - `warning()`: For non-critical issues.
    - `error()`: For errors preventing an operation. Include `exc_info=True` for exceptions.
- **Code Comments**: Write meaningful comments that explain the intent or the "why" behind a piece of code, especially for complex algorithms or non-obvious design choices. The code itself should explain the "how". Avoid comments that merely restate what the code does.
- **Separation of Concerns**: Keep UI logic separate from business logic. The `core` package should not depend on the `ui` package. The `ui` package, specifically `MainWindow`, should be as "dumb" as possible, delegating all logic to the `AppController`.
- **File Operations**: All file system operations (move, rename, delete) MUST be handled by the `ImageFileOperations` class in `src/core/image_file_ops.py`. This ensures that file manipulations are centralized and handled consistently.
- **Threading**: All long-running tasks MUST be executed in a background thread using the `WorkerManager`. This ensures the UI remains responsive. Workers should communicate with the main thread via Qt signals.
- **State Management**: The application's state (e.g., list of loaded images, cache data) is managed by the `AppState` class. Avoid storing state directly in the `MainWindow` or other UI components.
- **Styling**: All UI components should be styled using stylesheets. The application uses a dark theme defined in `src/ui/dark_theme.qss`. When creating new UI components or modifying existing ones, ensure that the styling is consistent with this theme.

## 4. Rotation Suggestion Acceptance & Auto-Advance

Rotation acceptance supports:
- Single-item accept with automatic advance to the next surviving visible image.
- Multi-selection accept (removes all selected suggestions and chooses the next logical item).
- Accept-all flow (safe iteration avoids mutating the dict during traversal).

Selection advancement uses `select_next_surviving_path(visible_paths_before, deleted_paths, anchor_path_before, visible_paths_after)` in `src/ui/selection_utils.py`.

When modifying rotation view behavior:
1. Always capture `visible_paths_before` before mutating state.
2. Rebuild the view, then recompute `visible_paths_after`.
3. Use the helper to pick the next path; fall back gracefully (clear selection if none).

Avoid embedding complex selection heuristics directly inside UI methods—extend the helper and add tests instead.

### 4.1 Selection Utilities (`selection_utils.py`)

`select_next_surviving_path` centralizes the heuristic for choosing the next item after removals (rotations accepted, deletions, filtering). Design goals:
- Deterministic and local: prefers nearby surviving items to avoid unexpected jumps.
- Resilient to edge cases: empty before/after lists, anchor missing, all removed, non-contiguous multi-removals.
- Pure & testable: no side effects, operates only on provided lists.

Heuristic summary:
1. Keep current (anchor) if it still exists.
2. Else anchor to (a) original anchor index if present, (b) first removed path’s prior index, (c) a longest-common-prefix proximity guess, else midpoint.
3. Scan forward for first surviving candidate.
4. If none, scan backward.
5. Fallback: last remaining visible item.

When modifying this function:
- Maintain O(n) complexity (no nested scans over large lists beyond single passes).
- Add/adjust tests in `test_selection_logic.py`, `test_selection_logic_edges.py`, and `test_selection_logic_perf.py`.
- Do not special-case UI states; supply any additional metadata through parameters instead of coupling to widgets.

## 5. Lazy Model Loading Pattern

`model_rotation_detector.py` implements a singleton with an internal `_LazyState`:
- No heavy imports at module import time.
- `_ensure_session_loaded()` performs one-time guarded initialization.
- Failures (missing deps/model) leave the detector in a disabled state returning `0` (no rotation) rather than raising.

To add new ML-oriented features, follow this pattern:
1. Define a lightweight facade class (singleton optional).
2. Keep state in a small dataclass `_LazyState` / `_State`.
3. Place heavy imports inside the guarded loader.
4. Expose pure, side-effect-free methods that short-circuit if not initialized.

Do NOT introduce environment flag conditionals to skip imports—prefer structural lazy loading.

## 6. Testing Strategy

Current layers:
- Unit tests for selection logic (`test_selection_logic*`).
- Edge & performance tests exercise large path lists to verify O(n) behavior and resilience.
- Integration tests for rotation acceptance (skipped automatically if `sample/` assets absent or GUI constraints unmet).
- Lazy loader tests ensure model instantiation is deferred and disabled mode returns 0.
- About dialog test runs with `block=False` to avoid modal blocking.

Guidelines for new tests:
1. Favor pure function extraction for logic heavy UI code to enable headless unit tests.
2. Use non-blocking dialog patterns (`block=False`) where modal exec would hang CI.
3. Skip GUI-heavy tests gracefully when prerequisites (sample assets, GPU libs) are missing instead of failing.
4. When asserting selection advancement, test the helper function directly with synthetic path lists—only one integration test should cover the end-to-end GUI path.


## 7. Adding New Image Feature Pipelines

For a new feature (e.g., "sharpness heatmap"):
1. Add module in `core/image_features/` with lazy pattern if heavy deps.
2. Provide a batch orchestrator if needed (mirroring `rotation_detector.py`).
3. Extend `WorkerManager` for background processing.
4. Add progress + completion signals; handle them in `AppController` to update `AppState`.
5. Surface UI affordances (menu action, sidebar toggle) in `menu_manager` / `MainWindow`.
6. Add tests: one logic unit test + one optional integration (skip if assets missing).

## 8. Updating This Guide

When you:
- Introduce a new architectural pattern (e.g., another lazy subsystem).
- Deprecate or rename a helper (ensure alias removal after migration and note here if pattern is broadly used).
- Add core selection/navigation heuristics.

Update the relevant sections instead of appending ad hoc notes at the bottom.

### 8.1 Recent Refactor Summary (Encapsulation Phase)

Refactors completed:
  - Extracted navigation & hotkey handling (legacy behaviors restored: Ctrl includes deleted, cyclic horizontal navigation, smart vertical grouping).
  - Split deletion responsibilities: presentation/marking vs filesystem deletion.
  - Moved clustering grouping & sorting to `SimilarityController.prepare_clusters()`; `MainWindow` now only renders structure.
  - Introduced protocol-driven design (e.g., `AppStateSimilarityView`) for type clarity and decoupling.
  - Added deterministic cluster sorting strategies with PCA + timestamp fallback (see docstring in `ClusterUtils.sort_clusters_by_similarity_time`).

When adding new controllers, follow the listed Extension Pattern to maintain consistency.

## 9. Common Pitfalls

| Scenario | Recommended Action |
|----------|--------------------|
| Need to bypass heavy model in tests | Rely on lazy loader / stub state injection, not env vars |
| Rotation view crashes due to empty model | Guard with early returns; tests should skip rather than fail |
| Adding file operations outside `ImageFileOperations` | Refactor into `ImageFileOperations` to centralize side effects |
| Blocking modal dialogs in CI | Use `block=False` in tests, keep blocking in production UI |

---
This guide reflects the state after introducing auto-advance rotation acceptance, selection heuristic refactor, lazy model loading, and updated testing patterns.
