import types

from src.core.image_features import model_rotation_detector as mrd


def test_lazy_state_initial():
    det = mrd.ModelRotationDetector()
    st = det._state
    assert st.session is None
    assert st.tried_load is False
    assert st.load_failed is False


def test_predict_returns_zero_when_model_missing(monkeypatch):
    det = mrd.ModelRotationDetector()
    monkeypatch.setattr(det, "_resolve_model_path", lambda: None)
    angle = det.predict_rotation_angle("/nonexistent.jpg")
    assert angle == 0
    assert det._state.tried_load is True
    assert det._state.load_failed is True


def test_predict_uses_stubbed_session(monkeypatch):
    det = mrd.ModelRotationDetector()
    st = det._state
    st.tried_load = True
    st.load_failed = False

    class DummySession:
        def run(self, outputs, feed):  # noqa: D401 - simple stub
            return [[0]]

    class DummyTransforms:
        def __call__(self, img):  # returns object with unsqueeze->cpu->numpy chain
            class _T:
                def unsqueeze(self_inner, _n):
                    return types.SimpleNamespace(
                        cpu=lambda: types.SimpleNamespace(
                            numpy=lambda: __import__("numpy").zeros((1, 3, 2, 2))
                        )
                    )

            return _T()

    st.session = DummySession()
    st.transforms = lambda x: DummyTransforms()  # type: ignore
    st.output_name = "out"
    st.input_name = "in"

    angle = det.predict_rotation_angle("/nonexistent.jpg", image=object())
    assert angle in {0, 90, 180, -90}


def test_disabled_mode_returns_zero(monkeypatch):
    det = mrd.ModelRotationDetector()
    st = det._state
    st.tried_load = True
    st.load_failed = True
    st.session = None
    assert det.predict_rotation_angle("whatever.jpg") == 0
