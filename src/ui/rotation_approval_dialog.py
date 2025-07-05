import logging
import os
from typing import Dict, List, Optional, Tuple
from PIL import Image
from PIL.ImageQt import ImageQt
from PyQt6.QtCore import Qt, pyqtSignal, QThread, QPropertyAnimation, QEasingCurve, QRect, QTimer
from PyQt6.QtGui import QPixmap, QIcon, QPalette, QFont
from PyQt6.QtWidgets import (
    QDialog, QVBoxLayout, QHBoxLayout, QPushButton, QLabel, QScrollArea,
    QWidget, QFrame, QSplitter, QCheckBox, QProgressBar, QApplication,
    QButtonGroup, QGridLayout, QSizePolicy, QSpacerItem
)

from src.core.image_pipeline import ImagePipeline


class RotationApprovalItem(QFrame):
    """Widget displaying a single image with rotation suggestion for approval."""
    
    approval_changed = pyqtSignal(str, bool)  # file_path, approved
    
    def __init__(self, file_path: str, net_rotation: int, model_rotation: int, image_pipeline: ImagePipeline, parent=None):
        super().__init__(parent)
        self.file_path = file_path
        self.net_rotation = net_rotation # For approval checkbox text and is_approved
        self.model_rotation = model_rotation # For "After Rotation" image display
        self.image_pipeline = image_pipeline
        self.approved = True  # Default to approved
        self.is_hovering = False
        
        # Modern card-like styling
        self.setFrameStyle(QFrame.Shape.NoFrame)
        self.setStyleSheet("""
            RotationApprovalItem {
                background-color: #2b2b2b;
                border: 1px solid #404040;
                border-radius: 8px;
                margin: 8px;
                padding: 0px;
            }
            RotationApprovalItem:hover {
                border-color: #0078d4;
                background-color: #323232;
            }
        """)
        
        # Add subtle shadow effect
        self.setAttribute(Qt.WidgetAttribute.WA_StyledBackground, True)
        self.setMinimumWidth(350)
        self.setMaximumWidth(400)
        
        self._setup_ui()

    def _setup_ui(self):
        """Setup the UI layout for this item."""
        layout = QVBoxLayout(self)
        layout.setContentsMargins(12, 12, 12, 12)
        layout.setSpacing(8)
        
        # Header with filename and rotation info
        header_layout = QHBoxLayout()
        header_layout.setContentsMargins(0, 0, 0, 0)
        
        filename = os.path.basename(self.file_path)
        self.filename_label = QLabel(filename)
        self.filename_label.setStyleSheet("""
            QLabel {
                color: #ffffff;
                font-weight: bold;
                font-size: 13px;
                background: transparent;
            }
        """)
        self.filename_label.setWordWrap(True)
        header_layout.addWidget(self.filename_label, 1)
        
        # Rotation badge
        if self.net_rotation != 0:
            if self.net_rotation == 90:
                rotation_text = "90° ↻"
            elif self.net_rotation == -90:
                rotation_text = "90° ↺"
            else:  # 180
                rotation_text = "180°"

            self.rotation_badge = QLabel(rotation_text)
            self.rotation_badge.setStyleSheet("""
                QLabel {
                    background-color: #0078d4;
                    color: white;
                    border-radius: 10px;
                    padding: 4px 8px;
                    font-weight: bold;
                    font-size: 11px;
                    min-width: 20px;
                }
            """)
            self.rotation_badge.setAlignment(Qt.AlignmentFlag.AlignCenter)
            header_layout.addWidget(self.rotation_badge)
        
        layout.addLayout(header_layout)
        
        # Image comparison area
        images_layout = QHBoxLayout()
        images_layout.setSpacing(12)
        
        # Before image
        before_container = QFrame()
        before_container.setStyleSheet("""
            QFrame {
                background-color: #1e1e1e;
                border: 1px solid #404040;
                border-radius: 6px;
            }
        """)
        before_layout = QVBoxLayout(before_container)
        before_layout.setContentsMargins(8, 8, 8, 8)
        before_layout.setSpacing(4)
        
        before_title = QLabel("Original")
        before_title.setStyleSheet("""
            QLabel {
                color: #cccccc;
                font-size: 11px;
                font-weight: bold;
                background: transparent;
            }
        """)
        before_title.setAlignment(Qt.AlignmentFlag.AlignCenter)
        before_layout.addWidget(before_title)
        
        self.before_image_label = QLabel("Loading...")
        self.before_image_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        self.before_image_label.setMinimumSize(150, 120)
        self.before_image_label.setStyleSheet("""
            QLabel {
                border: 1px solid #555;
                background-color: #2a2a2a;
                border-radius: 4px;
            }
        """)
        before_layout.addWidget(self.before_image_label)
        images_layout.addWidget(before_container)
        
        # Arrow indicator (only if rotation suggested)
        if self.net_rotation != 0:
            arrow_label = QLabel("→")
            arrow_label.setStyleSheet("""
                QLabel {
                    color: #0078d4;
                    font-size: 24px;
                    font-weight: bold;
                    background: transparent;
                }
            """)
            arrow_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
            arrow_label.setMinimumWidth(30)
            images_layout.addWidget(arrow_label)
            
            # After image
            after_container = QFrame()
            after_container.setStyleSheet("""
                QFrame {
                    background-color: #1e1e1e;
                    border: 1px solid #404040;
                    border-radius: 6px;
                }
            """)
            after_layout = QVBoxLayout(after_container)
            after_layout.setContentsMargins(8, 8, 8, 8)
            after_layout.setSpacing(4)
            
            after_title = QLabel("After Rotation")
            after_title.setStyleSheet("""
                QLabel {
                    color: #cccccc;
                    font-size: 11px;
                    font-weight: bold;
                    background: transparent;
                }
            """)
            after_title.setAlignment(Qt.AlignmentFlag.AlignCenter)
            after_layout.addWidget(after_title)
            
            self.after_image_label = QLabel("Loading...")
            self.after_image_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
            self.after_image_label.setMinimumSize(150, 120)
            self.after_image_label.setStyleSheet("""
                QLabel {
                    border: 1px solid #555;
                    background-color: #2a2a2a;
                    border-radius: 4px;
                }
            """)
            after_layout.addWidget(self.after_image_label)
            images_layout.addWidget(after_container)
        else:
            self.after_image_label = None
        
        layout.addLayout(images_layout)
        
        # Approval controls
        controls_layout = QHBoxLayout()
        controls_layout.setContentsMargins(0, 8, 0, 0)
        
        if self.net_rotation != 0:
            if self.net_rotation == 90:
                checkbox_text = "Apply 90° clockwise rotation"
            elif self.net_rotation == -90:
                checkbox_text = "Apply 90° counter-clockwise rotation"
            else:  # 180
                checkbox_text = f"Apply 180° rotation"
            self.approval_checkbox = QCheckBox(checkbox_text)
            self.approval_checkbox.setChecked(True)
            self.approval_checkbox.setStyleSheet("""
                QCheckBox {
                    color: #ffffff;
                    font-size: 12px;
                    spacing: 8px;
                    background: transparent;
                }
                QCheckBox::indicator {
                    width: 16px;
                    height: 16px;
                    border-radius: 3px;
                    border: 2px solid #666;
                    background-color: #2a2a2a;
                }
                QCheckBox::indicator:checked {
                    background-color: #0078d4;
                    border-color: #0078d4;
                }
                QCheckBox::indicator:checked:hover {
                    background-color: #106ebe;
                }
            """)
            self.approval_checkbox.toggled.connect(self._on_approval_changed)
            controls_layout.addWidget(self.approval_checkbox)
        else:
            no_rotation_label = QLabel("✓ Already properly oriented")
            no_rotation_label.setStyleSheet("""
                QLabel {
                    color: #4CAF50;
                    font-size: 12px;
                    font-weight: bold;
                    background: transparent;
                }
            """)
            controls_layout.addWidget(no_rotation_label)
            self.approval_checkbox = None
        
        controls_layout.addStretch()
        layout.addLayout(controls_layout)
    
    def set_pixmaps(self, before_pixmap: Optional[QPixmap], after_pixmap: Optional[QPixmap]):
        """Set the pixmaps for the 'before' and 'after' labels."""
        if before_pixmap and not before_pixmap.isNull():
            scaled_before = before_pixmap.scaled(150, 120, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation)
            self.before_image_label.setPixmap(scaled_before)
        else:
            self.before_image_label.setText("Load Error")

        if self.after_image_label:
            if after_pixmap and not after_pixmap.isNull():
                scaled_after = after_pixmap.scaled(150, 120, Qt.AspectRatioMode.KeepAspectRatio, Qt.TransformationMode.SmoothTransformation)
                self.after_image_label.setPixmap(scaled_after)
            else:
                self.after_image_label.setText("Load Error")
    
    def _get_rotation_transform(self, rotation_degrees: int):
        """Get QTransform for the specified rotation."""
        from PyQt6.QtGui import QTransform
        transform = QTransform()
        transform.rotate(rotation_degrees)
        return transform
    
    def _on_approval_changed(self, checked: bool):
        """Handle approval checkbox change."""
        self.approved = checked
        self.approval_changed.emit(self.file_path, checked)
    
    def is_approved(self) -> bool:
        """Return whether this rotation is approved."""
        return self.approved and self.net_rotation != 0


class DialogImageLoader(QThread):
    """Worker to load images for the rotation dialog in the background."""
    image_loaded = pyqtSignal(str, QPixmap, QPixmap) # path, before_pixmap, after_pixmap
    finished = pyqtSignal()

    def __init__(self, items_to_load: Dict[str, Dict], image_pipeline: ImagePipeline, apply_auto_edits: bool, parent=None):
        super().__init__(parent)
        self.items_to_load = items_to_load
        self.image_pipeline = image_pipeline
        self.apply_auto_edits = apply_auto_edits
        self.is_stopped = False

    def stop(self):
        self.is_stopped = True

    def run(self):
        for path, data in self.items_to_load.items():
            if self.is_stopped:
                break
            
            try:
                # Before Image (EXIF Corrected)
                before_pixmap = self.image_pipeline.get_preview_qpixmap(
                    path, display_max_size=(400, 300), apply_auto_edits=self.apply_auto_edits
                )

                # After Image (Model Corrected)
                raw_pil_image = self.image_pipeline.get_pil_image_for_processing(
                    path, target_mode="RGBA", apply_auto_edits=True,
                    use_preloaded_preview_if_available=False, apply_exif_transpose=False
                )
                
                after_pixmap = None
                if raw_pil_image:
                    raw_pil_image.thumbnail((400, 300), Image.Resampling.LANCZOS)
                    raw_pixmap = QPixmap.fromImage(ImageQt(raw_pil_image))
                    from PyQt6.QtGui import QTransform
                    transform = QTransform()
                    transform.rotate(data['model_rotation'])
                    after_pixmap = raw_pixmap.transformed(transform, Qt.TransformationMode.SmoothTransformation)
                
                if not self.is_stopped:
                    self.image_loaded.emit(path, before_pixmap, after_pixmap)
            
            except Exception as e:
                logging.error(f"Error loading image '{path}' for dialog worker: {e}")
        
        self.finished.emit()


class RotationApprovalDialog(QDialog):
    """Dialog for reviewing and approving automatic rotation suggestions."""
    
    def __init__(self, rotation_suggestions: Dict[str, int], image_pipeline: ImagePipeline, apply_auto_edits: bool, parent=None):
        super().__init__(parent)
        self.rotation_suggestions = rotation_suggestions
        self.image_pipeline = image_pipeline
        self.apply_auto_edits = apply_auto_edits
        self.approved_rotations: Dict[str, int] = {}
        self.show_all_images = False  # Toggle for showing all vs only needing rotation
        self.approval_items_map: Dict[str, RotationApprovalItem] = {}
        self.image_loader_thread: Optional[DialogImageLoader] = None

        # Filter to only show images that need rotation
        # rotation_suggestions now contains {'net_rotation': int, 'model_rotation': int}
        self.items_needing_rotation = {
            path: data for path, data in rotation_suggestions.items()
            if data['net_rotation'] != 0
        }
        
        self.setWindowTitle("Auto Rotation Analysis Results")
        self.setModal(True)
        self.resize(1000, 700)
        
        # Modern dialog styling
        self.setStyleSheet("""
            QDialog {
                background-color: #1e1e1e;
                color: #ffffff;
            }
            QScrollArea {
                border: none;
                background-color: #2b2b2b;
            }
            QScrollBar:vertical {
                background-color: #3c3c3c;
                width: 12px;
                border-radius: 6px;
            }
            QScrollBar::handle:vertical {
                background-color: #5a5a5a;
                border-radius: 6px;
                min-height: 20px;
            }
            QScrollBar::handle:vertical:hover {
                background-color: #6a6a6a;
            }
        """)
        
        self._setup_ui()
        self._populate_items()
        self._start_image_loader()
    
    def _setup_ui(self):
        """Setup the main dialog UI."""
        layout = QVBoxLayout(self)
        layout.setContentsMargins(20, 20, 20, 20)
        layout.setSpacing(16)
        
        # Header section
        header_frame = QFrame()
        header_frame.setStyleSheet("""
            QFrame {
                background-color: #2b2b2b;
                border: 1px solid #404040;
                border-radius: 8px;
                padding: 12px;
            }
        """)
        header_layout = QVBoxLayout(header_frame)
        header_layout.setContentsMargins(16, 16, 16, 16)
        
        # Title and stats
        title_layout = QHBoxLayout()
        
        total_images = len(self.rotation_suggestions)
        needs_rotation = len(self.items_needing_rotation)
        
        title_label = QLabel("🔄 Auto Rotation Analysis")
        title_label.setStyleSheet("""
            QLabel {
                color: #ffffff;
                font-size: 18px;
                font-weight: bold;
                background: transparent;
            }
        """)
        title_layout.addWidget(title_label)
        
        title_layout.addStretch()
        
        # Statistics
        stats_label = QLabel(f"Analyzed: {total_images} • Need rotation: {needs_rotation}")
        stats_label.setStyleSheet("""
            QLabel {
                color: #cccccc;
                font-size: 13px;
                background: transparent;
            }
        """)
        title_layout.addWidget(stats_label)
        
        header_layout.addLayout(title_layout)
        
        # Controls row
        controls_layout = QHBoxLayout()
        
        # View toggle
        self.view_toggle_btn = QPushButton("Show All Images" if not self.show_all_images else "Show Only Needing Rotation")
        self.view_toggle_btn.clicked.connect(self._toggle_view_mode)
        self.view_toggle_btn.setStyleSheet("""
            QPushButton {
                background-color: #404040;
                color: #ffffff;
                border: 1px solid #555;
                border-radius: 6px;
                padding: 8px 16px;
                font-size: 12px;
            }
            QPushButton:hover {
                background-color: #4a4a4a;
                border-color: #666;
            }
        """)
        controls_layout.addWidget(self.view_toggle_btn)
        
        controls_layout.addStretch()
        
        # Selection buttons
        select_all_btn = QPushButton("✓ Select All")
        select_all_btn.clicked.connect(self._select_all)
        select_all_btn.setStyleSheet("""
            QPushButton {
                background-color: #0078d4;
                color: white;
                border: none;
                border-radius: 6px;
                padding: 8px 16px;
                font-size: 12px;
                font-weight: bold;
            }
            QPushButton:hover {
                background-color: #106ebe;
            }
        """)
        controls_layout.addWidget(select_all_btn)
        
        select_none_btn = QPushButton("✗ Select None")
        select_none_btn.clicked.connect(self._select_none)
        select_none_btn.setStyleSheet("""
            QPushButton {
                background-color: #6c757d;
                color: white;
                border: none;
                border-radius: 6px;
                padding: 8px 16px;
                font-size: 12px;
            }
            QPushButton:hover {
                background-color: #5a6268;
            }
        """)
        controls_layout.addWidget(select_none_btn)
        
        header_layout.addLayout(controls_layout)
        layout.addWidget(header_frame)
        
        # Scroll area for items
        scroll_area = QScrollArea()
        scroll_area.setWidgetResizable(True)
        scroll_area.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)
        scroll_area.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)
        scroll_area.setStyleSheet("""
            QScrollArea {
                border: 1px solid #404040;
                border-radius: 8px;
                background-color: #2b2b2b;
            }
        """)
        
        self.scroll_widget = QWidget()
        self.scroll_layout = QVBoxLayout(self.scroll_widget)  # Use a vertical layout for a list of items
        self.scroll_layout.setContentsMargins(16, 16, 16, 16)
        self.scroll_layout.setSpacing(12)
        scroll_area.setWidget(self.scroll_widget)
        
        layout.addWidget(scroll_area)
        
        # Bottom buttons
        button_frame = QFrame()
        button_frame.setStyleSheet("""
            QFrame {
                background-color: #2b2b2b;
                border: 1px solid #404040;
                border-radius: 8px;
            }
        """)
        button_layout = QHBoxLayout(button_frame)
        button_layout.setContentsMargins(16, 12, 16, 12)
        
        # Info label
        self.selection_info_label = QLabel()
        self.selection_info_label.setStyleSheet("""
            QLabel {
                color: #cccccc;
                font-size: 12px;
                background: transparent;
            }
        """)
        button_layout.addWidget(self.selection_info_label)
        
        button_layout.addStretch()
        
        cancel_btn = QPushButton("Cancel")
        cancel_btn.clicked.connect(self.reject)
        cancel_btn.setStyleSheet("""
            QPushButton {
                background-color: #6c757d;
                color: white;
                border: none;
                border-radius: 6px;
                padding: 10px 20px;
                font-size: 13px;
                min-width: 80px;
            }
            QPushButton:hover {
                background-color: #5a6268;
            }
        """)
        button_layout.addWidget(cancel_btn)
        
        self.apply_btn = QPushButton("Apply Selected Rotations")
        self.apply_btn.clicked.connect(self.accept)
        self.apply_btn.setDefault(True)
        self.apply_btn.setStyleSheet("""
            QPushButton {
                background-color: #0078d4;
                color: white;
                border: none;
                border-radius: 6px;
                padding: 10px 20px;
                font-size: 13px;
                font-weight: bold;
                min-width: 120px;
            }
            QPushButton:hover {
                background-color: #106ebe;
            }
            QPushButton:default {
                border: 2px solid #4a9eff;
            }
        """)
        button_layout.addWidget(self.apply_btn)
        
        layout.addWidget(button_frame)
    
    def _toggle_view_mode(self):
        """Toggle between showing all images vs only those needing rotation."""
        self.show_all_images = not self.show_all_images
        new_mode = "all images" if self.show_all_images else "only needing rotation"
        logging.info(f"Rotation dialog view mode toggled to show {new_mode}.")
        self.view_toggle_btn.setText("Show All Images" if not self.show_all_images else "Show Only Needing Rotation")
        self._populate_items()

    def _populate_items(self):
        """Populate the scroll area with rotation approval items."""
        # Clear existing widgets from the layout
        while self.scroll_layout.count():
            child = self.scroll_layout.takeAt(0)
            if child and child.widget():
                child.widget().deleteLater()

        self.approval_items_map: Dict[str, RotationApprovalItem] = {}

        items_to_populate = self.rotation_suggestions if self.show_all_images else self.items_needing_rotation
        
        if not items_to_populate:
            info_text = "Analyzed images are all correctly oriented." if self.show_all_images else "No images found that require rotation."
            info_label = QLabel(f"<i>{info_text}</i>")
            info_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
            self.scroll_layout.addWidget(info_label)
        else:
            # Sort to always show images needing rotation first, then by path
            sorted_paths = sorted(items_to_populate.keys(), key=lambda p: (self.rotation_suggestions[p]['net_rotation'] == 0, p))
            for path in sorted_paths:
                data = self.rotation_suggestions[path]
                net_rotation = data['net_rotation']
                model_rotation = data['model_rotation']
                item = RotationApprovalItem(path, net_rotation, model_rotation, self.image_pipeline)
                item.approval_changed.connect(self._on_item_approval_changed)
                self.approval_items_map[path] = item
                self.scroll_layout.addWidget(item)

        self.scroll_layout.addStretch()
        self._update_approved_rotations()
        self._start_image_loader()
    
    def _on_item_approval_changed(self, file_path: str, approved: bool):
        """Handle when an item's approval status changes."""
        self._update_approved_rotations()
    
    def _update_approved_rotations(self):
        """Update the approved rotations dictionary."""
        self.approved_rotations = {}
        for item in self.approval_items_map.values():
            if item.is_approved():
                self.approved_rotations[item.file_path] = item.net_rotation # Ensure net_rotation is stored for approval
    
    def _select_all(self):
        """Select all rotation suggestions."""
        logging.info("Rotation dialog 'Select All' clicked.")
        for item in self.approval_items_map.values():
            if item.approval_checkbox and item.approval_checkbox.isEnabled():
                item.approval_checkbox.setChecked(True)
    
    def _select_none(self):
        """Deselect all rotation suggestions."""
        logging.info("Rotation dialog 'Select None' clicked.")
        for item in self.approval_items_map.values():
            if item.approval_checkbox and item.approval_checkbox.isEnabled():
                item.approval_checkbox.setChecked(False)
    
    def get_approved_rotations(self) -> Dict[str, int]:
        """Get the approved rotations."""
        return self.approved_rotations.copy()

    def _start_image_loader(self):
        """Starts the background thread for loading images."""
        # Stop any existing loader thread before starting a new one
        try:
            if self.image_loader_thread and self.image_loader_thread.isRunning():
                self.image_loader_thread.stop()
                self.image_loader_thread.wait()
        except RuntimeError:
            logging.warning("Previous image loader thread was already deleted. Creating a new one.")
            self.image_loader_thread = None

        items_to_load = self.rotation_suggestions if self.show_all_images else self.items_needing_rotation
        
        # Create and hold a persistent reference to the thread
        self.image_loader_thread = DialogImageLoader(items_to_load, self.image_pipeline, self.apply_auto_edits, self)
        self.image_loader_thread.image_loaded.connect(self._on_image_loaded)
        
        # Clean up the thread object once it has finished executing
        self.image_loader_thread.finished.connect(self.image_loader_thread.deleteLater)
        
        self.image_loader_thread.start()

    def _on_image_loaded(self, path: str, before_pixmap: QPixmap, after_pixmap: QPixmap):
        """Slot to update an item when its images are loaded."""
        if path in self.approval_items_map:
            self.approval_items_map[path].set_pixmaps(before_pixmap, after_pixmap)
    
    def closeEvent(self, event):
        """Ensure the image loader thread is stopped when the dialog closes."""
        try:
            if self.image_loader_thread and self.image_loader_thread.isRunning():
                logging.debug("Dialog closing, stopping image loader thread.")
                self.image_loader_thread.stop()
                self.image_loader_thread.wait(1000) # Wait up to 1 second
        except RuntimeError:
            # This can happen if the thread object is already deleted, which is fine.
            logging.debug("Image loader thread was already deleted when closing dialog.")
            pass
        super().closeEvent(event)


class RotationDetectionWorker(QThread):
    """Worker thread for detecting rotation suggestions in images."""
    
    progress_update = pyqtSignal(int, int, str)  # current, total, basename
    rotation_detected = pyqtSignal(str, int)  # image_path, suggested_rotation
    model_not_found = pyqtSignal(str) # model_path
    finished = pyqtSignal()
    error = pyqtSignal(str)
    
    def __init__(self, image_paths: List[str], image_pipeline: ImagePipeline, apply_auto_edits: bool = False, parent=None):
        super().__init__(parent)
        self.image_paths = image_paths
        self.image_pipeline = image_pipeline
        self.apply_auto_edits = apply_auto_edits
        self._should_stop = False
    
    def stop(self):
        """Request the worker to stop."""
        self._should_stop = True
    
    def run(self):
        """Run the rotation detection process."""
        try:
            from src.core.image_features.rotation_detector import RotationDetector
            from src.core.image_features.model_rotation_detector import ModelNotFoundError
            
            def result_callback(image_path: str, suggested_rotation: int):
                if not self._should_stop:
                    self.rotation_detected.emit(image_path, suggested_rotation)
            
            def progress_callback(current: int, total: int, basename: str):
                if not self._should_stop:
                    self.progress_update.emit(current, total, basename)
            
            def should_continue_callback() -> bool:
                return not self._should_stop
            
            # Pass the image pipeline instance to the detector
            detector = RotationDetector(self.image_pipeline)
            detector.detect_rotation_in_batch(
                image_paths=self.image_paths,
                result_callback=result_callback,
                progress_callback=progress_callback,
                should_continue_callback=should_continue_callback
            )
            
            if not self._should_stop:
                self.finished.emit()
        
        except ModelNotFoundError as e:
            logging.error(f"Rotation model not found during worker execution: {e}")
            if not self._should_stop:
                self.model_not_found.emit(str(e)) # Emit the model path
        except Exception as e:
            logging.error(f"Error in rotation detection worker: {e}")
            if not self._should_stop:
                self.error.emit(str(e))