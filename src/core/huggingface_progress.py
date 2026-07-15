import io
import time
from collections.abc import Callable


ProgressCallback = Callable[[int, str], None]


def _format_bytes(value: float) -> str:
    units = ("B", "KB", "MB", "GB")
    amount = float(value)
    for unit in units:
        if amount < 1024 or unit == units[-1]:
            if unit == "B":
                return f"{int(amount)} {unit}"
            return f"{amount:.1f} {unit}"
        amount /= 1024
    return f"{amount:.1f} GB"


def build_hf_tqdm_class(
    callback: ProgressCallback | None,
    *,
    label: str,
    min_interval_seconds: float = 0.25,
):
    """Build a tqdm class that reports Hugging Face progress through Qt signals.

    Hugging Face's download helpers write tqdm bars to stderr by default. In the
    desktop app that is invisible to users and noisy in logs, so this class keeps
    tqdm's accounting but suppresses terminal rendering.
    """
    from tqdm.auto import tqdm

    class AppProgressTqdm(tqdm):
        def __init__(self, *args, **kwargs):
            kwargs.setdefault("file", io.StringIO())
            super().__init__(*args, **kwargs)
            self._last_app_emit = 0.0
            self._emit_app_progress(force=True)

        def display(self, *args, **kwargs):
            return None

        def clear(self, *args, **kwargs):
            return None

        def update(self, n=1):
            result = super().update(n)
            self._emit_app_progress()
            return result

        def close(self):
            self._emit_app_progress(force=True)
            return super().close()

        def _emit_app_progress(self, *, force: bool = False):
            if callback is None:
                return
            now = time.monotonic()
            if not force and now - self._last_app_emit < min_interval_seconds:
                return
            self._last_app_emit = now

            total = self.total or 0
            current = self.n or 0
            percent = int((current / total) * 100) if total else -1
            unit = str(getattr(self, "unit", "") or "")
            desc = str(getattr(self, "desc", "") or "").strip()
            prefix = label if not desc else f"{label}: {desc}"

            if unit.upper() == "B" and total:
                message = f"{prefix} {_format_bytes(current)} / {_format_bytes(total)}"
            elif total:
                message = f"{prefix} {int(current)}/{int(total)}"
            else:
                message = f"{prefix} {_format_bytes(current)} downloaded"

            callback(percent, message)

    return AppProgressTqdm
