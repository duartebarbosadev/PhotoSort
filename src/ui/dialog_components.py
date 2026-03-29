from PyQt6.QtCore import Qt
from PyQt6.QtGui import QFont
from PyQt6.QtWidgets import QFrame, QHBoxLayout, QLabel, QPushButton, QVBoxLayout


def _bind_drag_target(dialog, target, state):
    """Attach drag handlers to a specific widget surface for a frameless dialog."""

    def _press(e):
        if e.button() == Qt.MouseButton.LeftButton:
            state["offset"] = (
                e.globalPosition().toPoint() - dialog.frameGeometry().topLeft()
            )
            e.accept()

    def _move(e):
        if (e.buttons() & Qt.MouseButton.LeftButton) and state["offset"] is not None:
            dialog.move(e.globalPosition().toPoint() - state["offset"])
            e.accept()

    target.mousePressEvent = _press  # type: ignore[assignment]
    target.mouseMoveEvent = _move  # type: ignore[assignment]


def make_dialog_draggable(dialog):
    """Attach mouse handlers to a frameless dialog to support click-drag moving."""
    drag_state = {"offset": None}
    dialog._drag_state = drag_state  # type: ignore[attr-defined]
    _bind_drag_target(dialog, dialog, drag_state)


def build_dialog_header(title, icon_text, parent_layout):
    """Build a standard dialog header bar with icon and title."""
    header = QFrame()
    header.setObjectName("dialogHeader")
    h_layout = QHBoxLayout(header)
    h_layout.setContentsMargins(22, 14, 22, 14)
    h_layout.setSpacing(10)

    icon_lbl = QLabel(icon_text)
    icon_lbl.setObjectName("dialogHeaderIcon")
    h_layout.addWidget(icon_lbl)

    title_lbl = QLabel(title)
    title_lbl.setObjectName("dialogHeaderTitle")
    font = QFont()
    font.setPointSize(13)
    font.setBold(True)
    title_lbl.setFont(font)
    h_layout.addWidget(title_lbl)
    h_layout.addStretch()

    dialog = header.window()
    drag_state = getattr(dialog, "_drag_state", None)
    if drag_state is not None:
        _bind_drag_target(dialog, header, drag_state)
        _bind_drag_target(dialog, icon_lbl, drag_state)
        _bind_drag_target(dialog, title_lbl, drag_state)

    parent_layout.addWidget(header)
    return header


def build_dialog_footer(parent_layout, buttons):
    """Build a footer bar and return the frame and buttons keyed by object name."""
    footer = QFrame()
    footer.setObjectName("dialogFooter")
    f_layout = QHBoxLayout(footer)
    f_layout.setContentsMargins(22, 10, 22, 14)
    f_layout.setSpacing(10)
    f_layout.addStretch()

    created_buttons = {}
    for text, obj_name, callback, is_default in buttons:
        btn = QPushButton(text)
        btn.setObjectName(obj_name)
        btn.clicked.connect(callback)
        if is_default:
            btn.setDefault(True)
        f_layout.addWidget(btn)
        created_buttons[obj_name] = btn

    parent_layout.addWidget(footer)
    return footer, created_buttons


def build_card(object_name="dialogCard"):
    """Create a card frame and its inner VBoxLayout. Returns (card, layout)."""
    card = QFrame()
    card.setObjectName(object_name)
    layout = QVBoxLayout(card)
    layout.setSpacing(10)
    layout.setContentsMargins(16, 14, 16, 14)
    return card, layout
