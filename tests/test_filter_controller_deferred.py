from src.ui.controllers.filter_controller import FilterController


class DummyCtx:
    # Intentionally no proxy_model at start
    def __init__(self):
        self.app_state = object()
        self.refresh_called = 0

    def refresh_filter(self):
        self.refresh_called += 1


class DummyProxy:
    def __init__(self):
        self.current_rating_filter = None
        self.current_cluster_filter_id = None
        self.show_folders_mode_ref = None
        self.current_view_mode_ref = None
        self._regex = ""

    def setFilterRegularExpression(self, regex):
        self._regex = regex


def test_deferred_initialization_then_apply_all():
    ctx = DummyCtx()
    fc = FilterController(ctx)
    # Initial push should be pending because no proxy yet
    assert fc._initial_push_pending is True
    # Provide proxy later
    proxy = DummyProxy()
    ctx.proxy_model = proxy  # type: ignore
    # Call apply_all which should detect pending and initialize then return
    fc.apply_all(show_folders=False, current_view_mode="grid")
    assert fc._initial_push_pending is False
    assert proxy.current_rating_filter == "Show All"
    assert proxy.current_cluster_filter_id == -1
    assert ctx.refresh_called == 1


def test_deferred_initialized_via_ensure():
    ctx = DummyCtx()
    fc = FilterController(ctx)
    proxy = DummyProxy()
    ctx.proxy_model = proxy  # type: ignore
    fc.ensure_initialized(show_folders=True, current_view_mode="list")
    assert fc._initial_push_pending is False
    assert proxy.show_folders_mode_ref is True
    assert proxy.current_view_mode_ref == "list"
    assert ctx.refresh_called == 1
