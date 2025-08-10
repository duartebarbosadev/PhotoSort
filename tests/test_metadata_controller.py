from src.ui.controllers.metadata_controller import MetadataController


class DummySidebar:
    def __init__(self):
        self.updated = []

    def update_selection(self, paths):
        self.updated.append(paths)


class DummyCtx:
    def __init__(self):
        self.metadata_sidebar = DummySidebar()
        self.sidebar_visible = True
        self._paths = []

    def get_selected_file_paths(self):
        return self._paths

    def ensure_metadata_sidebar(self):
        pass


def test_metadata_refresh_updates_on_change():
    ctx = DummyCtx()
    mc = MetadataController(ctx)
    ctx._paths = ["a.jpg", "b.jpg"]
    mc.refresh_for_selection()
    assert ctx.metadata_sidebar.updated == [["a.jpg", "b.jpg"]]
    # No change -> no additional update
    mc.refresh_for_selection()
    assert ctx.metadata_sidebar.updated == [["a.jpg", "b.jpg"]]
    # Change selection
    ctx._paths = ["b.jpg"]
    mc.refresh_for_selection()
    assert ctx.metadata_sidebar.updated[-1] == ["b.jpg"]


def test_metadata_refresh_ignores_when_hidden():
    ctx = DummyCtx()
    ctx.sidebar_visible = False
    mc = MetadataController(ctx)
    ctx._paths = ["x.jpg"]
    mc.refresh_for_selection()
    assert ctx.metadata_sidebar.updated == []
