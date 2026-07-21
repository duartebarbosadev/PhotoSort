from PyQt6.QtWidgets import QApplication
from PyQt6.QtGui import QPixmap
from PyQt6.QtCore import Qt
from PyQt6.QtTest import QSignalSpy, QTest

from src.ui.advanced_image_viewer import IndividualViewer, SynchronizedImageViewer


# Ensure a QApplication exists for widget tests.
_app = QApplication.instance() or QApplication([])


def _make_image(path: str, size: int = 16):
    pixmap = QPixmap(size, size)
    pixmap.fill(Qt.GlobalColor.red)
    return {"pixmap": pixmap, "path": path, "rating": 0}


def test_viewer_pool_shrinks_after_single_selection():
    viewer = SynchronizedImageViewer()

    images = [_make_image(f"img_{idx}.jpg") for idx in range(5)]
    viewer.set_images_data(images)

    assert len(viewer.image_viewers) == len(images)

    viewer.set_image_data(images[0])

    assert len(viewer.image_viewers) == 1
    assert viewer.image_viewers[0].get_file_path() == images[0]["path"]

    viewer.deleteLater()


def test_viewer_pool_matches_multi_selection_size():
    viewer = SynchronizedImageViewer()

    images = [_make_image(f"multi_{idx}.jpg") for idx in range(4)]

    viewer.set_images_data(images)
    assert len(viewer.image_viewers) == 4

    reduced_images = images[:2]
    viewer.set_images_data(reduced_images)
    assert len(viewer.image_viewers) == 2

    viewer.set_image_data(reduced_images[0])
    assert len(viewer.image_viewers) == 1

    viewer.deleteLater()


def test_preview_upgrade_does_not_rebuild_view_mode():
    viewer = SynchronizedImageViewer()
    initial = _make_image("upgrade.jpg", size=16)
    viewer.set_image_data(initial)
    focused_events = []
    viewer.focused_image_changed.connect(
        lambda index, path: focused_events.append((index, path))
    )

    upgraded = _make_image("upgrade.jpg", size=32)
    assert viewer.update_image_pixmap(upgraded["path"], upgraded["pixmap"], rating=3)

    assert focused_events == []
    assert viewer.get_primary_viewer().get_current_pixmap().size().width() == 32
    viewer.deleteLater()


def test_preview_upgrade_can_preserve_normalized_crop():
    viewer = SynchronizedImageViewer()
    viewer.resize(700, 500)
    viewer.show()
    initial = _make_image("detail.jpg", size=400)
    viewer.set_image_data(initial)
    QTest.qWait(80)
    image_view = viewer.get_primary_viewer().image_view
    image_view.set_zoom_factor(image_view.get_zoom_factor() * 1.8)
    image_view.centerOn(280, 120)
    old_fractional_scale = image_view.get_zoom_factor() * image_view.sceneRect().width()

    upgraded = _make_image("detail.jpg", size=1200)
    assert viewer.update_image_pixmap(
        upgraded["path"],
        upgraded["pixmap"],
        preserve_view=True,
    )

    new_fractional_scale = image_view.get_zoom_factor() * image_view.sceneRect().width()
    assert abs(new_fractional_scale - old_fractional_scale) < 1.0
    viewer.deleteLater()


def test_loading_placeholder_preserves_path_for_async_upgrade():
    viewer = SynchronizedImageViewer()
    viewer.set_image_data({"pixmap": None, "path": "pending.arw", "rating": 2})

    assert viewer.displays_path("pending.arw")
    upgraded = _make_image("pending.arw", size=32)
    assert viewer.update_image_pixmap("pending.arw", upgraded["pixmap"], rating=2)
    assert viewer.get_primary_viewer().get_current_pixmap().width() == 32
    viewer.deleteLater()


def test_side_by_side_keeps_pending_preview_slot_visible():
    viewer = SynchronizedImageViewer()
    viewer.resize(800, 500)
    viewer.show()
    _app.processEvents()

    viewer.set_images_data(
        [
            _make_image("cached-winner.jpg"),
            {"pixmap": None, "path": "loading-challenger.arw", "rating": 0},
        ]
    )
    _app.processEvents()

    assert len(viewer.image_viewers) == 2
    assert all(not slot.isHidden() for slot in viewer.image_viewers)
    assert all(size > 0 for size in viewer.viewer_splitter.sizes())
    assert viewer.image_viewers[1].get_file_path() == "loading-challenger.arw"

    upgraded = _make_image("loading-challenger.arw", size=32)
    assert viewer.update_image_pixmap(upgraded["path"], upgraded["pixmap"])
    assert all(not slot.isHidden() for slot in viewer.image_viewers)

    viewer.deleteLater()


def test_preview_upgrade_updates_every_slot_showing_same_rotation_source():
    viewer = SynchronizedImageViewer()
    loading = {"pixmap": None, "path": "rotation.arw", "rating": 0}
    viewer.set_images_data([loading.copy(), loading.copy()])

    upgraded = _make_image("rotation.arw", size=40)
    assert viewer.update_image_pixmap("rotation.arw", upgraded["pixmap"])
    assert [slot.get_current_pixmap().width() for slot in viewer.image_viewers] == [
        40,
        40,
    ]
    viewer.deleteLater()


def test_repeated_layout_changes_coalesce_to_one_fit():
    viewer = SynchronizedImageViewer()
    timeout_spy = QSignalSpy(viewer._layout_fit_timer.timeout)

    for _ in range(10):
        viewer._schedule_layout_fit()
    QTest.qWait(80)

    assert len(timeout_spy) == 1
    viewer.deleteLater()


def test_fit_stays_on_preview_but_zoom_and_actual_size_request_detail():
    viewer = SynchronizedImageViewer()
    viewer.set_image_data(_make_image("detail-intent.jpg", size=300))
    requests = QSignalSpy(viewer.detail_requested)

    viewer._fit_all()
    assert len(requests) == 0
    viewer._zoom_in_all()
    viewer._actual_size_all()

    assert [event[0] for event in requests] == ["zoom", "actual_size"]
    viewer.deleteLater()


def test_individual_viewer_context_menu_includes_show_in_explorer(monkeypatch):
    viewer = IndividualViewer()
    viewer._file_path = "/tmp/example.jpg"

    captured_actions = []

    def fake_exec(menu, *_args, **_kwargs):
        captured_actions.extend(action.text() for action in menu.actions())
        return None

    monkeypatch.setattr("src.ui.advanced_image_viewer.QMenu.exec", fake_exec)

    viewer._show_context_menu(viewer.rect().center())

    assert "Show in Explorer" in captured_actions

    viewer.deleteLater()
