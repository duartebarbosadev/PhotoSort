import datetime
from src.ui.helpers.cluster_utils import ClusterUtils


def build_fd(path):
    return {"path": path}


def test_group_images_by_cluster_basic():
    data = [build_fd("a.jpg"), build_fd("b.jpg"), build_fd("c.jpg")]
    clusters = {"a.jpg": 1, "b.jpg": 2, "c.jpg": 1}
    grouped = ClusterUtils.group_images_by_cluster(data, clusters)
    assert set(grouped.keys()) == {1, 2}
    assert sorted([fd["path"] for fd in grouped[1]]) == ["a.jpg", "c.jpg"]


def test_cluster_timestamps():
    data = [build_fd("a.jpg"), build_fd("b.jpg"), build_fd("c.jpg")]
    clusters = {"a.jpg": 1, "b.jpg": 2, "c.jpg": 1}
    grouped = ClusterUtils.group_images_by_cluster(data, clusters)
    today = datetime.date.today()
    date_cache = {"a.jpg": today, "b.jpg": today + datetime.timedelta(days=1)}
    ts = ClusterUtils.get_cluster_timestamps(grouped, date_cache)
    assert ts[1] == today
    assert ts[2] == today + datetime.timedelta(days=1)


def test_sort_clusters_fallback_time():
    # Only one centroid -> fallback to time ordering
    today = datetime.date.today()
    data = [build_fd("a.jpg"), build_fd("b.jpg"), build_fd("c.jpg")]
    clusters = {"a.jpg": 2, "b.jpg": 1, "c.jpg": 2}
    grouped = ClusterUtils.group_images_by_cluster(data, clusters)
    date_cache = {
        "a.jpg": today,
        "b.jpg": today - datetime.timedelta(days=2),
        "c.jpg": today - datetime.timedelta(days=1),
    }
    order = ClusterUtils.sort_clusters_by_similarity_time(
        grouped, embeddings_cache={}, date_cache=date_cache
    )
    # Expect cluster 1 (earliest date) then 2
    assert order == [1, 2]


def test_sort_clusters_with_embeddings():
    today = datetime.date.today()
    data = [build_fd("a.jpg"), build_fd("b.jpg"), build_fd("c.jpg"), build_fd("d.jpg")]
    clusters = {"a.jpg": 1, "b.jpg": 1, "c.jpg": 2, "d.jpg": 2}
    grouped = ClusterUtils.group_images_by_cluster(data, clusters)
    date_cache = {p: today for p in ["a.jpg", "b.jpg", "c.jpg", "d.jpg"]}
    # Distinct embeddings so PCA produces deterministic ordering by first component
    embeddings = {
        "a.jpg": [0.0, 0.0],
        "b.jpg": [0.0, 0.2],
        "c.jpg": [5.0, 5.0],
        "d.jpg": [5.2, 5.0],
    }
    order = ClusterUtils.sort_clusters_by_similarity_time(
        grouped, embeddings, date_cache
    )
    assert set(order) == {1, 2}
    assert len(order) == 2
