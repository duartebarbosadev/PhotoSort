import os
from types import SimpleNamespace
from unittest.mock import Mock

os.environ.setdefault("QT_QPA_PLATFORM", "offscreen")

from ui.main_window import MainWindow


def test_organize_trash_delegates_to_background_batch(tmp_path):
    first = str(tmp_path / "first.jpg")
    second = str(tmp_path / "second.jpg")
    (tmp_path / "first.jpg").write_bytes(b"first")
    (tmp_path / "second.jpg").write_bytes(b"second")

    started: dict[str, object] = {}

    def start_batch(targets, represented_by_target, completion):
        started.update(
            targets=targets,
            represented_by_target=represented_by_target,
            completion=completion,
        )
        return True

    finish = Mock()
    context = SimpleNamespace(
        dialog_manager=SimpleNamespace(
            show_confirm_delete_dialog=lambda _paths: True,
        ),
        grouping_step_widget=SimpleNamespace(known_directory_paths=lambda: set()),
        _start_deletion_batch=start_batch,
        _finish_ad_hoc_deletion=finish,
    )

    MainWindow._trash_from_organize(context, "", [first, second])

    assert started["targets"] == [first, second]
    assert started["represented_by_target"] == {
        first: [first],
        second: [second],
    }

    started["completion"]([first], [first], {second: "failed"}, set())
    finish.assert_called_once_with(
        [first],
        [first],
        {second: "failed"},
        operation_label="Trash",
    )
