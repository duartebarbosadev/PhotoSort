"""Shared cancellation-aware waiting for background analysis batches."""

from collections.abc import Callable, Iterable, Iterator
from concurrent.futures import FIRST_COMPLETED, Future, wait


def completed_until_cancelled(
    futures: Iterable[Future], should_cancel: Callable[[], bool]
) -> Iterator[Future]:
    """Yield completed work without needing a result to notice cancellation.

    Executor owners still cancel queued tasks and decide how running work drains.
    Already cancelled futures are skipped, including executor-cancelled futures
    that never pass through a worker thread to notify as_completed waiters.
    """
    pending = set(futures)
    while pending and not should_cancel():
        pending = {future for future in pending if not future.cancelled()}
        if not pending:
            break
        done, pending = wait(pending, timeout=0.1, return_when=FIRST_COMPLETED)
        for future in done:
            if should_cancel():
                return
            if not future.cancelled():
                yield future
