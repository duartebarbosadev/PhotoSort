from src.ui.controllers.navigation_controller import NavigationController
from PyQt6.QtCore import Qt, QModelIndex


class DummyIndex(QModelIndex):
    def __init__(self, row: int):
        super().__init__()
        self._row = row

    def isValid(self):
        return self._row >= 0

    def row(self):
        return self._row


class DummyItem:
    def __init__(self, path: str):
        self._path = path

    def data(self, role):
        if role == Qt.ItemDataRole.UserRole:
            return {"path": self._path}
        return None


class DummyView:
    def __init__(self, ctx):
        self.ctx = ctx
        self._current = DummyIndex(-1)

    def currentIndex(self):
        return self._current

    def selectionModel(self):
        return None

    # Simulate setCurrentIndex used in controller code path (indirectly)
    def setCurrentIndex(self, idx):
        self._current = idx

    def setFocus(self, *_args, **_kwargs):
        pass

    def viewport(self):
        class V:
            def update(self_inner):
                pass

        return V()


class Ctx:
    def __init__(self, paths, deleted=None):
        self.paths = list(paths)
        self.deleted = set(deleted or [])
        self.last_selected = None
        self.view = DummyView(self)

    # Protocol methods
    def get_active_view(self):
        return self.view

    def is_valid_image_index(self, proxy_index):
        return proxy_index.isValid()

    def map_to_source(self, proxy_index):
        return proxy_index

    def item_from_source(self, source_index):
        row = source_index.row()
        if 0 <= row < len(self.paths):
            return DummyItem(self.paths[row])
        # fallback to last selected
        if self.last_selected and self.last_selected in self.paths:
            return DummyItem(self.last_selected)
        if self.paths:
            return DummyItem(self.paths[0])
        return None

    def get_group_sibling_images(self, current_proxy_index):
        indices = [DummyIndex(i) for i in range(len(self.paths))]
        return None, indices, None

    def find_first_visible_item(self):
        return DummyIndex(0) if self.paths else DummyIndex(-1)

    def find_proxy_index_for_path(self, path):
        if path in self.paths:
            idx = DummyIndex(self.paths.index(path))
            self.last_selected = path
            self.view.setCurrentIndex(idx)
            return idx
        return DummyIndex(-1)

    def get_all_visible_image_paths(self):
        return list(self.paths)

    def get_marked_deleted(self):
        return list(self.deleted)

    def validate_and_select_image_candidate(self, proxy_index, direction, log_skip):
        # Called after controller finds a candidate; ensure selection updates
        row = proxy_index.row()
        if 0 <= row < len(self.paths):
            self.last_selected = self.paths[row]


def test_navigation_controller_group_cycle_left_right():
    ctx = Ctx(["a", "b", "c"], deleted=["b"])
    nav = NavigationController(ctx)
    # simulate current path 'a'
    ctx.find_proxy_index_for_path("a")
    nav.navigate_group("right")  # should skip b -> c
    assert ctx.last_selected == "c"
    nav.navigate_group("right")  # wrap to a (b deleted)
    assert ctx.last_selected == "a"
    nav.navigate_group("left")  # wrap inverse -> c
    assert ctx.last_selected == "c"


def test_navigation_controller_linear_down_up():
    ctx = Ctx(["a", "b", "c", "d"], deleted=["c"])
    nav = NavigationController(ctx)
    # start with none
    ctx.last_selected = None
    nav.navigate_linear("down")  # first non-deleted -> a
    assert ctx.last_selected == "a"
    nav.navigate_linear("down")  # -> b
    assert ctx.last_selected == "b"
    nav.navigate_linear("down")  # skip c (deleted) -> d
    assert ctx.last_selected == "d"
    nav.navigate_linear("down")  # stays None (end)
    assert ctx.last_selected == "d"  # unchanged
    nav.navigate_linear("up")  # move up skipping c -> b
    assert ctx.last_selected == "b"
    nav.navigate_linear("up")  # -> a
    assert ctx.last_selected == "a"


def test_navigation_include_deleted_linear():
    """When skip_deleted is False, deleted images should be navigable in linear mode."""
    ctx = Ctx(["a", "b", "c"], deleted=["b"])
    nav = NavigationController(ctx)
    ctx.find_proxy_index_for_path("a")
    nav.navigate_linear("down", skip_deleted=False)  # should land on deleted 'b'
    assert ctx.last_selected == "b"


def test_navigation_include_deleted_group():
    """When skip_deleted is False, group navigation should visit deleted images."""
    ctx = Ctx(["a", "b", "c"], deleted=["b"])
    nav = NavigationController(ctx)
    ctx.find_proxy_index_for_path("a")
    nav.navigate_group("right", skip_deleted=False)  # should land on deleted 'b'
    assert ctx.last_selected == "b"
