"""
Metadata Sidebar Widget for PhotoRanker
Displays comprehensive image metadata in a modern, elegant sidebar
"""

import os
import logging
from datetime import datetime, date
from typing import Dict, Any, Optional, List
from PyQt6.QtWidgets import (
    QWidget, QVBoxLayout, QHBoxLayout, QLabel, QFrame, QScrollArea,
    QSizePolicy, QTextEdit, QProgressBar, QApplication
)
from PyQt6.QtCore import Qt, QPropertyAnimation, QEasingCurve, QRect, pyqtSignal, QTimer
from PyQt6.QtGui import QFont, QPixmap, QPalette, QColor, QIcon, QPainter, QBrush, QLinearGradient

class MetadataCard(QFrame):
    """A card widget that displays a category of metadata with smooth animations"""
    
    def __init__(self, title: str, icon: str = "📷", parent=None):
        super().__init__(parent)
        self.setObjectName("metadataCard")
        self.title = title
        self.icon = icon
        self.is_expanded = True
        self.animation = None
        self.content_widget = None
        
        self.setup_ui()
        self.setup_animations()
    
    def setup_ui(self):
        """Setup the card UI structure"""
        self.setFrameStyle(QFrame.Shape.Box)
        self.setSizePolicy(QSizePolicy.Policy.Preferred, QSizePolicy.Policy.Maximum)
        
        main_layout = QVBoxLayout(self)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)
        
        # Header with title and expand/collapse functionality
        self.header = QFrame()
        self.header.setObjectName("cardHeader")
        self.header.setFixedHeight(40)
        self.header.setCursor(Qt.CursorShape.PointingHandCursor)
        
        header_layout = QHBoxLayout(self.header)
        header_layout.setContentsMargins(12, 8, 12, 8)
        
        # Icon and title
        self.icon_label = QLabel(self.icon)
        self.icon_label.setFont(QFont("Segoe UI Emoji", 14))
        self.title_label = QLabel(self.title)
        self.title_label.setFont(QFont("Segoe UI", 10, QFont.Weight.Bold))
        
        # Expand/collapse indicator
        self.expand_indicator = QLabel("▼")
        self.expand_indicator.setFont(QFont("Segoe UI", 8))
        
        header_layout.addWidget(self.icon_label)
        header_layout.addWidget(self.title_label)
        header_layout.addStretch()
        header_layout.addWidget(self.expand_indicator)
        
        # Content area
        self.content_widget = QWidget()
        self.content_widget.setObjectName("cardContent")
        self.content_widget.setSizePolicy(QSizePolicy.Policy.Preferred, QSizePolicy.Policy.Maximum)
        self.content_layout = QVBoxLayout(self.content_widget)
        self.content_layout.setContentsMargins(12, 8, 12, 12)
        self.content_layout.setSpacing(6)
        
        main_layout.addWidget(self.header)
        main_layout.addWidget(self.content_widget)
        
        # Connect click handler
        self.header.mousePressEvent = self.toggle_expanded
    
    def setup_animations(self):
        """Setup smooth expand/collapse animations"""
        self.animation = QPropertyAnimation(self.content_widget, b"maximumHeight")
        self.animation.setDuration(200)
        self.animation.setEasingCurve(QEasingCurve.Type.OutCubic)
    
    def toggle_expanded(self, event=None):
        """Toggle the expanded state of the card"""
        self.is_expanded = not self.is_expanded
        
        if self.is_expanded:
            self.expand_indicator.setText("▼")
            self.animation.setStartValue(0)
            self.animation.setEndValue(self.content_widget.sizeHint().height())
        else:
            self.expand_indicator.setText("▶")
            self.animation.setStartValue(self.content_widget.height())
            self.animation.setEndValue(0)
        
        self.animation.start()
    
    def add_info_row(self, label: str, value: str, value_color: str = None):
        """Add an information row to the card"""
        row = QHBoxLayout()
        row.setContentsMargins(0, 2, 0, 2)
        
        label_widget = QLabel(label + ":")
        label_widget.setObjectName("metadataLabel")
        label_widget.setFixedWidth(80)
        
        value_widget = QLabel(str(value) if value is not None else "N/A")
        value_widget.setObjectName("metadataValue")
        value_widget.setWordWrap(True)
        
        if value_color:
            value_widget.setStyleSheet(f"color: {value_color};")
        
        row.addWidget(label_widget)
        row.addWidget(value_widget, 1)
        
        self.content_layout.addLayout(row)
    
    def add_progress_bar(self, label: str, value: float, max_value: float = 100.0, 
                        color: str = "#0078D4"):
        """Add a progress bar visualization"""
        row = QVBoxLayout()
        row.setContentsMargins(0, 4, 0, 4)
        
        label_widget = QLabel(label)
        label_widget.setObjectName("metadataLabel")
        
        progress = QProgressBar()
        progress.setObjectName("metadataProgress")
        progress.setRange(0, int(max_value))
        progress.setValue(int(value))
        progress.setFixedHeight(6)
        progress.setTextVisible(False)
        
        # Custom style for the progress bar
        progress.setStyleSheet(f"""
            QProgressBar::chunk {{
                background-color: {color};
                border-radius: 3px;
            }}
        """)
        
        row.addWidget(label_widget)
        row.addWidget(progress)
        
        self.content_layout.addLayout(row)

class MetadataSidebar(QWidget):
    """Modern sidebar widget displaying comprehensive image metadata"""
    
    # Signal emitted when sidebar wants to be hidden
    hide_requested = pyqtSignal()
    
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setObjectName("metadataSidebar")
        self.current_image_path = None
        self.raw_metadata = {}
        
        self.setup_ui()
        self.setup_animations()
        
        # Update timer for smooth transitions
        self.update_timer = QTimer()
        self.update_timer.setSingleShot(True)
        self.update_timer.timeout.connect(self._delayed_update)
    
    
    def setup_ui(self):
        """Setup the sidebar UI"""
        self.setMinimumWidth(280)
        self.setMaximumWidth(400)
        self.setSizePolicy(QSizePolicy.Policy.Preferred, QSizePolicy.Policy.Expanding)
        
        main_layout = QVBoxLayout(self)
        main_layout.setContentsMargins(0, 0, 0, 0)
        main_layout.setSpacing(0)
        
        # Header with title and close button
        header = self.create_header()
        main_layout.addWidget(header)
        
        # Scrollable content area
        scroll_area = QScrollArea()
        scroll_area.setObjectName("metadataScrollArea")
        scroll_area.setWidgetResizable(True)
        scroll_area.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        scroll_area.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAsNeeded)
        
        # Content widget
        self.content_widget = QWidget()
        self.content_layout = QVBoxLayout(self.content_widget)
        self.content_layout.setContentsMargins(8, 8, 8, 8)
        self.content_layout.setSpacing(8)
        # Don't add stretch here - we'll add it after all cards
        
        scroll_area.setWidget(self.content_widget)
        main_layout.addWidget(scroll_area)
        
        # Initial state
        self.show_placeholder()
    
    def create_header(self):
        """Create the sidebar header"""
        header = QFrame()
        header.setObjectName("sidebarHeader")
        header.setFixedHeight(48)
        
        layout = QHBoxLayout(header)
        layout.setContentsMargins(16, 12, 16, 12)
        
        # Icon and title
        icon_label = QLabel("📋")
        icon_label.setFont(QFont("Segoe UI Emoji", 16))
        
        title_label = QLabel("Image Details")
        title_label.setFont(QFont("Segoe UI", 12, QFont.Weight.Bold))
        
        # Close button
        close_button = QLabel("✕")
        close_button.setObjectName("closeButton")
        close_button.setFont(QFont("Segoe UI", 12))
        close_button.setFixedSize(24, 24)
        close_button.setAlignment(Qt.AlignmentFlag.AlignCenter)
        close_button.setCursor(Qt.CursorShape.PointingHandCursor)
        close_button.mousePressEvent = lambda e: self.hide_requested.emit()
        
        layout.addWidget(icon_label)
        layout.addWidget(title_label)
        layout.addStretch()
        layout.addWidget(close_button)
        
        return header
    
    def setup_animations(self):
        """Setup sidebar animations"""
        self.slide_animation = QPropertyAnimation(self, b"geometry")
        self.slide_animation.setDuration(250)
        self.slide_animation.setEasingCurve(QEasingCurve.Type.OutCubic)
    
    def show_placeholder(self):
        """Show placeholder content when no image is selected"""
        self.clear_content()
        
        placeholder = QLabel("Select an image to view detailed metadata")
        placeholder.setObjectName("placeholderText")
        placeholder.setAlignment(Qt.AlignmentFlag.AlignCenter)
        placeholder.setWordWrap(True)
        placeholder.setStyleSheet("""
            QLabel#placeholderText {
                color: #777777;
                font-size: 11pt;
                padding: 40px 20px;
            }
        """)
        
        self.content_layout.insertWidget(0, placeholder)
    
    def clear_content(self):
        """Clear all content from the sidebar"""
        while self.content_layout.count() > 0:  # Clear everything
            child = self.content_layout.takeAt(0)
            if child.widget():
                child.widget().deleteLater()
    
    def update_metadata(self, image_path: str, metadata: Dict[str, Any],
                       raw_exif: Dict[str, Any] = None):
        """Update the sidebar with new metadata"""
        self.current_image_path = image_path
        self.raw_metadata = raw_exif or {}
        
        # Update immediately - no artificial delay needed since data is pre-cached
        self._delayed_update()
    
    def _delayed_update(self):
        """Perform the actual metadata update"""
        if not self.current_image_path:
            self.show_placeholder()
            return
        
        self.clear_content()
        
        try:
            # File Information Card
            self.add_file_info_card()
            
            # Camera & Capture Settings Card
            self.add_camera_settings_card()
            
            # Image Properties Card
            self.add_image_properties_card()
            
            # Technical Details Card
            self.add_technical_details_card()
            
            # Add stretch at the end to push all content up
            self.content_layout.addStretch()
            
        except Exception as e:
            logging.error(f"Error updating metadata sidebar: {e}", exc_info=True)
            self.show_error_message(str(e))
    
    def add_file_info_card(self):
        """Add file information card"""
        card = MetadataCard("File Information", "📁")
        
        if os.path.exists(self.current_image_path):
            # File name
            card.add_info_row("Name", os.path.basename(self.current_image_path))
            
            # File size - prefer from metadata if available
            file_size = self.raw_metadata.get("FileSize")
            if file_size and file_size != "Unknown":
                card.add_info_row("Size", str(file_size))
            else:
                # Fallback to filesystem size
                stat = os.stat(self.current_image_path)
                size_mb = stat.st_size / (1024 * 1024)
                if size_mb < 1:
                    size_str = f"{stat.st_size / 1024:.1f} KB"
                else:
                    size_str = f"{size_mb:.2f} MB"
                card.add_info_row("Size", size_str)
            
            # Modified date
            stat = os.stat(self.current_image_path)
            mod_time = datetime.fromtimestamp(stat.st_mtime)
            card.add_info_row("Modified", mod_time.strftime("%B %d, %Y"))
            
            # File extension
            ext = os.path.splitext(self.current_image_path)[1].upper()
            card.add_info_row("Format", ext.lstrip('.'))
        
        self.content_layout.insertWidget(-1, card)
    
    def add_camera_settings_card(self):
        """Add camera and capture settings card"""
        logging.info(f"[MetadataSidebar] add_camera_settings_card called")
        logging.info(f"[MetadataSidebar] Raw metadata keys available: {len(self.raw_metadata)} total")
        
        # Log some sample keys to debug
        sample_keys = list(self.raw_metadata.keys())[:20]
        logging.info(f"[MetadataSidebar] Sample metadata keys: {sample_keys}")
        
        card = MetadataCard("Camera & Settings", "📷")
        
        # Check if we have any camera-related metadata using pyexiv2 format
        make = (self.raw_metadata.get("Exif.Image.Make") or
                self.raw_metadata.get("EXIF:Make") or
                self.raw_metadata.get("Make"))
        model = (self.raw_metadata.get("Exif.Image.Model") or
                 self.raw_metadata.get("EXIF:Model") or
                 self.raw_metadata.get("Model"))
        
        logging.info(f"[MetadataSidebar] Camera make: '{make}', model: '{model}'")
        
        # Only add camera info if we have some data, or show that it's not available
        if make or model:
            if make and model:
                camera = f"{make} {model}"
            elif model:
                camera = model
            elif make:
                camera = make
            
            logging.info(f"[MetadataSidebar] Final camera string: '{camera}'")
            card.add_info_row("Camera", camera)
        else:
            # Check if this might be a screenshot or non-camera image
            filename = os.path.basename(self.current_image_path) if self.current_image_path else ""
            if any(term in filename.lower() for term in ["screenshot", "captura", "screen", "desktop"]):
                card.add_info_row("Source", "Screen capture")
            else:
                card.add_info_row("Camera", "No camera data available")
        
        # Only show lens and camera settings if we have camera data
        if make or model:
            # Lens information - check pyexiv2 format first
            lens = (self.raw_metadata.get("Exif.Photo.LensModel") or
                    self.raw_metadata.get("Exif.Photo.LensSpecification") or
                    self.raw_metadata.get("LensModel") or
                    self.raw_metadata.get("EXIF:LensModel") or
                    self.raw_metadata.get("LensInfo") or
                    self.raw_metadata.get("EXIF:LensInfo"))
            if lens:
                card.add_info_row("Lens", lens)
            
            # Capture settings - check pyexiv2 format first
            focal_length = (self.raw_metadata.get("Exif.Photo.FocalLength") or
                          self.raw_metadata.get("FocalLength") or
                          self.raw_metadata.get("EXIF:FocalLength"))
            if focal_length:
                # Clean up focal length display (remove 'mm' if already present)
                fl_str = str(focal_length)
                if not fl_str.endswith('mm'):
                    fl_str += "mm"
                card.add_info_row("Focal Length", fl_str)
            
            aperture = (self.raw_metadata.get("Exif.Photo.FNumber") or
                       self.raw_metadata.get("Exif.Photo.ApertureValue") or
                       self.raw_metadata.get("FNumber") or
                       self.raw_metadata.get("EXIF:FNumber") or
                       self.raw_metadata.get("EXIF:ApertureValue"))
            if aperture:
                if isinstance(aperture, str) and aperture.startswith('f/'):
                    card.add_info_row("Aperture", aperture)
                else:
                    card.add_info_row("Aperture", f"f/{aperture}")
            
            shutter_speed = (self.raw_metadata.get("ExposureTime") or
                           self.raw_metadata.get("EXIF:ExposureTime") or
                           self.raw_metadata.get("EXIF:ShutterSpeedValue"))
            if shutter_speed:
                # Format shutter speed nicely
                ss_str = str(shutter_speed)
                if not ss_str.endswith('s') and '/' in ss_str:
                    ss_str += "s"
                card.add_info_row("Shutter Speed", ss_str)
            
            iso = (self.raw_metadata.get("Exif.Photo.ISOSpeedRatings") or
                   self.raw_metadata.get("ISO") or
                   self.raw_metadata.get("EXIF:ISO") or
                   self.raw_metadata.get("EXIF:ISOSpeedRatings"))
            if iso:
                card.add_info_row("ISO", f"ISO {iso}")
            
            # Flash
            flash = (self.raw_metadata.get("Exif.Photo.Flash") or
                    self.raw_metadata.get("Flash") or
                    self.raw_metadata.get("EXIF:Flash"))
            if flash:
                card.add_info_row("Flash", str(flash))
        
        self.content_layout.insertWidget(-1, card)
    
    def add_technical_details_card(self):
        """Add technical image details card"""
        card = MetadataCard("Technical Details", "⚙️")
        
        # Color space
        color_space = (self.raw_metadata.get("Exif.ColorSpace.ColorSpace") or
                      self.raw_metadata.get("ColorSpace") or
                      self.raw_metadata.get("EXIF:ColorSpace"))
        if color_space:
            card.add_info_row("Color Space", str(color_space))
        
        # White balance
        white_balance = (self.raw_metadata.get("Exif.Photo.WhiteBalance") or
                        self.raw_metadata.get("WhiteBalance") or
                        self.raw_metadata.get("EXIF:WhiteBalance"))
        if white_balance:
            wb_map = {"0": "Auto", "1": "Manual", "Auto": "Auto", "Manual": "Manual"}
            wb_display = wb_map.get(str(white_balance), str(white_balance))
            card.add_info_row("White Balance", wb_display)
        
        # Metering mode
        metering = (self.raw_metadata.get("Exif.Photo.MeteringMode") or
                   self.raw_metadata.get("MeteringMode") or
                   self.raw_metadata.get("EXIF:MeteringMode"))
        if metering:
            metering_map = {
                "0": "Unknown", "1": "Average", "2": "Center-weighted",
                "3": "Spot", "4": "Multi-spot", "5": "Multi-segment", "6": "Partial"
            }
            metering_display = metering_map.get(str(metering), str(metering))
            card.add_info_row("Metering", metering_display)
        
        # Exposure mode
        exposure_mode = (self.raw_metadata.get("Exif.Photo.ExposureMode") or
                        self.raw_metadata.get("ExposureMode") or
                        self.raw_metadata.get("EXIF:ExposureMode"))
        if exposure_mode:
            exp_map = {"0": "Auto", "1": "Manual", "2": "Auto bracket"}
            exp_display = exp_map.get(str(exposure_mode), str(exposure_mode))
            card.add_info_row("Exposure Mode", exp_display)
        
        # Exposure compensation
        exp_compensation = (self.raw_metadata.get("Exif.Photo.ExposureCompensation") or
                           self.raw_metadata.get("ExposureCompensation") or
                           self.raw_metadata.get("EXIF:ExposureCompensation"))
        if exp_compensation:
            try:
                comp_val = float(exp_compensation)
                if comp_val > 0:
                    card.add_info_row("Exposure Comp.", f"+{comp_val:.1f} EV")
                elif comp_val < 0:
                    card.add_info_row("Exposure Comp.", f"{comp_val:.1f} EV")
                else:
                    card.add_info_row("Exposure Comp.", "0 EV")
            except:
                card.add_info_row("Exposure Comp.", str(exp_compensation))
        
        # Scene capture type
        scene_type = (self.raw_metadata.get("Exif.Photo.SceneCaptureType") or
                     self.raw_metadata.get("SceneCaptureType") or
                     self.raw_metadata.get("EXIF:SceneCaptureType"))
        if scene_type:
            scene_map = {"0": "Standard", "1": "Landscape", "2": "Portrait", "3": "Night"}
            scene_display = scene_map.get(str(scene_type), str(scene_type))
            card.add_info_row("Scene Type", scene_display)
        
        # Orientation
        orientation = (self.raw_metadata.get("Exif.Image.Orientation") or
                      self.raw_metadata.get("Orientation") or
                      self.raw_metadata.get("EXIF:Orientation"))
        if orientation:
            orient_map = {
                "1": "Normal", "2": "Flipped H", "3": "Rotated 180°",
                "4": "Flipped V", "5": "Rotated 90° CCW + Flipped H",
                "6": "Rotated 90° CW", "7": "Rotated 90° CW + Flipped H",
                "8": "Rotated 90° CCW"
            }
            orient_display = orient_map.get(str(orientation), f"Code {orientation}")
            card.add_info_row("Orientation", orient_display)
        
        # Software/Firmware
        software = (self.raw_metadata.get("Exif.Image.Software") or
                   self.raw_metadata.get("Software") or
                   self.raw_metadata.get("EXIF:Software"))
        if software:
            card.add_info_row("Software", str(software))
        
        self.content_layout.insertWidget(-1, card)
    
    def add_image_properties_card(self):
        """Add image properties card"""
        card = MetadataCard("Image Properties", "🖼️")
        
        # Dimensions - try multiple possible tag names, pyexiv2 format first
        width = (self.raw_metadata.get("pixel_width") or  # From basic metadata
                self.raw_metadata.get("Exif.Photo.PixelXDimension") or
                self.raw_metadata.get("Exif.Image.ImageWidth") or
                self.raw_metadata.get("EXIF:ImageWidth") or
                self.raw_metadata.get("ImageWidth") or
                self.raw_metadata.get("EXIF:ExifImageWidth"))
        height = (self.raw_metadata.get("pixel_height") or  # From basic metadata
                 self.raw_metadata.get("Exif.Photo.PixelYDimension") or
                 self.raw_metadata.get("Exif.Image.ImageLength") or
                 self.raw_metadata.get("EXIF:ImageHeight") or
                 self.raw_metadata.get("ImageHeight") or
                 self.raw_metadata.get("EXIF:ExifImageHeight"))
        
        if width and height:
            try:
                width_int = int(width)
                height_int = int(height)
                megapixels = (width_int * height_int) / 1_000_000
                card.add_info_row("Dimensions", f"{width_int} × {height_int}")
                card.add_info_row("Megapixels", f"{megapixels:.1f} MP")
            except (ValueError, TypeError):
                card.add_info_row("Dimensions", f"{width} × {height}")
        
        # Color space
        color_space = (self.raw_metadata.get("ColorSpace") or
                      self.raw_metadata.get("EXIF:ColorSpace"))
        if color_space:
            card.add_info_row("Color Space", str(color_space))
        
        # Orientation
        orientation = (self.raw_metadata.get("Orientation") or
                      self.raw_metadata.get("EXIF:Orientation"))
        if orientation:
            card.add_info_row("Orientation", str(orientation))
        
        # Bit depth
        bits_per_sample = (self.raw_metadata.get("BitsPerSample") or
                          self.raw_metadata.get("EXIF:BitsPerSample"))
        if bits_per_sample:
            card.add_info_row("Bit Depth", f"{bits_per_sample}-bit")
        
        self.content_layout.insertWidget(-1, card)
    
    
    
    def add_debug_metadata_card(self):
        """Add debug card showing raw metadata - for troubleshooting"""
        if not self.raw_metadata:
            return
            
        logging.info(f"[MetadataSidebar] add_debug_metadata_card called with {len(self.raw_metadata)} keys")
        card = MetadataCard("Debug: Raw Metadata", "🔍")
        
        # Show first 15 key-value pairs for debugging
        items_shown = 0
        for key, value in self.raw_metadata.items():
            if items_shown >= 15:  # Limit to avoid UI clutter
                card.add_info_row("...", f"(showing {items_shown} of {len(self.raw_metadata)} total)")
                break
            
            # Truncate very long values
            value_str = str(value)
            if len(value_str) > 50:
                value_str = value_str[:47] + "..."
            
            card.add_info_row(key, value_str)
            items_shown += 1
        
        if items_shown == 0:
            card.add_info_row("Status", "No metadata keys found")
        
        self.content_layout.insertWidget(-1, card)
    
    def show_error_message(self, error: str):
        """Show an error message in the sidebar"""
        self.clear_content()
        
        error_label = QLabel(f"Error loading metadata:\n{error}")
        error_label.setObjectName("errorText")
        error_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        error_label.setWordWrap(True)
        error_label.setStyleSheet("""
            QLabel#errorText {
                color: #C92C2C;
                font-size: 10pt;
                padding: 20px;
            }
        """)
        
        self.content_layout.insertWidget(0, error_label)