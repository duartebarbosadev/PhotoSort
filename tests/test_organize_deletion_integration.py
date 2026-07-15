import os
from types import SimpleNamespace
from unittest.mock import Mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from ui.main_window import MainWindow


def test_organize_immediate_trash_removes_only_successful_paths(
    tmp_path, monkeypatch
):
    first = str(tmp_path / "first.jpg")
    second = str(tmp_path / "second.jpg")
    (tmp_path / "first.jpg").write_bytes(b"first")
    (tmp_path / "second.jpg").write_bytes(b"second")

    removed: list[str] = []
    invalidated: list[str] = []
    grouping_removed: list[list[str]] = []
    errors: list[tuple[str, str]] = []
    context = SimpleNamespace(
        dialog_manager=SimpleNamespace(
            show_confirm_delete_dialog=lambda _paths: True,
            show_error_dialog=lambda title, message: errors.append((title, message)),
        ),
        app_state=SimpleNamespace(remove_data_for_path=removed.append),
        image_pipeline=SimpleNamespace(invalidate_path=invalidated.append),
        thumbnail_loader=SimpleNamespace(invalidate_paths=Mock()),
        grouping_step_widget=SimpleNamespace(
            remove_deleted_paths=grouping_removed.append
        ),
        proxy_model=SimpleNamespace(invalidate=Mock()),
        mark_cull_model_dirty=Mock(),
        _refresh_workflow_deletion_state=Mock(),
        statusBar=lambda: SimpleNamespace(showMessage=Mock()),
    )

    def move_to_trash(path: str) -> tuple[bool, str]:
        if path == first:
            return True, "Moved to trash."
        return False, "second.jpg could not be moved"

    monkeypatch.setattr(
        "ui.main_window.ImageFileOperations.move_to_trash", move_to_trash
    )

    MainWindow._trash_from_organize(context, "", [first, second])

    assert removed == [first]
    assert invalidated == [first]
    assert grouping_removed == [[first]]
    context.thumbnail_loader.invalidate_paths.assert_called_once_with([first])
    assert errors == [("Trash Error", "second.jpg could not be moved")]
