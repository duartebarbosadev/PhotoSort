# PhotoSort Developer Guide

This guide provides a high-level overview of the PhotoSort application architecture, development workflow, and coding conventions.

## 1. Project Structure

The application is structured into two main packages: `core` and `ui`.

- **`src/`**: The root source directory.
  - **`main.py`**: The application entry point. Handles application setup, command-line argument parsing, and instantiates the `MainWindow`.
  - **`core/`**: Contains the application's business logic, independent of the UI.
    - **`app_settings.py`**: Manages persistent application settings using `QSettings`. All settings-related logic (getting, setting, defaults) should be here.
    - **`caching/`**: Caching mechanisms for thumbnails, previews, ratings, and EXIF data. To add a new cache, create a new class in this directory following the existing examples. The rating cache is cleared alongside the EXIF cache.
    - **`image_features/`**: Image analysis features like blur detection. New features that analyze image properties should be added here.
      - **`model_rotation_detector.py`**: Implements the deep learning model (ONNX) for detecting image orientation.
      - **`rotation_detector.py`**: Orchestrates the model-based rotation detection for batches of images.
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
    - **`dialog_manager.py`**: Manages the creation and display of all dialog boxes.
    - **`menu_manager.py`**: Manages the main menu bar and its actions.
    - **`left_panel.py`**, **`metadata_sidebar.py`**, **`advanced_image_viewer.py`**: Reusable UI components.

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

- **Logging**: All logging should be done using the `logging` module. Use `logging.debug()` for detailed development information and `logging.info()` for general application flow. Errors should be logged with `logging.error()` or `logging.warning()`. **Do not use `print()` statements.**
- **Code Comments**: Write meaningful comments that explain the intent or the "why" behind a piece of code, especially for complex algorithms or non-obvious design choices. The code itself should explain the "how". Avoid comments that merely restate what the code does.
- **Separation of Concerns**: Keep UI logic separate from business logic. The `core` package should not depend on the `ui` package. The `ui` package, specifically `MainWindow`, should be as "dumb" as possible, delegating all logic to the `AppController`.
- **File Operations**: All file system operations (move, rename, delete) MUST be handled by the `ImageFileOperations` class in `src/core/image_file_ops.py`. This ensures that file manipulations are centralized and handled consistently.
- **Threading**: All long-running tasks MUST be executed in a background thread using the `WorkerManager`. This ensures the UI remains responsive. Workers should communicate with the main thread via Qt signals.
- **State Management**: The application's state (e.g., list of loaded images, cache data) is managed by the `AppState` class. Avoid storing state directly in the `MainWindow` or other UI components.
- **Styling**: All UI components should be styled using stylesheets. The application uses a dark theme defined in `src/ui/dark_theme.qss`. When creating new UI components or modifying existing ones, ensure that the styling is consistent with this theme.
