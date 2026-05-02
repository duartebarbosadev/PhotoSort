class SelectionError(Exception):
    """Base error for selection failures."""


class MissingDependencyError(SelectionError):
    """Raised when an optional runtime dependency is required but unavailable."""


class NoSupportedImagesError(SelectionError):
    """Raised when the caller provides no supported image files."""


class NoScorableImagesError(SelectionError):
    """Raised when no images can be scored successfully."""

    def __init__(
        self,
        message: str,
        *,
        failures: list[tuple[str, str]] | None = None,
    ) -> None:
        super().__init__(message)
        self.failures = failures or []
