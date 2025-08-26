from typing import List, Tuple
import webbrowser
import os
import logging

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
)
from PyQt6.QtCore import Qt, QSize
from PyQt6.QtGui import QIcon
from PyQt6.QtWidgets import QApplication

from src.core.app_settings import (
    get_rotation_confirm_lossy,
    is_pytorch_cuda_available,
    get_preview_cache_size_gb,
    get_exif_cache_size_mb,
)
from src.core.image_processing.raw_image_processor import is_raw_extension
from src.core.image_features.model_rotation_detector import (
    ModelRotationDetector,
    ModelNotFoundError,
)

logger = logging.getLogger(__name__)


class DialogManager:
    """A manager class for handling the creation of dialogs."""

    def __init__(self, parent):
        """
        Initialize the DialogManager.

        Args:
            parent: The parent widget, typically the MainWindow.
        """
        self.parent = parent

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

        tech_items = [
            f"🧠 Embeddings: SentenceTransformer (CLIP) on {'GPU (CUDA)' if is_pytorch_cuda_available() else 'CPU'}",
            f"🤖 Rotation Model: ONNX Runtime on {onnx_provider}",
            f"🔍 {clustering_info}",
            "📋 Metadata: pyexiv2 • 🎨 Interface: PyQt6 • 🐍 Runtime: Python",
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
        github_button.clicked.connect(
            lambda: webbrowser.open("https://github.com/duartebarbosadev/PhotoSort")
        )
        github_layout.addWidget(github_button)

        content_layout.addLayout(github_layout)

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

        if block:
            dialog.exec()
            logger.info("Closed about dialog")
        else:  # non-blocking path for automated tests
            # Keep a reference to prevent garbage collection in tests
            self._about_dialog_ref = dialog  # type: ignore[attr-defined]
            dialog.show()
            logger.info("Showing about dialog (non-blocking mode)")

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

    def show_cache_management_dialog(self):
        """Show the cache management dialog."""
        logger.info("Showing cache management dialog")
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
                apply_auto_edits=self._should_apply_raw_processing(file_path),
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

        # Preload thumbnails with progress indication
        if deleted_file_paths:
            self.parent.show_loading_overlay(
                f"Loading previews for {len(deleted_file_paths)} images..."
            )
            QApplication.processEvents()  # Ensure overlay appears immediately

            def progress_callback(processed: int, total: int):
                self.parent.update_loading_text(
                    f"Loading previews for {len(deleted_file_paths)} images... ({processed}/{total})"
                )
                QApplication.processEvents()

            # Preload thumbnails for the files to be deleted
            self.parent.image_pipeline.preload_thumbnails(
                deleted_file_paths,
                apply_auto_edits=self._has_raw_images(deleted_file_paths),  # Enable RAW processing only if RAW files present
                progress_callback=progress_callback,
            )

            self.parent.hide_loading_overlay()
            QApplication.processEvents()

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
        QMessageBox.warning(self.parent, "Potential Cache Overflow", warning_msg)
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

        # Preload thumbnails with progress indication
        if marked_files:
            self.parent.show_loading_overlay(
                f"Loading previews for {len(marked_files)} images..."
            )
            QApplication.processEvents()  # Ensure overlay appears immediately

            def progress_callback(processed: int, total: int):
                self.parent.update_loading_text(
                    f"Loading previews for {len(marked_files)} images... ({processed}/{total})"
                )
                QApplication.processEvents()

            # Preload thumbnails for the files to be deleted
            self.parent.image_pipeline.preload_thumbnails(
                marked_files,
                apply_auto_edits=self._has_raw_images(marked_files),  # Enable RAW processing only if RAW files present
                progress_callback=progress_callback,
            )

            self.parent.hide_loading_overlay()
            QApplication.processEvents()

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
        # Preload thumbnails with progress indication
        if marked_files:
            self.parent.show_loading_overlay(
                f"Loading previews for {len(marked_files)} images..."
            )
            QApplication.processEvents()  # Ensure overlay appears immediately

            def progress_callback(processed: int, total: int):
                self.parent.update_loading_text(
                    f"Loading previews for {len(marked_files)} images... ({processed}/{total})"
                )
                QApplication.processEvents()

            # Preload thumbnails for the files to be deleted
            self.parent.image_pipeline.preload_thumbnails(
                marked_files,
                apply_auto_edits=self._has_raw_images(marked_files),  # Enable RAW processing only if RAW files present
                progress_callback=progress_callback,
            )

            self.parent.hide_loading_overlay()
            QApplication.processEvents()

        dialog = QDialog(self.parent)
        dialog.setWindowTitle("Confirm Close")
        dialog.setObjectName("closeConfirmationDialog")
        dialog.setModal(True)
        dialog.setMinimumSize(500, 300)

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
                apply_auto_edits=self._should_apply_raw_processing(file_path),
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
        ok_button = dialog.addButton(QMessageBox.StandardButton.Ok)

        dialog.setDefaultButton(ok_button)

        if download_button:
            download_button.clicked.connect(
                lambda: webbrowser.open(
                    "https://github.com/duartebarbosadev/deep-image-orientation-detection/releases"
                )
            )

        dialog.exec()
        logger.info("Closed model not found dialog")
