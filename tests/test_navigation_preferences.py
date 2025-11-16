from src.ui.helpers.navigation_utils import (
    find_next_multi_image_cluster_head,
    find_next_rating_match,
    find_next_in_same_multi_cluster,
)


def test_find_next_rating_match_down_skips_deleted():
    paths = ["a", "b", "c", "d"]
    ratings = {"a": 1, "b": 5, "c": 5, "d": 3}
    deleted = {"b"}

    target = find_next_rating_match(
        paths,
        direction="down",
        current_index=0,
        target_rating=5,
        rating_lookup=lambda p: ratings.get(p),
        skip_deleted=True,
        is_deleted=lambda p: p in deleted,
    )

    assert target == "c"


def test_find_next_rating_match_up_from_end():
    paths = ["img1", "img2", "img3"]
    ratings = {"img1": 2, "img2": 4, "img3": 4}

    target = find_next_rating_match(
        paths,
        direction="up",
        current_index=-1,
        target_rating=4,
        rating_lookup=lambda p: ratings.get(p),
        skip_deleted=True,
        is_deleted=lambda _p: False,
    )

    assert target == "img3"


def test_find_next_multi_cluster_head_down():
    paths = ["a", "b", "c", "d", "e", "f"]
    clusters = {"a": 1, "b": 1, "c": 2, "d": 3, "e": 3, "f": 4}

    target = find_next_multi_image_cluster_head(
        paths,
        direction="down",
        current_index=1,
        cluster_lookup=lambda p: clusters.get(p),
        skip_deleted=True,
        is_deleted=lambda _p: False,
    )

    assert target == "d"


def test_find_next_multi_cluster_head_up_skips_singletons_and_deleted():
    paths = ["a", "b", "c", "d", "e", "f"]
    clusters = {"a": 1, "b": 1, "c": 2, "d": 3, "e": 3, "f": 3}
    deleted = {"a"}

    target = find_next_multi_image_cluster_head(
        paths,
        direction="up",
        current_index=5,
        cluster_lookup=lambda p: clusters.get(p),
        skip_deleted=True,
        is_deleted=lambda p: p in deleted,
    )

    assert target == "b"


def test_find_next_in_same_multi_cluster_moves_inside_then_jumps():
    paths = ["a", "b", "c", "d", "e", "f"]
    clusters = {"a": 1, "b": 1, "c": 2, "d": 3, "e": 3, "f": 3}

    # Move within cluster 3
    next_inside = find_next_in_same_multi_cluster(
        paths,
        direction="down",
        current_index=3,
        cluster_lookup=lambda p: clusters.get(p),
        skip_deleted=True,
        is_deleted=lambda _p: False,
    )
    assert next_inside == "e"

    # From last element in cluster 3, expect jump target is None (edge inside cluster)
    end_inside = find_next_in_same_multi_cluster(
        paths,
        direction="down",
        current_index=5,
        cluster_lookup=lambda p: clusters.get(p),
        skip_deleted=True,
        is_deleted=lambda _p: False,
    )
    assert end_inside is None


def test_find_next_multi_cluster_head_none_when_no_multis():
    paths = ["solo1", "solo2", "solo3"]
    clusters = {p: idx for idx, p in enumerate(paths)}

    target = find_next_multi_image_cluster_head(
        paths,
        direction="down",
        current_index=0,
        cluster_lookup=lambda p: clusters.get(p),
        skip_deleted=True,
        is_deleted=lambda _p: False,
    )

    assert target is None
