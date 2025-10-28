"""
AI Best Shot Picker Settings UI Component

This module provides a reusable widget for configuring AI Best Shot Picker settings.
Can be integrated into the preferences dialog or used standalone.

Usage:
    settings_widget = AIBestShotSettingsWidget()
    # Add to a dialog or layout
    dialog_layout.addWidget(settings_widget)
    
    # Get current values
    config = settings_widget.get_configuration()
    
    # Apply settings
    settings_widget.apply_settings()
"""

import logging
from PyQt6.QtCore import Qt, pyqtSignal
from PyQt6.QtWidgets import (
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
    QLabel,
    QLineEdit,
    QPushButton,
    QSpinBox,
    QGroupBox,
    QFormLayout,
    QMessageBox,
    QDialog,
    QDialogButtonBox,
)

from core import app_settings
from core.ai.best_shot_picker import BestShotPicker

logger = logging.getLogger(__name__)


class AIBestShotSettingsWidget(QWidget):
    """
    Widget for configuring AI Best Shot Picker settings.
    
    Provides inputs for:
    - API URL
    - API Key
    - Model name
    - Timeout
    
    Includes a test connection button to verify settings.
    """

    settings_changed = pyqtSignal()  # Emitted when settings are modified

    def __init__(self, parent=None):
        super().__init__(parent)
        self._setup_ui()
        self._load_settings()

    def _setup_ui(self):
        """Set up the user interface."""
        layout = QVBoxLayout(self)
        layout.setSpacing(15)

        # Section title
        title = QLabel("AI Best Shot Picker")
        title.setStyleSheet("font-weight: bold; font-size: 13px;")
        layout.addWidget(title)

        # Description
        desc = QLabel(
            "Configure the AI service for automatic best image selection. "
            "Uses vision language models to analyze and compare images."
        )
        desc.setWordWrap(True)
        desc.setStyleSheet("color: #666; font-size: 11px;")
        layout.addWidget(desc)

        # Settings group
        settings_group = QGroupBox("API Configuration")
        settings_layout = QFormLayout(settings_group)
        settings_layout.setSpacing(10)

        # API URL
        self.api_url_input = QLineEdit()
        self.api_url_input.setPlaceholderText("http://localhost:1234/v1")
        self.api_url_input.textChanged.connect(self.settings_changed.emit)
        settings_layout.addRow("API URL:", self.api_url_input)

        # API Key
        self.api_key_input = QLineEdit()
        self.api_key_input.setPlaceholderText("not-needed (for local LM Studio)")
        self.api_key_input.setEchoMode(QLineEdit.EchoMode.Password)
        self.api_key_input.textChanged.connect(self.settings_changed.emit)
        
        # Add show/hide button for API key
        api_key_layout = QHBoxLayout()
        api_key_layout.addWidget(self.api_key_input)
        
        show_key_button = QPushButton("Show")
        show_key_button.setMaximumWidth(60)
        show_key_button.setCheckable(True)
        show_key_button.toggled.connect(self._toggle_api_key_visibility)
        api_key_layout.addWidget(show_key_button)
        
        settings_layout.addRow("API Key:", api_key_layout)

        # Model name
        self.model_input = QLineEdit()
        self.model_input.setPlaceholderText("local-model")
        self.model_input.textChanged.connect(self.settings_changed.emit)
        settings_layout.addRow("Model:", self.model_input)

        # Timeout
        self.timeout_spinbox = QSpinBox()
        self.timeout_spinbox.setMinimum(10)
        self.timeout_spinbox.setMaximum(600)
        self.timeout_spinbox.setValue(120)
        self.timeout_spinbox.setSuffix(" seconds")
        self.timeout_spinbox.valueChanged.connect(self.settings_changed.emit)
        settings_layout.addRow("Timeout:", self.timeout_spinbox)

        layout.addWidget(settings_group)

        # Test connection button
        test_button_layout = QHBoxLayout()
        test_button_layout.addStretch()
        
        self.test_button = QPushButton("Test Connection")
        self.test_button.clicked.connect(self._test_connection)
        test_button_layout.addWidget(self.test_button)
        
        layout.addLayout(test_button_layout)

        # Help text
        help_text = QLabel(
            "<b>Quick Setup with LM Studio:</b><br>"
            "1. Download LM Studio from <a href='https://lmstudio.ai'>lmstudio.ai</a><br>"
            "2. Install a vision model (e.g., qwen2-vl-7b)<br>"
            "3. Load the model with mmproj file<br>"
            "4. Start the local server<br>"
            "5. Use default settings above"
        )
        help_text.setWordWrap(True)
        help_text.setOpenExternalLinks(True)
        help_text.setStyleSheet(
            "background-color: #f0f0f0; padding: 10px; "
            "border-radius: 5px; font-size: 11px;"
        )
        layout.addWidget(help_text)

        layout.addStretch()

    def _toggle_api_key_visibility(self, show: bool):
        """Toggle API key visibility."""
        if show:
            self.api_key_input.setEchoMode(QLineEdit.EchoMode.Normal)
        else:
            self.api_key_input.setEchoMode(QLineEdit.EchoMode.Password)

    def _load_settings(self):
        """Load current settings from app_settings."""
        self.api_url_input.setText(app_settings.get_ai_best_shot_api_url())
        self.api_key_input.setText(app_settings.get_ai_best_shot_api_key())
        self.model_input.setText(app_settings.get_ai_best_shot_model())
        self.timeout_spinbox.setValue(app_settings.get_ai_best_shot_timeout())

    def _test_connection(self):
        """Test the connection to the AI service."""
        logger.info("Testing AI Best Shot Picker connection...")
        
        # Disable button during test
        self.test_button.setEnabled(False)
        self.test_button.setText("Testing...")
        
        try:
            # Create picker with current settings
            picker = BestShotPicker(
                base_url=self.api_url_input.text() or app_settings.DEFAULT_AI_BEST_SHOT_API_URL,
                api_key=self.api_key_input.text() or app_settings.DEFAULT_AI_BEST_SHOT_API_KEY,
                model=self.model_input.text() or app_settings.DEFAULT_AI_BEST_SHOT_MODEL,
                timeout=self.timeout_spinbox.value(),
            )
            
            # Test connection
            if picker.test_connection():
                QMessageBox.information(
                    self,
                    "Connection Successful",
                    "Successfully connected to the AI service!\n\n"
                    "The best shot picker is ready to use.",
                )
                logger.info("AI connection test successful")
            else:
                QMessageBox.warning(
                    self,
                    "Connection Failed",
                    "Could not connect to the AI service.\n\n"
                    "Please check:\n"
                    "• LM Studio is running\n"
                    "• A vision model is loaded\n"
                    "• The local server is started\n"
                    "• The API URL is correct",
                )
                logger.warning("AI connection test failed")
                
        except Exception as e:
            QMessageBox.critical(
                self,
                "Connection Error",
                f"An error occurred while testing the connection:\n\n{str(e)}",
            )
            logger.error(f"AI connection test error: {e}")
            
        finally:
            # Re-enable button
            self.test_button.setEnabled(True)
            self.test_button.setText("Test Connection")

    def apply_settings(self):
        """Apply the current settings to app_settings."""
        api_url = self.api_url_input.text() or app_settings.DEFAULT_AI_BEST_SHOT_API_URL
        api_key = self.api_key_input.text() or app_settings.DEFAULT_AI_BEST_SHOT_API_KEY
        model = self.model_input.text() or app_settings.DEFAULT_AI_BEST_SHOT_MODEL
        timeout = self.timeout_spinbox.value()

        app_settings.set_ai_best_shot_api_url(api_url)
        app_settings.set_ai_best_shot_api_key(api_key)
        app_settings.set_ai_best_shot_model(model)
        app_settings.set_ai_best_shot_timeout(timeout)

        logger.info(
            f"AI Best Shot Picker settings saved: "
            f"url={api_url}, model={model}, timeout={timeout}"
        )

    def get_configuration(self) -> dict:
        """
        Get the current configuration as a dictionary.
        
        Returns:
            dict: Configuration with keys: api_url, api_key, model, timeout
        """
        return {
            "api_url": self.api_url_input.text() or app_settings.DEFAULT_AI_BEST_SHOT_API_URL,
            "api_key": self.api_key_input.text() or app_settings.DEFAULT_AI_BEST_SHOT_API_KEY,
            "model": self.model_input.text() or app_settings.DEFAULT_AI_BEST_SHOT_MODEL,
            "timeout": self.timeout_spinbox.value(),
        }

    def reset_to_defaults(self):
        """Reset all settings to their default values."""
        self.api_url_input.setText(app_settings.DEFAULT_AI_BEST_SHOT_API_URL)
        self.api_key_input.setText(app_settings.DEFAULT_AI_BEST_SHOT_API_KEY)
        self.model_input.setText(app_settings.DEFAULT_AI_BEST_SHOT_MODEL)
        self.timeout_spinbox.setValue(app_settings.DEFAULT_AI_BEST_SHOT_TIMEOUT)
        self.settings_changed.emit()


# Standalone dialog for testing
class AIBestShotSettingsDialog(QDialog):
    """Standalone dialog for AI Best Shot Picker settings."""
    
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("AI Best Shot Picker Settings")
        self.setModal(True)
        self.setMinimumWidth(500)
        
        layout = QVBoxLayout(self)
        
        # Add settings widget
        self.settings_widget = AIBestShotSettingsWidget(self)
        layout.addWidget(self.settings_widget)
        
        # Add buttons
        button_box = QDialogButtonBox(
            QDialogButtonBox.StandardButton.Ok | 
            QDialogButtonBox.StandardButton.Cancel |
            QDialogButtonBox.StandardButton.RestoreDefaults
        )
        button_box.accepted.connect(self.accept)
        button_box.rejected.connect(self.reject)
        
        # Connect restore defaults button
        restore_button = button_box.button(QDialogButtonBox.StandardButton.RestoreDefaults)
        if restore_button:
            restore_button.clicked.connect(self.settings_widget.reset_to_defaults)
        
        layout.addWidget(button_box)
        
    def accept(self):
        """Apply settings and close dialog."""
        self.settings_widget.apply_settings()
        super().accept()


if __name__ == "__main__":
    """
    Test the settings widget standalone.
    Usage: python -m ui.helpers.ai_best_shot_settings
    """
    import sys
    from PyQt6.QtWidgets import QApplication
    
    app = QApplication(sys.argv)
    
    # Test the dialog
    dialog = AIBestShotSettingsDialog()
    if dialog.exec():
        print("Settings saved!")
        config = dialog.settings_widget.get_configuration()
        print(f"Configuration: {config}")
    else:
        print("Cancelled")
    
    sys.exit(0)
