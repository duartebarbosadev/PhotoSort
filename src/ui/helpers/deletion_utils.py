from __future__ import annotations
from dataclasses import dataclass
from typing import Optional

# Lightweight UI-agnostic description of how a marked/unmarked item should look.
# The actual QColor / palette lookup happens in MainWindow; we only decide suffix logic here.


@dataclass(frozen=True)
class DeletionPresentation:
    text: str
    is_marked: bool
    is_blurred: Optional[bool]


def build_item_text(basename: str, is_marked: bool, is_blurred: Optional[bool]) -> str:
    """Return the display text given mark + blur states.

    Rules (mirrors legacy inline logic):
    - Append (DELETED) when marked.
    - Append (Blurred) when blurred.
    - Order: filename (DELETED) (Blurred)
    """
    parts = [basename]
    if is_marked:
        parts.append("(DELETED)")
    if is_blurred:
        parts.append("(Blurred)")
    return " ".join(parts)


def build_presentation(
    basename: str, is_marked: bool, is_blurred: Optional[bool]
) -> DeletionPresentation:
    return DeletionPresentation(
        text=build_item_text(basename, is_marked, is_blurred),
        is_marked=is_marked,
        is_blurred=is_blurred,
    )
