import logging
from typing import Optional, List, Dict, Any
from PyQt6.QtCore import Qt, QRectF, QPointF, pyqtSignal, QTimer, QPoint
from PyQt6.QtGui import (
    QPixmap,
    QWheelEvent,
    QMouseEvent,
    QKeyEvent,
    QPainter,
    QBrush,
    QColor,
    QAction,
    QIcon,
)
from PyQt6.QtWidgets import (
    QWidget,
    QVBoxLayout,
    QHBoxLayout,
    QGraphicsView,
    QGraphicsScene,
    QGraphicsPixmapItem,
    QFrame,
    QLabel,
    QPushButton,
    QSlider,
    QSplitter,
    QButtonGroup,
    QMenu,
    QStyle,
)

logger = logging.getLogger(__name__)


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
        self._min_zoom = 0.01  # Fixed minimum zoom at 1% instead of dynamic
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
        self._photo_item.setTransformationMode(
            Qt.TransformationMode.SmoothTransformation
        )
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
            QPainter.RenderHint.Antialiasing
            | QPainter.RenderHint.SmoothPixmapTransform
            | QPainter.RenderHint.TextAntialiasing
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
        logger.debug(f"ZoomableImageView set image called with pixmap: {pixmap}")
        if pixmap and not pixmap.isNull():
            # Remove any lingering text items before showing the new image
            for item in self._scene.items():
                if hasattr(item, "_is_text_item"):
                    self._scene.removeItem(item)

            # CRITICAL: Set new image directly without clearing - prevents black flash
            self._photo_item.setPixmap(pixmap)
            self._empty = False
            self._scene.setSceneRect(QRectF(pixmap.rect()))

            # Only auto-fit if the view is properly initialized and visible
            viewport = self.viewport()
            if (
                self.isVisible()
                and viewport
                and viewport.width() > 0
                and viewport.height() > 0
            ):
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

    def current_pixmap(self) -> Optional[QPixmap]:
        """Return the currently displayed pixmap if available."""
        pixmap = self._photo_item.pixmap()
        if pixmap and not pixmap.isNull():
            return pixmap
        return None

    def get_zoom_factor(self) -> float:
        """Get current zoom factor"""
        return self._zoom_factor

    def get_display_zoom_percentage(self) -> int:
        """Get zoom percentage for display, where minimum zoom shows as 0%"""
        if self._zoom_factor <= self._min_zoom:
            return 0
        else:
            # Calculate percentage above minimum zoom
            # When at 1.0x (100% actual size), show 100%
            # Scale the display so min_zoom = 0% and 1.0 = 100%
            if self._min_zoom >= 1.0:
                # If minimum zoom is already >= 100%, use actual percentage
                return int(self._zoom_factor * 100)
            else:
                # Scale from min_zoom=0% to 1.0=100%, and beyond for higher zoom levels
                if self._zoom_factor <= 1.0:
                    # Scale from min_zoom=0% to 1.0=100%
                    normalized = (self._zoom_factor - self._min_zoom) / (
                        1.0 - self._min_zoom
                    )
                    return max(0, int(normalized * 100))
                else:
                    # Above 1.0, show actual percentage (100%+ for zoom levels above 1.0)
                    base_percentage = 100  # 100% at 1.0x zoom
                    additional_percentage = (self._zoom_factor - 1.0) * 100
                    return int(base_percentage + additional_percentage)

    def set_zoom_factor(self, factor: float, center_point: Optional[QPointF] = None):
        """Set zoom factor with optional center point"""
        factor = max(self._min_zoom, min(self._max_zoom, factor))

        # If no center point is provided (e.g., keyboard zoom),
        # use the current viewport center in scene coordinates.
        if center_point is None:
            viewport = self.viewport()
            if viewport:
                center_point = self.mapToScene(viewport.rect().center())
            else:
                # Fallback if viewport is not available, though it should be by this point
                center_point = QPointF(0, 0)

        # Prevent division by zero
        if self._zoom_factor <= 0:
            self._zoom_factor = 1.0
            self.resetTransform()

        # Calculate the scale change
        scale_change = factor / self._zoom_factor

        # Get the point in view coordinates that corresponds to the scene point
        # This is the point that should remain fixed relative to the viewport
        point_in_view_before_scale = self.mapFromScene(center_point)

        # Apply the zoom
        self.scale(scale_change, scale_change)
        self._zoom_factor = factor

        # After scaling, the scene point has moved relative to the viewport.
        # We need to calculate the new position of the scene point in view coordinates.
        point_in_view_after_scale = self.mapFromScene(center_point)

        # Calculate the delta needed to move the view so the scene point is back at its original view position
        delta_x = point_in_view_after_scale.x() - point_in_view_before_scale.x()
        delta_y = point_in_view_after_scale.y() - point_in_view_before_scale.y()

        # Adjust scrollbars to move the view
        h_bar = self.horizontalScrollBar()
        v_bar = self.verticalScrollBar()
        if h_bar:
            h_bar.setValue(h_bar.value() + int(delta_x))
        if v_bar:
            v_bar.setValue(v_bar.value() + int(delta_y))

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
            # Set minimum zoom to current fit level (functional behavior stays the same)
            self._min_zoom = self._zoom_factor
            viewport = self.viewport()
            if viewport:
                center_point = self.mapToScene(viewport.rect().center())
            else:
                center_point = QPointF(0, 0)  # Fallback
            self.zoom_changed.emit(self._zoom_factor, center_point)

    def zoom_to_actual_size(self):
        """Zoom to 100% (actual size)"""
        if self._empty:
            return
        self.set_zoom_factor(1.0)

    def wheelEvent(self, event: Optional[QWheelEvent]):
        """Handle mouse wheel for zooming"""
        if self._empty or event is None:
            super().wheelEvent(
                event
            )  # Call super to ensure event propagation if not handled
            return

        # Get the mouse position in scene coordinates
        mouse_pos = self.mapToScene(event.position().toPoint())

        # Zoom in or out based on wheel direction
        if event.angleDelta().y() > 0:
            self.zoom_in(mouse_pos)
        else:
            self.zoom_out(mouse_pos)

    def mousePressEvent(self, event: Optional[QMouseEvent]):
        """Handle mouse press for panning"""
        if event is None:
            super().mousePressEvent(event)
            return

        if event.button() == Qt.MouseButton.LeftButton:
            self._last_pan_point = event.position()
            self._is_panning = True
            self.setCursor(Qt.CursorShape.ClosedHandCursor)
        super().mousePressEvent(event)

    def mouseMoveEvent(self, event: Optional[QMouseEvent]):
        """Handle mouse move for panning and coordinate tracking"""
        if event is None:
            super().mouseMoveEvent(event)
            return

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
            if h_bar:
                h_bar.setValue(h_bar.value() - int(delta.x()))
            if v_bar:
                v_bar.setValue(v_bar.value() - int(delta.y()))

            viewport = self.viewport()
            if viewport:
                center = self.mapToScene(viewport.rect().center())
            else:
                center = QPointF(0, 0)  # Fallback
            self.pan_changed.emit(center)

        super().mouseMoveEvent(event)

    def mouseReleaseEvent(self, event: Optional[QMouseEvent]):
        """Handle mouse release"""
        if event is None:
            super().mouseReleaseEvent(event)
            return

        if event.button() == Qt.MouseButton.LeftButton:
            self._is_panning = False
            self.setCursor(Qt.CursorShape.ArrowCursor)
        super().mouseReleaseEvent(event)

    def keyPressEvent(self, event: Optional[QKeyEvent]):
        """Handle keyboard shortcuts, allowing number keys to propagate up."""
        if event is None:
            super().keyPressEvent(event)
            return

        key = event.key()

        # Let number keys 1-9 pass up to the parent for focus switching
        if Qt.Key.Key_1 <= key <= Qt.Key.Key_9:
            super().keyPressEvent(event)  # Propagate event to parent
            return

        if key == Qt.Key.Key_Plus or key == Qt.Key.Key_Equal:
            self.zoom_in()
        elif key == Qt.Key.Key_Minus:
            self.zoom_out()
        elif key == Qt.Key.Key_0:
            self.fit_in_view()
        elif key == Qt.Key.Key_Backspace:  # Another common key, ensure it propagates
            super().keyPressEvent(event)
        else:
            # Propagate unhandled keys to the parent
            super().keyPressEvent(event)

    def clear(self):
        """Clear the image display with smooth transition"""
        logger.debug("ZoomableImageView cleared")
        # Remove any lingering text items
        for item in self._scene.items():
            if hasattr(item, "_is_text_item"):
                self._scene.removeItem(item)

        # Use transparent pixmap instead of completely empty to reduce flash
        self._empty = True
        transparent_pixmap = QPixmap(1, 1)
        transparent_pixmap.fill(Qt.GlobalColor.transparent)
        self._photo_item.setPixmap(transparent_pixmap)

    def setText(self, text: str):
        """Set text display with improved rendering"""
        logger.debug(f"ZoomableImageView.setText called with text: {text}")

        # Only proceed if we actually want to show text
        if not text or text.strip() == "":
            logger.debug("Text is empty, returning")
            return

        # Remove any existing text items first
        for item in self._scene.items():
            if hasattr(item, "_is_text_item"):
                self._scene.removeItem(item)

        # Only clear photo item after we're ready to show text
        transparent_pixmap = QPixmap(1, 1)
        transparent_pixmap.fill(Qt.GlobalColor.transparent)
        self._photo_item.setPixmap(transparent_pixmap)
        self._empty = True

        from PyQt6.QtWidgets import QGraphicsTextItem
        from PyQt6.QtGui import QFont

        text_item = QGraphicsTextItem(text)
        setattr(text_item, "_is_text_item", True)

        # Style the text with better contrast
        font = QFont()
        font.setPointSize(14)
        text_item.setFont(font)
        text_item.setDefaultTextColor(
            QColor(180, 180, 180)
        )  # Lighter for better contrast

        # Get viewport dimensions
        viewport = self.viewport()
        if viewport:
            viewport_rect: QRectF = viewport.rect()
        else:
            viewport_rect = QRectF(0, 0, 1, 1)

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
    deleteRequested = pyqtSignal(str)  # file_path
    deleteOthersRequested = pyqtSignal(str)  # file_path of the image to keep
    markAsDeletedRequested = pyqtSignal(str)  # file_path
    markOthersAsDeletedRequested = pyqtSignal(str)  # file_path of the image to keep
    unmarkAsDeletedRequested = pyqtSignal(str)  # file_path
    unmarkOthersAsDeletedRequested = pyqtSignal(str)  # file_path of the image to keep

    def __init__(self, parent=None):
        super().__init__(parent)
        self._file_path = None
        self._is_selected = False

        self._setup_ui()
        self._connect_signals()

    def get_file_path(self) -> Optional[str]:
        """Get the file path of the currently displayed image."""
        return self._file_path

    def get_current_pixmap(self) -> Optional[QPixmap]:
        """Expose the currently displayed pixmap for external consumers."""
        return self.image_view.current_pixmap()

    def _is_marked_for_deletion(self) -> bool:
        """Check if the current file is marked for deletion."""
        if self._file_path:
            # Find the SynchronizedImageViewer parent (grandparent of the QSplitter)
            parent = self.parent()
            if parent:
                grandparent = parent.parent()
                if grandparent and hasattr(grandparent, "is_marked_for_deletion"):
                    return grandparent.is_marked_for_deletion(self._file_path)
        return False

    def _has_any_marked_for_deletion(self) -> bool:
        """Check if there are any files marked for deletion."""
        # Find the SynchronizedImageViewer parent (grandparent of the QSplitter)
        parent = self.parent()
        if parent:
            grandparent = parent.parent()
            if grandparent and hasattr(grandparent, "has_any_marked_for_deletion"):
                return grandparent.has_any_marked_for_deletion()
        return False

    def _is_single_image_mode(self) -> bool:
        """Check if we're in a mode where there are no other images to affect."""
        # Find the SynchronizedImageViewer parent (grandparent of the QSplitter)
        parent = self.parent()
        if parent:
            grandparent = parent.parent()
            if grandparent and hasattr(grandparent, "is_single_image_mode"):
                return grandparent.is_single_image_mode()
        # Default to True (hide others) if we can't determine the mode
        return True

    def _setup_ui(self):
        """Setup the layout with image view and control bar."""
        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)

        self.image_view = ZoomableImageView()
        layout.addWidget(self.image_view, 1)  # Image view takes up all available space

        # --- Control Bar ---
        self.control_bar = QFrame()
        self.control_bar.setObjectName(
            "imageActionControls"
        )  # Use a specific object name for styling
        self.control_bar.setFixedHeight(35)

        control_layout = QHBoxLayout(self.control_bar)
        control_layout.setContentsMargins(10, 0, 10, 0)
        control_layout.setSpacing(15)

        control_layout.addStretch()

        # Rating controls
        self.rating_widget = self._create_rating_controls()
        control_layout.addWidget(self.rating_widget)

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

        self.delete_button = QPushButton("🗑️")
        self.delete_button.setToolTip("Delete image (Backspace)")
        layout.addWidget(self.delete_button)

        return widget

    def _connect_signals(self):
        """Connect button signals to handlers."""
        for i, btn in enumerate(self.star_buttons):
            btn.clicked.connect(
                lambda checked, rating=i + 1: self._on_rating_button_clicked(rating)
            )
        self.delete_button.clicked.connect(self._on_delete_button_clicked)

    def _on_rating_button_clicked(self, rating_override=None):
        """Handle rating button clicks and emit signal."""
        if self._file_path is None:
            return

        rating = rating_override
        if rating is None:
            sender_obj = self.sender()
            if sender_obj:
                rating = sender_obj.property("ratingValue")

        # Implement toggle functionality: if the same rating is clicked again, clear the rating
        current_rating = 0
        # Get the current display rating from the buttons
        for i, btn in enumerate(self.star_buttons):
            if btn.text() == "★":
                current_rating = i + 1

        # If the clicked rating is the same as the current rating, clear the rating
        if rating == current_rating:
            rating = 0

        if rating is not None:
            self.ratingChanged.emit(self._file_path, rating)
            self.update_rating_display(rating)  # Update own display immediately

    def _on_delete_button_clicked(self):
        """Handle delete button click."""
        if self._file_path is None:
            return
        # Emit a signal to the main window to handle the deletion
        # We'll need to add this signal to the IndividualViewer class
        self.deleteRequested.emit(self._file_path)

    def set_data(
        self, pixmap: QPixmap, file_path: str, rating: int, label: Optional[str]
    ):
        """Set all data for the viewer at once."""
        logger.debug(f"IndividualViewer.set_data called with file_path: {file_path}")
        self._file_path = file_path
        self.image_view.set_image(pixmap)
        logger.debug(f"Updating rating display to {rating}")
        self.update_rating_display(rating)

    def update_rating_display(self, rating: int):
        """Update the star buttons to reflect the current rating."""
        for i, btn in enumerate(self.star_buttons):
            btn.setText("★" if i < rating else "☆")

    def clear(self):
        """Clear the viewer and its associated data."""
        logger.debug("Clearing image_view")
        self._file_path = None
        self.image_view.clear()
        self.update_rating_display(0)

    def has_image(self) -> bool:
        """Check if the internal image view has an image."""
        return self.image_view.has_image()

    def set_selected(self, is_selected: bool):
        """Set the visual selection state of the viewer."""
        if self._is_selected != is_selected:
            self._is_selected = is_selected
            if is_selected:
                self.setProperty("selected", True)
                self.setStyleSheet(
                    "IndividualViewer[selected='true'] { border: 2px solid #0078d7; }"
                )
            else:
                self.setProperty("selected", False)
                self.setStyleSheet("IndividualViewer { border: none; }")

            # Refresh style
            current_style = self.style()
            if current_style:
                current_style.unpolish(self)
                current_style.polish(self)

    def mousePressEvent(self, event: QMouseEvent):
        """Handle mouse press events to show context menu on right-click."""
        if event.button() == Qt.MouseButton.RightButton and self._file_path is not None:
            self._show_context_menu(event.pos())
        else:
            super().mousePressEvent(event)

    def _show_context_menu(self, position: QPoint):
        """Show context menu with delete options."""
        menu = QMenu(self)

        # Delete action
        delete_action = QAction("Delete", self)
        delete_action.triggered.connect(
            lambda: self.deleteRequested.emit(self._file_path)
        )
        menu.addAction(delete_action)

        # Check if we're in single image mode (no other images to affect)
        is_single_mode = self._is_single_image_mode()

        # Delete Others action - only show if not in single image mode
        if not is_single_mode:
            delete_others_action = QAction("Delete Others", self)
            delete_others_action.triggered.connect(
                lambda: self.deleteOthersRequested.emit(self._file_path)
            )
            menu.addAction(delete_others_action)

        # Add separator
        menu.addSeparator()

        # Toggle Mark for Deletion action
        is_marked = self._is_marked_for_deletion()
        mark_toggle_action = QAction(
            "Unmark for Deletion" if is_marked else "Mark for Deletion", self
        )
        mark_toggle_action.triggered.connect(
            lambda: (
                self.unmarkAsDeletedRequested.emit(self._file_path)
                if is_marked
                else self.markAsDeletedRequested.emit(self._file_path)
            )
        )

        # Add visual indication of marked state
        if is_marked:
            mark_toggle_action.setIcon(
                QIcon(
                    self.style().standardIcon(QStyle.StandardPixmap.SP_DialogYesButton)
                )
            )

        menu.addAction(mark_toggle_action)

        # Mark Others as Deleted action - only show if not in single image mode
        if not is_single_mode:
            mark_others_deleted_action = QAction("Mark Others as Deleted", self)
            mark_others_deleted_action.triggered.connect(
                lambda: self.markOthersAsDeletedRequested.emit(self._file_path)
            )
            menu.addAction(mark_others_deleted_action)

        # Add separator
        menu.addSeparator()

        # Unmark Others as Deleted action
        # Only show this option if there are any files marked for deletion AND not in single mode
        if not is_single_mode and self._has_any_marked_for_deletion():
            unmark_others_deleted_action = QAction("Unmark Others as Deleted", self)
            unmark_others_deleted_action.triggered.connect(
                lambda: self.unmarkOthersAsDeletedRequested.emit(self._file_path)
            )
            menu.addAction(unmark_others_deleted_action)

        # Show menu at the cursor position
        menu.exec(self.mapToGlobal(position))


class SynchronizedImageViewer(QWidget):
    """
    Container for synchronized IndividualViewers with a central toolbar.
    """

    # Forward signals from IndividualViewer instances
    ratingChanged = pyqtSignal(str, int)
    deleteRequested = pyqtSignal(str)  # file_path
    deleteOthersRequested = pyqtSignal(str)  # file_path of the image to keep
    deleteMultipleRequested = pyqtSignal(list)  # list of file_paths to delete
    markAsDeletedRequested = pyqtSignal(str)  # file_path
    markOthersAsDeletedRequested = pyqtSignal(str)  # file_path of the image to keep
    unmarkAsDeletedRequested = pyqtSignal(str)  # file_path
    unmarkOthersAsDeletedRequested = pyqtSignal(str)  # file_path of the image to keep
    focused_image_changed = pyqtSignal(int, str)  # index, file_path
    side_by_side_availability_changed = pyqtSignal(bool)
    cleared = pyqtSignal()

    def __init__(self, parent=None):
        super().__init__(parent)

        self.image_viewers: List[IndividualViewer] = []
        self.sync_enabled = True
        self._updating_sync = False
        self._focused_index = 0
        self._view_mode = "single"
        self._is_marked_for_deletion_func = None
        self._has_any_marked_for_deletion_func = None

        self._setup_ui()
        self.setFocusPolicy(
            Qt.FocusPolicy.StrongFocus
        )  # Allow widget to receive key events

    def set_is_marked_for_deletion_func(self, func):
        """Set the function to check if a file is marked for deletion."""
        self._is_marked_for_deletion_func = func

    def set_has_any_marked_for_deletion_func(self, func):
        """Set the function to check if there are any files marked for deletion."""
        self._has_any_marked_for_deletion_func = func

    def is_single_image_mode(self) -> bool:
        """
        Check if we're in a mode where there are no other images to affect.
        Returns True if we're in single or focused mode (only one image visible),
        False if we're in side-by-side mode (multiple images visible).
        """
        return self._view_mode in ["single", "focused"]

    def is_marked_for_deletion(self, file_path: str) -> bool:
        """Check if a file is marked for deletion."""
        if self._is_marked_for_deletion_func:
            return self._is_marked_for_deletion_func(file_path)
        return False

    def has_any_marked_for_deletion(self) -> bool:
        """Check if there are any files marked for deletion."""
        if self._has_any_marked_for_deletion_func:
            return self._has_any_marked_for_deletion_func()
        return False

    def _setup_ui(self):
        """Setup the user interface with a modern, sleek toolbar."""

        layout = QVBoxLayout(self)
        layout.setContentsMargins(0, 0, 0, 0)
        layout.setSpacing(0)

        # --- Modern Toolbar ---
        self.controls_frame = QFrame()
        self.controls_frame.setObjectName("advancedViewerToolbar")
        controls_layout = QHBoxLayout(self.controls_frame)
        controls_layout.setContentsMargins(12, 6, 12, 6)
        controls_layout.setSpacing(16)

        # -- View Mode Section --
        view_mode_container = QFrame()
        view_mode_container.setObjectName("viewModeContainer")
        view_mode_layout = QHBoxLayout(view_mode_container)
        view_mode_layout.setContentsMargins(4, 2, 4, 2)
        view_mode_layout.setSpacing(0)

        self.view_mode_group = QButtonGroup(self)

        # Single View Button
        self.single_view_btn = QPushButton("▢")
        self.single_view_btn.setToolTip("Single View")
        self.single_view_btn.setCheckable(True)
        self.single_view_btn.setChecked(True)
        self.single_view_btn.setObjectName("viewModeButton")
        self.single_view_btn.setProperty("position", "left")
        self.single_view_btn.clicked.connect(lambda: self._set_view_mode("single"))
        self.view_mode_group.addButton(self.single_view_btn)
        view_mode_layout.addWidget(self.single_view_btn)

        # Side by Side Button
        self.side_by_side_btn = QPushButton("▢▢")
        self.side_by_side_btn.setToolTip("Side by Side")
        self.side_by_side_btn.setCheckable(True)
        self.side_by_side_btn.setObjectName("viewModeButton")
        self.side_by_side_btn.setProperty("position", "right")
        self.side_by_side_btn.clicked.connect(
            lambda: self._set_view_mode("side_by_side")
        )
        self.view_mode_group.addButton(self.side_by_side_btn)
        view_mode_layout.addWidget(self.side_by_side_btn)

        controls_layout.addWidget(view_mode_container)

        # Vertical separator
        separator1 = QFrame()
        separator1.setObjectName("toolbarSeparator")
        separator1.setFrameShape(QFrame.Shape.VLine)
        controls_layout.addWidget(separator1)

        # -- Zoom Controls Section --
        zoom_container = QFrame()
        zoom_container.setObjectName("zoomContainer")
        zoom_layout = QHBoxLayout(zoom_container)
        zoom_layout.setContentsMargins(8, 2, 8, 2)
        zoom_layout.setSpacing(10)

        # Zoom Out Button
        self.zoom_out_btn = QPushButton("−")
        self.zoom_out_btn.setToolTip("Zoom Out (-)")
        self.zoom_out_btn.setObjectName("zoomButton")
        self.zoom_out_btn.clicked.connect(self._zoom_out_all)
        zoom_layout.addWidget(self.zoom_out_btn)

        # Zoom Slider
        self.zoom_slider = QSlider(Qt.Orientation.Horizontal)
        self.zoom_slider.setObjectName("zoomSlider")
        self.zoom_slider.setRange(0, 500)  # 0% to 500% display range
        self.zoom_slider.setValue(0)  # Start at minimum zoom (0% display)
        self.zoom_slider.setToolTip("Zoom Level")
        self.zoom_slider.valueChanged.connect(self._zoom_slider_changed)
        self.zoom_slider.setFixedWidth(140)
        zoom_layout.addWidget(self.zoom_slider)

        # Zoom In Button
        self.zoom_in_btn = QPushButton("+")
        self.zoom_in_btn.setToolTip("Zoom In (+)")
        self.zoom_in_btn.setObjectName("zoomButton")
        self.zoom_in_btn.clicked.connect(self._zoom_in_all)
        zoom_layout.addWidget(self.zoom_in_btn)

        # Zoom percentage label
        self.zoom_label = QLabel("0%")
        self.zoom_label.setObjectName("zoomLabel")
        self.zoom_label.setMinimumWidth(55)
        self.zoom_label.setAlignment(Qt.AlignmentFlag.AlignCenter)
        zoom_layout.addWidget(self.zoom_label)

        controls_layout.addWidget(zoom_container)

        # -- Fit Controls Section --
        fit_container = QFrame()
        fit_container.setObjectName("fitContainer")
        fit_layout = QHBoxLayout(fit_container)
        fit_layout.setContentsMargins(8, 2, 8, 2)
        fit_layout.setSpacing(8)

        # Fit to View Button
        self.fit_btn = QPushButton("⊡")
        self.fit_btn.setToolTip("Fit to View (0)")
        self.fit_btn.setObjectName("fitButton")
        self.fit_btn.clicked.connect(self._fit_all)
        fit_layout.addWidget(self.fit_btn)

        # 1:1 View Button
        self.actual_size_btn = QPushButton("1:1")
        self.actual_size_btn.setToolTip("Actual Size (100%)")
        self.actual_size_btn.setObjectName("actualSizeButton")
        self.actual_size_btn.clicked.connect(self._actual_size_all)
        fit_layout.addWidget(self.actual_size_btn)

        controls_layout.addWidget(fit_container)

        # Spacer
        controls_layout.addStretch()

        # -- Sync Toggle --
        self.sync_button = QPushButton("⟲ Sync")
        self.sync_button.setCheckable(True)
        self.sync_button.setChecked(True)
        self.sync_button.setToolTip("Synchronize Pan & Zoom")
        self.sync_button.setObjectName("syncButton")
        self.sync_button.toggled.connect(self._toggle_sync)
        controls_layout.addWidget(self.sync_button)

        layout.addWidget(self.controls_frame)

        # --- Image Viewer Container ---
        self.viewer_splitter = QSplitter(Qt.Orientation.Horizontal)
        self.viewer_splitter.setObjectName("advancedViewerSplitter")
        self.viewer_splitter.setHandleWidth(2)
        self.viewer_splitter.splitterMoved.connect(self._on_splitter_moved)
        layout.addWidget(self.viewer_splitter, 1)

        self._create_viewer()
        self._update_controls_visibility()

    def _on_splitter_moved(self, pos, index):
        QTimer.singleShot(50, self._fit_visible_images_after_layout_change)

    def _create_viewer(self) -> IndividualViewer:
        viewer = IndividualViewer()
        viewer.image_view.zoom_changed.connect(self._on_zoom_changed)
        viewer.image_view.pan_changed.connect(self._on_pan_changed)
        viewer.ratingChanged.connect(self.ratingChanged)
        viewer.deleteRequested.connect(self.deleteRequested)
        viewer.deleteOthersRequested.connect(self._on_delete_others_requested)
        viewer.markAsDeletedRequested.connect(self.markAsDeletedRequested)
        viewer.markOthersAsDeletedRequested.connect(self.markOthersAsDeletedRequested)
        viewer.unmarkAsDeletedRequested.connect(self.unmarkAsDeletedRequested)
        viewer.unmarkOthersAsDeletedRequested.connect(
            self.unmarkOthersAsDeletedRequested
        )
        self.image_viewers.append(viewer)
        self.viewer_splitter.addWidget(viewer)
        return viewer

    def _resize_viewer_pool(self, target_count: int):
        """Ensure the viewer pool matches the desired size."""
        target_count = max(1, target_count)
        current_count = len(self.image_viewers)

        if current_count != target_count:
            logger.debug(f"Resizing viewer pool from {current_count} to {target_count}")

        while len(self.image_viewers) < target_count:
            self._create_viewer()

        while len(self.image_viewers) > target_count:
            viewer = self.image_viewers.pop()
            if viewer.parent() is self.viewer_splitter:
                viewer.setParent(None)
            viewer.deleteLater()

        if self._focused_index >= len(self.image_viewers):
            self._focused_index = max(0, len(self.image_viewers) - 1)

    def _get_current_view_mode(self):
        return "side_by_side" if self.side_by_side_btn.isChecked() else "single"

    def _toggle_sync(self, checked: bool):
        self.sync_enabled = checked

    def _update_controls_visibility(self):
        visible_viewers = sum(1 for v in self.image_viewers if v.isVisible())
        self.sync_button.setVisible(visible_viewers > 1)

        num_images_loaded = sum(1 for v in self.image_viewers if v.has_image())
        is_available = num_images_loaded >= 2
        self.side_by_side_btn.setEnabled(is_available)
        self.side_by_side_availability_changed.emit(is_available)

    def _set_view_mode(self, mode: str, focused_index: int = -1):
        """Set the display mode to single, focused, or side-by-side."""
        logger.debug(
            f"Setting view mode to '{mode}' with focused index: {focused_index}"
        )
        num_images = sum(1 for v in self.image_viewers if v.has_image())
        logger.debug(f"Number of viewers with images: {num_images}")

        if focused_index != -1:
            logger.debug(f"Setting focused index to {focused_index}")
            self._focused_index = focused_index

        # If going to single mode with multiple images, switch to focused mode instead
        if mode == "single" and num_images > 1:
            mode = "focused"
            logger.debug("Switching to 'focused' mode (more than one image present).")

        logger.debug(f"Setting internal view mode to '{mode}'")
        self._view_mode = mode

        if mode == "focused":
            logger.debug(f"Configuring focused mode with index {self._focused_index}")
            for i, viewer in enumerate(self.image_viewers):
                is_focused = i == self._focused_index
                logger.debug(f"Viewer {i} focused: {is_focused}")
                viewer.setVisible(is_focused)
                viewer.set_selected(is_focused)
            if (
                self._focused_index < len(self.image_viewers)
                and self.image_viewers[self._focused_index].has_image()
            ):
                path = self.image_viewers[self._focused_index]._file_path
                logger.debug(
                    f"Emitting focused_image_changed for index {self._focused_index}, path: {path}"
                )
                self.focused_image_changed.emit(
                    self._focused_index, path if path else ""
                )
            self.single_view_btn.setChecked(True)

        elif mode == "single":
            logger.debug("Configuring single mode")
            for i, viewer in enumerate(self.image_viewers):
                is_first = i == 0
                logger.debug(f"Viewer {i} visible: {is_first}")
                viewer.setVisible(is_first)
                viewer.set_selected(is_first and num_images > 0)
            if len(self.image_viewers) > 0 and self.image_viewers[0].has_image():
                path = self.image_viewers[0]._file_path
                logger.debug(
                    f"Emitting focused_image_changed for single mode, path: {path}"
                )
                self.focused_image_changed.emit(0, path if path else "")
            self.single_view_btn.setChecked(True)

        elif mode == "side_by_side":
            logger.debug("Configuring side-by-side mode")
            # When returning to side-by-side, clear any focused image state
            logger.debug(
                "Emitting focused_image_changed with empty state for side-by-side mode"
            )
            self.focused_image_changed.emit(-1, "")  # index=-1, empty path

            num_active_viewers = sum(1 for v in self.image_viewers if v.has_image())
            logger.debug(f"Number of active viewers: {num_active_viewers}")
            if num_active_viewers == 0:
                num_active_viewers = 1  # Show at least one empty viewer
                logger.debug("Setting num_active_viewers to 1 (no images)")

            for i, viewer in enumerate(self.image_viewers):
                visible = i < num_active_viewers
                logger.debug(f"Viewer {i} visible: {visible}")
                viewer.setVisible(visible)
                viewer.set_selected(False)  # No selection in side-by-side

            if (
                hasattr(self, "viewer_splitter")
                and self.viewer_splitter.count() > 0
                and num_active_viewers > 0
            ):
                total_width = self.viewer_splitter.width()
                logger.debug(f"Splitter total width: {total_width}")
                if total_width > 0:
                    base_size = total_width // num_active_viewers
                    remainder = total_width % num_active_viewers
                    sizes = [
                        base_size + 1 if i < remainder else base_size
                        for i in range(num_active_viewers)
                    ]
                    sizes.extend(
                        [0] * (self.viewer_splitter.count() - num_active_viewers)
                    )
                    logger.debug(f"Setting splitter sizes: {sizes}")
                    self.viewer_splitter.setSizes(sizes)

            self.side_by_side_btn.setChecked(True)

        self._update_controls_visibility()
        logger.debug("Scheduling fit_visible_images_after_layout_change")
        QTimer.singleShot(50, self._fit_visible_images_after_layout_change)

    def set_focused_viewer(self, index: int):
        """Public method to set the focused viewer by index."""
        num_images = sum(1 for v in self.image_viewers if v.has_image())
        if 0 <= index < num_images:
            self._set_view_mode("focused", focused_index=index)

    def set_focused_viewer_by_path(self, file_path: str):
        """Public method to set the focused viewer by file path."""
        for i, viewer in enumerate(self.image_viewers):
            if viewer.get_file_path() == file_path:
                self.set_focused_viewer(i)
                return

    def _fit_visible_images_after_layout_change(self):
        for viewer in self.image_viewers:
            if viewer.isVisible() and viewer.has_image():
                viewer.image_view.fit_in_view()

    def get_focused_image_path_if_any(self) -> Optional[str]:
        """
        If the view is in 'focused' mode (one image shown out of many),
        returns the file path of the focused image. Otherwise, returns None.
        """
        # Rely on the internal view mode state
        if hasattr(self, "_view_mode") and self._view_mode == "focused":
            # In focused mode, there should be more than one image loaded total
            num_with_image = sum(1 for v in self.image_viewers if v.has_image())
            if num_with_image > 1:
                if 0 <= self._focused_index < len(self.image_viewers):
                    viewer = self.image_viewers[self._focused_index]
                    # Check if this viewer is actually visible and has our file
                    if viewer.isVisible() and viewer.has_image() and viewer._file_path:
                        return viewer._file_path
        return None

    def get_primary_viewer(self) -> Optional[IndividualViewer]:
        """Return the primary (first) viewer instance if available."""
        if not self.image_viewers:
            return None
        return self.image_viewers[0]

    def set_image_data(
        self,
        image_data: Dict[str, Any],
        viewer_index: int = 0,
        preserve_view_mode: bool = False,
    ):
        """Sets the data for a single viewer and clears others."""
        logger.debug(
            f"set image data {image_data} called with viewer_index={viewer_index}, preserve_view_mode={preserve_view_mode}"
        )

        desired_viewer_count = max(viewer_index + 1, 1)
        self._resize_viewer_pool(desired_viewer_count)
        viewer_index = min(viewer_index, len(self.image_viewers) - 1)

        # Update all viewers: set data for the target, clear others
        for i, viewer in enumerate(self.image_viewers):
            if i == viewer_index:
                pixmap = image_data.get("pixmap")
                logger.debug(
                    f"Setting data for viewer {i}, pixmap valid: {bool(pixmap)}"
                )
                if pixmap:
                    viewer.set_data(
                        pixmap,
                        image_data.get("path", ""),  # Ensure path is always a string
                        image_data.get("rating", 0),
                        image_data.get("label"),
                    )
                else:
                    logger.debug(f"Clearing viewer {i} due to invalid pixmap")
                    viewer.clear()  # Clear if pixmap is invalid
            else:
                logger.debug(f"Clearing viewer {i}")
                viewer.clear()

        # If not preserving view mode, set to single/focused view
        if not preserve_view_mode:
            logger.debug("Setting view mode to single")
            # When a new single image is set, it becomes the new focus.
            # We must reset the focused index to 0 (where the new image is displayed)
            # to prevent the viewer from reverting to a stale focused index.
            self._focused_index = viewer_index
            self._set_view_mode("single")

        self._update_controls_visibility()

    def set_images_data(self, images_data: List[Dict[str, Any]]):
        """Populate viewers with a list of image data."""
        num_images = len(images_data)

        if num_images == 0:
            self.clear()
            return

        if num_images == 1:
            self.set_image_data(images_data[0], 0)
            return

        self._resize_viewer_pool(num_images)

        # Update all viewers, then hide unused ones
        for i, viewer in enumerate(self.image_viewers):
            if i < num_images:
                data = images_data[i]
                if data and data.get("pixmap"):
                    viewer.set_data(
                        data["pixmap"],
                        data["path"],
                        data.get("rating", 0),
                        data.get("label"),
                    )
                else:
                    viewer.clear()
            else:
                viewer.clear()

        self._set_view_mode("side_by_side")

    def clear(self):
        logger.debug("clear called")
        self._resize_viewer_pool(1)
        for i, viewer in enumerate(self.image_viewers):
            logger.debug(f"Clearing viewer {i}")
            viewer.clear()
        logger.debug("Setting view mode to single")
        self._set_view_mode("single")
        self.cleared.emit()

    def setText(self, text: str):
        """Clears all viewers, resets to single view, and displays the given text centrally."""
        logger.debug(f"setText called with: {text}")
        self.clear()

        # Now, set the text on the first viewer, which is now the only one visible.
        if self.image_viewers:
            logger.debug("Setting text on first viewer")
            # Use a QTimer to ensure the layout has updated before we set the text.
            # This helps in correctly centering the text after a mode change.
            QTimer.singleShot(0, lambda: self.image_viewers[0].image_view.setText(text))

    def _zoom_in_all(self):
        for viewer in self.image_viewers:
            if viewer.isVisible():
                viewer.image_view.zoom_in()

    def _zoom_out_all(self):
        for viewer in self.image_viewers:
            if viewer.isVisible():
                viewer.image_view.zoom_out()

    def _fit_all(self):
        for viewer in self.image_viewers:
            if viewer.isVisible():
                viewer.image_view.fit_in_view()

    def _actual_size_all(self):
        for viewer in self.image_viewers:
            if viewer.isVisible():
                viewer.image_view.zoom_to_actual_size()

    def _ui_initialized(self) -> bool:
        return hasattr(self, "viewer_splitter")

    def _zoom_slider_changed(self, value: int):
        if self._updating_sync:
            return

        # Convert display percentage back to actual zoom factor
        for viewer in self.image_viewers:
            if viewer.isVisible():
                view = viewer.image_view
                if value == 0:
                    # 0% display = minimum zoom level
                    zoom_factor = view._min_zoom
                elif value <= 100:
                    # Scale from 0-100% display to min_zoom-1.0 actual zoom
                    if view._min_zoom >= 1.0:
                        # If minimum zoom is already >= 100%, use actual percentage
                        zoom_factor = value / 100.0
                    else:
                        # Scale from 0%=min_zoom to 100%=1.0
                        normalized = value / 100.0
                        zoom_factor = view._min_zoom + normalized * (
                            1.0 - view._min_zoom
                        )
                else:
                    # Above 100% display = above 1.0x actual zoom
                    zoom_factor = value / 100.0

                view.set_zoom_factor(zoom_factor)

    def _on_zoom_changed(self, zoom_factor: float, center_point: QPointF):
        if self._updating_sync:
            return
        sender_view = self.sender()
        if not isinstance(sender_view, ZoomableImageView):
            return

        # Use display percentage instead of actual zoom percentage
        display_percentage = sender_view.get_display_zoom_percentage()
        self.zoom_label.setText(f"{display_percentage}%")
        self._updating_sync = True
        self.zoom_slider.setValue(display_percentage)
        self._updating_sync = False

        if self.sync_enabled:
            sender_scene_rect = sender_view.sceneRect()
            norm_x = (
                (center_point.x() - sender_scene_rect.left())
                / sender_scene_rect.width()
                if sender_scene_rect.width() > 0
                else 0.5
            )
            norm_y = (
                (center_point.y() - sender_scene_rect.top())
                / sender_scene_rect.height()
                if sender_scene_rect.height() > 0
                else 0.5
            )
            normalized_pos = QPointF(norm_x, norm_y)

            self._updating_sync = True
            for viewer in self.image_viewers:
                if (
                    viewer.image_view != sender_view
                    and viewer.isVisible()
                    and viewer.has_image()
                ):
                    target_scene_rect = viewer.image_view.sceneRect()
                    if target_scene_rect.width() > 0:
                        target_x = (
                            target_scene_rect.left()
                            + normalized_pos.x() * target_scene_rect.width()
                        )
                        target_y = (
                            target_scene_rect.top()
                            + normalized_pos.y() * target_scene_rect.height()
                        )
                        viewer.image_view.set_zoom_factor(
                            zoom_factor, QPointF(target_x, target_y)
                        )
                    else:
                        viewer.image_view.set_zoom_factor(zoom_factor)
            self._updating_sync = False

    def _on_pan_changed(self, center_point: QPointF):
        if not self.sync_enabled or self._updating_sync:
            return
        sender_view = self.sender()
        self._updating_sync = True
        for viewer in self.image_viewers:
            if viewer.image_view != sender_view and viewer.isVisible():
                viewer.image_view.centerOn(center_point)
        self._updating_sync = False

    def _on_delete_others_requested(self, file_path_to_keep: str):
        """Handle delete others request by collecting file paths and emitting a bulk delete signal."""
        file_paths_to_delete = []
        for viewer in self.image_viewers:
            # Collect all other images except the one that requested to keep
            if viewer._file_path is not None and viewer._file_path != file_path_to_keep:
                file_paths_to_delete.append(viewer._file_path)

        # Emit the bulk delete request if there are files to delete
        if file_paths_to_delete:
            self.deleteMultipleRequested.emit(file_paths_to_delete)

    def fit_to_viewport(self):
        for viewer in self.image_viewers:
            if viewer.isVisible() and viewer.has_image():
                viewer.image_view.fit_in_view()
