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


def test_navigation_group_cyclic_with_deleted_items():
    """Test that group navigation cycles correctly and skips deleted items when skip_deleted=True."""
    ctx = Ctx(["a", "b", "c"], deleted=["b"])
    nav = NavigationController(ctx)
    # simulate current path 'a'
    ctx.find_proxy_index_for_path("a")

    # Navigate right: should skip deleted 'b' and go to 'c'
    nav.navigate_group("right")
    assert ctx.last_selected == "c"

    # Navigate right again: should wrap around to 'a'
    nav.navigate_group("right")
    assert ctx.last_selected == "a"

    # Navigate left: should go to 'c'
    nav.navigate_group("left")
    assert ctx.last_selected == "c"


def test_navigation_linear_sequential_with_boundaries():
    """Test linear navigation handles start/end boundaries and deleted items correctly."""
    ctx = Ctx(["a", "b", "c", "d"], deleted=["c"])
    nav = NavigationController(ctx)

    # Start with no selection
    ctx.last_selected = None

    # Navigate down: should select first non-deleted item 'a'
    nav.navigate_linear("down")
    assert ctx.last_selected == "a"

    # Navigate down: should go to 'b'
    nav.navigate_linear("down")
    assert ctx.last_selected == "b"

    # Navigate down: should skip deleted 'c' and go to 'd'
    nav.navigate_linear("down")
    assert ctx.last_selected == "d"

    # Navigate down at end: should stay at 'd'
    nav.navigate_linear("down")
    assert ctx.last_selected == "d"

    # Navigate up: should skip 'c' and go to 'b'
    nav.navigate_linear("up")
    assert ctx.last_selected == "b"

    # Navigate up: should go to 'a'
    nav.navigate_linear("up")
    assert ctx.last_selected == "a"


def test_navigation_linear_includes_deleted_when_skip_false():
    """Test that linear navigation includes deleted items when skip_deleted=False."""
    ctx = Ctx(["a", "b", "c"], deleted=["b"])
    nav = NavigationController(ctx)
    ctx.find_proxy_index_for_path("a")
    nav.navigate_linear("down", skip_deleted=False)  # should land on deleted 'b'
    assert ctx.last_selected == "b"


def test_navigation_group_includes_deleted_when_skip_false():
    """Test that group navigation includes deleted items when skip_deleted=False."""
    ctx = Ctx(["a", "b", "c"], deleted=["b"])
    nav = NavigationController(ctx)
    ctx.find_proxy_index_for_path("a")
    nav.navigate_group("right", skip_deleted=False)  # should land on deleted 'b'
    assert ctx.last_selected == "b"


def test_navigation_skip_deleted_with_multiple_deleted_items():
    """Test navigation correctly skips multiple deleted items in both directions.
    Scenario: 5 images [a, b, c, d, e] where b and d are deleted, user selects c.
    - Down navigation should skip d and go to e
    - Up navigation should skip b and go to a
    """
    ctx = Ctx(["a", "b", "c", "d", "e"], deleted=["b", "d"])
    nav = NavigationController(ctx)

    # Select image c (middle item, surrounded by deleted items)
    ctx.find_proxy_index_for_path("c")
    assert ctx.last_selected == "c"

    # Navigate right - should skip d (deleted) and go to e
    nav.navigate_linear("down", skip_deleted=True)
    assert ctx.last_selected == "e", "Should go to e when skipping deleted d"

    # Reset to c and navigate left - should skip b (deleted) and go to a
    ctx.find_proxy_index_for_path("c")
    nav.navigate_linear("up", skip_deleted=True)
    assert ctx.last_selected == "a", "Should go to a when skipping deleted b"
