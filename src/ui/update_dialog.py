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
    QFrame,
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
        self.setWindowFlags(Qt.WindowType.Dialog | Qt.WindowType.FramelessWindowHint)
        self.setModal(True)
        self.resize(550, 500)  # More space for release notes
        self.setObjectName("updateNotificationDialog")

        # Enable dragging for the entire dialog
        self._drag_pos = None

        self._setup_ui()

    def _setup_ui(self):
        """Set up the dialog UI."""
        layout = QVBoxLayout(self)
        layout.setSpacing(18)
        layout.setContentsMargins(25, 25, 25, 25)

        # Title with icon
        title_layout = QHBoxLayout()
        title_layout.setSpacing(12)

        icon_label = QLabel("🔄")
        icon_label.setObjectName("updateIcon")
        icon_label.setStyleSheet("font-size: 20px; color: #0084FF;")
        title_layout.addWidget(icon_label)

        title_label = QLabel(f"PhotoSort {self.update_info.version} Available")
        title_label.setObjectName("updateTitle")
        title_font = QFont()
        title_font.setPointSize(16)
        title_font.setBold(True)
        title_label.setFont(title_font)
        title_layout.addWidget(title_label)

        title_layout.addStretch()
        layout.addLayout(title_layout)

        # Version comparison section
        version_frame = QFrame()
        version_frame.setObjectName("versionFrame")
        version_layout = QHBoxLayout(version_frame)
        version_layout.setContentsMargins(15, 12, 15, 12)

        current_version_label = QLabel(f"Current: {self.current_version}")
        current_version_label.setObjectName("currentVersionLabel")
        version_layout.addWidget(current_version_label)

        arrow_label = QLabel("→")
        arrow_label.setObjectName("versionArrow")
        arrow_label.setStyleSheet("font-size: 16px; color: #0084FF; font-weight: bold;")
        version_layout.addWidget(arrow_label)

        new_version_label = QLabel(f"New: {self.update_info.version}")
        new_version_label.setObjectName("newVersionLabel")
        version_layout.addWidget(new_version_label)

        version_layout.addStretch()
        layout.addWidget(version_frame)

        # Release notes section
        notes_label = QLabel("What's New:")
        notes_label.setObjectName("notesLabel")
        notes_font = QFont()
        notes_font.setPointSize(12)
        notes_font.setBold(True)
        notes_label.setFont(notes_font)
        layout.addWidget(notes_label)

        # Scrollable text area for release notes
        notes_area = QTextEdit()
        notes_area.setObjectName("releaseNotesArea")
        notes_area.setReadOnly(True)
        notes_area.setMinimumHeight(200)

        # Convert markdown to HTML for better rendering
        release_notes = self.update_info.release_notes or "No release notes available."
        html_notes = self._convert_markdown_to_html(release_notes)
        notes_area.setHtml(html_notes)

        layout.addWidget(notes_area)

        # Settings section
        settings_frame = QFrame()
        settings_frame.setObjectName("settingsFrame")
        settings_layout = QVBoxLayout(settings_frame)
        settings_layout.setContentsMargins(15, 12, 15, 12)

        # Checkbox for disabling update checks
        self.disable_checks_checkbox = QCheckBox(
            "Don't check for updates automatically"
        )
        self.disable_checks_checkbox.setObjectName("updateCheckbox")
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

        settings_layout.addWidget(self.disable_checks_checkbox)
        layout.addWidget(settings_frame)

        # Button layout with smaller buttons
        button_layout = QHBoxLayout()
        button_layout.setSpacing(10)

        # Later/Close button
        later_button = QPushButton("Later")
        later_button.setObjectName("laterButton")
        later_button.clicked.connect(self._on_close_clicked)
        button_layout.addWidget(later_button)

        button_layout.addStretch()

        # Download Update button
        download_button = QPushButton("Download Update")
        download_button.setObjectName("downloadButton")
        download_button.setDefault(True)
        download_button.clicked.connect(self._on_download_update_clicked)
        button_layout.addWidget(download_button)

        layout.addLayout(button_layout)

    def mousePressEvent(self, event):
        """Handle mouse press for dialog dragging."""
        if event.button() == Qt.MouseButton.LeftButton:
            self._drag_pos = (
                event.globalPosition().toPoint() - self.frameGeometry().topLeft()
            )
            event.accept()

    def mouseMoveEvent(self, event):
        """Handle mouse move for dialog dragging."""
        if event.buttons() == Qt.MouseButton.LeftButton and self._drag_pos is not None:
            self.move(event.globalPosition().toPoint() - self._drag_pos)
            event.accept()

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
        self.setWindowFlags(Qt.WindowType.Dialog | Qt.WindowType.FramelessWindowHint)
        self.setModal(True)
        self.resize(380, 140)
        self.setObjectName("updateCheckDialog")

        # Enable dragging for the entire dialog
        self._drag_pos = None

        self._setup_ui()

    def _setup_ui(self):
        """Set up the dialog UI."""
        layout = QVBoxLayout(self)
        layout.setSpacing(18)
        layout.setContentsMargins(25, 25, 25, 25)

        # Title with icon
        title_layout = QHBoxLayout()
        title_layout.setSpacing(12)

        icon_label = QLabel("🔍")
        icon_label.setObjectName("checkIcon")
        icon_label.setStyleSheet("font-size: 18px; color: #0084FF;")
        title_layout.addWidget(icon_label)

        title_label = QLabel("Check for Updates")
        title_label.setObjectName("checkTitle")
        title_font = QFont()
        title_font.setPointSize(14)
        title_font.setBold(True)
        title_label.setFont(title_font)
        title_layout.addWidget(title_label)

        title_layout.addStretch()
        layout.addLayout(title_layout)

        # Status section
        status_layout = QHBoxLayout()
        status_icon = QLabel("⏳")
        status_icon.setObjectName("statusIcon")
        status_icon.setStyleSheet("font-size: 14px;")
        status_layout.addWidget(status_icon)

        self.status_label = QLabel("Checking for updates...")
        self.status_label.setObjectName("statusLabel")
        status_layout.addWidget(self.status_label)
        status_layout.addStretch()

        layout.addLayout(status_layout)

        # Add stretch to center content vertically
        layout.addStretch()

        # Button layout
        button_layout = QHBoxLayout()
        button_layout.addStretch()

        self.bottom_close_button = QPushButton("Close")
        self.bottom_close_button.setObjectName("checkCloseButton")
        self.bottom_close_button.clicked.connect(self.reject)
        self.bottom_close_button.setEnabled(False)
        button_layout.addWidget(self.bottom_close_button)

        layout.addLayout(button_layout)

    def mousePressEvent(self, event):
        """Handle mouse press for dialog dragging."""
        if event.button() == Qt.MouseButton.LeftButton:
            self._drag_pos = (
                event.globalPosition().toPoint() - self.frameGeometry().topLeft()
            )
            event.accept()

    def mouseMoveEvent(self, event):
        """Handle mouse move for dialog dragging."""
        if event.buttons() == Qt.MouseButton.LeftButton and self._drag_pos is not None:
            self.move(event.globalPosition().toPoint() - self._drag_pos)
            event.accept()

    def set_status(self, message: str, enable_close: bool = False):
        """Update the status message."""
        self.status_label.setText(message)
        self.bottom_close_button.setEnabled(enable_close)
