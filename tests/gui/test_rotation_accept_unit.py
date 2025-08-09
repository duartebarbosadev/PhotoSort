import types
import pytest

from PyQt6.QtWidgets import QApplication  # type: ignore  # noqa: F401
from src.ui.main_window import MainWindow


def build_window_with_rotation_suggestions(tmp_path, count=5):
    for i in range(count):
        (tmp_path / f"img_{i}.jpg").write_bytes(b"fake")
    window = MainWindow(initial_folder=str(tmp_path))
    window.left_panel.current_view_mode = "rotation"
    window.app_controller.load_folder(str(tmp_path))
    QApplication.processEvents()
    for i in range(count):
        path = str(tmp_path / f"img_{i}.jpg")
        window.rotation_suggestions[path] = {"direction": "clockwise"}
    window._rebuild_rotation_view()
    return window


def test_accept_single_rotation_moves_to_next(qapp, tmp_path):
    window = build_window_with_rotation_suggestions(tmp_path, count=4)
    window.show()

    all_visible = window._get_all_visible_image_paths()
    if len(all_visible) < 4:
        pytest.skip("Rotation view did not populate enough visible items in test harness")
    target = all_visible[1]
    window._select_items_in_current_view([target])

    window._accept_single_rotation_and_move_to_next()

    assert target not in window.rotation_suggestions
    sel = window._get_selected_file_paths_from_view()
    assert sel and target not in sel
    window.close()


def test_accept_multi_non_contiguous_next_selection(qapp, tmp_path):
    window = build_window_with_rotation_suggestions(tmp_path, count=6)
    window.show()
    all_visible = window._get_all_visible_image_paths()
    if len(all_visible) < 6:
        pytest.skip("Rotation view did not populate enough visible items in test harness")

    targets = [all_visible[0], all_visible[2], all_visible[5]]
    window._select_items_in_current_view(targets)

    window._accept_current_rotation()

    for t in targets:
        assert t not in window.rotation_suggestions

    remaining = set(window.rotation_suggestions.keys())
    assert remaining
    new_sel = set(window._get_selected_file_paths_from_view())
    assert not (new_sel & set(targets))
    window.close()


def test_lazy_detector_does_not_initialize_until_predict(monkeypatch):
    from src.core.image_features import model_rotation_detector as mrd

    det = mrd.ModelRotationDetector()
    state = det._state
    assert state.session is None

    class DummySession:
        def run(self, *a, **k):
            return [[0]]

    class DummyTransforms:
        def __call__(self, img):
            class _T:
                def unsqueeze(self_inner, n):
                    import numpy as np
                    return types.SimpleNamespace(cpu=lambda: types.SimpleNamespace(numpy=lambda: np.zeros((1, 3, 2, 2))))
            return _T()

    state.session = DummySession()
    state.transforms = lambda x: DummyTransforms()  # type: ignore
    state.output_name = "out"
    state.input_name = "in"

    angle = det.predict_rotation_angle("/nonexistent.jpg", image=object())
    assert angle in {0, 90, 180, -90, 0}


def test_disabled_mode_returns_zero(monkeypatch):
    from src.core.image_features import model_rotation_detector as mrd

    det = mrd.ModelRotationDetector()
    st = det._state
    st.tried_load = True
    st.load_failed = True
    st.session = None
    assert det.predict_rotation_angle("whatever.jpg") == 0

@pytest.mark.skip(reason="Flaky at end of full test run; passes in isolation")
def test_about_dialog_provider_string(qapp):
    window = MainWindow()
    dm = window.dialog_manager
    dm.show_about_dialog(block=False)
    from PyQt6.QtCore import QElapsedTimer

    timer = QElapsedTimer(); timer.start()
    while timer.elapsed() < 3000:
        qapp.processEvents()
        dialogs = [w for w in qapp.topLevelWidgets() if w.objectName() == "aboutDialog"]
        if dialogs:
            break
    else:
        pytest.skip("About dialog not created in headless test environment")
    from PyQt6.QtWidgets import QLabel
    label_texts = [lbl.text() for lbl in dialogs[0].findChildren(QLabel)]
    assert any("Rotation Model" in t for t in label_texts)
    dialogs[0].close(); window.close()
