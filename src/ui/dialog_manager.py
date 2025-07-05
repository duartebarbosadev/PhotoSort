from typing import List, Tuple
import webbrowser

from PyQt6.QtWidgets import (
    QDialog, QVBoxLayout, QHBoxLayout, QLabel, QPushButton, QCheckBox, QMessageBox, QFrame, QGridLayout, QSpacerItem,
    QComboBox, QSizePolicy
)
from PyQt6.QtCore import Qt
from PyQt6.QtGui import QFont, QPixmap, QPainter, QLinearGradient, QColor

from src.core.app_settings import (
    get_rotation_confirm_lossy, is_pytorch_cuda_available, get_preview_cache_size_gb,
    set_preview_cache_size_gb, get_exif_cache_size_mb, set_exif_cache_size_mb
)


class DialogManager:
    """A manager class for handling the creation of dialogs."""

    def __init__(self, parent):
        """
        Initialize the DialogManager.
        
        Args:
            parent: The parent widget, typically the MainWindow.
        """
        self.parent = parent

    def show_about_dialog(self):
        """Show the 'About' dialog with application and technology information."""
        dialog = QDialog(self.parent)
        dialog.setWindowTitle("About PhotoSort")
        dialog.setObjectName("aboutDialog")
        dialog.setModal(True)
        dialog.setFixedSize(480, 420)
        dialog.setAttribute(Qt.WidgetAttribute.WA_DeleteOnClose)

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

        # App info (left side)
        app_info_layout = QVBoxLayout()
        app_info_layout.setSpacing(3)
        
        title_label = QLabel("PhotoSort")
        title_label.setObjectName("aboutTitle")
        app_info_layout.addWidget(title_label)

        version_label = QLabel("Version 1.0b")
        version_label.setObjectName("aboutVersion")
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
        tech_items = [
            f"🧠 Embeddings: SentenceTransformer (CLIP) on {'GPU (CUDA)' if is_pytorch_cuda_available() else 'CPU'}",
            f"🔍 {clustering_info}",
            "📋 Metadata: pyexiv2 • 🎨 Interface: PyQt6 • 🐍 Runtime: Python"
        ]

        for item in tech_items:
            item_label = QLabel(item)
            item_label.setObjectName("aboutTechItem")
            item_label.setWordWrap(True)
            tech_layout.addWidget(item_label)

        content_layout.addWidget(tech_frame)

        # GitHub section - just the button
        github_layout = QHBoxLayout()
        github_layout.addStretch()

        # GitHub button
        github_button = QPushButton("🔗 View on GitHub")
        github_button.setObjectName("aboutGithubButton")
        github_button.clicked.connect(lambda: webbrowser.open("https://github.com/duartebarbosadev/PhotoSort"))
        github_layout.addWidget(github_button)

        content_layout.addLayout(github_layout)

        # Add content to main layout
        main_layout.addLayout(content_layout)

        # Spacer
        main_layout.addSpacerItem(QSpacerItem(20, 20, QSizePolicy.Policy.Minimum, QSizePolicy.Policy.Expanding))

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

        dialog.exec()

    def show_lossy_rotation_confirmation_dialog(self, filename: str, rotation_type: str) -> Tuple[bool, bool]:
        """
        Show a confirmation dialog for lossy rotation with a 'never ask again' option.
        
        Args:
            filename: The name of the file being rotated.
            rotation_type: A description of the rotation (e.g., "90° clockwise").
            
        Returns:
            A tuple containing (proceed_with_rotation: bool, never_ask_again: bool).
        """
        if not get_rotation_confirm_lossy():
            return True, False  # Proceed without asking if the setting is disabled

        dialog = QDialog(self.parent)
        dialog.setWindowTitle("Confirm Lossy Rotation")
        dialog.setObjectName("lossyRotationDialog")
        dialog.setModal(True)
        dialog.setFixedSize(480, 200)

        layout = QVBoxLayout(dialog)
        layout.setSpacing(20)
        layout.setContentsMargins(25, 25, 25, 25)

        message_text = f"Lossless rotation failed for:\n{filename}"
        warning_text = f"Proceed with lossy rotation {rotation_type}?\nThis will re-encode the image and may reduce quality."
        
        if "images" in filename.lower(): # Batch operation
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
        proceed = (result == QDialog.DialogCode.Accepted)
        never_ask_again = never_ask_checkbox.isChecked()

        return proceed, never_ask_again

    def show_cache_management_dialog(self):
        """Show the cache management dialog."""
        dialog = QDialog(self.parent)
        dialog.setWindowTitle("Cache Management")
        dialog.setObjectName("cacheManagementDialog")
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
        delete_thumb_cache_button.clicked.connect(self.parent._clear_thumbnail_cache_action)
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
        self.parent.preview_cache_size_options_gb = [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0]
        self.parent.preview_cache_size_combo.addItems(
            [f"{size:.2f} GB" for size in self.parent.preview_cache_size_options_gb])

        current_conf_gb = get_preview_cache_size_gb()
        try:
            current_index = self.parent.preview_cache_size_options_gb.index(current_conf_gb)
            self.parent.preview_cache_size_combo.setCurrentIndex(current_index)
        except ValueError:
            self.parent.preview_cache_size_combo.addItem(f"{current_conf_gb:.2f} GB (Custom)")
            self.parent.preview_cache_size_combo.setCurrentIndex(self.parent.preview_cache_size_combo.count() - 1)

        preview_layout.addWidget(self.parent.preview_cache_size_combo, 2, 1)

        apply_preview_limit_button = QPushButton("Apply New Limit")
        apply_preview_limit_button.setObjectName("applyPreviewLimitButton")
        apply_preview_limit_button.clicked.connect(self.parent._apply_preview_cache_limit_action)
        preview_layout.addWidget(apply_preview_limit_button, 3, 0, 1, 2)

        delete_preview_cache_button = QPushButton("Clear Preview Cache")
        delete_preview_cache_button.setObjectName("deletePreviewCacheButton")
        delete_preview_cache_button.clicked.connect(self.parent._clear_preview_cache_action)
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
        self.parent.exif_cache_size_combo.addItems([f"{size} MB" for size in self.parent.exif_cache_size_options_mb])

        current_exif_conf_mb = get_exif_cache_size_mb()
        try:
            current_exif_index = self.parent.exif_cache_size_options_mb.index(current_exif_conf_mb)
            self.parent.exif_cache_size_combo.setCurrentIndex(current_exif_index)
        except ValueError:
            self.parent.exif_cache_size_combo.addItem(f"{current_exif_conf_mb} MB (Custom)")
            self.parent.exif_cache_size_combo.setCurrentIndex(self.parent.exif_cache_size_combo.count() - 1)
        exif_layout.addWidget(self.parent.exif_cache_size_combo, 2, 1)

        apply_exif_limit_button = QPushButton("Apply New EXIF Limit")
        apply_exif_limit_button.setObjectName("applyExifLimitButton")
        apply_exif_limit_button.clicked.connect(self.parent._apply_exif_cache_limit_action)
        exif_layout.addWidget(apply_exif_limit_button, 3, 0, 1, 2)

        delete_exif_cache_button = QPushButton("Clear EXIF && Rating Caches")
        delete_exif_cache_button.setObjectName("deleteExifCacheButton")
        delete_exif_cache_button.clicked.connect(self.parent._clear_exif_cache_action)
        exif_layout.addWidget(delete_exif_cache_button, 4, 0, 1, 2)
        main_layout.addWidget(exif_frame)

        main_layout.addSpacerItem(QSpacerItem(20, 40, QSizePolicy.Policy.Minimum, QSizePolicy.Policy.Expanding))
        close_button = QPushButton("Close")
        close_button.setObjectName("cacheDialogCloseButton")
        close_button.clicked.connect(dialog.accept)
        main_layout.addWidget(close_button)

        self.parent._update_cache_dialog_labels()
        dialog.setLayout(main_layout)
        dialog.exec()

    def show_confirm_delete_dialog(self, deleted_file_paths: List[str]) -> bool:
        """
        Shows a confirmation dialog for deleting files.
        
        Args:
            deleted_file_paths: A list of paths to the files to be deleted.
            
        Returns:
            True if the user confirms the deletion, False otherwise.
        """
        dialog = QMessageBox(self.parent)
        dialog.setWindowTitle("Confirm Delete")

        def get_truncated_path(path):
            parts = path.replace('\\', '/').split('/')
            return f".../{'/'.join(parts[-4:])}" if len(parts) > 4 else path

        num_selected = len(deleted_file_paths)
        if num_selected == 1:
            truncated_path = get_truncated_path(deleted_file_paths[0])
            dialog.setText(f"Are you sure you want to move this image to the trash?\n\n{truncated_path}")
        else:
            if num_selected <= 10:
                file_list = "\n".join([get_truncated_path(p) for p in deleted_file_paths])
                message = f"Are you sure you want to move {num_selected} images to the trash?\n\n{file_list}"
            else:
                file_list = "\n".join([get_truncated_path(p) for p in deleted_file_paths[:10]])
                message = f"Are you sure you want to move {num_selected} images to the trash?\n\n{file_list}\n\n... and {num_selected - 10} more"
            dialog.setText(message)
            dialog.setMinimumSize(600, 400)

        dialog.setIcon(QMessageBox.Icon.Warning)
        dialog.setStandardButtons(QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No)
        dialog.setDefaultButton(QMessageBox.StandardButton.Yes)

        yes_button = dialog.button(QMessageBox.StandardButton.Yes)
        if yes_button:
            yes_button.setObjectName("confirmDeleteYesButton")

        no_button = dialog.button(QMessageBox.StandardButton.No)
        if no_button:
            no_button.setObjectName("confirmDeleteNoButton")

        # Styling is handled by dark_theme.qss

        reply = dialog.exec()
        return reply == QMessageBox.StandardButton.Yes

    def show_potential_cache_overflow_warning(self, estimated_preview_data_needed_for_folder_bytes: int,
                                              preview_cache_limit_bytes: int):
        """
        Shows a warning about potential cache overflow.
        
        Args:
            estimated_preview_data_needed_for_folder_bytes: The estimated size of the folder's previews.
            preview_cache_limit_bytes: The current preview cache limit.
        """
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
        QMessageBox.warning(self.parent, "Potential Cache Overflow", warning_msg)

    def show_commit_deletions_dialog(self, count: int) -> bool:
        """
        Shows a confirmation dialog for committing marked deletions.
        
        Args:
            count: The number of files to be deleted.
            
        Returns:
            True if the user confirms, False otherwise.
        """
        reply = QMessageBox.question(
            self.parent, "Confirm Deletion",
            f"Are you sure you want to move {count} marked image(s) to trash?",
            QMessageBox.StandardButton.Yes | QMessageBox.StandardButton.No,
            QMessageBox.StandardButton.No
        )
        return reply == QMessageBox.StandardButton.Yes
    def show_model_not_found_dialog(self, model_path: str):
        """Show a dialog informing the user that the rotation model is missing."""
        dialog = QMessageBox(self.parent)
        dialog.setWindowTitle("Rotation Model Not Found")
        dialog.setIcon(QMessageBox.Icon.Warning)

        text = (
            f"The automatic rotation feature requires a model file that was not found at:\n"
            f"<b>{model_path}</b>\n\n"
            "Please download the model and place it in the correct directory to enable this feature."
        )
        dialog.setText(text)

        detailed_text = (
            "You can download the model from the official GitHub repository.\n\n"
            "1. Click 'Download Model' to open the releases page.\n"
            "2. Download the 'orientation_model_v1_0.9753.onnx' file.\n"
            "3. Place the downloaded file inside the 'models' folder in the application directory.\n"
            "4. Restart the application or re-run the rotation analysis."
        )
        dialog.setInformativeText(detailed_text)

        download_button = dialog.addButton("Download Model", QMessageBox.ButtonRole.ActionRole)
        ok_button = dialog.addButton(QMessageBox.StandardButton.Ok)

        dialog.setDefaultButton(ok_button)

        download_button.clicked.connect(lambda: webbrowser.open("https://github.com/duartebarbosadev/deep-image-orientation-detection/releases"))

        dialog.exec()