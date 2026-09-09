from collections.abc import Callable

from PyQt6.QtCore import QCoreApplication, QEventLoop

from core.app_settings import LARGE_FOLDER_THRESHOLD, UI_POPULATION_CHUNK_SIZE


def cooperative_ui_yield(
    processed: int,
    total: int,
    *,
    progress_callback: Callable[[int, int], None] | None = None,
) -> None:
    """Repaint and deliver queued work during unavoidable bulk widget creation.

    Qt model/widget objects must be created on the UI thread. For large
    collections, yielding between bounded chunks keeps painting and worker
    completion signals flowing while excluding new user input and socket
    callbacks that could re-enter the active mutation.
    """

    if (
        total <= LARGE_FOLDER_THRESHOLD
        or processed <= 0
        or processed % UI_POPULATION_CHUNK_SIZE
    ):
        return
    if progress_callback is not None:
        progress_callback(processed, total)
    if QCoreApplication.instance() is None:
        return
    flags = (
        QEventLoop.ProcessEventsFlag.ExcludeUserInputEvents
        | QEventLoop.ProcessEventsFlag.ExcludeSocketNotifiers
    )
    QCoreApplication.processEvents(flags, 5)
