import os
import logging
from typing import Optional, List, Tuple, Dict, Any
from PyQt6.QtCore import Qt, QRectF, QPointF, pyqtSignal, QTimer, QSize
from PyQt6.QtGui import QPixmap, QWheelEvent, QMouseEvent, QKeyEvent, QPainter, QBrush, QColor
from PyQt6.QtWidgets import (QWidget, QVBoxLayout, QHBoxLayout, QGraphicsView, 
                           QGraphicsScene, QGraphicsPixmapItem, QFrame, QLabel,
                           QPushButton, QSlider, QCheckBox, QSplitter, QButtonGroup)

class ZoomableImageView(QGraphicsView):
    """
    Advanced image view with smooth zoom, pan, and coordinate tracking
    """
    # Signals
    zoom_changed = pyqtSignal(float, QPointF)  # zoom_factor, center_on
    pan_changed = pyqtSignal(QPointF)  # center_point
    coordinates_changed = pyqtSignal(QPointF)  # mouse coordinates in scene
    
    def __init__(self, parent=None):
        super().__init__(parent)
        
        # Zoom settings
        self._zoom_factor = 1.0
        self._min_zoom = 0.1
        self._max_zoom = 20.0
        self._zoom_step = 1.15
        
        # State tracking
        self._empty = True
        self._last_pan_point = QPointF()
        self._is_panning = False
        self._pending_pixmap = None  # For smooth transitions
        
        # Setup scene and view
        self._scene = QGraphicsScene(self)
        self._photo_item = QGraphicsPixmapItem()
        self._photo_item.setTransformationMode(Qt.TransformationMode.SmoothTransformation)
        self._scene.addItem(self._photo_item)
        self.setScene(self._scene)
        
        # View configuration - optimized for smooth rendering
        self.setTransformationAnchor(QGraphicsView.ViewportAnchor.AnchorUnderMouse)
        self.setResizeAnchor(QGraphicsView.ViewportAnchor.AnchorUnderMouse)
        self.setDragMode(QGraphicsView.DragMode.NoDrag)
        self.setVerticalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        self.setHorizontalScrollBarPolicy(Qt.ScrollBarPolicy.ScrollBarAlwaysOff)
        
        # Use dark gray background instead of black to reduce flash visibility
        self.setBackgroundBrush(QBrush(QColor(32, 32, 32)))
        
        self.setFrameShape(QFrame.Shape.NoFrame)
        
        # Enhanced render hints for smoother transitions
        self.setRenderHints(
            QPainter.RenderHint.Antialiasing | 
            QPainter.RenderHint.SmoothPixmapTransform |
            QPainter.RenderHint.TextAntialiasing
        )
        
        # Enable smart viewport updating for better performance
        self.setViewportUpdateMode(QGraphicsView.ViewportUpdateMode.SmartViewportUpdate)
        
        # Enable mouse tracking for coordinate display
        self.setMouseTracking(True)
        
        # Timer for debouncing resize events
        self._resize_timer = QTimer()
        self._resize_timer.setSingleShot(True)
        self._resize_timer.timeout.connect(self.fit_in_view)
        
    def resizeEvent(self, event):
        """Refit image to view on resize, with debouncing."""
        if self.has_image():
            self._resize_timer.start(50)  # 50ms debounce delay
        super().resizeEvent(event)

    def has_image(self) -> bool:
        """Check if an image is loaded"""
        return not self._empty
        
    def set_image(self, pixmap: QPixmap):
        """Set the image to display with smooth transition"""
        if pixmap and not pixmap.isNull():
            # CRITICAL: Set new image directly without clearing - prevents black flash
            self._photo_item.setPixmap(pixmap)
            self._empty = False
            self._scene.setSceneRect(QRectF(pixmap.rect()))
            
            # Only auto-fit if the view is properly initialized and visible
            if self.isVisible() and self.viewport().width() > 0 and self.viewport().height() > 0:
                # Minimal delay to ensure pixmap is rendered before fitting
                self.fit_in_view()
        else:
            # Only clear if we're actually setting an empty image
            if not self._empty:
                # Use transparent 1x1 pixmap instead of completely empty to reduce flash
                transparent_pixmap = QPixmap(1, 1)
                transparent_pixmap.fill(Qt.GlobalColor.transparent)
                self._photo_item.setPixmap(transparent_pixmap)
                self._empty = True
                
    def get_zoom_factor(self) -> float:
        """Get current zoom factor"""
        return self._zoom_factor
        
    def set_zoom_factor(self, factor: float, center_point: Optional[QPointF] = None):
        """Set zoom factor with optional center point"""
        factor = max(self._min_zoom, min(self._max_zoom, factor))
        
        if center_point is None:
            center_point = self.mapToScene(self.viewport().rect().center())
        
        # Prevent division by zero
        if self._zoom_factor <= 0:
            self._zoom_factor = 1.0
            self.resetTransform()
        
        # Calculate the scale change
        scale_change = factor / self._zoom_factor
        
        # Apply the zoom
        self.scale(scale_change, scale_change)
        self._zoom_factor = factor
        
        # Center on the specified point
        self.centerOn(center_point)
        
        self.zoom_changed.emit(self._zoom_factor, center_point)
        
    def zoom_in(self, center_point: Optional[QPointF] = None):
        """Zoom in by zoom step"""
        self.set_zoom_factor(self._zoom_factor * self._zoom_step, center_point)
        
    def zoom_out(self, center_point: Optional[QPointF] = None):
        """Zoom out by zoom step, snapping to the minimum zoom level."""
        # Calculate the potential new zoom factor
        new_factor = self._zoom_factor / self._zoom_step
        
        # If the new factor is below the minimum, clamp it to the minimum
        if new_factor < self._min_zoom:
            new_factor = self._min_zoom
            
        # Apply the zoom only if it results in a change
        if new_factor != self._zoom_factor:
            self.set_zoom_factor(new_factor, center_point)
        
    def fit_in_view(self):
        """Fit image to view while maintaining aspect ratio"""
        if self._empty:
            return
            
        rect = QRectF(self._photo_item.pixmap().rect())
        if not rect.isNull():
            self.setSceneRect(rect)
            # Use the built-in fitInView for robust fitting
            self.fitInView(rect, Qt.AspectRatioMode.KeepAspectRatio)
            # Update internal zoom factor to match the new transform
            self._zoom_factor = self.transform().m11()
            self._min_zoom = self._zoom_factor
            center_point = self.mapToScene(self.viewport().rect().center())
            self.zoom_changed.emit(self._zoom_factor, center_point)
            
    def zoom_to_actual_size(self):
        """Zoom to 100% (actual size)"""
        if self._empty:
            return
        self.set_zoom_factor(1.0)
        
    def wheelEvent(self, event: QWheelEvent):
        """Handle mouse wheel for zooming"""
        if self._empty:
            return
            
        # Get the mouse position in scene coordinates
        mouse_pos = self.mapToScene(event.position().toPoint())
        
        # Zoom in or out based on wheel direction
        if event.angleDelta().y() > 0:
            self.zoom_in(mouse_pos)
        else:
            self.zoom_out(mouse_pos)
            
    def mousePressEvent(self, event: QMouseEvent):
        """Handle mouse press for panning"""
        if event.button() == Qt.MouseButton.LeftButton:
            self._last_pan_point = event.position()
            self._is_panning = True
            self.setCursor(Qt.CursorShape.ClosedHandCursor)
        super().mousePressEvent(event)
        
    def mouseMoveEvent(self, event: QMouseEvent):
        """Handle mouse move for panning and coordinate tracking"""
        # Emit coordinates for tracking
        scene_pos = self.mapToScene(event.position().toPoint())
        self.coordinates_changed.emit(scene_pos)
        
        # Handle panning
        if self._is_panning and event.buttons() == Qt.MouseButton.LeftButton:
            delta = event.position() - self._last_pan_point
            self._last_pan_point = event.position()
            
            # Pan the view
            h_bar = self.horizontalScrollBar()
            v_bar = self.verticalScrollBar()
            h_bar.setValue(h_bar.value() - int(delta.x()))
            v_bar.setValue(v_bar.value() - int(delta.y()))
            
            center = self.mapToScene(self.viewport().rect().center())
            self.pan_changed.emit(center)
            
        super().mouseMoveEvent(event)
        
    def mouseReleaseEvent(self, event: QMouseEvent):
        """Handle mouse release"""
        if event.button() == Qt.MouseButton.LeftButton:
            self._is_panning = False
            self.setCursor(Qt.CursorShape.ArrowCursor)
        super().mouseReleaseEvent(event)
        
    def keyPressEvent(self, event: QKeyEvent):
        """Handle keyboard shortcuts"""
        if event.key() == Qt.Key.Key_Plus or event.key() == Qt.Key.Key_Equal:
            self.zoom_in()
        elif event.key() == Qt.Key.Key_Minus:
            self.zoom_out()
        elif event.key() == Qt.Key.Key_0:
            self.fit_in_view()
        elif event.key() == Qt.Key.Key_1:
            self.zoom_to_actual_size()
        else:
            super().keyPressEvent(event)
    
    def clear(self):
        """Clear the image display with smooth transition"""
        # Use transparent pixmap instead of completely empty to reduce flash
        self._empty = True
        transparent_pixmap = QPixmap(1, 1)
        transparent_pixmap.fill(Qt.GlobalColor.transparent)
        self._photo_item.setPixmap(transparent_pixmap)
        
    def setText(self, text: str):
        """Set text display with improved rendering"""
        import traceback
        
        
        # Only proceed if we actually want to show text
        if not text or text.strip() == "":
            return
        
        # Remove any existing text items first
        for item in self._scene.items():
            if hasattr(item, '_is_text_item'):
                self._scene.removeItem(item)
        
        # Only clear photo item after we're ready to show text
        transparent_pixmap = QPixmap(1, 1)
        transparent_pixmap.fill(Qt.GlobalColor.transparent)
        self._photo_item.setPixmap(transparent_pixmap)
        self._empty = True
        
        from PyQt6.QtWidgets import QGraphicsTextItem
        from PyQt6.QtGui import QFont
        
        text_item = QGraphicsTextItem(text)
        text_item._is_text_item = True
        
        # Style the text with better contrast
        font = QFont()
        font.setPointSize(14)
        text_item.setFont(font)
        text_item.setDefaultTextColor(QColor(180, 180, 180))  # Lighter for better contrast
        
        # Get viewport dimensions
        viewport_rect = self.viewport().rect()
        
        # Set scene rect to match viewport
        self._scene.setSceneRect(0, 0, viewport_rect.width(), viewport_rect.height())
        
        # Add text to scene
        self._scene.addItem(text_item)
        
        # Center the text in the scene
        text_rect = text_item.boundingRect()
        center_x = (viewport_rect.width() - text_rect.width()) / 2
        center_y = (viewport_rect.height() - text_rect.height()) / 2
        text_item.setPos(center_x, center_y)
        
        # Reset view to show the scene normally
        self.resetTransform()
        self._zoom_factor = 1.0
        self.fitInView(self._scene.sceneRect(), Qt.AspectRatioMode.KeepAspectRatio)

class IndividualViewer(QWidget):
    """
    A self-contained widget that holds a ZoomableImageView and its own
    rating/color controls. This allows for per-image controls in side-by-side view.
    """
    # Signals to bubble up to the main window/controller
    ratingChanged = pyqtSignal(str, int)  # file_path, rating
    labelChanged = pyqtSignal(str, str)   # file_path, label

    def __init__(self, parent=None):
        super().__init__(parent)
        self._file_path = None
        
        self._setup_ui()
        self._connect_signals()
        
    def _setup_ui(self):
        """Setup the layout with image view and control bar."""
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)
        
        self.image_view = ZoomableImageView()
        layout.addWidget(self.image_view, 1) # Image view takes up all available space
        
        # --- Control Bar ---
        self.control_bar = QFrame()
        self.control_bar.setObjectName("imageActionControls") # Use a specific object name for styling
        self.control_bar.setFixedHeight(35)
        
        control_layout = QHBoxLayout(self.control_bar)
        control_layout.setContentsMargins(10, 0, 10, 0)
        control_layout.setSpacing(15)
        
        control_layout.addStretch()
        
        # Rating controls
        self.rating_widget = self._create_rating_controls()
        control_layout.addWidget(self.rating_widget)
        
        # Color label controls
        self.color_widget = self._create_color_controls()
        control_layout.addWidget(self.color_widget)
        
        control_layout.addStretch()
        
        layout.addWidget(self.control_bar)

    def _create_rating_controls(self) -> QWidget:
        """Creates the star rating buttons widget."""
        widget = QWidget()
        layout = QHBoxLayout(widget)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(2)
        
        self.star_buttons = []
        for i in range(1, 6):
            btn = QPushButton("☆")
            btn.setProperty("ratingValue", i)
            layout.addWidget(btn)
            self.star_buttons.append(btn)
            
        self.clear_rating_button = QPushButton("X")
        self.clear_rating_button.setToolTip("Clear rating (0 stars)")
        layout.addWidget(self.clear_rating_button)
        
        return widget

    def _create_color_controls(self) -> QWidget:
        """Creates the color label buttons widget."""
        widget = QWidget()
        layout = QHBoxLayout(widget)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(3)
        
        self.color_buttons = {}
        colors = ["Red", "Yellow", "Green", "Blue", "Purple"]
        
        for color_name in colors:
            btn = QPushButton("")
            btn.setToolTip(f"Set label to {color_name}")
            btn.setProperty("labelValue", color_name)
            layout.addWidget(btn)
            self.color_buttons[color_name] = btn
            
        self.clear_color_label_button = QPushButton("X")
        self.clear_color_label_button.setToolTip("Clear color label")
        layout.addWidget(self.clear_color_label_button)
        
        return widget
        
    def _connect_signals(self):
        """Connect button signals to handlers."""
        for i, btn in enumerate(self.star_buttons):
            # Use lambda with default parameter to capture the rating value correctly
            btn.clicked.connect(lambda checked, rating=i+1: self._on_rating_button_clicked(rating))
        self.clear_rating_button.clicked.connect(lambda: self._on_rating_button_clicked(0))
        
        for btn in self.color_buttons.values():
            btn.clicked.connect(self._on_color_button_clicked)
        self.clear_color_label_button.clicked.connect(lambda: self._on_color_button_clicked(None))
        
    def _on_rating_button_clicked(self, rating_override=None):
        """Handle rating button clicks and emit signal."""
        if self._file_path is None: return
        
        rating = rating_override
        if rating is None:
            sender = self.sender()
            rating = sender.property("ratingValue")
        
        if rating is not None:
            self.ratingChanged.emit(self._file_path, rating)
            self.update_rating_display(rating) # Update own display immediately

    def _on_color_button_clicked(self, label_override=None):
        """Handle color button clicks and emit signal."""
        if self._file_path is None: return
        
        label = label_override
        if label is None and label_override is not False: # Distinguish None from clear button
            sender = self.sender()
            label = sender.property("labelValue")

        self.labelChanged.emit(self._file_path, label)
        self.update_label_display(label) # Update own display immediately

    def set_data(self, pixmap: QPixmap, file_path: str, rating: int, label: Optional[str]):
        """Set all data for the viewer at once."""
        self._file_path = file_path
        self.image_view.set_image(pixmap)
        self.update_rating_display(rating)
        self.update_label_display(label)

    def update_rating_display(self, rating: int):
        """Update the star buttons to reflect the current rating."""
        for i, btn in enumerate(self.star_buttons):
            btn.setText("★" if i < rating else "☆")

    def update_label_display(self, label: Optional[str]):
        """Update the color buttons to reflect the current label."""
        for color_name, btn in self.color_buttons.items():
            is_selected = (color_name == label)
            btn.setProperty("selected", is_selected)
            # Re-polish to apply stylesheet changes
            btn.style().unpolish(btn)
            btn.style().polish(btn)
            
        self.clear_color_label_button.setProperty("selected", label is None)
        self.clear_color_label_button.style().unpolish(self.clear_color_label_button)
        self.clear_color_label_button.style().polish(self.clear_color_label_button)

    def clear(self):
        """Clear the viewer and its associated data."""
        self._file_path = None
        self.image_view.clear()
        self.update_rating_display(0)
        self.update_label_display(None)

    def has_image(self) -> bool:
        """Check if the internal image view has an image."""
        return self.image_view.has_image()

class SynchronizedImageViewer(QWidget):
    """
    Container for synchronized IndividualViewers with a central toolbar.
    """
    # Forward signals from IndividualViewer instances
    ratingChanged = pyqtSignal(str, int)
    labelChanged = pyqtSignal(str, str)
    
    def __init__(self, parent=None):
        super().__init__(parent)
        
        self.image_viewers: List[IndividualViewer] = []
        self.sync_enabled = True
        self._updating_sync = False
        
        self._setup_ui()
        
    def _setup_ui(self):
        """Setup the user interface with a more modern, compact toolbar."""
        from PyQt6.QtGui import QIcon
        from PyQt6.QtWidgets import QStyle
        
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)

        # --- Toolbar ---
        self.controls_frame = QFrame()
        self.controls_frame.setObjectName("advancedViewerToolbar")
        controls_layout = QHBoxLayout(self.controls_frame)
        controls_layout.setContentsMargins(5, 0, 5, 0)
        controls_layout.setSpacing(8)

        # -- View Mode Buttons --
        self.view_mode_group = QButtonGroup(self)
        self.single_view_btn = QPushButton("Single")
        self.single_view_btn.setCheckable(True)
        self.single_view_btn.setChecked(True)
        self.single_view_btn.clicked.connect(lambda: self._set_view_mode("single"))
        self.view_mode_group.addButton(self.single_view_btn)
        controls_layout.addWidget(self.single_view_btn)

        self.side_by_side_btn = QPushButton("Side by Side")
        self.side_by_side_btn.setCheckable(True)
        self.side_by_side_btn.clicked.connect(lambda: self._set_view_mode("side_by_side"))
        self.view_mode_group.addButton(self.side_by_side_btn)
        controls_layout.addWidget(self.side_by_side_btn)
        
        separator = QFrame()
        separator.setFrameShape(QFrame.Shape.VLine)
        separator.setFrameShadow(QFrame.Shadow.Sunken)
        controls_layout.addWidget(separator)

        # -- Zoom Controls --
        self.zoom_out_btn = QPushButton(); self.zoom_out_btn.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_ArrowLeft)); self.zoom_out_btn.setToolTip("Zoom Out (-)"); self.zoom_out_btn.clicked.connect(self._zoom_out_all); controls_layout.addWidget(self.zoom_out_btn)
        self.zoom_slider = QSlider(Qt.Orientation.Horizontal); self.zoom_slider.setRange(10, 2000); self.zoom_slider.setValue(100); self.zoom_slider.setToolTip("Zoom"); self.zoom_slider.valueChanged.connect(self._zoom_slider_changed); controls_layout.addWidget(self.zoom_slider)
        self.zoom_in_btn = QPushButton(); self.zoom_in_btn.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_ArrowRight)); self.zoom_in_btn.setToolTip("Zoom In (+)"); self.zoom_in_btn.clicked.connect(self._zoom_in_all); controls_layout.addWidget(self.zoom_in_btn)
        self.zoom_label = QLabel("100%"); self.zoom_label.setMinimumWidth(45); controls_layout.addWidget(self.zoom_label)

        # -- Fit Buttons --
        self.fit_btn = QPushButton(); self.fit_btn.setIcon(self.style().standardIcon(QStyle.StandardPixmap.SP_FileDialogToParent)); self.fit_btn.setToolTip("Fit to View (0)"); self.fit_btn.clicked.connect(self._fit_all); controls_layout.addWidget(self.fit_btn)
        self.actual_size_btn = QPushButton("1:1"); self.actual_size_btn.setToolTip("Actual Size (100%) (1)"); self.actual_size_btn.clicked.connect(self._actual_size_all); controls_layout.addWidget(self.actual_size_btn)
        controls_layout.addStretch()

        # -- Sync Button --
        self.sync_button = QPushButton("Sync"); self.sync_button.setCheckable(True); self.sync_button.setChecked(True); self.sync_button.setToolTip("Synchronize Pan & Zoom"); self.sync_button.toggled.connect(self._toggle_sync); controls_layout.addWidget(self.sync_button)
        layout.addWidget(self.controls_frame)

        # --- Image Viewer Container ---
        self.viewer_splitter = QSplitter(Qt.Orientation.Horizontal); self.viewer_splitter.setObjectName("advancedViewerSplitter"); self.viewer_splitter.setHandleWidth(2); self.viewer_splitter.splitterMoved.connect(self._on_splitter_moved); layout.addWidget(self.viewer_splitter, 1)

        self._create_viewer()
        self._update_controls_visibility()
        
    def _on_splitter_moved(self, pos, index):
        QTimer.singleShot(50, self._fit_visible_images_after_layout_change)

    def _create_viewer(self) -> IndividualViewer:
        viewer = IndividualViewer()
        viewer.image_view.zoom_changed.connect(self._on_zoom_changed)
        viewer.image_view.pan_changed.connect(self._on_pan_changed)
        viewer.ratingChanged.connect(self.ratingChanged)
        viewer.labelChanged.connect(self.labelChanged)
        
        self.image_viewers.append(viewer)
        self.viewer_splitter.addWidget(viewer)
        return viewer

    def _get_current_view_mode(self):
        return "side_by_side" if self.side_by_side_btn.isChecked() else "single"

    def _toggle_sync(self, checked: bool):
        self.sync_enabled = checked

    def _update_controls_visibility(self):
        visible_viewers = sum(1 for v in self.image_viewers if v.isVisible())
        self.sync_button.setVisible(visible_viewers > 1)
        
        num_images_loaded = sum(1 for v in self.image_viewers if v.has_image())
        self.side_by_side_btn.setEnabled(num_images_loaded >= 2)

    def _set_view_mode(self, mode: str):
        if mode == "single":
            for i, viewer in enumerate(self.image_viewers):
                viewer.setVisible(i == 0)
            self.single_view_btn.setChecked(True)
        elif mode == "side_by_side":
            while len(self.image_viewers) < 2: self._create_viewer()
            for i, viewer in enumerate(self.image_viewers): viewer.setVisible(i < 2)
            self.side_by_side_btn.setChecked(True)
        
        if hasattr(self, 'viewer_splitter'):
            if mode == "side_by_side" and len(self.image_viewers) >= 2:
                total_width = self.viewer_splitter.width()
                self.viewer_splitter.setSizes([total_width // 2, total_width // 2] if total_width > 0 else [1, 1])
            else: self.viewer_splitter.setSizes([1] + [0] * (len(self.image_viewers) - 1))

        self._update_controls_visibility()
        QTimer.singleShot(0, self._fit_visible_images_after_layout_change)
        
    def _fit_visible_images_after_layout_change(self):
        for viewer in self.image_viewers:
            if viewer.isVisible() and viewer.has_image():
                viewer.image_view.fit_in_view()
            
    def set_image_data(self, image_data: Dict[str, Any], viewer_index: int = 0):
        if viewer_index < len(self.image_viewers):
            pixmap = image_data.get('pixmap')
            if pixmap:
                self.image_viewers[viewer_index].set_data(
                    pixmap,
                    image_data.get('path'),
                    image_data.get('rating', 0),
                    image_data.get('label')
                )
        self._set_view_mode("single")
        self._update_controls_visibility()

    def set_images_data(self, images_data: List[Dict[str, Any]]):
        if len(images_data) >= 2:
            self._set_view_mode("side_by_side")
            for i, data in enumerate(images_data[:2]):
                if i < len(self.image_viewers) and data.get('pixmap'):
                    self.image_viewers[i].set_data(
                        data['pixmap'], data['path'], data.get('rating', 0), data.get('label')
                    )
            for i in range(2, len(self.image_viewers)): self.image_viewers[i].clear()
        elif images_data:
            self.set_image_data(images_data[0], 0)
        self._update_controls_visibility()

    def clear(self):
        for viewer in self.image_viewers: viewer.clear()
        self._set_view_mode("single")
            
    def _zoom_in_all(self):
        for viewer in self.image_viewers:
            if viewer.isVisible(): viewer.image_view.zoom_in()
                
    def _zoom_out_all(self):
        for viewer in self.image_viewers:
            if viewer.isVisible(): viewer.image_view.zoom_out()
                
    def _fit_all(self):
        for viewer in self.image_viewers:
            if viewer.isVisible(): viewer.image_view.fit_in_view()
                
    def _actual_size_all(self):
        for viewer in self.image_viewers:
            if viewer.isVisible(): viewer.image_view.zoom_to_actual_size()
    
    def _ui_initialized(self) -> bool:
        return hasattr(self, 'viewer_splitter')
                
    def _zoom_slider_changed(self, value: int):
        if self._updating_sync: return
        zoom_factor = value / 100.0
        for viewer in self.image_viewers:
            if viewer.isVisible(): viewer.image_view.set_zoom_factor(zoom_factor)
                
    def _on_zoom_changed(self, zoom_factor: float, center_point: QPointF):
        if self._updating_sync: return
        sender_view = self.sender()
        if not isinstance(sender_view, ZoomableImageView): return

        self.zoom_label.setText(f"{int(zoom_factor * 100)}%")
        self._updating_sync = True
        self.zoom_slider.setValue(int(zoom_factor * 100))
        self._updating_sync = False
        
        if self.sync_enabled:
            sender_scene_rect = sender_view.sceneRect()
            norm_x = (center_point.x() - sender_scene_rect.left()) / sender_scene_rect.width() if sender_scene_rect.width() > 0 else 0.5
            norm_y = (center_point.y() - sender_scene_rect.top()) / sender_scene_rect.height() if sender_scene_rect.height() > 0 else 0.5
            normalized_pos = QPointF(norm_x, norm_y)

            self._updating_sync = True
            for viewer in self.image_viewers:
                if viewer.image_view != sender_view and viewer.isVisible() and viewer.has_image():
                    target_scene_rect = viewer.image_view.sceneRect()
                    if target_scene_rect.width() > 0:
                        target_x = target_scene_rect.left() + normalized_pos.x() * target_scene_rect.width()
                        target_y = target_scene_rect.top() + normalized_pos.y() * target_scene_rect.height()
                        viewer.image_view.set_zoom_factor(zoom_factor, QPointF(target_x, target_y))
                    else:
                        viewer.image_view.set_zoom_factor(zoom_factor)
            self._updating_sync = False

    def _on_pan_changed(self, center_point: QPointF):
        if not self.sync_enabled or self._updating_sync: return
        sender_view = self.sender()
        self._updating_sync = True
        for viewer in self.image_viewers:
            if viewer.image_view != sender_view and viewer.isVisible():
                viewer.image_view.centerOn(center_point)
        self._updating_sync = False
        
    def fit_to_viewport(self):
        for viewer in self.image_viewers:
            if viewer.isVisible() and viewer.has_image():
                viewer.image_view.fit_in_view()

    def setText(self, text: str):
        if self.image_viewers: self.image_viewers[0].image_view.setText(text)