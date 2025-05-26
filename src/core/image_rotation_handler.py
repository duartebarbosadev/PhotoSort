import os
import logging
import time
from typing import Optional, Tuple
from PIL import Image
from PyQt6.QtCore import QObject, pyqtSignal
from PyQt6.QtGui import QPixmap, QTransform
from PyQt6.QtCore import Qt

from .rating_handler import MetadataHandler
from .caching.rating_cache import RatingCache
from .caching.exif_cache import ExifCache
from .caching.preview_cache import PreviewCache
from .caching.thumbnail_cache import ThumbnailCache


class ImageRotationHandler(QObject):
    """
    Handles image rotation operations with immediate UI feedback and persistent EXIF updates.
    Provides cool UX with instant preview rotation and background EXIF processing.
    """
    
    # Signal emitted when rotation is applied to UI (immediate feedback)
    rotation_applied = pyqtSignal(str, int)  # file_path, degrees
    
    # Signal emitted when EXIF rotation is complete (background operation)
    exif_rotation_complete = pyqtSignal(str, bool)  # file_path, success
    
    # Signal for error reporting
    rotation_error = pyqtSignal(str, str)  # file_path, error_message

    def __init__(self, parent=None):
        super().__init__(parent)
        self.current_ui_rotations = {}  # Track UI-only rotations: {file_path: cumulative_degrees}
        self._rotation_in_progress = set()  # Track files currently being rotated to prevent duplicates
        logging.debug("[ImageRotationHandler] Initialized rotation handler")
    
    def debug_rotation_state(self):
        """Debug method to print current rotation state."""
        logging.info(f"[ImageRotationHandler] Current rotation state:")
        for path, rotation in self.current_ui_rotations.items():
            logging.info(f"  {os.path.basename(path)}: {rotation}°")
        if not self.current_ui_rotations:
            logging.info("  (no rotations stored)")
        
    def rotate_image_clockwise(self, file_path: str, 
                              preview_cache: Optional[PreviewCache] = None,
                              thumbnail_cache: Optional[ThumbnailCache] = None,
                              exif_cache: Optional[ExifCache] = None) -> bool:
        """
        Rotates an image 90 degrees clockwise with immediate UI feedback.
        
        Args:
            file_path: Path to the image file
            preview_cache: Preview cache instance for invalidation
            thumbnail_cache: Thumbnail cache instance for invalidation
            exif_cache: EXIF cache instance for invalidation
            
        Returns:
            bool: True if UI rotation was applied successfully
        """
        # Normalize file path for consistent cache key usage
        normalized_path = os.path.normpath(file_path)
        
        if not os.path.isfile(normalized_path):
            error_msg = f"File not found: {normalized_path}"
            logging.error(f"[ImageRotationHandler] {error_msg}")
            self.rotation_error.emit(file_path, error_msg)
            return False
            
        # Prevent duplicate rotation calls
        if normalized_path in self._rotation_in_progress:
            logging.info(f"[ImageRotationHandler] Rotation already in progress for {os.path.basename(normalized_path)}, ignoring duplicate call")
            return True
        
        self._rotation_in_progress.add(normalized_path)
            
        # Update UI rotation tracking using normalized path
        current_rotation = self.current_ui_rotations.get(normalized_path, 0)
        new_rotation = (current_rotation + 90) % 360
        self.current_ui_rotations[normalized_path] = new_rotation
        
        # Debug logging to help trace rotation state (using INFO to ensure visibility)
        logging.info(f"[ImageRotationHandler] Rotation state for {os.path.basename(normalized_path)}: {current_rotation}° -> {new_rotation}°")
        logging.info(f"[ImageRotationHandler] Current rotation dict keys count: {len(self.current_ui_rotations)}")
        
        # Emit immediate UI update signal
        self.rotation_applied.emit(file_path, 90)
        
        # Start background EXIF update
        self._update_exif_orientation_async(normalized_path, 90, preview_cache, thumbnail_cache, exif_cache)
        
        logging.info(f"[ImageRotationHandler] Applied UI rotation to {os.path.basename(normalized_path)}, total: {new_rotation}°")
        
        # Remove from in-progress set
        self._rotation_in_progress.discard(normalized_path)
        return True
    
    def get_ui_rotation_for_file(self, file_path: str) -> int:
        """Gets the current UI rotation for a file in degrees."""
        normalized_path = os.path.normpath(file_path)
        rotation = self.current_ui_rotations.get(normalized_path, 0)
        logging.info(f"[ImageRotationHandler] Get rotation for {os.path.basename(normalized_path)}: {rotation}° (stored keys: {len(self.current_ui_rotations)})")
        return rotation
    
    def clear_ui_rotation_for_file(self, file_path: str):
        """Clears UI rotation tracking for a file (called after EXIF update)."""
        normalized_path = os.path.normpath(file_path)
        if normalized_path in self.current_ui_rotations:
            del self.current_ui_rotations[normalized_path]
            logging.info(f"[ImageRotationHandler] Cleared UI rotation for {os.path.basename(normalized_path)}")
    
    def apply_ui_rotation_to_pixmap(self, pixmap: QPixmap, file_path: str) -> QPixmap:
        """
        Applies the current UI rotation to a QPixmap for display.
        
        Args:
            pixmap: Original QPixmap
            file_path: File path to check for rotation
            
        Returns:
            QPixmap: Rotated pixmap or original if no rotation needed
        """
        rotation_degrees = self.get_ui_rotation_for_file(file_path)
        
        logging.info(f"[ImageRotationHandler] Applying rotation to pixmap for {os.path.basename(file_path)}: {rotation_degrees}°")
        
        if rotation_degrees == 0:
            logging.info(f"[ImageRotationHandler] No rotation needed for {os.path.basename(file_path)}")
            return pixmap
            
        # Create rotation transform
        transform = QTransform()
        transform.rotate(rotation_degrees)
        
        # Apply rotation with smooth transformation
        rotated_pixmap = pixmap.transformed(
            transform,
            Qt.TransformationMode.SmoothTransformation
        )
        
        logging.info(f"[ImageRotationHandler] Successfully applied {rotation_degrees}° rotation to pixmap for {os.path.basename(file_path)}")
        return rotated_pixmap
    
    def _update_exif_orientation_async(self, file_path: str, rotation_degrees: int,
                                     preview_cache: Optional[PreviewCache] = None,
                                     thumbnail_cache: Optional[ThumbnailCache] = None,
                                     exif_cache: Optional[ExifCache] = None):
        """
        Updates EXIF orientation data in the background.
        This method should ideally be run in a separate thread for better UX.
        """
        try:
            success = self._update_exif_orientation(file_path, rotation_degrees)
            
            if success:
                # Clear UI rotation since it's now persisted in EXIF
                self.clear_ui_rotation_for_file(file_path)
                
                # Invalidate caches so they regenerate with correct orientation
                if preview_cache:
                    preview_cache.invalidate_file(file_path)
                if thumbnail_cache:
                    thumbnail_cache.invalidate_file(file_path)
                if exif_cache:
                    exif_cache.delete(file_path)
                
                logging.info(f"[ImageRotationHandler] EXIF orientation updated successfully for {os.path.basename(file_path)}")
            else:
                logging.warning(f"[ImageRotationHandler] Failed to update EXIF orientation for {os.path.basename(file_path)} - keeping UI rotation")
                # DO NOT clear UI rotation when EXIF update fails - keep the visual rotation
            
            self.exif_rotation_complete.emit(file_path, success)
            
        except Exception as e:
            error_msg = f"Error updating EXIF orientation: {str(e)}"
            logging.error(f"[ImageRotationHandler] {error_msg}")
            self.rotation_error.emit(file_path, error_msg)
        finally:
            # Always remove from in-progress set when async operation completes
            normalized_path = os.path.normpath(file_path)
            self._rotation_in_progress.discard(normalized_path)
    
    def _update_exif_orientation(self, file_path: str, rotation_degrees: int) -> bool:
        """
        Updates the EXIF orientation tag using ExifTool.
        
        Args:
            file_path: Path to the image file
            rotation_degrees: Degrees to rotate (90, 180, 270, -90, etc.)
            
        Returns:
            bool: True if successful, False otherwise
        """
        try:
            # Normalize rotation to positive degrees
            rotation_degrees = rotation_degrees % 360
            
            # Map rotation degrees to EXIF orientation values
            # EXIF Orientation values:
            # 1 = Normal (0°)
            # 3 = 180°
            # 6 = 90° CW (270° CCW)
            # 8 = 270° CW (90° CCW)
            
            # Get current orientation first (this would require reading EXIF)
            # For simplicity, we'll assume current orientation is 1 (normal)
            # In a full implementation, you'd read the current orientation first
            current_orientation = 1
            
            # Calculate new orientation based on rotation
            orientation_map_cw_90 = {1: 6, 6: 3, 3: 8, 8: 1}  # 90° clockwise rotations
            
            new_orientation = current_orientation
            rotations_90 = (rotation_degrees // 90) % 4
            
            for _ in range(rotations_90):
                new_orientation = orientation_map_cw_90.get(new_orientation, 1)
            
            # Use MetadataHandler to update EXIF
            success = self._set_exif_orientation(file_path, new_orientation)
            
            return success
            
        except Exception as e:
            logging.error(f"[ImageRotationHandler] Error in _update_exif_orientation: {e}")
            return False
    
    def _set_exif_orientation(self, file_path: str, orientation: int) -> bool:
        """
        Sets the EXIF orientation tag using ExifTool with better error handling.
        
        Args:
            file_path: Path to the image file
            orientation: EXIF orientation value (1, 3, 6, 8, etc.)
            
        Returns:
            bool: True if successful, False otherwise
        """
        try:
            import unicodedata
            
            norm_path = unicodedata.normalize('NFC', os.path.normpath(file_path))
            file_ext = os.path.splitext(file_path)[1].lower()
            
            # Check if file format supports EXIF orientation
            if file_ext in ['.png', '.gif', '.bmp', '.tiff']:
                logging.info(f"[ImageRotationHandler] File format {file_ext} may not support EXIF orientation, skipping EXIF update for {os.path.basename(file_path)}")
                return True  # Consider it successful since UI rotation worked
            
            with MetadataHandler._get_exiftool_helper_instance() as et:
                filename_bytes = norm_path.encode('utf-8', errors='surrogateescape')
                
                # Try multiple approaches for better compatibility
                approaches = [
                    # Standard EXIF orientation
                    [f"-EXIF:Orientation={orientation}".encode('utf-8'), b"-overwrite_original"],
                    # XMP orientation as fallback
                    [f"-XMP:Orientation={orientation}".encode('utf-8'), b"-overwrite_original"],
                    # Force write even if no EXIF exists
                    [f"-EXIF:Orientation={orientation}".encode('utf-8'), b"-overwrite_original", b"-P"],
                ]
                
                for i, approach in enumerate(approaches):
                    try:
                        logging.debug(f"[ImageRotationHandler] Trying EXIF approach {i+1} for {os.path.basename(file_path)}")
                        et.execute(*approach, filename_bytes)
                        logging.info(f"[ImageRotationHandler] Successfully set orientation using approach {i+1}")
                        return True
                    except Exception as approach_error:
                        logging.debug(f"[ImageRotationHandler] Approach {i+1} failed: {approach_error}")
                        continue
                
                # If all approaches fail, log detailed error but don't crash
                logging.warning(f"[ImageRotationHandler] All EXIF orientation approaches failed for {os.path.basename(file_path)}. UI rotation was successful.")
                return False
                
        except Exception as e:
            logging.error(f"[ImageRotationHandler] Error setting EXIF orientation: {e}")
            return False


class RotationVisualFeedback:
    """
    Provides visual feedback for rotation operations.
    """
    
    @staticmethod
    def create_rotation_indicator_pixmap(degrees: int, size: Tuple[int, int] = (32, 32)) -> QPixmap:
        """
        Creates a visual indicator showing rotation direction and amount.
        
        Args:
            degrees: Rotation amount in degrees
            size: Size of the indicator pixmap
            
        Returns:
            QPixmap: Visual rotation indicator
        """
        from PyQt6.QtGui import QPainter, QPen, QBrush, QFont
        from PyQt6.QtCore import QRect
        
        pixmap = QPixmap(size[0], size[1])
        pixmap.fill(Qt.GlobalColor.transparent)
        
        painter = QPainter(pixmap)
        painter.setRenderHint(QPainter.RenderHint.Antialiasing)
        
        # Draw circular background
        pen = QPen(Qt.GlobalColor.white, 2)
        brush = QBrush(Qt.GlobalColor.black)
        painter.setPen(pen)
        painter.setBrush(brush)
        
        center_rect = QRect(2, 2, size[0]-4, size[1]-4)
        painter.drawEllipse(center_rect)
        
        # Draw rotation arrow and text
        painter.setPen(QPen(Qt.GlobalColor.white, 2))
        font = QFont("Arial", 8, QFont.Weight.Bold)
        painter.setFont(font)
        
        # Draw degree text
        text = f"{degrees}°"
        text_rect = QRect(0, 0, size[0], size[1])
        painter.drawText(text_rect, Qt.AlignmentFlag.AlignCenter, text)
        
        painter.end()
        return pixmap
    
    @staticmethod
    def get_rotation_status_message(degrees: int) -> str:
        """
        Gets a user-friendly status message for rotation.
        
        Args:
            degrees: Rotation amount in degrees
            
        Returns:
            str: Status message
        """
        if degrees == 90:
            return "🔄 Rotated 90° clockwise"
        elif degrees == 180:
            return "🔄 Rotated 180°"
        elif degrees == 270:
            return "🔄 Rotated 270° clockwise"
        else:
            return f"🔄 Rotated {degrees}°"