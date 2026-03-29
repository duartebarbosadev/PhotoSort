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
from PyQt6.QtGui import QIcon, QDesktopServices, QFont

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


def _make_dialog_draggable(dialog):
    """Attach mouse handlers to a frameless dialog to support click-drag moving."""
    _state = {"offset": None}

    def _press(e):
        if e.button() == Qt.MouseButton.LeftButton:
            _state["offset"] = (
                e.globalPosition().toPoint() - dialog.frameGeometry().topLeft()
            )
            e.accept()

    def _move(e):
        if (e.buttons() & Qt.MouseButton.LeftButton) and _state["offset"] is not None:
            dialog.move(e.globalPosition().toPoint() - _state["offset"])
            e.accept()

    dialog.mousePressEvent = _press  # type: ignore[assignment]
    dialog.mouseMoveEvent = _move  # type: ignore[assignment]


def _build_dialog_header(title, icon_text, parent_layout):
    """Build a standard dialog header bar with icon and title. Returns the header frame."""
    header = QFrame()
    header.setObjectName("dialogHeader")
    h_layout = QHBoxLayout(header)
    h_layout.setContentsMargins(22, 14, 22, 14)
    h_layout.setSpacing(10)
    icon_lbl = QLabel(icon_text)
    icon_lbl.setObjectName("dialogHeaderIcon")
    h_layout.addWidget(icon_lbl)
    title_lbl = QLabel(title)
    title_lbl.setObjectName("dialogHeaderTitle")
    font = QFont()
    font.setPointSize(13)
    font.setBold(True)
    title_lbl.setFont(font)
    h_layout.addWidget(title_lbl)
    h_layout.addStretch()
    parent_layout.addWidget(header)
    return header


def _build_dialog_footer(dialog, parent_layout, buttons):
    """Build a footer bar with the given buttons list [(text, objectName, callback, is_default)].
    Returns the footer frame."""
    footer = QFrame()
    footer.setObjectName("dialogFooter")
    f_layout = QHBoxLayout(footer)
    f_layout.setContentsMargins(22, 10, 22, 14)
    f_layout.setSpacing(10)
    f_layout.addStretch()
    for text, obj_name, callback, is_default in buttons:
        btn = QPushButton(text)
        btn.setObjectName(obj_name)
        btn.clicked.connect(callback)
        if is_default:
            btn.setDefault(True)
        f_layout.addWidget(btn)
    parent_layout.addWidget(footer)
    return footer


def _build_card(object_name="dialogCard"):
    """Create a card frame and its inner VBoxLayout. Returns (card, layout)."""
    card = QFrame()
    card.setObjectName(object_name)
    layout = QVBoxLayout(card)
    layout.setSpacing(10)
    layout.setContentsMargins(16, 14, 16, 14)
    return card, layout


def _build_kv_row(key_text, value_widget, parent_layout):
    """Build a horizontal key-value row inside a card."""
    row = QHBoxLayout()
    row.setSpacing(0)
    key = QLabel(key_text)
    key.setObjectName("kvKey")
    row.addWidget(key)
    row.addStretch()
    if isinstance(value_widget, str):
        val = QLabel(value_widget)
        val.setObjectName("kvValue")
        row.addWidget(val)
    else:
        value_widget.setObjectName("kvValue")
        row.addWidget(value_widget)
    parent_layout.addLayout(row)


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
        dialog.setFixedSize(500, 480)
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)
        _make_dialog_draggable(dialog)

        outer = QVBoxLayout(dialog)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        # --- Header ---
        _build_dialog_header("About PhotoSort", "📷", outer)

        # --- Scrollable content ---
        scroll = QScrollArea()
        scroll.setWidgetResizable(True)
        scroll.setFrameShape(QFrame.Shape.NoFrame)
        scroll.setObjectName("aboutScrollArea")
        outer.addWidget(scroll)

        content = QFrame()
        content.setObjectName("dialogContent")
        scroll.setWidget(content)
        body = QVBoxLayout(content)
        body.setSpacing(12)
        body.setContentsMargins(20, 14, 20, 14)

        # Version info card
        version_text = None
        try:
            from core.build_info import VERSION  # type: ignore

            version_text = str(VERSION).strip() or None
        except (ImportError, AttributeError):
            version_text = None

        if version_text:
            ver_card, ver_layout = _build_card("dialogCard")
            ver_layout.setSpacing(6)
            ver_lbl = QLabel(f"Version {version_text}")
            ver_lbl.setObjectName("aboutVersion")
            ver_layout.addWidget(ver_lbl)
            body.addWidget(ver_card)

        # Technology card
        tech_card, tech_layout = _build_card("dialogCard")

        tech_title = QLabel("Technology Stack")
        tech_title.setObjectName("cardSectionTitle")
        tech_layout.addWidget(tech_title)

        sep = QFrame()
        sep.setObjectName("cardSeparator")
        sep.setFrameShape(QFrame.Shape.HLine)
        sep.setFixedHeight(1)
        tech_layout.addWidget(sep)

        # Get ONNX provider information
        try:
            model_detector = ModelRotationDetector()
            onnx_provider = (
                getattr(model_detector._state, "provider_name", None)
                or "N/A (model not loaded)"
            )
        except ModelNotFoundError:
            onnx_provider = "N/A (model not found)"
        except Exception:
            onnx_provider = "N/A (error)"

        embeddings_label_ref = None
        embeddings_val = QLabel("SentenceTransformer (CLIP)")
        embeddings_val.setObjectName("kvValue")
        embeddings_label_ref = embeddings_val
        _build_kv_row("Embeddings", embeddings_val, tech_layout)
        _build_kv_row("Rotation Model", f"ONNX Runtime · {onnx_provider}", tech_layout)
        _build_kv_row("Clustering", "DBSCAN (scikit-learn)", tech_layout)
        _build_kv_row("Metadata", "pyexiv2", tech_layout)
        _build_kv_row("Interface", "PyQt6", tech_layout)

        body.addWidget(tech_card)

        # Quick actions card
        actions_card, actions_layout = _build_card("dialogCard")
        act_title = QLabel("Quick Actions")
        act_title.setObjectName("cardSectionTitle")
        actions_layout.addWidget(act_title)

        sep2 = QFrame()
        sep2.setObjectName("cardSeparator")
        sep2.setFrameShape(QFrame.Shape.HLine)
        sep2.setFixedHeight(1)
        actions_layout.addWidget(sep2)

        btn_row = QHBoxLayout()
        btn_row.setSpacing(8)
        for text, obj, callback in [
            ("Models Folder", "aboutModelsButton", self._open_models_folder),
            ("Logs Folder", "aboutLogsButton", self._open_logs_folder),
            (
                "GitHub",
                "aboutGithubButton",
                lambda: webbrowser.open(
                    "https://github.com/duartebarbosadev/PhotoSort"
                ),
            ),
        ]:
            btn = QPushButton(text)
            btn.setObjectName(obj)
            btn.clicked.connect(callback)
            btn_row.addWidget(btn)
        actions_layout.addLayout(btn_row)
        body.addWidget(actions_card)

        body.addStretch()

        # --- Footer ---
        _build_dialog_footer(
            dialog,
            outer,
            [
                ("Close", "aboutCloseButton", dialog.accept, True),
            ],
        )

        # Start CUDA detection worker
        worker_manager = self.parent.app_controller.worker_manager
        if embeddings_label_ref:

            def update_embeddings_label(device_name: str):
                device_key = (device_name or "cpu").lower()
                friendly = {
                    "cuda": "GPU (CUDA)",
                    "mps": "GPU (Apple MPS)",
                    "cpu": "CPU",
                }
                label_text = friendly.get(device_key, device_key.upper())
                try:
                    if embeddings_label_ref:
                        embeddings_label_ref.setText(
                            f"SentenceTransformer (CLIP) · {label_text}"
                        )
                except RuntimeError:
                    pass

            worker_manager.cuda_detection_finished.connect(update_embeddings_label)
            worker_manager.start_cuda_detection()

        if block:
            dialog.exec()
            logger.info("Closed about dialog")
        else:
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
            return True, False

        dialog = QDialog(self.parent)
        dialog.setWindowTitle("Confirm Lossy Rotation")
        dialog.setObjectName("lossyRotationDialog")
        dialog.setModal(True)
        dialog.setFixedSize(500, 260)
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)
        _make_dialog_draggable(dialog)

        outer = QVBoxLayout(dialog)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        # Header
        _build_dialog_header("Lossy Rotation", "⚠", outer)

        # Content
        body = QVBoxLayout()
        body.setSpacing(14)
        body.setContentsMargins(22, 16, 22, 10)

        card, card_layout = _build_card("dialogCard")
        card_layout.setSpacing(10)

        message_text = f"Lossless rotation failed for:\n{filename}"
        warning_text = f"Proceed with lossy rotation {rotation_type}?\nThis will re-encode the image and may reduce quality."
        if "images" in filename.lower():
            warning_text = f"Proceed with lossy rotation {rotation_type} for all selected images?\nThis will re-encode the images and may reduce quality."

        message_label = QLabel(message_text)
        message_label.setObjectName("lossyRotationMessageLabel")
        message_label.setWordWrap(True)
        card_layout.addWidget(message_label)

        warning_label = QLabel(warning_text)
        warning_label.setObjectName("lossyRotationWarningLabel")
        warning_label.setWordWrap(True)
        card_layout.addWidget(warning_label)

        body.addWidget(card)

        never_ask_checkbox = QCheckBox("Don't ask again for lossy rotations")
        never_ask_checkbox.setObjectName("neverAskAgainCheckbox")
        body.addWidget(never_ask_checkbox)

        outer.addLayout(body)
        outer.addStretch()

        # Footer
        _build_dialog_footer(
            dialog,
            outer,
            [
                ("Cancel", "lossyRotationCancelButton", dialog.reject, False),
                ("Proceed", "lossyRotationProceedButton", dialog.accept, True),
            ],
        )

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
            get_best_shot_batch_size,
            set_best_shot_batch_size,
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
        dialog.resize(640, 620)
        dialog.setMinimumSize(520, 480)
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)
        _make_dialog_draggable(dialog)

        outer = QVBoxLayout(dialog)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        # Header
        _build_dialog_header("Preferences", "⚙", outer)

        # Scrollable content
        scroll_area = QScrollArea()
        scroll_area.setObjectName("preferencesScrollArea")
        scroll_area.setWidgetResizable(True)
        scroll_area.setFrameShape(QFrame.Shape.NoFrame)
        outer.addWidget(scroll_area)

        content_frame = QFrame()
        content_frame.setObjectName("dialogContent")
        content_layout = QVBoxLayout(content_frame)
        content_layout.setSpacing(12)
        content_layout.setContentsMargins(20, 14, 20, 14)
        scroll_area.setWidget(content_frame)

        # --- Performance Mode Card ---
        perf_card, perf_layout = _build_card("dialogCard")
        perf_title = QLabel("Performance Mode")
        perf_title.setObjectName("cardSectionTitle")
        perf_layout.addWidget(perf_title)

        sep = QFrame()
        sep.setObjectName("cardSeparator")
        sep.setFrameShape(QFrame.Shape.HLine)
        sep.setFixedHeight(1)
        perf_layout.addWidget(sep)

        desc_label = QLabel(
            "Control how many CPU threads PhotoSort uses for processing."
        )
        desc_label.setObjectName("cardDescription")
        desc_label.setWordWrap(True)
        perf_layout.addWidget(desc_label)

        balanced_radio = QRadioButton("Balanced (Recommended)")
        balanced_radio.setObjectName("balancedRadio")
        balanced_desc = QLabel("Uses 85% of CPU cores to keep system responsive")
        balanced_desc.setObjectName("radioDescription")
        perf_layout.addWidget(balanced_radio)
        perf_layout.addWidget(balanced_desc)

        performance_radio = QRadioButton("Performance")
        performance_radio.setObjectName("performanceRadio")
        perf_desc = QLabel("Uses all available CPU cores for maximum speed")
        perf_desc.setObjectName("radioDescription")
        perf_layout.addWidget(performance_radio)
        perf_layout.addWidget(perf_desc)

        custom_radio = QRadioButton("Custom")
        custom_radio.setObjectName("customRadio")
        perf_layout.addWidget(custom_radio)

        max_threads = os.cpu_count() or 4
        current_thread_count = min(get_custom_thread_count(), max_threads)

        thread_count_label = QLabel(
            f"Thread count: {current_thread_count} (max: {max_threads})"
        )
        thread_count_label.setObjectName("radioDescription")
        perf_layout.addWidget(thread_count_label)

        thread_count_slider = QSlider(Qt.Orientation.Horizontal)
        thread_count_slider.setObjectName("threadCountSlider")
        thread_count_slider.setMinimum(1)
        thread_count_slider.setMaximum(max_threads)
        thread_count_slider.setValue(current_thread_count)
        thread_count_slider.setTickPosition(QSlider.TickPosition.TicksBelow)
        tick_interval = max(1, max_threads // 8)
        thread_count_slider.setTickInterval(tick_interval)
        thread_count_slider.setEnabled(False)

        def on_slider_changed(value):
            thread_count_label.setText(f"Thread count: {value} (max: {max_threads})")

        thread_count_slider.valueChanged.connect(on_slider_changed)
        perf_layout.addWidget(thread_count_slider)

        current_mode = get_performance_mode()
        if current_mode == PerformanceMode.BALANCED:
            balanced_radio.setChecked(True)
        elif current_mode == PerformanceMode.PERFORMANCE:
            performance_radio.setChecked(True)
        else:
            custom_radio.setChecked(True)
            thread_count_slider.setEnabled(True)

        def on_custom_toggled(checked):
            thread_count_slider.setEnabled(checked)
            thread_count_label.setEnabled(checked)

        custom_radio.toggled.connect(on_custom_toggled)
        on_custom_toggled(custom_radio.isChecked())

        note_label = QLabel("Changes take effect immediately for new operations.")
        note_label.setObjectName("cardNote")
        note_label.setWordWrap(True)
        perf_layout.addWidget(note_label)

        content_layout.addWidget(perf_card)

        # --- AI Engine Card ---
        ai_card, ai_layout = _build_card("dialogCard")
        ai_title = QLabel("AI Rating Engine")
        ai_title.setObjectName("cardSectionTitle")
        ai_layout.addWidget(ai_title)

        sep2 = QFrame()
        sep2.setObjectName("cardSeparator")
        sep2.setFrameShape(QFrame.Shape.HLine)
        sep2.setFixedHeight(1)
        ai_layout.addWidget(sep2)

        ai_desc_label = QLabel(
            "Configure the OpenAI-compatible vision model used for best-shot analysis and AI star ratings."
        )
        ai_desc_label.setObjectName("cardDescription")
        ai_desc_label.setWordWrap(True)
        ai_layout.addWidget(ai_desc_label)

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
            timeout_value = int(openai_config.get("timeout") or DEFAULT_OPENAI_TIMEOUT)
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

        best_shot_batch_label = QLabel("Best-shot Batch Size")
        best_shot_batch_spin = QSpinBox()
        best_shot_batch_spin.setObjectName("bestShotBatchSpin")
        best_shot_batch_spin.setRange(2, 12)
        best_shot_batch_spin.setValue(get_best_shot_batch_size())
        openai_form.addWidget(best_shot_batch_label, 6, 0)
        openai_form.addWidget(best_shot_batch_spin, 6, 1)

        best_prompt_label = QLabel("Best Shot Prompt")
        best_prompt_edit = QPlainTextEdit()
        best_prompt_edit.setObjectName("openAIBestPromptEdit")
        best_prompt_edit.setPlaceholderText(
            "Leave blank to use the default best-shot prompt."
        )
        best_prompt_edit.setPlainText(best_prompt_value)
        best_prompt_edit.setMinimumHeight(80)
        openai_form.addWidget(best_prompt_label, 7, 0, Qt.AlignmentFlag.AlignTop)
        openai_form.addWidget(best_prompt_edit, 7, 1)

        rating_prompt_label = QLabel("Rating Prompt")
        rating_prompt_edit = QPlainTextEdit()
        rating_prompt_edit.setObjectName("openAIRatingPromptEdit")
        rating_prompt_edit.setPlaceholderText(
            "Leave blank to use the default rating prompt."
        )
        rating_prompt_edit.setPlainText(rating_prompt_value)
        rating_prompt_edit.setMinimumHeight(80)
        openai_form.addWidget(rating_prompt_label, 8, 0, Qt.AlignmentFlag.AlignTop)
        openai_form.addWidget(rating_prompt_edit, 8, 1)

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

        ai_layout.addLayout(openai_form)
        btn_row = QHBoxLayout()
        btn_row.setContentsMargins(0, 0, 0, 0)
        btn_row.setSpacing(6)
        btn_row.addWidget(test_connection_button)
        btn_row.addStretch()
        ai_layout.addLayout(btn_row)
        content_layout.addWidget(ai_card)

        content_layout.addStretch()

        # Footer
        _build_dialog_footer(
            dialog,
            outer,
            [
                ("Cancel", "preferencesCancelButton", dialog.reject, False),
                ("Save", "preferencesSaveButton", lambda: None, True),
            ],
        )

        def save_preferences():
            if balanced_radio.isChecked():
                set_performance_mode(PerformanceMode.BALANCED)
            elif performance_radio.isChecked():
                set_performance_mode(PerformanceMode.PERFORMANCE)
            else:
                set_performance_mode(PerformanceMode.CUSTOM)
                set_custom_thread_count(thread_count_slider.value())

            api_key_text = api_key_input.text().strip()
            base_url_text = base_url_input.text().strip()
            model_text = model_combo.currentText().strip()
            max_tokens_value = max_tokens_spin.value()
            timeout_value = timeout_spin.value()
            max_workers_value = max_workers_spin.value()
            best_shot_batch_value = best_shot_batch_spin.value()
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

            if best_shot_batch_value != get_best_shot_batch_size():
                set_best_shot_batch_size(best_shot_batch_value)

            logger.info(
                "Preferences saved: mode=%s, custom_threads=%s",
                get_performance_mode().value,
                get_custom_thread_count(),
            )
            dialog.accept()

        # Re-bind the save button to the actual save function
        footer_widget = outer.itemAt(outer.count() - 1).widget()
        for btn in footer_widget.findChildren(QPushButton):
            if btn.objectName() == "preferencesSaveButton":
                btn.clicked.connect(save_preferences)
                break

        dialog.setLayout(outer)
        dialog.exec()
        logger.info("Closed preferences dialog")

    def _build_cache_card(self, title_text, icon_text, parent_layout):
        """Build a modern cache section card and return its content layout."""
        card = QFrame()
        card.setObjectName("cacheCard")
        card_layout = QVBoxLayout(card)
        card_layout.setSpacing(12)
        card_layout.setContentsMargins(18, 14, 18, 14)

        # Card header with icon
        header = QHBoxLayout()
        header.setSpacing(8)
        icon = QLabel(icon_text)
        icon.setObjectName("cacheCardIcon")
        header.addWidget(icon)
        title = QLabel(title_text)
        title.setObjectName("cacheCardTitle")
        header.addWidget(title)
        header.addStretch()
        card_layout.addLayout(header)

        # Separator
        sep = QFrame()
        sep.setObjectName("cacheCardSeparator")
        sep.setFrameShape(QFrame.Shape.HLine)
        sep.setFixedHeight(1)
        card_layout.addWidget(sep)

        parent_layout.addWidget(card)
        return card_layout

    def _build_cache_row(self, label_text, value_label, parent_layout):
        """Build a key-value row for a cache card."""
        row = QHBoxLayout()
        row.setSpacing(0)
        key = QLabel(label_text)
        key.setObjectName("cacheRowKey")
        row.addWidget(key)
        row.addStretch()
        value_label.setObjectName("cacheRowValue")
        row.addWidget(value_label)
        parent_layout.addLayout(row)

    def show_cache_management_dialog(self):
        """Show the cache management dialog."""
        logger.info("Showing cache management dialog")
        dialog = QDialog(self.parent)
        dialog.setWindowTitle("Cache Management")
        dialog.setObjectName("cacheManagementDialog")
        dialog.setMinimumSize(480, 520)
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)
        _make_dialog_draggable(dialog)

        outer_layout = QVBoxLayout(dialog)
        outer_layout.setContentsMargins(0, 0, 0, 0)
        outer_layout.setSpacing(0)

        # Dialog header
        _build_dialog_header("Cache Management", "⚙", outer_layout)

        # Scrollable content
        scroll_area = QScrollArea()
        scroll_area.setWidgetResizable(True)
        scroll_area.setObjectName("cacheManagementScrollArea")
        scroll_area.setFrameShape(QFrame.Shape.NoFrame)
        outer_layout.addWidget(scroll_area)

        content_widget = QFrame()
        content_widget.setObjectName("cacheDialogContent")
        scroll_area.setWidget(content_widget)
        main_layout = QVBoxLayout(content_widget)
        main_layout.setSpacing(12)
        main_layout.setContentsMargins(20, 12, 20, 12)

        # --- Thumbnail Cache Card ---
        thumb_card_layout = self._build_cache_card("Thumbnails", "🖼", main_layout)
        self.parent.thumb_cache_usage_label = QLabel()
        self._build_cache_row(
            "Disk Usage", self.parent.thumb_cache_usage_label, thumb_card_layout
        )

        delete_thumb_cache_button = QPushButton("Clear Thumbnails")
        delete_thumb_cache_button.setObjectName("deleteThumbnailCacheButton")
        delete_thumb_cache_button.clicked.connect(
            self.parent._clear_thumbnail_cache_action
        )
        thumb_card_layout.addWidget(delete_thumb_cache_button)

        # --- Preview Image Cache Card ---
        preview_card_layout = self._build_cache_card(
            "Preview Images", "🔍", main_layout
        )
        self.parent.preview_cache_configured_limit_label = QLabel()
        self._build_cache_row(
            "Size Limit",
            self.parent.preview_cache_configured_limit_label,
            preview_card_layout,
        )
        self.parent.preview_cache_usage_label = QLabel()
        self._build_cache_row(
            "Disk Usage", self.parent.preview_cache_usage_label, preview_card_layout
        )

        # Limit selector row
        limit_row = QHBoxLayout()
        limit_row.setSpacing(8)
        limit_label = QLabel("New Limit")
        limit_label.setObjectName("cacheRowKey")
        limit_row.addWidget(limit_label)
        limit_row.addStretch()

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
        limit_row.addWidget(self.parent.preview_cache_size_combo)

        apply_preview_limit_button = QPushButton("Apply")
        apply_preview_limit_button.setObjectName("applyPreviewLimitButton")
        apply_preview_limit_button.clicked.connect(
            self.parent._apply_preview_cache_limit_action
        )
        limit_row.addWidget(apply_preview_limit_button)
        preview_card_layout.addLayout(limit_row)

        delete_preview_cache_button = QPushButton("Clear Preview Cache")
        delete_preview_cache_button.setObjectName("deletePreviewCacheButton")
        delete_preview_cache_button.clicked.connect(
            self.parent._clear_preview_cache_action
        )
        preview_card_layout.addWidget(delete_preview_cache_button)

        # --- EXIF Cache Card ---
        exif_card_layout = self._build_cache_card("EXIF Metadata", "📋", main_layout)
        self.parent.exif_cache_configured_limit_label = QLabel()
        self._build_cache_row(
            "Size Limit",
            self.parent.exif_cache_configured_limit_label,
            exif_card_layout,
        )
        self.parent.exif_cache_usage_label = QLabel()
        self._build_cache_row(
            "Disk Usage", self.parent.exif_cache_usage_label, exif_card_layout
        )

        # EXIF limit selector row
        exif_limit_row = QHBoxLayout()
        exif_limit_row.setSpacing(8)
        exif_limit_label = QLabel("New Limit")
        exif_limit_label.setObjectName("cacheRowKey")
        exif_limit_row.addWidget(exif_limit_label)
        exif_limit_row.addStretch()

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
        exif_limit_row.addWidget(self.parent.exif_cache_size_combo)

        apply_exif_limit_button = QPushButton("Apply")
        apply_exif_limit_button.setObjectName("applyExifLimitButton")
        apply_exif_limit_button.clicked.connect(
            self.parent._apply_exif_cache_limit_action
        )
        exif_limit_row.addWidget(apply_exif_limit_button)
        exif_card_layout.addLayout(exif_limit_row)

        delete_exif_cache_button = QPushButton("Clear EXIF && Rating Caches")
        delete_exif_cache_button.setObjectName("deleteExifCacheButton")
        delete_exif_cache_button.clicked.connect(self.parent._clear_exif_cache_action)
        exif_card_layout.addWidget(delete_exif_cache_button)

        # --- Analysis Cache Card ---
        analysis_card_layout = self._build_cache_card("Analysis", "🧠", main_layout)
        self.parent.analysis_cache_usage_label = QLabel()
        self._build_cache_row(
            "Disk Usage", self.parent.analysis_cache_usage_label, analysis_card_layout
        )

        clear_analysis_cache_button = QPushButton("Clear Analysis Cache")
        clear_analysis_cache_button.setObjectName("clearAnalysisCacheButton")
        clear_analysis_cache_button.clicked.connect(
            self.parent._clear_analysis_cache_action
        )
        analysis_card_layout.addWidget(clear_analysis_cache_button)

        main_layout.addStretch()

        # Footer
        _build_dialog_footer(
            dialog,
            outer_layout,
            [
                ("Close", "cacheDialogCloseButton", dialog.accept, True),
            ],
        )

        self.parent._update_cache_dialog_labels()
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
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)
        _make_dialog_draggable(dialog)

        outer = QVBoxLayout(dialog)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        # Header
        _build_dialog_header(title_text, "🗑", outer)

        # Content
        body = QVBoxLayout()
        body.setContentsMargins(20, 14, 20, 8)
        body.setSpacing(10)

        info_label = QLabel(message_text)
        info_label.setObjectName("deleteDialogInfo")
        info_label.setWordWrap(True)
        body.addWidget(info_label)

        # Thumbnail grid
        list_widget = QListWidget()
        list_widget.setObjectName("deleteDialogListWidget")
        list_widget.setIconSize(QSize(128, 128))
        list_widget.setViewMode(QListWidget.ViewMode.IconMode)
        list_widget.setResizeMode(QListWidget.ResizeMode.Adjust)
        list_widget.setMovement(QListWidget.Movement.Static)
        list_widget.setWordWrap(True)
        list_widget.setSpacing(10)

        for file_path in files:
            thumbnail_pixmap = self.parent.image_pipeline.get_thumbnail_qpixmap(
                file_path,
                apply_orientation=True,
            )
            if thumbnail_pixmap:
                icon = QIcon(thumbnail_pixmap)
                item = QListWidgetItem(icon, os.path.basename(file_path))
                item.setSizeHint(QSize(148, 168))
                item.setTextAlignment(Qt.AlignmentFlag.AlignCenter)
                list_widget.addItem(item)

        body.addWidget(list_widget)
        outer.addLayout(body)

        # Footer
        _build_dialog_footer(
            dialog,
            outer,
            [
                ("Cancel", "deleteDialogCancelButton", dialog.reject, False),
                ("Move to Trash", "deleteDialogConfirmButton", dialog.accept, True),
            ],
        )

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
                    progress_callback=lambda processed, total_count: (
                        self.parent.update_loading_text(
                            f"{base_message} ({processed}/{total_count})"
                        )
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

    def _show_marked_files_confirmation_dialog(
        self,
        marked_files: List[str],
        *,
        window_title: str,
        dialog_title: str,
        message: str,
        ignore_button_text: str,
        commit_button_text: str,
        log_context: str,
        object_name_prefix: str,
    ) -> str:
        """
        Shared helper to present a confirmation dialog for pending deletions.
        """
        if marked_files:
            self._preload_thumbnails_for_dialog(marked_files)

        dialog = QDialog(self.parent)
        dialog.setWindowTitle(window_title)
        dialog.setObjectName(f"{object_name_prefix}Dialog")
        dialog.setModal(True)
        dialog.setMinimumSize(600, 450)
        dialog.setWindowFlags(dialog.windowFlags() | Qt.WindowType.FramelessWindowHint)
        _make_dialog_draggable(dialog)

        outer = QVBoxLayout(dialog)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        # Header
        _build_dialog_header(dialog_title, "⚠", outer)

        # Content
        body = QVBoxLayout()
        body.setContentsMargins(20, 14, 20, 8)
        body.setSpacing(10)

        message_label = QLabel(message)
        message_label.setObjectName(f"{object_name_prefix}Message")
        message_label.setWordWrap(True)
        body.addWidget(message_label)

        # Thumbnail grid
        list_widget = QListWidget()
        list_widget.setObjectName(f"{object_name_prefix}ListWidget")
        list_widget.setIconSize(QSize(128, 128))
        list_widget.setViewMode(QListWidget.ViewMode.IconMode)
        list_widget.setResizeMode(QListWidget.ResizeMode.Adjust)
        list_widget.setMovement(QListWidget.Movement.Static)
        list_widget.setWordWrap(True)
        list_widget.setSpacing(10)

        for file_path in marked_files:
            thumbnail_pixmap = self.parent.image_pipeline.get_thumbnail_qpixmap(
                file_path,
                apply_orientation=True,
            )
            if thumbnail_pixmap:
                icon = QIcon(thumbnail_pixmap)
            else:
                icon = self.parent.style().standardIcon(
                    QStyle.StandardPixmap.SP_FileIcon
                )

            item = QListWidgetItem(icon, os.path.basename(file_path))
            item.setSizeHint(QSize(148, 168))
            item.setTextAlignment(Qt.AlignmentFlag.AlignCenter)
            list_widget.addItem(item)

        body.addWidget(list_widget)
        outer.addLayout(body)

        # Footer with 3 buttons
        footer = QFrame()
        footer.setObjectName("dialogFooter")
        f_layout = QHBoxLayout(footer)
        f_layout.setContentsMargins(22, 10, 22, 14)
        f_layout.setSpacing(10)
        f_layout.addStretch()

        cancel_button = QPushButton("Cancel")
        cancel_button.setObjectName(f"{object_name_prefix}CancelButton")
        cancel_button.clicked.connect(dialog.reject)
        f_layout.addWidget(cancel_button)

        ignore_button = QPushButton(ignore_button_text)
        ignore_button.setObjectName(f"{object_name_prefix}IgnoreButton")
        ignore_button.clicked.connect(lambda: dialog.done(1))
        f_layout.addWidget(ignore_button)

        commit_button = QPushButton(commit_button_text)
        commit_button.setObjectName(f"{object_name_prefix}CommitButton")
        commit_button.clicked.connect(lambda: dialog.done(2))
        commit_button.setDefault(True)
        f_layout.addWidget(commit_button)

        outer.addWidget(footer)

        logger.info(
            f"Showing {log_context.lower()} dialog with {len(marked_files)} marked files"
        )
        result = dialog.exec()

        if result == 1:
            self.log_dialog_interaction(
                log_context, ignore_button_text, f"{len(marked_files)} files"
            )
            return "ignore"
        elif result == 2:
            self.log_dialog_interaction(
                log_context, commit_button_text, f"{len(marked_files)} files"
            )
            return "commit"
        else:
            self.log_dialog_interaction(
                log_context, "Cancel", f"{len(marked_files)} files"
            )
            return "cancel"

    def show_close_confirmation_dialog(self, marked_files: List[str]) -> str:
        """
        Shows a confirmation dialog when closing the application with marked files.

        Args:
            marked_files: The list of files marked for deletion.

        Returns:
            A string indicating the user's choice: "commit", "ignore", or "cancel".
        """
        message = (
            f"You have {len(marked_files)} image(s) marked for deletion that have not been committed.\n\n"
            "What would you like to do?"
        )
        return self._show_marked_files_confirmation_dialog(
            marked_files,
            window_title="Confirm Close",
            dialog_title="Uncommitted Deletions",
            message=message,
            ignore_button_text="Ignore and Close",
            commit_button_text="Commit and Close",
            log_context="Close Confirmation",
            object_name_prefix="closeDialog",
        )

    def show_folder_change_confirmation_dialog(self, marked_files: List[str]) -> str:
        """
        Shows a confirmation dialog when changing folders with marked files.

        Args:
            marked_files: The list of files marked for deletion.

        Returns:
            A string indicating the user's choice: "commit", "ignore", or "cancel".
        """
        message = (
            f"You have {len(marked_files)} image(s) marked for deletion that have not been committed.\n\n"
            "Would you like to commit or ignore them before opening a different folder?"
        )
        return self._show_marked_files_confirmation_dialog(
            marked_files,
            window_title="Confirm Folder Change",
            dialog_title="Uncommitted Deletions",
            message=message,
            ignore_button_text="Ignore and Switch",
            commit_button_text="Commit and Switch",
            log_context="Folder Change Confirmation",
            object_name_prefix="folderChangeDialog",
        )

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
