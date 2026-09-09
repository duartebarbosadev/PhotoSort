import pyexiv2  # noqa: F401  # Must be first to avoid Windows crash

from unittest.mock import Mock

from workers.easy_delete_worker import EasyDeleteWorker


def _worker(paths, tmp_path):
    return EasyDeleteWorker(
        paths,
        image_pipeline=Mock(),
        analysis_cache=Mock(),
        folder_path=str(tmp_path),
        fingerprints={},
    )


def test_an_unreadable_photo_is_not_ranked_as_the_blurriest(tmp_path):
    """A decode failure must not masquerade as a sharpness measurement.

    ``_get_sharpness`` reports 0.0 both for a genuinely flat image and for a file
    it could not decode. Because one sharpness point dominates every other
    tie-breaker in ``_keep_score``, treating the failure as a real score would
    make an unreadable photo lose every duplicate comparison and be suggested for
    deletion purely because it could not be read.
    """

    readable = str(tmp_path / "readable.jpg")
    unreadable = str(tmp_path / "unreadable.jpg")

    worker = _worker([readable, unreadable], tmp_path)
    worker._sharpness_cache[readable] = 120.0
    worker._mark_sharpness_unmeasurable(unreadable)

    assert worker._sharpness_is_known(readable) is True
    assert worker._sharpness_is_known(unreadable) is False

    # With sharpness excluded, the unreadable file is no longer forced to the
    # bottom of the ranking by a score it never earned.
    assert worker._keep_score(unreadable, use_sharpness=False) == worker._keep_score(
        readable, use_sharpness=False
    )
    assert worker._keep_score(unreadable, use_sharpness=True) < worker._keep_score(
        readable, use_sharpness=True
    )


def test_the_suggestion_never_cites_a_sharpness_that_was_not_measured(tmp_path):
    """The explanation must not quote 0.0 as if it were a real measurement."""

    keep = str(tmp_path / "keep.jpg")
    delete = str(tmp_path / "delete.jpg")

    worker = _worker([keep, delete], tmp_path)
    worker._sharpness_cache[keep] = 90.0
    worker._mark_sharpness_unmeasurable(delete)
    worker._files_are_identical = lambda *_args, **_kwargs: False
    worker._exif_field_count = lambda _path: 0
    worker._file_size = lambda _path: 0

    reason = worker._duplicate_reason(delete, keep, identical=False)
    delete_reason, _keep_reason = worker._duplicate_suggestion_reasons(
        delete, keep, identical=False
    )

    assert "sharpness" not in reason
    assert "sharpness" not in delete_reason
