"""
First-Run Intro Video Dialog
Plays a short introductory video the first time the user opens PhotoSort.
"""

import logging

from PyQt6.QtCore import QEasingCurve, QPropertyAnimation, QRectF, QUrl, Qt
from PyQt6.QtGui import QGuiApplication, QKeyEvent, QPainterPath, QRegion
from PyQt6.QtMultimedia import QAudioOutput, QMediaPlayer
from PyQt6.QtMultimediaWidgets import QGraphicsVideoItem
from PyQt6.QtWidgets import (
    QDialog,
    QFrame,
    QGraphicsDropShadowEffect,
    QGraphicsScene,
    QGraphicsView,
    QPushButton,
    QVBoxLayout,
)

logger = logging.getLogger(__name__)

# Native aspect ratio of the intro video (16:9) used to size the dialog.
_INTRO_ASPECT_RATIO = 16 / 9
_INTRO_MAX_WIDTH = 1000
_INTRO_SCREEN_FRACTION = 0.72
_CORNER_RADIUS = 18


class IntroVideoDialog(QDialog):
    """A frameless, modal dialog that plays PhotoSort's first-run intro video.

    Presents the clip in a rounded, drop-shadowed card with a subtle fade-in,
    with a floating "Skip Intro" button layered directly on top of the video.

    The video is rendered with QGraphicsVideoItem inside a QGraphicsView
    rather than QVideoWidget: QVideoWidget paints through a native platform
    surface that always renders above ordinary sibling widgets, making any
    overlay button layered on top of it unreliable (invisible, or visible but
    unclickable, depending on the platform). QGraphicsVideoItem instead draws
    through the normal Qt graphics/paint pipeline, so a regular QPushButton
    child can be safely stacked on top of it.

    The dialog also closes automatically once playback finishes.
    """

    def __init__(self, video_path: str, parent=None):
        super().__init__(parent)
        self._video_path = video_path

        self.setWindowTitle("Welcome to PhotoSort")
        self.setWindowFlags(Qt.WindowType.Dialog | Qt.WindowType.FramelessWindowHint)
        self.setModal(True)
        self.setAttribute(Qt.WidgetAttribute.WA_TranslucentBackground, True)
        self.setObjectName("introVideoDialog")

        self._resize_to_screen()
        self._setup_ui()
        self._setup_video()

    def _resize_to_screen(self):
        """Size the dialog to a comfortable fraction of the primary screen."""
        screen = QGuiApplication.primaryScreen()
        available = screen.availableGeometry() if screen else None
        max_width = _INTRO_MAX_WIDTH
        if available is not None:
            max_width = min(
                _INTRO_MAX_WIDTH, int(available.width() * _INTRO_SCREEN_FRACTION)
            )
        width = max(480, max_width)
        height = int(width / _INTRO_ASPECT_RATIO)
        self.resize(width, height)

        if available is not None:
            self.move(
                available.center().x() - width // 2,
                available.center().y() - height // 2,
            )

    def _setup_ui(self):
        """Build the rounded video card with a skip button overlaid on top."""
        outer = QVBoxLayout(self)
        outer.setContentsMargins(0, 0, 0, 0)
        outer.setSpacing(0)

        card = QFrame()  # Rounded, shadowed container that hosts the video view
        card.setObjectName("introVideoCard")
        card_layout = QVBoxLayout(card)
        card_layout.setContentsMargins(0, 0, 0, 0)
        card_layout.setSpacing(0)

        shadow = QGraphicsDropShadowEffect(card)
        shadow.setBlurRadius(48)
        shadow.setOffset(0, 12)
        shadow.setColor(Qt.GlobalColor.black)
        card.setGraphicsEffect(shadow)

        self.video_scene = QGraphicsScene(self)
        self.video_item = QGraphicsVideoItem()
        self.video_scene.addItem(self.video_item)

        self.video_view = QGraphicsView(self.video_scene, card)
        self.video_view.setObjectName("introVideoView")
        self.video_view.setFrameShape(QFrame.Shape.NoFrame)
        self.video_view.setHorizontalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAlwaysOff
        )
        self.video_view.setVerticalScrollBarPolicy(
            Qt.ScrollBarPolicy.ScrollBarAlwaysOff
        )
        card_layout.addWidget(self.video_view)

        outer.addWidget(card)

        # Floating skip control layered directly over the top-right corner of
        # the video. Safe here because the video is drawn via QGraphicsView's
        # normal (non-native) paint pipeline, so ordinary widget stacking works.
        self.skip_button = QPushButton("Skip Intro  ⏭", self.video_view.viewport())
        self.skip_button.setObjectName("introSkipButton")
        self.skip_button.setCursor(Qt.CursorShape.PointingHandCursor)
        self.skip_button.clicked.connect(self._on_skip_clicked)
        self.skip_button.adjustSize()
        self.skip_button.raise_()

        self._position_skip_button()

    def _position_skip_button(self):
        margin = 18
        self.skip_button.move(
            self.video_view.viewport().width() - self.skip_button.width() - margin,
            margin,
        )

    def _resize_video_item(self):
        size = self.video_view.viewport().size()
        self.video_item.setSize(size.toSizeF())
        self.video_scene.setSceneRect(0, 0, size.width(), size.height())

    def _update_mask(self):
        """Clip the whole window to rounded corners.

        The video is drawn through a QGraphicsView, whose viewport doesn't
        honor border-radius from the stylesheet either, so we mask the
        top-level window instead to guarantee rounded corners on every
        platform.
        """
        path = QPainterPath()
        path.addRoundedRect(QRectF(self.rect()), _CORNER_RADIUS, _CORNER_RADIUS)
        self.setMask(QRegion(path.toFillPolygon().toPolygon()))

    def _setup_video(self):
        self.media_player = QMediaPlayer(self)
        self.audio_output = QAudioOutput(self)
        self.media_player.setAudioOutput(self.audio_output)
        self.media_player.setVideoOutput(self.video_item)
        self.media_player.setSource(QUrl.fromLocalFile(self._video_path))
        self.media_player.mediaStatusChanged.connect(self._on_media_status_changed)
        self.media_player.errorOccurred.connect(self._on_media_error)

    def _fade_in(self):
        self.setWindowOpacity(0.0)
        self._fade_animation = QPropertyAnimation(self, b"windowOpacity", self)
        self._fade_animation.setDuration(320)
        self._fade_animation.setStartValue(0.0)
        self._fade_animation.setEndValue(1.0)
        self._fade_animation.setEasingCurve(QEasingCurve.Type.OutCubic)
        self._fade_animation.start()

    def showEvent(self, event):
        super().showEvent(event)
        self._resize_video_item()
        self._position_skip_button()
        self.skip_button.raise_()
        self._update_mask()
        self._fade_in()
        self.media_player.play()

    def resizeEvent(self, event):
        super().resizeEvent(event)
        self._resize_video_item()
        self._position_skip_button()
        self.skip_button.raise_()
        self._update_mask()

    def keyPressEvent(self, event: QKeyEvent):
        if event.key() in (Qt.Key.Key_Escape, Qt.Key.Key_Space, Qt.Key.Key_Return):
            self._on_skip_clicked()
            return
        super().keyPressEvent(event)

    def _on_media_status_changed(self, status: QMediaPlayer.MediaStatus):
        if status == QMediaPlayer.MediaStatus.EndOfMedia:
            logger.debug("Intro video finished playing.")
            self.accept()

    def _on_media_error(self, error, error_string: str):
        if error != QMediaPlayer.Error.NoError:
            logger.warning("Intro video playback error: %s", error_string)
            self.accept()

    def _on_skip_clicked(self):
        logger.debug("Intro video skipped by user.")
        self.media_player.stop()
        self.accept()

    def closeEvent(self, event):
        self.media_player.stop()
        super().closeEvent(event)
