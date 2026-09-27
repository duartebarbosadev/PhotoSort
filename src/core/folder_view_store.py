"""Durable presentation bookmarks, independent of disposable analysis caches."""

import hashlib
import json
import logging
import os
from pathlib import Path
import stat
import tempfile
from collections.abc import Callable

from core.runtime_paths import resolve_user_data_dir

logger = logging.getLogger(__name__)
WORKFLOWS = ("organize", "easy_delete", "fix_rotation", "pick_best", "cull")
_SINGLE_PATH_FIELDS = ("path", "review")
_MULTI_PATH_FIELDS = ("selected", "before", "after", "comparison")


def bookmark_file_identity(info: os.stat_result) -> list[int]:
    """Use the shared scan/mutation snapshot; never read image contents."""
    return [info.st_dev, info.st_ino, info.st_size, info.st_mtime_ns, info.st_ctime_ns]


def bookmark_paths(bookmark: dict) -> set[str]:
    paths = set()
    for view in bookmark.get("views", {}).values():
        for field in _SINGLE_PATH_FIELDS:
            path = view.get(field)
            if isinstance(path, str):
                paths.add(path)
        for field in _MULTI_PATH_FIELDS:
            values = view.get(field)
            if isinstance(values, list):
                paths.update(path for path in values if isinstance(path, str))
    return paths


def folder_key(folder: str) -> str:
    return os.path.normcase(os.path.abspath(os.path.normpath(folder)))


class FolderViewStore:
    """Read/write one atomic JSON document per folder; called by WorkerManager."""

    def __init__(self, directory: str | None = None):
        self.directory = directory

    def _path(self, folder: str) -> Path:
        root = self.directory or resolve_user_data_dir("folder-views")
        digest = hashlib.sha256(folder_key(folder).encode()).hexdigest()
        return Path(root) / f"{digest}.json"

    @staticmethod
    def _file_identity(path: str) -> list[int] | None:
        try:
            info = os.stat(path)
            return bookmark_file_identity(info) if stat.S_ISREG(info.st_mode) else None
        except OSError:
            return None

    def load(
        self,
        folder: str,
        *,
        include_identities: bool = False,
        should_continue: Callable[[], bool] | None = None,
    ) -> dict:
        try:
            if should_continue is not None and not should_continue():
                return {}
            data = json.loads(self._path(folder).read_text(encoding="utf-8"))
            if (
                not isinstance(data, dict)
                or data.get("version") not in (1, 2)
                or data.get("folder") != folder_key(folder)
                or data.get("workflow") not in WORKFLOWS
                or not isinstance(data.get("views"), dict)
            ):
                return {}
            bookmark = {
                "workflow": data["workflow"],
                "views": {
                    key: value
                    for key, value in data["views"].items()
                    if key in WORKFLOWS and isinstance(value, dict)
                },
            }
            identities = data.get("identities", {}) if data["version"] == 2 else {}
            if not isinstance(identities, dict):
                identities = {}
            valid_paths = set()
            for path in bookmark_paths(bookmark):
                if should_continue is not None and not should_continue():
                    return {}
                expected = identities.get(path)
                if (
                    isinstance(expected, list)
                    and len(expected) == 5
                    and all(type(value) is int for value in expected)
                    and self._file_identity(path) == expected
                ):
                    valid_paths.add(path)
            for view in bookmark["views"].values():
                for field in _SINGLE_PATH_FIELDS:
                    if field in view and (
                        not isinstance(view[field], str)
                        or view[field] not in valid_paths
                    ):
                        view[field] = None
                for field in _MULTI_PATH_FIELDS:
                    values = view.get(field)
                    if isinstance(values, list):
                        surviving = [
                            p for p in values if isinstance(p, str) and p in valid_paths
                        ]
                        # A saved comparison is meaningful only with all its photos.
                        view[field] = (
                            []
                            if field == "comparison" and surviving != values
                            else surviving
                        )
            if include_identities:
                bookmark["identities"] = {p: identities[p] for p in valid_paths}
            return bookmark
        except FileNotFoundError:
            return {}
        except OSError, ValueError:
            logger.warning(
                "Could not read saved folder view for %s", folder, exc_info=True
            )
            return {}

    def save(
        self, folder: str, bookmark: dict, *, identities: dict | None = None
    ) -> None:
        if identities is None:
            identities = {p: self._file_identity(p) for p in bookmark_paths(bookmark)}
        path = self._path(folder)
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w", encoding="utf-8", dir=path.parent, delete=False
            ) as output:
                temporary = output.name
                json.dump(
                    {
                        **bookmark,
                        "version": 2,
                        "folder": folder_key(folder),
                        "identities": identities,
                    },
                    output,
                )
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, path)
        finally:
            if temporary and os.path.exists(temporary):
                os.unlink(temporary)
