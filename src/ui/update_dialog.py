"""
Update Notification Dialog
Shows available updates to the user.
"""

import logging
import webbrowser

from PyQt6.QtCore import Qt
from PyQt6.QtGui import QFont
from PyQt6.QtWidgets import (
    QDialog,
    QVBoxLayout,
    QHBoxLayout,
    QLabel,
    QPushButton,
    QTextEdit,
    QCheckBox,
)

from core.update_checker import UpdateInfo
from core.app_settings import set_update_check_enabled

logger = logging.getLogger(__name__)


class UpdateNotificationDialog(QDialog):
    """Dialog to notify users about available updates."""

    def __init__(self, update_info: UpdateInfo, current_version: str, parent=None):
        super().__init__(parent)
        self.update_info = update_info
        self.current_version = current_version

        self.setWindowTitle("Update Available")
        self.setWindowFlags(Qt.WindowType.Dialog | Qt.WindowType.WindowCloseButtonHint)
        # Frameless window for fancy UI
        self.setWindowFlags(self.windowFlags() | Qt.WindowType.FramelessWindowHint)
        self.setModal(True)
        self.resize(500, 400)

        self._setup_ui()

    def _setup_ui(self):
        """Set up the dialog UI."""
        layout = QVBoxLayout()
        layout.setSpacing(15)
        layout.setContentsMargins(20, 20, 20, 20)

        # Header with icon and title
        header_layout = QHBoxLayout()

        # Title label
        title_label = QLabel(f"PhotoSort {self.update_info.version} is available!")
        title_font = QFont()
        title_font.setPointSize(14)
        title_font.setBold(True)
        title_label.setFont(title_font)
        header_layout.addWidget(title_label)

        header_layout.addStretch()
        layout.addLayout(header_layout)

        # Version info
        version_info = QLabel(f"Current version: {self.current_version}")
        version_info.setStyleSheet("color: #666; font-size: 11px;")
        layout.addWidget(version_info)

        # Scrollable text area for release notes
        notes_area = QTextEdit()
        notes_area.setReadOnly(True)
        notes_area.setMaximumHeight(150)

        # Convert markdown to HTML for better rendering
        release_notes = self.update_info.release_notes or "No release notes available."
        # Basic markdown to HTML conversion
        html_notes = self._convert_markdown_to_html(release_notes)
        notes_area.setHtml(html_notes)

        layout.addWidget(notes_area)

        # Checkbox for disabling update checks
        self.disable_checks_checkbox = QCheckBox(
            "Don't check for updates automatically"
        )
        self.disable_checks_checkbox.setToolTip(
            "You can re-enable update checks in the Help menu"
        )

        # Auto-check if user has previously disabled automatic updates
        from core.app_settings import get_update_check_enabled

        current_setting = get_update_check_enabled()
        logger.info(
            f"Creating update dialog. Current update check setting: {current_setting}"
        )
        if not current_setting:
            self.disable_checks_checkbox.setChecked(True)
            logger.info("Checkbox auto-checked because updates are currently disabled")

        layout.addWidget(self.disable_checks_checkbox)

        # Button layout
        button_layout = QHBoxLayout()

        # Close button
        close_button = QPushButton("Close")
        close_button.clicked.connect(self._on_close_clicked)
        button_layout.addWidget(close_button)

        button_layout.addStretch()

        # Download Update button (renamed from View Release)
        download_update_button = QPushButton("Download Update")
        download_update_button.setDefault(True)
        download_update_button.clicked.connect(self._on_download_update_clicked)
        button_layout.addWidget(download_update_button)

        layout.addLayout(button_layout)

        self.setLayout(layout)

    def _on_close_clicked(self):
        """Handle 'Close' button click."""
        logger.info(
            f"Close button clicked. Checkbox checked: {self.disable_checks_checkbox.isChecked()}"
        )
        if self.disable_checks_checkbox.isChecked():
            set_update_check_enabled(False)
            logger.info("Automatic update checks disabled by user")
        else:
            set_update_check_enabled(True)
            logger.info("Automatic update checks re-enabled by user")

        self.reject()

    def _on_download_update_clicked(self):
        """Handle 'Download Update' button click."""
        logger.info(
            f"Download Update button clicked. Checkbox checked: {self.disable_checks_checkbox.isChecked()}"
        )
        if self.disable_checks_checkbox.isChecked():
            set_update_check_enabled(False)
            logger.info("Automatic update checks disabled by user")
        else:
            set_update_check_enabled(True)
            logger.info("Automatic update checks re-enabled by user")

        # Try to open download URL first, fallback to release page
        url_to_open = self.update_info.download_url or self.update_info.release_url

        try:
            webbrowser.open(url_to_open)
            logger.info(f"Opened update URL: {url_to_open}")
        except Exception as e:
            logger.error(f"Failed to open update URL: {e}")

        self.accept()

    def _convert_markdown_to_html(self, markdown_text: str) -> str:
        """Convert basic markdown formatting to HTML."""
        if not markdown_text:
            return "No release notes available."

        html = markdown_text

        # Convert headers
        html = (
            html.replace("### ", "<h3>")
            .replace("\n# ", "</h3>\n<h1>")
            .replace("\n## ", "</h1>\n<h2>")
            .replace("\n### ", "</h2>\n<h3>")
        )
        if html.startswith("# "):
            html = "<h1>" + html[2:]
        elif html.startswith("## "):
            html = "<h2>" + html[3:]
        elif html.startswith("### "):
            html = "<h3>" + html[4:]

        # Close any open headers at the end
        if "<h1>" in html and "</h1>" not in html:
            html += "</h1>"
        elif "<h2>" in html and "</h2>" not in html:
            html += "</h2>"
        elif "<h3>" in html and "</h3>" not in html:
            html += "</h3>"

        # Convert bold text
        import re

        html = re.sub(r"\*\*(.*?)\*\*", r"<strong>\1</strong>", html)
        html = re.sub(r"__(.*?)__", r"<strong>\1</strong>", html)

        # Convert italic text
        html = re.sub(r"\*(.*?)\*", r"<em>\1</em>", html)
        html = re.sub(r"_(.*?)_", r"<em>\1</em>", html)

        # Convert bullet points
        lines = html.split("\n")
        in_list = False
        result_lines = []

        for line in lines:
            stripped = line.strip()
            if stripped.startswith("- ") or stripped.startswith("* "):
                if not in_list:
                    result_lines.append("<ul>")
                    in_list = True
                result_lines.append(f"<li>{stripped[2:]}</li>")
            else:
                if in_list:
                    result_lines.append("</ul>")
                    in_list = False
                if stripped:
                    result_lines.append(f"<p>{stripped}</p>")

        if in_list:
            result_lines.append("</ul>")

        return "\n".join(result_lines)


class UpdateCheckDialog(QDialog):
    """Simple dialog for manual update checks."""

    def __init__(self, parent=None):
        super().__init__(parent)
        self.setWindowTitle("Check for Updates")
        self.setWindowFlags(Qt.WindowType.Dialog | Qt.WindowType.WindowCloseButtonHint)
        # Frameless window for fancy UI
        self.setWindowFlags(self.windowFlags() | Qt.WindowType.FramelessWindowHint)
        self.setModal(True)
        self.resize(300, 150)

        self._setup_ui()

    def _setup_ui(self):
        """Set up the dialog UI."""
        layout = QVBoxLayout()
        layout.setSpacing(15)
        layout.setContentsMargins(20, 20, 20, 20)

        # Status label
        self.status_label = QLabel("Checking for updates...")
        layout.addWidget(self.status_label)

        # Button layout
        button_layout = QHBoxLayout()
        button_layout.addStretch()

        self.close_button = QPushButton("Close")
        self.close_button.clicked.connect(self.reject)
        self.close_button.setEnabled(False)
        button_layout.addWidget(self.close_button)

        layout.addLayout(button_layout)
        self.setLayout(layout)

    def set_status(self, message: str, enable_close: bool = False):
        """Update the status message."""
        self.status_label.setText(message)
        self.close_button.setEnabled(enable_close)
