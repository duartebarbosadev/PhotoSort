from PyQt6.QtCore import QSortFilterProxyModel, Qt
from PyQt6.QtGui import QStandardItem, QStandardItemModel
from PyQt6.QtWidgets import QApplication

from ui.controllers.deletion_mark_controller import DeletionMarkController

_app = QApplication.instance() or QApplication([])


class _MarkState:
    def __init__(self):
        self.marked_for_deletion: set[str] = set()

    def is_marked_for_deletion(self, path: str) -> bool:
        return path in self.marked_for_deletion

    def set_deletion_marks(self, state: dict[str, bool]) -> int:
        changed = 0
        for path, marked in state.items():
            if marked != (path in self.marked_for_deletion):
                changed += 1
            if marked:
                self.marked_for_deletion.add(path)
            else:
                self.marked_for_deletion.discard(path)
        return changed

    def get_marked_files(self):
        return list(self.marked_for_deletion)


def _model(paths: list[str]):
    source = QStandardItemModel()
    for path in paths:
        item = QStandardItem(path)
        item.setData({"path": path, "is_blurred": False}, Qt.ItemDataRole.UserRole)
        source.appendRow(item)
    proxy = QSortFilterProxyModel()
    proxy.setSourceModel(source)
    return source, proxy


def test_toggle_and_clear_batches_never_call_per_path_lookup():
    paths = [f"{index}.jpg" for index in range(100)]
    state = _MarkState()
    source, proxy = _model(paths)
    controller = DeletionMarkController(state, state.is_marked_for_deletion)

    def forbidden_lookup(_path):
        raise AssertionError("bulk operations must not traverse once per path")

    assert (
        controller.toggle_paths(
            paths[:75],
            forbidden_lookup,
            source,
            proxy,
        )
        == 75
    )
    assert state.marked_for_deletion == set(paths[:75])

    assert (
        controller.clear_all_and_update(
            forbidden_lookup,
            source,
            proxy,
        )
        == 75
    )
    assert state.marked_for_deletion == set()
