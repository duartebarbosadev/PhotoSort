"""Durable presentation bookmarks, independent of disposable analysis caches."""

import hashlib
import json
import logging
import os
from pathlib import Path
import tempfile

from core.runtime_paths import resolve_user_data_dir

logger = logging.getLogger(__name__)
WORKFLOWS = ("organize", "easy_delete", "fix_rotation", "pick_best", "cull")


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

    def load(self, folder: str) -> dict:
        try:
            data = json.loads(self._path(folder).read_text(encoding="utf-8"))
            if (
                not isinstance(data, dict)
                or data.get("version") != 1
                or data.get("folder") != folder_key(folder)
                or data.get("workflow") not in WORKFLOWS
                or not isinstance(data.get("views"), dict)
            ):
                return {}
            return {
                "workflow": data["workflow"],
                "views": {
                    key: value
                    for key, value in data["views"].items()
                    if key in WORKFLOWS and isinstance(value, dict)
                },
            }
        except FileNotFoundError:
            return {}
        except OSError, ValueError:
            logger.warning(
                "Could not read saved folder view for %s", folder, exc_info=True
            )
            return {}

    def save(self, folder: str, bookmark: dict) -> None:
        path = self._path(folder)
        path.parent.mkdir(parents=True, exist_ok=True)
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(
                mode="w", encoding="utf-8", dir=path.parent, delete=False
            ) as output:
                temporary = output.name
                json.dump(
                    {**bookmark, "version": 1, "folder": folder_key(folder)}, output
                )
                output.flush()
                os.fsync(output.fileno())
            os.replace(temporary, path)
        finally:
            if temporary and os.path.exists(temporary):
                os.unlink(temporary)
