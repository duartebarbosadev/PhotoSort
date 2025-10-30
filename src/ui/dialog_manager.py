from typing import List, Tuple, Set, Optional
import webbrowser
import os
import logging
import time

from PyQt6.QtWidgets import (
    QDialog,
    QVBoxLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QCheckBox,
    QMessageBox,
    QFrame,
    QGridLayout,
    QSpacerItem,
    QComboBox,
    QSizePolicy,
    QScrollArea,
    QListWidget,
    QListWidgetItem,
    QStyle,
    QRadioButton,
    QSlider,
    QLineEdit,
    QPlainTextEdit,
    QSpinBox,
)
from PyQt6.QtCore import Qt, QSize, QUrl, QEventLoop, QThread
from PyQt6.QtGui import QIcon, QDesktopServices

from core.app_settings import (
    get_rotation_confirm_lossy,
    get_preview_cache_size_gb,
    get_exif_cache_size_mb,
)
from core.image_processing.raw_image_processor import is_raw_extension
from core.image_features.model_rotation_detector import (
    ModelRotationDetector,
    ModelNotFoundError,
)
from workers.thumbnail_preload_worker import ThumbnailPreloadWorker

logger = logging.getLogger(__name__)


class DialogManager:
    """A manager class for handling the creation of dialogs."""

    THUMBNAIL_PRELOAD_ASYNC_THRESHOLD = 20

    def __init__(self, parent):
        """
        Initialize the DialogManager.

        Args:
            parent: The parent widget, typically the MainWindow.
        """
        self.parent = parent
        # Instance-level placeholder for non-blocking About dialog reference
        self._about_dialog_ref = None

    def _should_apply_raw_processing(self, file_path: str) -> bool:
        """Determine if RAW processing should be applied to the given file."""
        if not file_path:
            return False
        ext = os.path.splitext(file_path)[1].lower()
        return is_raw_extension(ext)

    def _has_raw_images(self, file_paths: List[str]) -> bool:
        """Check if any of the provided file paths are RAW image files."""
        for path in file_paths:
            if self._should_apply_raw_processing(path):
                return True
        return False

    def show_about_dialog(self, block: bool = True):
        """Show the 'About' dialog.

        Args:
            block: If True (default) runs dialog.exec() modally. If False, uses
                   dialog.show() and returns immediately (useful for tests).
        """
        logger.info("Showing about dialog")
        dialog = QDialog(self.parent)
        dialog.setWindowTitle("About PhotoSort")
        dialog.setObjectName("aboutDialog")
        dialog.setModal(True)
        dialog.setFixedSize(480, 420)
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        # Make window frameless for a cleaner UI
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)

        # Main layout
        main_layout = QVBoxLayout(dialog)
        main_layout.setSpacing(15)
        main_layout.setContentsMargins(25, 25, 25, 25)

        # Compact header section
        header_frame = QFrame()
        header_frame.setObjectName("aboutHeader")
        header_layout = QHBoxLayout(header_frame)
        header_layout.setSpacing(15)
        header_layout.setContentsMargins(20, 15, 20, 15)

        # Enable dragging the frameless window by the header
        _drag_state = {"offset": None}

        def _header_mouse_press(e):
            if e.button() == Qt.MouseButton.LeftButton:
                _drag_state["offset"] = (
                    e.globalPosition().toPoint() - dialog.frameGeometry().topLeft()
                )
                e.accept()

        def _header_mouse_move(e):
            if (e.buttons() & Qt.MouseButton.LeftButton) and _drag_state[
                "offset"
            ] is not None:
                dialog.move(e.globalPosition().toPoint() - _drag_state["offset"])
                e.accept()

        # Assign simple drag handlers to the header frame
        header_frame.mousePressEvent = _header_mouse_press  # type: ignore[assignment]
        header_frame.mouseMoveEvent = _header_mouse_move  # type: ignore[assignment]

        # App info (left side)
        app_info_layout = QVBoxLayout()
        app_info_layout.setSpacing(3)

        title_label = QLabel("PhotoSort")
        title_label.setObjectName("aboutTitle")
        app_info_layout.addWidget(title_label)

        # Version label (populated only in packaged builds)
        version_text = None
        try:
            # Populated by CI during packaged builds in core/build_info.py
            from core.build_info import VERSION  # type: ignore

            version_text = str(VERSION).strip() or None
        except (ImportError, AttributeError):
            version_text = None

        version_label = QLabel()
        version_label.setObjectName("aboutVersion")
        if version_text:
            version_label.setText(f"Version {version_text}")
            version_label.setVisible(True)
        else:
            # In local dev runs (python src/main.py), no version is shown
            version_label.setVisible(False)
        app_info_layout.addWidget(version_label)

        header_layout.addLayout(app_info_layout)
        header_layout.addStretch()

        main_layout.addWidget(header_frame)

        # Content section
        content_layout = QVBoxLayout()
        content_layout.setSpacing(15)

        # Technology section
        tech_title = QLabel("Technology Stack")
        tech_title.setObjectName("aboutSectionTitle")
        content_layout.addWidget(tech_title)

        # Tech details in a more compact grid
        tech_frame = QFrame()
        tech_frame.setObjectName("aboutTechFrame")
        tech_layout = QVBoxLayout(tech_frame)
        tech_layout.setSpacing(6)
        tech_layout.setContentsMargins(15, 10, 15, 10)

        clustering_info = "Clustering Algorithm: DBSCAN (scikit-learn)"
        # Get ONNX provider information
        try:
            model_detector = ModelRotationDetector()
            # Lazy detector exposes provider via internal state after load attempt
            onnx_provider = (
                getattr(model_detector._state, "provider_name", None)
                or "N/A (model not loaded)"
            )
        except ModelNotFoundError:
            onnx_provider = "N/A (model not found)"
        except Exception:
            onnx_provider = "N/A (error)"

        embeddings_label_ref = None
        tech_items = [
            "🧠 Embeddings: SentenceTransformer (CLIP)",
            f"🤖 Rotation Model: ONNX Runtime on {onnx_provider}",
            f"🔍 {clustering_info}",
            "📋 Metadata: pyexiv2 • 🎨 Interface: PyQt6 • 🐍 Runtime: Python",
        ]

        for i, item in enumerate(tech_items):
            item_label = QLabel(item)
            item_label.setObjectName("aboutTechItem")
            item_label.setWordWrap(True)
            tech_layout.addWidget(item_label)
            if i == 0:  # Embeddings item
                embeddings_label_ref = item_label

        content_layout.addWidget(tech_frame)

        # Actions row: Open Models + Logs + GitHub
        actions_layout = QHBoxLayout()
        actions_layout.addStretch()

        # Open Models Folder button
        models_button = QPushButton("📦 Open Models Folder")
        models_button.setObjectName("aboutModelsButton")
        models_button.setToolTip(
            "Open the folder where ONNX rotation models are stored"
        )
        models_button.clicked.connect(self._open_models_folder)
        actions_layout.addWidget(models_button)

        # Open Logs Folder button
        logs_button = QPushButton("🗂️ Open Logs Folder")
        logs_button.setObjectName("aboutLogsButton")
        logs_button.setToolTip("Open the folder where PhotoSort writes its log file")
        logs_button.clicked.connect(self._open_logs_folder)
        actions_layout.addWidget(logs_button)

        # GitHub button
        github_button = QPushButton("🔗 View on GitHub")
        github_button.setObjectName("aboutGithubButton")
        github_button.clicked.connect(
            lambda: webbrowser.open("https://github.com/duartebarbosadev/PhotoSort")
        )
        actions_layout.addWidget(github_button)

        content_layout.addLayout(actions_layout)

        # Add content to main layout
        main_layout.addLayout(content_layout)

        # Spacer
        main_layout.addSpacerItem(
            QSpacerItem(
                20, 20, QSizePolicy.Policy.Minimum, QSizePolicy.Policy.Expanding
            )
        )

        # Footer with close button
        footer_layout = QHBoxLayout()
        footer_layout.addStretch()

        close_button = QPushButton("Close")
        close_button.setObjectName("aboutCloseButton")
        close_button.clicked.connect(dialog.accept)
        close_button.setDefault(True)
        footer_layout.addWidget(close_button)

        main_layout.addLayout(footer_layout)

        # Styling is handled by dark_theme.qss

        # Start CUDA detection worker
        worker_manager = self.parent.app_controller.worker_manager
        if embeddings_label_ref:

            def update_embeddings_label(available):
                try:
                    if embeddings_label_ref:
                        embeddings_label_ref.setText(
                            f"🧠 Embeddings: SentenceTransformer (CLIP) on {'GPU (CUDA)' if available else 'CPU'}"
                        )
                except RuntimeError:
                    pass  # Label has been deleted

            worker_manager.cuda_detection_finished.connect(update_embeddings_label)
            worker_manager.start_cuda_detection()

        if block:
            dialog.exec()
            logger.info("Closed about dialog")
        else:  # non-blocking path for automated tests
            # Keep a reference to prevent garbage collection in tests
            self._about_dialog_ref = dialog  # type: ignore[attr-defined]
            dialog.show()
            logger.info("Showing about dialog (non-blocking mode)")

    def _open_logs_folder(self):
        """Open the application's logs directory in the system file browser."""
        try:
            logs_dir = os.path.join(os.path.expanduser("~"), ".photosort_logs")
            os.makedirs(logs_dir, exist_ok=True)
            url = QUrl.fromLocalFile(logs_dir)
            opened = QDesktopServices.openUrl(url)
            if not opened:
                logger.warning(
                    "QDesktopServices failed to open logs folder: %s", logs_dir
                )
        except Exception:
            logger.error("Failed to open logs folder", exc_info=True)

    def _open_models_folder(self):
        """Open the models directory (where ONNX model files live) in the system file browser.

        Strategy:
        - Prefer ./models next to the running app (CWD/models), creating it if missing.
        - Fallback to project-root/models (useful in dev), creating if needed.
        """
        try:
            cwd_models = os.path.join(os.getcwd(), "models")
            project_root = os.path.abspath(
                os.path.join(os.path.dirname(__file__), "..", "..")
            )
            dev_models = os.path.join(project_root, "models")

            # Choose the best target: existing CWD/models, else existing dev models, else create CWD/models
            target = (
                cwd_models
                if os.path.isdir(cwd_models)
                else (dev_models if os.path.isdir(dev_models) else cwd_models)
            )
            os.makedirs(target, exist_ok=True)
            url = QUrl.fromLocalFile(os.path.abspath(target))
            opened = QDesktopServices.openUrl(url)
            if not opened:
                logger.warning(
                    "QDesktopServices failed to open models folder: %s", target
                )
        except Exception:
            logger.error("Failed to open models folder", exc_info=True)

    def show_lossy_rotation_confirmation_dialog(
        self, filename: str, rotation_type: str
    ) -> Tuple[bool, bool]:
        """
        Show a confirmation dialog for lossy rotation with a 'never ask again' option.

        Args:
            filename: The name of the file being rotated.
            rotation_type: A description of the rotation (e.g., "90° clockwise").

        Returns:
            A tuple containing (proceed_with_rotation: bool, never_ask_again: bool).
        """
        logger.info(f"Showing lossy rotation confirmation dialog for {filename}")

        if not get_rotation_confirm_lossy():
            logger.info(
                "Lossy rotation confirmation disabled, proceeding without asking"
            )
            return True, False  # Proceed without asking if the setting is disabled

        dialog = QDialog(self.parent)
        dialog.setWindowTitle("Confirm Lossy Rotation")
        dialog.setObjectName("lossyRotationDialog")
        dialog.setModal(True)
        dialog.setFixedSize(480, 200)
        # Frameless window for fancy UI
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)

        layout = QVBoxLayout(dialog)
        layout.setSpacing(20)
        layout.setContentsMargins(25, 25, 25, 25)

        message_text = f"Lossless rotation failed for:\n{filename}"
        warning_text = f"Proceed with lossy rotation {rotation_type}?\nThis will re-encode the image and may reduce quality."

        if "images" in filename.lower():  # Batch operation
            warning_text = f"Proceed with lossy rotation {rotation_type} for all selected images?\nThis will re-encode the images and may reduce quality."

        message_label = QLabel(message_text)
        message_label.setObjectName("lossyRotationMessageLabel")
        message_label.setWordWrap(True)
        layout.addWidget(message_label)

        warning_label = QLabel(warning_text)
        warning_label.setObjectName("lossyRotationWarningLabel")
        warning_label.setWordWrap(True)
        layout.addWidget(warning_label)

        never_ask_checkbox = QCheckBox("Don't ask again for lossy rotations")
        never_ask_checkbox.setObjectName("neverAskAgainCheckbox")
        layout.addWidget(never_ask_checkbox)

        button_layout = QHBoxLayout()
        button_layout.addStretch()

        cancel_button = QPushButton("Cancel")
        cancel_button.setObjectName("lossyRotationCancelButton")
        cancel_button.clicked.connect(dialog.reject)
        button_layout.addWidget(cancel_button)

        proceed_button = QPushButton("Proceed with Lossy Rotation")
        proceed_button.setObjectName("lossyRotationProceedButton")
        proceed_button.clicked.connect(dialog.accept)
        proceed_button.setDefault(True)
        button_layout.addWidget(proceed_button)

        layout.addLayout(button_layout)

        # Styling is handled by dark_theme.qss

        result = dialog.exec()
        proceed = result == QDialog.DialogCode.Accepted
        never_ask_again = never_ask_checkbox.isChecked()

        logger.info(
            f"User {'proceeded' if proceed else 'cancelled'} lossy rotation dialog, "
            f"never ask again: {never_ask_again}"
        )

        return proceed, never_ask_again

    def show_preferences_dialog(self):
        """Show the application preferences dialog."""
        from core.app_settings import (
            PerformanceMode,
            get_performance_mode,
            set_performance_mode,
            get_custom_thread_count,
            set_custom_thread_count,
            get_best_shot_engine,
            set_best_shot_engine,
            get_openai_config,
            set_openai_config,
            DEFAULT_OPENAI_API_KEY,
            DEFAULT_OPENAI_MODEL,
            DEFAULT_OPENAI_BASE_URL,
            DEFAULT_OPENAI_MAX_TOKENS,
            DEFAULT_OPENAI_TIMEOUT,
            DEFAULT_OPENAI_MAX_WORKERS,
        )
        from core.ai.best_shot_pipeline import (
            DEFAULT_BEST_SHOT_PROMPT,
            DEFAULT_RATING_PROMPT,
        )

        logger.info("Showing preferences dialog")
        dialog = QDialog(self.parent)
        dialog.setWindowTitle("Preferences")
        dialog.setObjectName("preferencesDialog")
        dialog.setModal(True)
        dialog.resize(640, 600)
        dialog.setMinimumSize(520, 480)
        # Frameless window for consistent UI
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)

        main_layout = QVBoxLayout(dialog)
        main_layout.setSpacing(20)
        main_layout.setContentsMargins(25, 25, 25, 25)

        # Title
        title_label = QLabel("Preferences")
        title_label.setObjectName("aboutTitle")
        main_layout.addWidget(title_label)

        scroll_area = QScrollArea()
        scroll_area.setObjectName("preferencesScrollArea")
        scroll_area.setWidgetResizable(True)
        main_layout.addWidget(scroll_area)

        content_frame = QFrame()
        content_layout = QVBoxLayout(content_frame)
        content_layout.setSpacing(20)
        content_layout.setContentsMargins(0, 0, 0, 0)
        scroll_area.setWidget(content_frame)

        # Performance Mode Section
        perf_section_label = QLabel("Performance Mode")
        perf_section_label.setObjectName("preferencesSectionLabel")
        perf_section_label.setStyleSheet("font-weight: bold; font-size: 13px;")
        content_layout.addWidget(perf_section_label)

        # Description
        desc_label = QLabel(
            "Control how many CPU threads PhotoSort uses for processing:"
        )
        desc_label.setWordWrap(True)
        content_layout.addWidget(desc_label)

        # Radio buttons for performance mode
        balanced_radio = QRadioButton("Balanced (Recommended)")
        balanced_radio.setObjectName("balancedRadio")

        balanced_desc = QLabel("    Uses 85% of CPU cores to keep system responsive")
        balanced_desc.setObjectName("radioDescription")
        balanced_desc.setStyleSheet("color: #888; font-size: 11px; margin-left: 20px;")

        performance_radio = QRadioButton("Performance")
        performance_radio.setObjectName("performanceRadio")

        perf_desc = QLabel("    Uses all available CPU cores for maximum speed")
        perf_desc.setObjectName("radioDescription")
        perf_desc.setStyleSheet("color: #888; font-size: 11px; margin-left: 20px;")

        custom_radio = QRadioButton("Custom")
        custom_radio.setObjectName("customRadio")

        # Custom thread count slider in a horizontal layout
        custom_control_layout = QVBoxLayout()
        custom_control_layout.setContentsMargins(20, 5, 0, 0)
        custom_control_layout.setSpacing(5)

        # Get system CPU count
        max_threads = os.cpu_count() or 4

        # Label showing current value and range
        current_thread_count = min(get_custom_thread_count(), max_threads)
        thread_count_label = QLabel(
            f"Thread count: {current_thread_count} (max: {max_threads})"
        )
        thread_count_label.setObjectName("threadCountLabel")
        thread_count_label.setStyleSheet("color: #888; font-size: 11px;")
        custom_control_layout.addWidget(thread_count_label)

        # Slider
        thread_count_slider = QSlider(Qt.Orientation.Horizontal)
        thread_count_slider.setObjectName("threadCountSlider")
        thread_count_slider.setMinimum(1)
        thread_count_slider.setMaximum(max_threads)
        thread_count_slider.setValue(current_thread_count)
        thread_count_slider.setTickPosition(QSlider.TickPosition.TicksBelow)
        # Set tick interval based on max threads (show ~8 ticks)
        tick_interval = max(1, max_threads // 8)
        thread_count_slider.setTickInterval(tick_interval)
        thread_count_slider.setEnabled(False)

        # Update label when slider changes
        def on_slider_changed(value):
            thread_count_label.setText(f"Thread count: {value} (max: {max_threads})")

        thread_count_slider.valueChanged.connect(on_slider_changed)
        custom_control_layout.addWidget(thread_count_slider)

        # Set current mode
        current_mode = get_performance_mode()
        if current_mode == PerformanceMode.BALANCED:
            balanced_radio.setChecked(True)
        elif current_mode == PerformanceMode.PERFORMANCE:
            performance_radio.setChecked(True)
        else:  # CUSTOM
            custom_radio.setChecked(True)
            thread_count_slider.setEnabled(True)

        # Enable/disable slider based on custom radio selection
        def on_custom_toggled(checked):
            thread_count_slider.setEnabled(checked)
            thread_count_label.setEnabled(checked)

        custom_radio.toggled.connect(on_custom_toggled)
        on_custom_toggled(custom_radio.isChecked())

        # Add all radio options to layout
        content_layout.addWidget(balanced_radio)
        content_layout.addWidget(balanced_desc)
        content_layout.addWidget(performance_radio)
        content_layout.addWidget(perf_desc)
        content_layout.addWidget(custom_radio)
        content_layout.addLayout(custom_control_layout)

        # Note
        note_label = QLabel("Note: Changes take effect immediately for new operations.")
        note_label.setWordWrap(True)
        note_label.setStyleSheet(
            "color: #888; font-style: italic; font-size: 11px; margin-top: 10px;"
        )
        content_layout.addWidget(note_label)

        # AI Engine Section
        ai_section_label = QLabel("AI Rating Engine")
        ai_section_label.setObjectName("preferencesSectionLabel")
        ai_section_label.setStyleSheet("font-weight: bold; font-size: 13px;")
        content_layout.addWidget(ai_section_label)

        ai_desc_label = QLabel(
            "Choose between the on-device model and the OpenAI LLM for image ranking and ratings."
        )
        ai_desc_label.setWordWrap(True)
        content_layout.addWidget(ai_desc_label)

        engine_combo = QComboBox()
        engine_combo.setObjectName("bestShotEngineCombo")
        engine_combo.addItem("Local (on-device models)", "local")
        engine_combo.addItem("OpenAI (LLM)", "llm")
        content_layout.addWidget(engine_combo)

        openai_config = get_openai_config()
        api_key_value = openai_config.get("api_key") or DEFAULT_OPENAI_API_KEY
        model_value = openai_config.get("model") or DEFAULT_OPENAI_MODEL
        base_url_value = openai_config.get("base_url") or DEFAULT_OPENAI_BASE_URL
        try:
            max_tokens_value = int(
                openai_config.get("max_tokens") or DEFAULT_OPENAI_MAX_TOKENS
            )
        except (TypeError, ValueError):
            max_tokens_value = DEFAULT_OPENAI_MAX_TOKENS
        try:
            timeout_value = int(
                openai_config.get("timeout") or DEFAULT_OPENAI_TIMEOUT
            )
        except (TypeError, ValueError):
            timeout_value = DEFAULT_OPENAI_TIMEOUT
        try:
            max_workers_value = int(
                openai_config.get("max_workers") or DEFAULT_OPENAI_MAX_WORKERS
            )
        except (TypeError, ValueError):
            max_workers_value = DEFAULT_OPENAI_MAX_WORKERS
        best_prompt_value = (
            openai_config.get("best_shot_prompt") or DEFAULT_BEST_SHOT_PROMPT
        )
        rating_prompt_value = (
            openai_config.get("rating_prompt") or DEFAULT_RATING_PROMPT
        )

        current_engine = (get_best_shot_engine() or "local").lower()
        index = engine_combo.findData(current_engine)
        if index >= 0:
            engine_combo.setCurrentIndex(index)

        openai_frame = QFrame()
        openai_frame.setObjectName("openAISettingsFrame")
        openai_frame.setFrameShape(QFrame.Shape.StyledPanel)
        openai_layout = QVBoxLayout(openai_frame)
        openai_layout.setSpacing(12)
        openai_layout.setContentsMargins(15, 12, 15, 12)

        openai_info_label = QLabel(
            "Configure OpenAI access used by the LLM engine. Leave prompts blank to use defaults."
        )
        openai_info_label.setWordWrap(True)
        openai_info_label.setStyleSheet("color: #888; font-size: 11px;")
        openai_layout.addWidget(openai_info_label)

        openai_form = QGridLayout()
        openai_form.setHorizontalSpacing(12)
        openai_form.setVerticalSpacing(12)

        api_key_label = QLabel("API Key")
        api_key_input = QLineEdit()
        api_key_input.setObjectName("openAIKeyInput")
        api_key_input.setEchoMode(QLineEdit.EchoMode.Password)
        api_key_input.setPlaceholderText("sk-...")
        api_key_input.setClearButtonEnabled(True)
        api_key_input.setText(api_key_value)
        openai_form.addWidget(api_key_label, 0, 0)
        openai_form.addWidget(api_key_input, 0, 1)

        model_label = QLabel("Model")
        model_combo = QComboBox()
        model_combo.setObjectName("openAIModelCombo")
        model_combo.setEditable(True)
        model_combo.setInsertPolicy(QComboBox.InsertPolicy.NoInsert)
        model_combo.setSizePolicy(
            QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Fixed
        )
        if model_value:
            model_combo.addItem(model_value)
            model_combo.setCurrentText(model_value)
        else:
            model_combo.setCurrentText(DEFAULT_OPENAI_MODEL)

        fetch_models_button = QPushButton("Fetch Models")
        fetch_models_button.setObjectName("openAIFetchModelsButton")

        model_row = QHBoxLayout()
        model_row.setContentsMargins(0, 0, 0, 0)
        model_row.setSpacing(6)
        model_row.addWidget(model_combo)
        model_row.addWidget(fetch_models_button)

        openai_form.addWidget(model_label, 1, 0)
        openai_form.addLayout(model_row, 1, 1)

        base_url_label = QLabel("Base URL")
        base_url_input = QLineEdit()
        base_url_input.setObjectName("openAIBaseUrlInput")
        base_url_input.setPlaceholderText(DEFAULT_OPENAI_BASE_URL)
        base_url_input.setClearButtonEnabled(True)
        base_url_input.setText(base_url_value)
        openai_form.addWidget(base_url_label, 2, 0)
        openai_form.addWidget(base_url_input, 2, 1)

        max_tokens_label = QLabel("Max Tokens")
        max_tokens_spin = QSpinBox()
        max_tokens_spin.setObjectName("openAIMaxTokensSpin")
        max_tokens_spin.setRange(64, 32768)
        max_tokens_spin.setSingleStep(64)
        max_tokens_spin.setValue(max_tokens_value)
        openai_form.addWidget(max_tokens_label, 3, 0)
        openai_form.addWidget(max_tokens_spin, 3, 1)

        timeout_label = QLabel("Timeout (s)")
        timeout_spin = QSpinBox()
        timeout_spin.setObjectName("openAITimeoutSpin")
        timeout_spin.setRange(10, 600)
        timeout_spin.setSingleStep(5)
        timeout_spin.setValue(timeout_value)
        openai_form.addWidget(timeout_label, 4, 0)
        openai_form.addWidget(timeout_spin, 4, 1)

        max_workers_label = QLabel("Concurrent Workers")
        max_workers_spin = QSpinBox()
        max_workers_spin.setObjectName("openAIMaxWorkersSpin")
        max_workers_spin.setRange(1, 16)
        max_workers_spin.setValue(max_workers_value)
        openai_form.addWidget(max_workers_label, 5, 0)
        openai_form.addWidget(max_workers_spin, 5, 1)

        best_prompt_label = QLabel("Best Shot Prompt")
        best_prompt_edit = QPlainTextEdit()
        best_prompt_edit.setObjectName("openAIBestPromptEdit")
        best_prompt_edit.setPlaceholderText("Leave blank to use the default best-shot prompt.")
        best_prompt_edit.setPlainText(best_prompt_value)
        best_prompt_edit.setMinimumHeight(80)
        openai_form.addWidget(best_prompt_label, 6, 0, Qt.AlignmentFlag.AlignTop)
        openai_form.addWidget(best_prompt_edit, 6, 1)

        rating_prompt_label = QLabel("Rating Prompt")
        rating_prompt_edit = QPlainTextEdit()
        rating_prompt_edit.setObjectName("openAIRatingPromptEdit")
        rating_prompt_edit.setPlaceholderText("Leave blank to use the default rating prompt.")
        rating_prompt_edit.setPlainText(rating_prompt_value)
        rating_prompt_edit.setMinimumHeight(80)
        openai_form.addWidget(rating_prompt_label, 7, 0, Qt.AlignmentFlag.AlignTop)
        openai_form.addWidget(rating_prompt_edit, 7, 1)

        test_connection_button = QPushButton("Test Connection")
        test_connection_button.setObjectName("openAITestConnectionButton")

        def _resolve_or_default(value: str, default_value: str) -> str:
            stripped = value.strip()
            return stripped or default_value

        def _create_openai_client():
            try:
                from openai import OpenAI  # type: ignore
            except ImportError:
                QMessageBox.warning(
                    dialog,
                    "OpenAI Package Missing",
                    "Install the 'openai' package to test the connection.",
                )
                return None

            try:
                return OpenAI(
                    api_key=_resolve_or_default(
                        api_key_input.text(), DEFAULT_OPENAI_API_KEY
                    ),
                    base_url=_resolve_or_default(
                        base_url_input.text(), DEFAULT_OPENAI_BASE_URL
                    ),
                    timeout=timeout_spin.value(),
                )
            except Exception as exc:  # pragma: no cover - defensive
                QMessageBox.critical(
                    dialog,
                    "Client Creation Failed",
                    f"Unable to create OpenAI client:\n{exc}",
                )
                return None

        def _extract_model_ids(response) -> Set[str]:
            model_ids: Set[str] = set()
            data = getattr(response, "data", None)
            if data is None and isinstance(response, dict):
                data = response.get("data")
            if not data:
                return model_ids
            for entry in data:
                if isinstance(entry, dict):
                    identifier = entry.get("id") or entry.get("name")
                else:
                    identifier = getattr(entry, "id", None) or getattr(
                        entry, "name", None
                    )
                if identifier:
                    model_ids.add(str(identifier))
            return model_ids

        def handle_test_connection():
            client = _create_openai_client()
            if client is None:
                return
            test_connection_button.setEnabled(False)
            fetch_models_button.setEnabled(False)
            try:
                probe_timeout = min(timeout_spin.value(), 30)
                probe_client = (
                    client.with_options(timeout=probe_timeout)
                    if hasattr(client, "with_options")
                    else client
                )

                models_start = time.perf_counter()
                response = probe_client.models.list()
                models_duration = time.perf_counter() - models_start
                model_ids = _extract_model_ids(response)
                test_model = _resolve_or_default(
                    model_combo.currentText(), DEFAULT_OPENAI_MODEL
                )
                completion_duration: Optional[float] = None
                completion_error: Optional[Exception] = None
                try:
                    completion_client = (
                        client.with_options(timeout=probe_timeout)
                        if hasattr(client, "with_options")
                        else client
                    )
                    completion_start = time.perf_counter()
                    completion_client.chat.completions.create(
                        model=test_model,
                        messages=[
                            {
                                "role": "user",
                                "content": "PhotoSort connectivity check.",
                            }
                        ],
                        max_tokens=8,
                    )
                    completion_duration = time.perf_counter() - completion_start
                except Exception as exc:  # pragma: no cover - network dependent
                    completion_error = exc

                if completion_error is None:
                    QMessageBox.information(
                        dialog,
                        "Connection Successful",
                        (
                            f"Models endpoint responded in {models_duration:.2f}s ("
                            f"{len(model_ids)} models).\n"
                            f"Chat completion succeeded in {completion_duration:.2f}s using '{test_model}'."
                        ),
                    )
                else:
                    QMessageBox.warning(
                        dialog,
                        "Partial Success",
                        (
                            f"Models endpoint responded in {models_duration:.2f}s ("
                            f"{len(model_ids)} models).\n"
                            f"Chat completion failed for '{test_model}':\n{completion_error}"
                        ),
                    )
            except Exception as exc:  # pragma: no cover - network dependent
                QMessageBox.critical(
                    dialog,
                    "Connection Failed",
                    f"Connection test failed:\n{exc}",
                )
            finally:
                test_connection_button.setEnabled(True)
                fetch_models_button.setEnabled(True)

        def handle_fetch_models():
            client = _create_openai_client()
            if client is None:
                return
            test_connection_button.setEnabled(False)
            fetch_models_button.setEnabled(False)
            start = time.perf_counter()
            try:
                probe_client = (
                    client.with_options(timeout=min(timeout_spin.value(), 30))
                    if hasattr(client, "with_options")
                    else client
                )
                response = probe_client.models.list()
                duration = time.perf_counter() - start
                model_ids = _extract_model_ids(response)
                if not model_ids:
                    QMessageBox.information(
                        dialog,
                        "No Models Found",
                        "The endpoint is reachable but returned no models.",
                    )
                else:
                    existing_text = model_combo.currentText().strip()
                    sorted_ids = sorted(model_ids)
                    model_combo.blockSignals(True)
                    model_combo.clear()
                    for identifier in sorted_ids:
                        model_combo.addItem(identifier)
                    if existing_text and existing_text in model_ids:
                        model_combo.setCurrentText(existing_text)
                    else:
                        model_combo.setCurrentText(sorted_ids[0])
                        if existing_text and existing_text not in model_ids:
                            model_combo.insertItem(0, existing_text)
                            model_combo.setCurrentIndex(0)
                    model_combo.blockSignals(False)
                    QMessageBox.information(
                        dialog,
                        "Models Retrieved",
                        (
                            f"Loaded {len(model_ids)} models in {duration:.2f}s.\n"
                            "You can pick one from the dropdown."
                        ),
                    )
            except Exception as exc:  # pragma: no cover - network dependent
                QMessageBox.critical(
                    dialog,
                    "Fetch Models Failed",
                    f"Failed to fetch models:\n{exc}",
                )
            finally:
                fetch_models_button.setEnabled(True)
                test_connection_button.setEnabled(True)

        fetch_models_button.clicked.connect(handle_fetch_models)
        test_connection_button.clicked.connect(handle_test_connection)

        openai_layout.addLayout(openai_form)
        buttons_row = QHBoxLayout()
        buttons_row.setContentsMargins(0, 0, 0, 0)
        buttons_row.setSpacing(6)
        buttons_row.addWidget(test_connection_button)
        buttons_row.addStretch()
        openai_layout.addLayout(buttons_row)
        content_layout.addWidget(openai_frame)

        def update_openai_visibility():
            openai_frame.setVisible(engine_combo.currentData() == "llm")

        engine_combo.currentIndexChanged.connect(update_openai_visibility)
        update_openai_visibility()

        content_layout.addStretch()

        # Buttons
        button_layout = QHBoxLayout()
        button_layout.addStretch()

        cancel_button = QPushButton("Cancel")
        cancel_button.setObjectName("preferencesCancelButton")
        cancel_button.clicked.connect(dialog.reject)
        button_layout.addWidget(cancel_button)

        save_button = QPushButton("Save")
        save_button.setObjectName("preferencesSaveButton")
        save_button.setDefault(True)

        def save_preferences():
            if balanced_radio.isChecked():
                set_performance_mode(PerformanceMode.BALANCED)
            elif performance_radio.isChecked():
                set_performance_mode(PerformanceMode.PERFORMANCE)
            else:  # custom_radio.isChecked()
                set_performance_mode(PerformanceMode.CUSTOM)
                set_custom_thread_count(thread_count_slider.value())

            set_best_shot_engine(engine_combo.currentData())

            api_key_text = api_key_input.text().strip()
            base_url_text = base_url_input.text().strip()
            model_text = model_combo.currentText().strip()
            max_tokens_value = max_tokens_spin.value()
            timeout_value = timeout_spin.value()
            max_workers_value = max_workers_spin.value()
            best_prompt_text = best_prompt_edit.toPlainText()
            rating_prompt_text = rating_prompt_edit.toPlainText()

            def _value_or_none(value: str, default_value: str) -> Optional[str]:
                trimmed = value.strip()
                if not trimmed or trimmed == default_value:
                    return ""
                return trimmed

            set_openai_config(
                api_key=_value_or_none(api_key_text, DEFAULT_OPENAI_API_KEY),
                model=_value_or_none(model_text, DEFAULT_OPENAI_MODEL),
                base_url=_value_or_none(base_url_text, DEFAULT_OPENAI_BASE_URL),
                max_tokens=None
                if max_tokens_value == DEFAULT_OPENAI_MAX_TOKENS
                else max_tokens_value,
                timeout=None
                if timeout_value == DEFAULT_OPENAI_TIMEOUT
                else timeout_value,
                max_workers=None
                if max_workers_value == DEFAULT_OPENAI_MAX_WORKERS
                else max_workers_value,
                best_shot_prompt=None
                if best_prompt_text.strip() == DEFAULT_BEST_SHOT_PROMPT.strip()
                else best_prompt_text.strip() or None,
                rating_prompt=None
                if rating_prompt_text.strip() == DEFAULT_RATING_PROMPT.strip()
                else rating_prompt_text.strip() or None,
            )

            logger.info(
                "Preferences saved: mode=%s, custom_threads=%s, engine=%s",
                get_performance_mode().value,
                get_custom_thread_count(),
                get_best_shot_engine(),
            )
            dialog.accept()

        save_button.clicked.connect(save_preferences)
        button_layout.addWidget(save_button)

        main_layout.addLayout(button_layout)

        dialog.setLayout(main_layout)
        dialog.exec()
        logger.info("Closed preferences dialog")

    def show_cache_management_dialog(self):
        """Show the cache management dialog."""
        logger.info("Showing cache management dialog")
        dialog = QDialog(self.parent)
        dialog.setWindowTitle("Cache Management")
        dialog.setObjectName("cacheManagementDialog")
        # Frameless window for fancy UI
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)
        main_layout = QVBoxLayout(dialog)
        main_layout.setSpacing(15)

        # Thumbnail Cache Section
        thumb_section_title = QLabel("Thumbnail Cache")
        thumb_section_title.setObjectName("cacheSectionTitle")
        main_layout.addWidget(thumb_section_title)

        thumb_frame = QFrame()
        thumb_frame.setObjectName("cacheSectionFrame")
        thumb_layout = QGridLayout(thumb_frame)

        self.parent.thumb_cache_usage_label = QLabel()
        thumb_layout.addWidget(QLabel("Current Disk Usage:"), 0, 0)
        thumb_layout.addWidget(self.parent.thumb_cache_usage_label, 0, 1)

        delete_thumb_cache_button = QPushButton("Clear Thumbnail Cache")
        delete_thumb_cache_button.setObjectName("deleteThumbnailCacheButton")
        delete_thumb_cache_button.clicked.connect(
            self.parent._clear_thumbnail_cache_action
        )
        thumb_layout.addWidget(delete_thumb_cache_button, 1, 0, 1, 2)
        main_layout.addWidget(thumb_frame)

        # Preview Image Cache Section
        preview_section_title = QLabel("Preview Image Cache")
        preview_section_title.setObjectName("cacheSectionTitle")
        main_layout.addWidget(preview_section_title)

        preview_frame = QFrame()
        preview_frame.setObjectName("cacheSectionFrame")
        preview_layout = QGridLayout(preview_frame)

        self.parent.preview_cache_configured_limit_label = QLabel()
        preview_layout.addWidget(QLabel("Configured Size Limit:"), 0, 0)
        preview_layout.addWidget(self.parent.preview_cache_configured_limit_label, 0, 1)

        self.parent.preview_cache_usage_label = QLabel()
        preview_layout.addWidget(QLabel("Current Disk Usage:"), 1, 0)
        preview_layout.addWidget(self.parent.preview_cache_usage_label, 1, 1)

        preview_layout.addWidget(QLabel("Set New Limit (GB):"), 2, 0)
        self.parent.preview_cache_size_combo = QComboBox()
        self.parent.preview_cache_size_combo.setObjectName("previewCacheSizeCombo")
        self.parent.preview_cache_size_options_gb = [
            0.25,
            0.5,
            1.0,
            2.0,
            4.0,
            8.0,
            16.0,
        ]
        self.parent.preview_cache_size_combo.addItems(
            [f"{size:.2f} GB" for size in self.parent.preview_cache_size_options_gb]
        )

        current_conf_gb = get_preview_cache_size_gb()
        try:
            current_index = self.parent.preview_cache_size_options_gb.index(
                current_conf_gb
            )
            self.parent.preview_cache_size_combo.setCurrentIndex(current_index)
        except ValueError:
            self.parent.preview_cache_size_combo.addItem(
                f"{current_conf_gb:.2f} GB (Custom)"
            )
            self.parent.preview_cache_size_combo.setCurrentIndex(
                self.parent.preview_cache_size_combo.count() - 1
            )

        preview_layout.addWidget(self.parent.preview_cache_size_combo, 2, 1)

        apply_preview_limit_button = QPushButton("Apply New Limit")
        apply_preview_limit_button.setObjectName("applyPreviewLimitButton")
        apply_preview_limit_button.clicked.connect(
            self.parent._apply_preview_cache_limit_action
        )
        preview_layout.addWidget(apply_preview_limit_button, 3, 0, 1, 2)

        delete_preview_cache_button = QPushButton("Clear Preview Cache")
        delete_preview_cache_button.setObjectName("deletePreviewCacheButton")
        delete_preview_cache_button.clicked.connect(
            self.parent._clear_preview_cache_action
        )
        preview_layout.addWidget(delete_preview_cache_button, 4, 0, 1, 2)
        main_layout.addWidget(preview_frame)

        # EXIF Cache Section
        exif_section_title = QLabel("EXIF Metadata Cache")
        exif_section_title.setObjectName("cacheSectionTitle")
        main_layout.addWidget(exif_section_title)

        exif_frame = QFrame()
        exif_frame.setObjectName("cacheSectionFrame")
        exif_layout = QGridLayout(exif_frame)

        self.parent.exif_cache_configured_limit_label = QLabel()
        exif_layout.addWidget(QLabel("Configured Size Limit:"), 0, 0)
        exif_layout.addWidget(self.parent.exif_cache_configured_limit_label, 0, 1)

        self.parent.exif_cache_usage_label = QLabel()
        exif_layout.addWidget(QLabel("Current Disk Usage:"), 1, 0)
        exif_layout.addWidget(self.parent.exif_cache_usage_label, 1, 1)

        exif_layout.addWidget(QLabel("Set New Limit (MB):"), 2, 0)
        self.parent.exif_cache_size_combo = QComboBox()
        self.parent.exif_cache_size_combo.setObjectName("exifCacheSizeCombo")
        self.parent.exif_cache_size_options_mb = [64, 128, 256, 512, 1024]
        self.parent.exif_cache_size_combo.addItems(
            [f"{size} MB" for size in self.parent.exif_cache_size_options_mb]
        )

        current_exif_conf_mb = get_exif_cache_size_mb()
        try:
            current_exif_index = self.parent.exif_cache_size_options_mb.index(
                current_exif_conf_mb
            )
            self.parent.exif_cache_size_combo.setCurrentIndex(current_exif_index)
        except ValueError:
            self.parent.exif_cache_size_combo.addItem(
                f"{current_exif_conf_mb} MB (Custom)"
            )
            self.parent.exif_cache_size_combo.setCurrentIndex(
                self.parent.exif_cache_size_combo.count() - 1
            )
        exif_layout.addWidget(self.parent.exif_cache_size_combo, 2, 1)

        apply_exif_limit_button = QPushButton("Apply New EXIF Limit")
        apply_exif_limit_button.setObjectName("applyExifLimitButton")
        apply_exif_limit_button.clicked.connect(
            self.parent._apply_exif_cache_limit_action
        )
        exif_layout.addWidget(apply_exif_limit_button, 3, 0, 1, 2)

        delete_exif_cache_button = QPushButton("Clear EXIF && Rating Caches")
        delete_exif_cache_button.setObjectName("deleteExifCacheButton")
        delete_exif_cache_button.clicked.connect(self.parent._clear_exif_cache_action)
        exif_layout.addWidget(delete_exif_cache_button, 4, 0, 1, 2)
        main_layout.addWidget(exif_frame)

        main_layout.addSpacerItem(
            QSpacerItem(
                20, 40, QSizePolicy.Policy.Minimum, QSizePolicy.Policy.Expanding
            )
        )
        close_button = QPushButton("Close")
        close_button.setObjectName("cacheDialogCloseButton")
        close_button.clicked.connect(dialog.accept)
        main_layout.addWidget(close_button)

        self.parent._update_cache_dialog_labels()
        dialog.setLayout(main_layout)
        dialog.exec()
        logger.info("Closed cache management dialog")

    def _show_delete_confirmation_dialog(
        self, files: List[str], title_text: str, message_text: str
    ) -> bool:
        """
        Shows a custom, reusable confirmation dialog for deleting files,
        displaying thumbnails of the images to be deleted.

        Args:
            files: A list of paths to the files to be deleted.
            title_text: The window title for the dialog.
            message_text: The main message to display to the user.

        Returns:
            True if the user confirms, False otherwise.
        """
        logger.info(
            f"Showing delete confirmation dialog: {title_text} for {len(files)} files"
        )

        dialog = QDialog(self.parent)
        dialog.setWindowTitle(title_text)
        dialog.setObjectName("deleteConfirmationDialog")
        dialog.setModal(True)
        dialog.setMinimumSize(600, 450)
        # Frameless window for fancy UI
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)

        layout = QVBoxLayout(dialog)
        layout.setContentsMargins(15, 15, 15, 15)

        title = QLabel(message_text)
        title.setObjectName("deleteDialogTitle")
        layout.addWidget(title)

        info_label = QLabel("The following images will be moved to the system trash:")
        info_label.setObjectName("deleteDialogInfo")
        layout.addWidget(info_label)

        # Scroll area for the list of images
        scroll_area = QScrollArea()
        scroll_area.setWidgetResizable(True)
        scroll_area.setObjectName("deleteDialogScrollArea")
        scroll_area.setFrameShape(QFrame.Shape.NoFrame)

        # Use QListWidget for thumbnails and filenames
        list_widget = QListWidget()
        list_widget.setObjectName("deleteDialogListWidget")
        list_widget.setIconSize(QSize(128, 128))
        list_widget.setViewMode(QListWidget.ViewMode.IconMode)
        list_widget.setResizeMode(QListWidget.ResizeMode.Adjust)
        list_widget.setMovement(QListWidget.Movement.Static)
        list_widget.setWordWrap(True)
        list_widget.setSpacing(10)

        # Populate the list
        for file_path in files:
            # Get thumbnail with orientation correction
            thumbnail_pixmap = self.parent.image_pipeline.get_thumbnail_qpixmap(
                file_path,
                apply_orientation=True,  # Ensure orientation is applied
            )
            if thumbnail_pixmap:
                icon = QIcon(thumbnail_pixmap)
                item = QListWidgetItem(icon, os.path.basename(file_path))
                item.setSizeHint(QSize(148, 168))
                item.setTextAlignment(Qt.AlignmentFlag.AlignCenter)
                list_widget.addItem(item)

        scroll_area.setWidget(list_widget)
        layout.addWidget(scroll_area)

        # Buttons
        button_layout = QHBoxLayout()
        button_layout.setSpacing(15)  # Add spacing between buttons
        button_layout.addStretch()

        cancel_button = QPushButton("Cancel")
        cancel_button.setObjectName("deleteDialogCancelButton")
        cancel_button.setMinimumSize(120, 40)  # Make button larger
        cancel_button.clicked.connect(dialog.reject)
        button_layout.addWidget(cancel_button)

        confirm_button = QPushButton("Confirm Move to Trash")
        confirm_button.setObjectName("deleteDialogConfirmButton")
        confirm_button.setMinimumSize(200, 40)  # Make button larger
        confirm_button.setDefault(True)
        confirm_button.clicked.connect(dialog.accept)
        button_layout.addWidget(confirm_button)

        layout.addLayout(button_layout)

        result = dialog.exec()
        confirmed = result == QDialog.DialogCode.Accepted
        logger.info(
            f"User {'confirmed' if confirmed else 'cancelled'} delete confirmation dialog"
        )
        return confirmed

    def show_confirm_delete_dialog(self, deleted_file_paths: List[str]) -> bool:
        """
        Shows a confirmation dialog for deleting files.

        Args:
            deleted_file_paths: A list of paths to the files to be deleted.

        Returns:
            True if the user confirms the deletion, False otherwise.
        """
        logger.info(
            f"Showing confirm delete dialog for {len(deleted_file_paths)} files"
        )

        if deleted_file_paths:
            self._preload_thumbnails_for_dialog(deleted_file_paths)

        num_selected = len(deleted_file_paths)
        if num_selected == 1:
            message = "Are you sure you want to move this image to the trash?"
        else:
            message = f"Are you sure you want to move these {num_selected} images to the trash?"

        result = self._show_delete_confirmation_dialog(
            files=deleted_file_paths, title_text="Confirm Delete", message_text=message
        )

        logger.info(
            f"User {'confirmed' if result else 'cancelled'} confirm delete dialog"
        )
        return result

    def _preload_thumbnails_for_dialog(self, file_paths: List[str]):
        """Preload thumbnails with optional background worker to keep the UI responsive."""
        if not file_paths:
            return

        total = len(file_paths)
        base_message = f"Loading previews for {total} images..."
        self.parent.show_loading_overlay(base_message)

        if total < self.THUMBNAIL_PRELOAD_ASYNC_THRESHOLD:
            try:
                self.parent.image_pipeline.preload_thumbnails(
                    file_paths,
                    progress_callback=lambda processed,
                    total_count: self.parent.update_loading_text(
                        f"{base_message} ({processed}/{total_count})"
                    ),
                )
            except Exception:
                logger.error(
                    "Error preloading thumbnails for dialog.",
                    exc_info=True,
                )
            finally:
                self.parent.hide_loading_overlay()
            return

        loop = QEventLoop()
        thread = QThread(self.parent)
        worker = ThumbnailPreloadWorker(self.parent.image_pipeline)
        worker.moveToThread(thread)
        result = {"error": None}

        def _update_progress(current: int, total_count: int, message: str):
            progress_text = message or f"{base_message} ({current}/{total_count})"
            self.parent.update_loading_text(progress_text)

        def _on_finished():
            loop.quit()

        def _on_error(message: str):
            result["error"] = message
            loop.quit()

        worker.progress.connect(_update_progress)
        worker.finished.connect(_on_finished)
        worker.error.connect(_on_error)
        thread.started.connect(lambda: worker.preload_thumbnails(file_paths))

        try:
            thread.start()
            loop.exec()
        finally:
            worker.stop()
            thread.quit()
            thread.wait()
            worker.deleteLater()
            thread.deleteLater()
            self.parent.hide_loading_overlay()

        if result["error"]:
            logger.warning(
                f"Thumbnail preload error during dialog preparation: {result['error']}"
            )

    def show_potential_cache_overflow_warning(
        self,
        estimated_preview_data_needed_for_folder_bytes: int,
        preview_cache_limit_bytes: int,
    ):
        """
        Shows a warning about potential cache overflow.

        Args:
            estimated_preview_data_needed_for_folder_bytes: The estimated size of the folder's previews.
            preview_cache_limit_bytes: The current preview cache limit.
        """
        logger.info(
            f"Showing potential cache overflow warning: "
            f"estimated {estimated_preview_data_needed_for_folder_bytes / (1024 * 1024):.2f} MB needed, "
            f"limit {preview_cache_limit_bytes / (1024 * 1024 * 1024):.2f} GB"
        )

        warning_msg = (
            f"The images in the selected folder are estimated to require approximately "
            f"{estimated_preview_data_needed_for_folder_bytes / (1024 * 1024):.2f} MB for their previews. "
            f"Your current preview cache limit is "
            f"{preview_cache_limit_bytes / (1024 * 1024 * 1024):.2f} GB.\n\n"
            "This might exceed your cache capacity, potentially leading to frequent cache evictions "
            "and slower performance as previews are regenerated.\n\n"
            "Consider increasing the 'Preview Image Cache' size in "
            "Settings > Manage Cache for a smoother experience, or select a smaller folder."
        )

        # Replace static call with an instance to apply frameless styling
        warn_box = QMessageBox(self.parent)
        warn_box.setIcon(QMessageBox.Icon.Warning)
        warn_box.setWindowTitle("Potential Cache Overflow")
        warn_box.setText(warning_msg)
        warn_box.setStandardButtons(QMessageBox.StandardButton.Ok)
        warn_box.setDefaultButton(QMessageBox.StandardButton.Ok)
        # Frameless window for fancy UI
        warn_box.setWindowFlags(
            warn_box.windowFlags() | Qt.WindowType.FramelessWindowHint
        )
        warn_box.exec()
        logger.info("Closed potential cache overflow warning dialog")

    def show_commit_deletions_dialog(self, marked_files: List[str]) -> bool:
        """
        Shows a confirmation dialog for committing marked deletions.

        Args:
            marked_files: The list of files to be deleted.

        Returns:
            True if the user confirms, False otherwise.
        """
        logger.info(f"Showing commit deletions dialog for {len(marked_files)} files")

        if marked_files:
            self._preload_thumbnails_for_dialog(marked_files)

        count = len(marked_files)
        result = self._show_delete_confirmation_dialog(
            files=marked_files,
            title_text="Confirm Deletion",
            message_text=f"Are you sure you want to move {count} marked image(s) to trash?",
        )

        logger.info(
            f"User {'confirmed' if result else 'cancelled'} commit deletions dialog"
        )
        return result

    def log_dialog_interaction(self, dialog_name: str, action: str, details: str = ""):
        """
        Log user interactions with dialogs.

        Args:
            dialog_name: Name of the dialog
            action: Action taken by user
            details: Additional details about the action
        """
        log_message = f"Dialog Interaction: {dialog_name} - {action}"
        if details:
            log_message += f" - {details}"
        logger.info(log_message)

    def show_close_confirmation_dialog(self, marked_files: List[str]) -> str:
        """
        Shows a confirmation dialog when closing the application with marked files.

        Args:
            marked_files: The list of files marked for deletion.

        Returns:
            A string indicating the user's choice: "commit", "ignore", or "cancel".
        """
        if marked_files:
            self._preload_thumbnails_for_dialog(marked_files)

        dialog = QDialog(self.parent)
        dialog.setWindowTitle("Confirm Close")
        dialog.setObjectName("closeConfirmationDialog")
        dialog.setModal(True)
        dialog.setMinimumSize(500, 300)
        # Frameless window for fancy UI
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)

        layout = QVBoxLayout(dialog)
        layout.setContentsMargins(20, 20, 20, 20)
        layout.setSpacing(15)

        # Title
        title = QLabel("Uncommitted Deletions")
        title.setObjectName("closeDialogTitle")
        title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        layout.addWidget(title)

        # Message
        message = QLabel(
            f"You have {len(marked_files)} image(s) marked for deletion that have not been committed.\n\n"
            "What would you like to do?"
        )
        message.setObjectName("closeDialogMessage")
        message.setWordWrap(True)
        layout.addWidget(message)

        # Scroll area for the list of images
        scroll_area = QScrollArea()
        scroll_area.setWidgetResizable(True)
        scroll_area.setObjectName("closeDialogScrollArea")
        scroll_area.setFrameShape(QFrame.Shape.NoFrame)

        # Use QListWidget for thumbnails and filenames
        list_widget = QListWidget()
        list_widget.setObjectName("closeDialogListWidget")
        list_widget.setIconSize(QSize(64, 64))  # Smaller thumbnails for this dialog
        list_widget.setViewMode(QListWidget.ViewMode.IconMode)
        list_widget.setResizeMode(QListWidget.ResizeMode.Adjust)
        list_widget.setMovement(QListWidget.Movement.Static)
        list_widget.setWordWrap(True)
        list_widget.setSpacing(5)

        # Populate the list
        for file_path in marked_files:
            # Get thumbnail with orientation correction
            thumbnail_pixmap = self.parent.image_pipeline.get_thumbnail_qpixmap(
                file_path,
                apply_orientation=True,  # Ensure orientation is applied
            )
            if thumbnail_pixmap:
                # Scale down the thumbnail to fit the icon size
                scaled_pixmap = thumbnail_pixmap.scaled(
                    64,
                    64,
                    Qt.AspectRatioMode.KeepAspectRatio,
                    Qt.TransformationMode.SmoothTransformation,
                )
                icon = QIcon(scaled_pixmap)
            else:
                # Fallback to a generic icon if thumbnail generation fails
                icon = self.parent.style().standardIcon(
                    QStyle.StandardPixmap.SP_FileIcon
                )

            item = QListWidgetItem(icon, os.path.basename(file_path))
            item.setSizeHint(QSize(84, 104))
            item.setTextAlignment(Qt.AlignmentFlag.AlignCenter)
            list_widget.addItem(item)

        scroll_area.setWidget(list_widget)
        layout.addWidget(scroll_area)

        # Buttons
        button_layout = QHBoxLayout()
        button_layout.setSpacing(10)

        # Cancel button (don't close)
        cancel_button = QPushButton("Cancel")
        cancel_button.setObjectName("closeDialogCancelButton")
        cancel_button.clicked.connect(dialog.reject)
        button_layout.addWidget(cancel_button)

        # Ignore button (close without committing)
        ignore_button = QPushButton("Ignore and Close")
        ignore_button.setObjectName("closeDialogIgnoreButton")
        ignore_button.clicked.connect(
            lambda: dialog.done(1)
        )  # Custom result code for "ignore"
        button_layout.addWidget(ignore_button)

        # Commit button (commit and then close)
        commit_button = QPushButton("Commit and Close")
        commit_button.setObjectName("closeDialogCommitButton")
        commit_button.clicked.connect(
            lambda: dialog.done(2)
        )  # Custom result code for "commit"
        commit_button.setDefault(True)
        button_layout.addWidget(commit_button)

        layout.addLayout(button_layout)

        # Show the dialog and return the user's choice
        logger.info(
            f"Showing close confirmation dialog with {len(marked_files)} marked files"
        )
        result = dialog.exec()

        if result == 1:  # Ignore
            self.log_dialog_interaction(
                "Close Confirmation", "Ignore and Close", f"{len(marked_files)} files"
            )
            return "ignore"
        elif result == 2:  # Commit
            self.log_dialog_interaction(
                "Close Confirmation", "Commit and Close", f"{len(marked_files)} files"
            )
            return "commit"
        else:  # Cancel or closed
            self.log_dialog_interaction(
                "Close Confirmation", "Cancel", f"{len(marked_files)} files"
            )
            return "cancel"

    def show_model_not_found_dialog(self, model_path: str):
        """Show a dialog informing the user that the rotation model is missing."""
        logger.info(f"Showing model not found dialog for path: {model_path}")
        dialog = QMessageBox(self.parent)
        dialog.setWindowTitle("Rotation Model Not Found")
        dialog.setIcon(QMessageBox.Icon.Warning)
        # Frameless window for fancy UI
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)

        text = (
            f"The automatic rotation feature requires a model file that was not found at:\n"
            f"{model_path}\n\n"
            "Please download the model and place it in the correct directory to enable this feature."
        )
        dialog.setText(text)

        detailed_text = (
            "You can download the model from the official GitHub repository.\n\n"
            "1. Click 'Download Model' to open the releases page.\n"
            "2. Download the latest 'orientation_model.onnx' file.\n"
            "3. Place the downloaded file inside the 'models' folder in the application directory.\n"
            "4. Restart the application or re-run the rotation analysis."
        )
        dialog.setInformativeText(detailed_text)

        download_button = dialog.addButton(
            "Download Model", QMessageBox.ButtonRole.ActionRole
        )
        open_models_button = dialog.addButton(
            "Open Models Folder", QMessageBox.ButtonRole.ActionRole
        )
        ok_button = dialog.addButton(QMessageBox.StandardButton.Ok)

        dialog.setDefaultButton(ok_button)

        if download_button:
            download_button.clicked.connect(
                lambda: webbrowser.open(
                    "https://github.com/duartebarbosadev/deep-image-orientation-detection/releases"
                )
            )
        if open_models_button:
            open_models_button.clicked.connect(self._open_models_folder)

        dialog.exec()
        logger.info("Closed model not found dialog")

    def show_best_shot_models_missing_dialog(self, missing_models: list):
        """Show a dialog informing the user that best-shot models are missing.

        Args:
            missing_models: List of MissingModelInfo objects describing missing models.
        """
        logger.info(
            f"Showing best-shot models missing dialog for {len(missing_models)} model(s)"
        )
        dialog = QMessageBox(self.parent)
        dialog.setWindowTitle("Best Shot Models Not Found")
        dialog.setIcon(QMessageBox.Icon.Warning)
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)

        # Build main text
        model_names = ", ".join(m.name for m in missing_models)
        text = (
            f"The best shot analysis feature requires {len(missing_models)} model(s) "
            f"that were not found:\n\n{model_names}\n\n"
            "Please download the required models to enable this feature."
        )
        dialog.setText(text)

        # Build detailed instructions
        detailed_lines = [
            "Download instructions:\n",
        ]
        for i, model in enumerate(missing_models, 1):
            detailed_lines.append(
                f"{i}. {model.name} ({model.description})\n"
                f"   Download from: {model.download_url}\n"
                f"   Expected location: {model.expected_path}\n"
            )
        detailed_lines.append(
            "\nAfter downloading, place the models in the 'models' directory "
            "and restart the analysis."
        )
        dialog.setInformativeText("".join(detailed_lines))

        # Add buttons
        readme_button = dialog.addButton(
            "View Documentation", QMessageBox.ButtonRole.ActionRole
        )
        open_models_button = dialog.addButton(
            "Open Models Folder", QMessageBox.ButtonRole.ActionRole
        )
        ok_button = dialog.addButton(QMessageBox.StandardButton.Ok)

        dialog.setDefaultButton(ok_button)

        # Connect button actions
        if readme_button:
            readme_button.clicked.connect(
                lambda: webbrowser.open(
                    "https://github.com/duartebarbosadev/PhotoSort#experimental-ai-best-shot-ranking"
                )
            )
        if open_models_button:
            open_models_button.clicked.connect(self._open_models_folder)

        dialog.exec()
        logger.info("Closed best-shot models missing dialog")
