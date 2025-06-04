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
    zoom_changed = pyqtSignal(float)  # zoom_factor
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
                QTimer.singleShot(1, self.fit_in_view)
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
        
        self.zoom_changed.emit(self._zoom_factor)
        
    def zoom_in(self, center_point: Optional[QPointF] = None):
        """Zoom in by zoom step"""
        self.set_zoom_factor(self._zoom_factor * self._zoom_step, center_point)
        
    def zoom_out(self, center_point: Optional[QPointF] = None):
        """Zoom out by zoom step"""
        self.set_zoom_factor(self._zoom_factor / self._zoom_step, center_point)
        
    def fit_in_view(self):
        """Fit image to view while maintaining aspect ratio"""
        if self._empty:
            return
            
        rect = QRectF(self._photo_item.pixmap().rect())
        if not rect.isNull():
            self.setSceneRect(rect)
            unity = self.transform().mapRect(QRectF(0, 0, 1, 1))
            self.scale(1 / unity.width(), 1 / unity.height())
            
            view_rect = self.viewport().rect()
            scene_rect = self.transform().mapRect(rect)
            
            factor = min(view_rect.width() / scene_rect.width(),
                        view_rect.height() / scene_rect.height())
            self.scale(factor, factor)
            self._zoom_factor = factor
            self.centerOn(rect.center())
            self.zoom_changed.emit(self._zoom_factor)
            
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
        
        # Debug: Print who's calling this method
        print(f"[DEBUG] setText called with: '{text}'")
        print("Call stack:")
        for line in traceback.format_stack()[-3:-1]:
            print(f"  {line.strip()}")
        
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

class SynchronizedImageViewer(QWidget):
    """
    Container for synchronized image viewers with controls
    """
    
    def __init__(self, parent=None):
        super().__init__(parent)
        
        # Zoom settings
        self._zoom_factor = 1.0
        self._min_zoom = 0.1
        self._max_zoom = 20.0
        self._zoom_step = 1.15
        
        self.image_viewers: List[ZoomableImageView] = []
        self.sync_enabled = True
        self._updating_sync = False
        
        self._setup_ui()
        self._connect_signals()
        
    def _setup_ui(self):
        """Setup the user interface"""
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(5)
        
        # Controls
        controls_frame = QFrame()
        controls_frame.setObjectName("advancedViewerControls")
        controls_frame.setFixedHeight(40)
        controls_frame.setStyleSheet("""
            QFrame#advancedViewerControls {
                background-color: #2B2B2B;
                border-bottom: 1px solid #404040;
                padding: 5px;
            }
            QPushButton {
                background-color: #333333;
                color: #C0C0C0;
                border: 1px solid #404040;
                padding: 4px 8px;
                border-radius: 3px;
                min-width: 60px;
            }
            QPushButton:hover {
                background-color: #3D3D3D;
                border-color: #505050;
            }
            QPushButton:checked {
                background-color: #0078D4;
                border-color: #005A9E;
                color: white;
            }
        """)
        controls_layout = QHBoxLayout(controls_frame)
        controls_layout.setContentsMargins(5, 5, 5, 5)
        controls_layout.setSpacing(5)
        
        print("[DEBUG] Creating controls frame")
        
        # Sync checkbox
        self.sync_checkbox = QCheckBox("Synchronize Views")
        self.sync_checkbox.setChecked(True)
        self.sync_checkbox.toggled.connect(self._toggle_sync)
        controls_layout.addWidget(self.sync_checkbox)
        print("[DEBUG] Added sync checkbox")
        
        # Zoom controls
        controls_layout.addWidget(QLabel("Zoom:"))
        
        self.zoom_out_btn = QPushButton("-")
        self.zoom_out_btn.setMaximumWidth(30)
        self.zoom_out_btn.clicked.connect(self._zoom_out_all)
        controls_layout.addWidget(self.zoom_out_btn)
        
        self.zoom_slider = QSlider(Qt.Orientation.Horizontal)
        self.zoom_slider.setMinimum(10)  # 0.1x zoom
        self.zoom_slider.setMaximum(2000)  # 20x zoom
        self.zoom_slider.setValue(100)  # 1x zoom
        self.zoom_slider.setMaximumWidth(200)
        self.zoom_slider.valueChanged.connect(self._zoom_slider_changed)
        controls_layout.addWidget(self.zoom_slider)
        
        self.zoom_in_btn = QPushButton("+")
        self.zoom_in_btn.setMaximumWidth(30)
        self.zoom_in_btn.clicked.connect(self._zoom_in_all)
        controls_layout.addWidget(self.zoom_in_btn)
        
        self.zoom_label = QLabel("100%")
        self.zoom_label.setMinimumWidth(50)
        controls_layout.addWidget(self.zoom_label)
        print("[DEBUG] Added zoom controls")
        
        # View controls
        controls_layout.addStretch()
        
        self.fit_btn = QPushButton("Fit")
        self.fit_btn.clicked.connect(self._fit_all)
        controls_layout.addWidget(self.fit_btn)
        
        self.actual_size_btn = QPushButton("100%")
        self.actual_size_btn.clicked.connect(self._actual_size_all)
        controls_layout.addWidget(self.actual_size_btn)
        
        # View mode buttons - ALWAYS create them
        self.view_mode_group = QButtonGroup()
        
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
        
        print("[DEBUG] Added view mode buttons")
        
        # ALWAYS show the controls frame initially
        controls_frame.show()
        layout.addWidget(controls_frame)
        print(f"[DEBUG] Added controls frame to layout, height: {controls_frame.height()}")
        
        # Image viewer container
        self.viewer_splitter = QSplitter(Qt.Orientation.Horizontal)
        self.viewer_splitter.splitterMoved.connect(self._on_splitter_moved)
        layout.addWidget(self.viewer_splitter)
        print("[DEBUG] Added viewer splitter")
        
        # Create initial viewer
        self._create_viewer()
        print("[DEBUG] Created initial viewer")
        
        # Force the layout to update
        self.updateGeometry()
        print("[DEBUG] UI setup complete")
        
    def _on_splitter_moved(self, pos, index):
        """Handle splitter movement to refit images"""
        print(f"[DEBUG] Splitter moved to position {pos}, index {index}")
        QTimer.singleShot(100, self._fit_visible_images_after_layout_change)

    def _create_viewer(self) -> ZoomableImageView:
        """Create a new image viewer"""
        viewer = ZoomableImageView()
        viewer.zoom_changed.connect(self._on_zoom_changed)
        viewer.pan_changed.connect(self._on_pan_changed)
        
        self.image_viewers.append(viewer)
        self.viewer_splitter.addWidget(viewer)
        
        return viewer

    def _update_view_mode_buttons(self, num_images: int):
        """Update view mode button visibility based on number of images and current mode"""
        print(f"[DEBUG] _update_view_mode_buttons called with num_images: {num_images}")
        
        if not hasattr(self, 'single_view_btn') or not hasattr(self, 'side_by_side_btn'):
            print("[DEBUG] View mode buttons not yet initialized, skipping update")
            return
        
        # Show buttons only when we have 2+ images AND we're actually in side-by-side mode
        current_mode = self._get_current_view_mode()
        should_show_buttons = num_images >= 2 and current_mode == "side_by_side"
        
        if should_show_buttons:
            print("[DEBUG] Showing view mode buttons (side-by-side mode active)")
            self.single_view_btn.show()
            self.side_by_side_btn.show()
        else:
            print("[DEBUG] Hiding view mode buttons (not in side-by-side mode)")
            self.single_view_btn.hide()
            self.side_by_side_btn.hide()

    
    def _get_current_view_mode(self):
        """Get the current view mode"""
        if hasattr(self, 'side_by_side_btn') and self.side_by_side_btn.isChecked():
            return "side_by_side"
        return "single"

    def _connect_signals(self):
        """Connect internal signals"""
        pass
        
    def _toggle_sync(self, enabled: bool):
        """Toggle synchronization between viewers"""
        self.sync_enabled = enabled
        
    def _set_view_mode(self, mode: str):
        """Set the view mode (single or side-by-side)"""
        print(f"[DEBUG] _set_view_mode called with mode: {mode}")
        
        if not hasattr(self, 'single_view_btn') or not hasattr(self, 'side_by_side_btn'):
            print("[DEBUG] View mode buttons not initialized, skipping button state update")
            buttons_available = False
        else:
            buttons_available = True
        
        if mode == "single":
            # Hide all but first viewer
            for i, viewer in enumerate(self.image_viewers):
                viewer.setVisible(i == 0)
            if buttons_available:
                self.single_view_btn.setChecked(True)
                self.side_by_side_btn.setChecked(False)
            print("[DEBUG] Set to single view mode")
            
            # Hide buttons when switching to single mode
            if buttons_available:
                self.single_view_btn.hide()
                self.side_by_side_btn.hide()
                
        elif mode == "side_by_side":
            # Show two viewers
            while len(self.image_viewers) < 2:
                self._create_viewer()
            
            for i, viewer in enumerate(self.image_viewers):
                viewer.setVisible(i < 2)
            if buttons_available:
                self.single_view_btn.setChecked(False)
                self.side_by_side_btn.setChecked(True)
            print("[DEBUG] Set to side-by-side view mode")
            
            # Show buttons when switching to side-by-side mode
            if buttons_available:
                self.single_view_btn.show()
                self.side_by_side_btn.show()
        
        # Force the splitter to update its layout
        if hasattr(self, 'viewer_splitter'):
            self.viewer_splitter.updateGeometry()
            
            if mode == "side_by_side" and len(self.image_viewers) >= 2:
                total_width = self.viewer_splitter.width()
                if total_width > 0:
                    self.viewer_splitter.setSizes([total_width // 2, total_width // 2])
                    print(f"[DEBUG] Set splitter sizes to equal: [{total_width // 2}, {total_width // 2}]")
                else:
                    self.viewer_splitter.setSizes([1, 1])
                    print("[DEBUG] Set splitter sizes to [1, 1] (fallback)")
            elif mode == "single":
                self.viewer_splitter.setSizes([1, 0])
                print("[DEBUG] Set splitter sizes to [1, 0] for single view")
            
            from PyQt6.QtWidgets import QApplication
            QApplication.processEvents()
            self._fit_visible_images_after_layout_change()
        
    def _fit_visible_images_after_layout_change(self):
        """Fit visible images to their new view sizes after a layout change"""
        print("[DEBUG] _fit_visible_images_after_layout_change called")
        for i, viewer in enumerate(self.image_viewers):
            if viewer.isVisible() and viewer.has_image():
                print(f"[DEBUG] Fitting image in viewer {i} to new layout")
                viewer.fit_in_view()
            
    def set_image(self, pixmap: QPixmap, viewer_index: int = 0):
        """Set image for specific viewer with smooth transition"""
        print(f"[DEBUG] set_image called with viewer_index: {viewer_index}")
        if viewer_index < len(self.image_viewers):
            viewer = self.image_viewers[viewer_index]
            
            # CRITICAL: Set image directly without clearing - prevents black flash
            viewer.set_image(pixmap)
            
            # Fit image with minimal delay if UI is ready
            if self._ui_initialized() and viewer.isVisible():
                QTimer.singleShot(1, lambda: viewer.fit_in_view())
        
        # Update view mode buttons only for primary viewer
        if viewer_index == 0 and self._ui_initialized():
            # Always switch to single mode when setting a single image
            self._set_view_mode("single")

    def set_images(self, pixmaps: List[QPixmap]):
        """Set images for side-by-side comparison with smooth transition"""
        print(f"[DEBUG] set_images called with {len(pixmaps)} pixmaps")
        
        if len(pixmaps) >= 2:
            # Switch to side-by-side mode and show buttons
            self._set_view_mode("side_by_side")
            
            # Set images simultaneously to avoid flashing
            for i, pixmap in enumerate(pixmaps[:2]):  # Max 2 images
                if i < len(self.image_viewers):
                    self.image_viewers[i].set_image(pixmap)
                print(f"[DEBUG] Set pixmap {i} in viewer {i}")
                
            # Clear any additional viewers
            for i in range(2, len(self.image_viewers)):
                QTimer.singleShot(i, self.image_viewers[i].clear)
        else:
            # Single image mode - hide buttons
            self._set_view_mode("single")
            if pixmaps and len(self.image_viewers) > 0:
                self.image_viewers[0].set_image(pixmaps[0])
            
            # Clear secondary viewers with tiny delays to prevent simultaneous flashing
            for i in range(1, len(self.image_viewers)):
                QTimer.singleShot(i, self.image_viewers[i].clear)

    def clear(self):
        """Clear the image display with smooth transition"""
        print("[DEBUG] clear() called")
        
        # Clear primary viewer immediately, others with tiny delays
        for i, viewer in enumerate(self.image_viewers):
            if i == 0:
                viewer.clear()
            else:
                QTimer.singleShot(i, viewer.clear)
            print(f"[DEBUG] Cleared viewer {i}")
        
        # Hide buttons when clearing and set to single mode
        self._set_view_mode("single")
            
    def _zoom_in_all(self):
        """Zoom in all visible viewers"""
        for viewer in self.image_viewers:
            if viewer.isVisible():
                viewer.zoom_in()
                
    def _zoom_out_all(self):
        """Zoom out all visible viewers"""
        for viewer in self.image_viewers:
            if viewer.isVisible():
                viewer.zoom_out()
                
    def _fit_all(self):
        """Fit all visible viewers"""
        for viewer in self.image_viewers:
            if viewer.isVisible():
                viewer.fit_in_view()
                
    def _actual_size_all(self):
        """Set all visible viewers to actual size"""
        for viewer in self.image_viewers:
            if viewer.isVisible():
                viewer.zoom_to_actual_size()
    
    def _ui_initialized(self) -> bool:
        """Check if the UI components are initialized"""
        return (hasattr(self, 'zoom_label') and 
                hasattr(self, 'zoom_slider') and
                hasattr(self, 'single_view_btn') and 
                hasattr(self, 'side_by_side_btn') and
                hasattr(self, 'viewer_splitter'))
                
    def _zoom_slider_changed(self, value: int):
        """Handle zoom slider changes"""
        if self._updating_sync:
            return
            
        zoom_factor = value / 100.0
        for viewer in self.image_viewers:
            if viewer.isVisible():
                viewer.set_zoom_factor(zoom_factor)
                
    def _on_zoom_changed(self, zoom_factor: float):
        """Handle zoom changes from viewers"""
        if self._updating_sync:
            return
            
        sender = self.sender()
        
        # Update zoom label
        if hasattr(self, 'zoom_label') and self.zoom_label is not None:
            self.zoom_label.setText(f"{int(zoom_factor * 100)}%")
        
        # Update slider
        if hasattr(self, 'zoom_slider') and self.zoom_slider is not None:
            self._updating_sync = True
            self.zoom_slider.setValue(int(zoom_factor * 100))
            self._updating_sync = False
        
        # Sync other viewers if enabled
        if self.sync_enabled:
            self._updating_sync = True
            for viewer in self.image_viewers:
                if viewer != sender and viewer.isVisible():
                    viewer.set_zoom_factor(zoom_factor)
            self._updating_sync = False

    def _on_pan_changed(self, center_point: QPointF):
        """Handle pan changes from viewers"""
        if not self.sync_enabled or self._updating_sync:
            return
            
        sender = self.sender()
        
        # Sync other viewers
        self._updating_sync = True
        for viewer in self.image_viewers:
            if viewer != sender and viewer.isVisible():
                viewer.centerOn(center_point)
        self._updating_sync = False
        
    def fit_to_viewport(self):
        """Fit the visible images to the viewport while maintaining aspect ratio."""
        for i, viewer in enumerate(self.image_viewers):
            if viewer.isVisible() and viewer.has_image():
                # For side-by-side mode, adjust viewer width to half the available space
                if self._get_current_view_mode() == "side_by_side" and i < 2:
                    # Set viewer size to half the splitter width
                    total_width = self.viewer_splitter.width()
                    viewer.setFixedWidth(total_width // 2)
                viewer.fit_in_view()

    def setText(self, text: str):
        """Set text display (for backward compatibility)"""
        if self.image_viewers:
            self.image_viewers[0].setText(text)