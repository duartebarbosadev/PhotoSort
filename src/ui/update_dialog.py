"""
Update Notification Dialog
Shows available updates to the user.
"""

import logging
import webbrowser

from PyQt6.QtCore import Qt
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
from ui.dialog_components import (
    build_card,
    build_dialog_header,
    make_dialog_draggable,
)

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
        self.resize(550, 520)
        self.setObjectName("updateNotificationDialog")

        make_dialog_draggable(self)
        self._setup_ui()

    def _setup_ui(self):
        """Set up the dialog UI."""
        outer = QVBoxLayout(self)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        # Header
        build_dialog_header(
            f"PhotoSort {self.update_info.version} Available", "🔄", outer
        )

        # Content body
        body = QVBoxLayout()
        body.setSpacing(12)
        body.setContentsMargins(20, 14, 20, 14)

        # Version comparison card
        ver_card, ver_layout = build_card("dialogCard")
        ver_row = QHBoxLayout()
        ver_row.setSpacing(12)

        current_label = QLabel(f"Current: {self.current_version}")
        current_label.setObjectName("currentVersionLabel")
        ver_row.addWidget(current_label)

        arrow = QLabel("→")
        arrow.setObjectName("versionArrow")
        ver_row.addWidget(arrow)

        new_label = QLabel(f"New: {self.update_info.version}")
        new_label.setObjectName("newVersionLabel")
        ver_row.addWidget(new_label)

        ver_row.addStretch()
        ver_layout.addLayout(ver_row)
        body.addWidget(ver_card)

        # Release notes
        notes_label = QLabel("What's New")
        notes_label.setObjectName("cardSectionTitle")
        body.addWidget(notes_label)

        notes_area = QTextEdit()
        notes_area.setObjectName("releaseNotesArea")
        notes_area.setReadOnly(True)
        notes_area.setMinimumHeight(200)

        release_notes = self.update_info.release_notes or "No release notes available."
        html_notes = self._convert_markdown_to_html(release_notes)
        notes_area.setHtml(html_notes)
        body.addWidget(notes_area)

        # Settings card
        settings_card, settings_layout = build_card("dialogCard")
        self.disable_checks_checkbox = QCheckBox(
            "Don't check for updates automatically"
        )
        self.disable_checks_checkbox.setObjectName("updateCheckbox")
        self.disable_checks_checkbox.setToolTip(
            "You can re-enable update checks in the Help menu"
        )

        from core.app_settings import get_update_check_enabled

        current_setting = get_update_check_enabled()
        logger.info(
            f"Creating update dialog. Current update check setting: {current_setting}"
        )
        if not current_setting:
            self.disable_checks_checkbox.setChecked(True)
            logger.info("Checkbox auto-checked because updates are currently disabled")

        settings_layout.addWidget(self.disable_checks_checkbox)
        body.addWidget(settings_card)

        outer.addLayout(body)

        # Footer
        footer = QFrame()
        footer.setObjectName("dialogFooter")
        f_layout = QHBoxLayout(footer)
        f_layout.setContentsMargins(22, 10, 22, 14)
        f_layout.setSpacing(10)

        later_button = QPushButton("Later")
        later_button.setObjectName("laterButton")
        later_button.clicked.connect(self._on_close_clicked)
        f_layout.addWidget(later_button)

        f_layout.addStretch()

        download_button = QPushButton("Download Update")
        download_button.setObjectName("downloadButton")
        download_button.setDefault(True)
        download_button.clicked.connect(self._on_download_update_clicked)
        f_layout.addWidget(download_button)

        outer.addWidget(footer)

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
        """Handle 'Download Update' button click - opens GitHub releases page."""
        logger.info(
            f"Download Update button clicked. Checkbox checked: {self.disable_checks_checkbox.isChecked()}"
        )
        if self.disable_checks_checkbox.isChecked():
            set_update_check_enabled(False)
            logger.info("Automatic update checks disabled by user")
        else:
            set_update_check_enabled(True)
            logger.info("Automatic update checks re-enabled by user")

        # Always open the GitHub releases page
        url_to_open = self.update_info.release_url

        try:
            webbrowser.open(url_to_open)
            logger.info(f"Opened GitHub release page: {url_to_open}")
        except Exception as e:
            logger.error(f"Failed to open release page: {e}")

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
        self.resize(400, 160)
        self.setObjectName("updateCheckDialog")

        make_dialog_draggable(self)
        self._setup_ui()

    def _setup_ui(self):
        """Set up the dialog UI."""
        outer = QVBoxLayout(self)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        # Header
        build_dialog_header("Check for Updates", "🔍", outer)

        # Content
        body = QVBoxLayout()
        body.setSpacing(12)
        body.setContentsMargins(22, 14, 22, 14)

        status_layout = QHBoxLayout()
        status_icon = QLabel("⏳")
        status_icon.setObjectName("statusIcon")
        status_layout.addWidget(status_icon)

        self.status_label = QLabel("Checking for updates...")
        self.status_label.setObjectName("statusLabel")
        status_layout.addWidget(self.status_label)
        status_layout.addStretch()

        body.addLayout(status_layout)
        body.addStretch()

        outer.addLayout(body)

        # Footer
        footer = QFrame()
        footer.setObjectName("dialogFooter")
        f_layout = QHBoxLayout(footer)
        f_layout.setContentsMargins(22, 10, 22, 14)
        f_layout.setSpacing(10)
        f_layout.addStretch()

        self.bottom_close_button = QPushButton("Close")
        self.bottom_close_button.setObjectName("checkCloseButton")
        self.bottom_close_button.clicked.connect(self.reject)
        self.bottom_close_button.setEnabled(False)
        f_layout.addWidget(self.bottom_close_button)

        outer.addWidget(footer)

    def set_status(self, message: str, enable_close: bool = False):
        """Update the status message."""
        self.status_label.setText(message)
        self.bottom_close_button.setEnabled(enable_close)
