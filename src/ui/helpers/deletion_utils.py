from dataclasses import dataclass

# Lightweight UI-agnostic description of how a marked/unmarked item should look.
# The actual QColor / palette lookup happens in MainWindow; we only decide suffix logic here.


@dataclass(frozen=True)
class DeletionPresentation:
    text: str
    is_marked: bool
    is_best: bool | None
    is_blurred: bool | None


def build_item_text(
    basename: str,
    is_marked: bool,
    is_best: bool | None,
    is_blurred: bool | None,
) -> str:
    """Return the display text given mark + blur states.

    Rules (mirrors legacy inline logic):
    - Append (DELETED) when marked.
    - Append (Best) when flagged as best-shot winner.
    - Append (Blurred) when blurred.
    - Order: filename (DELETED) (Best) (Blurred)
    """
    parts = [basename]
    if is_marked:
        parts.append("(DELETED)")
    if is_best:
        parts.append("(Best)")
    if is_blurred:
        parts.append("(Blurred)")
    return " ".join(parts)


def build_presentation(
    basename: str,
    is_marked: bool,
    is_best: bool | None,
    is_blurred: bool | None,
) -> DeletionPresentation:
    return DeletionPresentation(
        text=build_item_text(basename, is_marked, is_best, is_blurred),
        is_marked=is_marked,
        is_best=is_best,
        is_blurred=is_blurred,
    )
