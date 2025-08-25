from src.ui.helpers.statusbar_utils import build_status_bar_info
from datetime import datetime


def test_statusbar_basic_no_cluster():
    md = {"rating": 3, "date": datetime(2025, 8, 10)}
    info = build_status_bar_info(
        file_path=__file__,
        metadata=md,
        width=1920,
        height=1080,
        cluster_lookup={},
        file_data_from_model={"is_blurred": False},
    )
    msg = info.to_message()
    assert "R: 3" in msg
    assert "D: 2025-08-10" in msg
    assert "Blurred: No" in msg


def test_statusbar_with_cluster_and_blur_yes(tmp_path):
    f = tmp_path / "a.jpg"
    f.write_text("x")
    md = {"rating": 5, "date": None}
    info = build_status_bar_info(
        file_path=str(f),
        metadata=md,
        width=100,
        height=200,
        cluster_lookup={str(f): 7},
        file_data_from_model={"is_blurred": True},
    )
    msg = info.to_message()
    assert "C: 7" in msg
    assert "R: 5" in msg
    assert "D: Unknown" in msg
    assert "100x200" in msg
    assert "Blurred: Yes" in msg


def test_statusbar_size_unavailable(monkeypatch, tmp_path):
    f = tmp_path / "b.jpg"
    f.write_text("x")

    def broken_getsize(path):
        raise OSError

    monkeypatch.setattr("os.path.getsize", broken_getsize)
    md = {"rating": 0, "date": None}
    info = build_status_bar_info(
        file_path=str(f),
        metadata=md,
        width=10,
        height=10,
        cluster_lookup=None,
        file_data_from_model=None,
    )
    assert "Size: N/A" in info.to_message()
