import pytest
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


@pytest.mark.parametrize(
    "initial_paths,new_paths,expected_final_length",
    [
        (["a.jpg", "b.jpg"], ["a.jpg", "b.jpg"], 1),  # Same selection, no extra update
        (["a.jpg", "b.jpg"], ["b.jpg"], 2),  # Changed selection, should update
        (["a.jpg"], ["a.jpg", "b.jpg", "c.jpg"], 2),  # Multiple files
        (
            [],
            ["new.jpg"],
            1,
        ),  # From empty to single - only updates when changing TO non-empty
    ],
)
def test_metadata_refresh_updates_on_selection_change(
    initial_paths, new_paths, expected_final_length
):
    """Test that metadata sidebar updates when selection changes but not when it stays the same."""
    ctx = DummyCtx()
    mc = MetadataController(ctx)

    # Set initial selection and refresh
    ctx._paths = initial_paths
    mc.refresh_for_selection()

    # Verify initial update (only if initial_paths is non-empty)
    initial_update_count = 1 if initial_paths else 0
    assert len(ctx.metadata_sidebar.updated) == initial_update_count
    if initial_paths:
        assert ctx.metadata_sidebar.updated[0] == initial_paths

    # Change selection and refresh again
    ctx._paths = new_paths
    mc.refresh_for_selection()

    # Should have expected number of total updates
    assert len(ctx.metadata_sidebar.updated) == expected_final_length
    if expected_final_length > initial_update_count:
        assert ctx.metadata_sidebar.updated[-1] == new_paths


def test_metadata_refresh_skipped_when_sidebar_hidden():
    """Test that metadata refresh is skipped when sidebar is not visible."""
    ctx = DummyCtx()
    ctx.sidebar_visible = False
    mc = MetadataController(ctx)
    ctx._paths = ["x.jpg"]
    mc.refresh_for_selection()
    assert ctx.metadata_sidebar.updated == []


@pytest.mark.parametrize("sidebar_visible", [True, False])
def test_metadata_sidebar_visibility_behavior(sidebar_visible):
    """Test metadata controller behavior with different sidebar visibility states."""
    ctx = DummyCtx()
    ctx.sidebar_visible = sidebar_visible
    mc = MetadataController(ctx)
    ctx._paths = ["test.jpg"]
    mc.refresh_for_selection()

    if sidebar_visible:
        assert len(ctx.metadata_sidebar.updated) == 1
        assert ctx.metadata_sidebar.updated[0] == ["test.jpg"]
    else:
        assert ctx.metadata_sidebar.updated == []
