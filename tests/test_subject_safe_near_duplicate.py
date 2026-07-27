from unittest.mock import Mock

import cv2
import numpy as np

import core.image_features.near_duplicate as near_duplicate
from core.caching.analysis_cache import AnalysisCache
from core.image_features.face_analysis import (
    FACE_DESCRIPTOR_VERSION,
    FaceDescriptor,
    SubjectDescriptor,
    face_descriptor_signature,
)
from core.image_features.near_duplicate import (
    CHANGE_COMPONENT_MIN_FRACTION,
    MAX_ALIGNMENT_ROTATION_DEGREES,
    MAX_ALIGNMENT_SCALE_CHANGE,
    MAX_ALIGNMENT_SHEAR,
    NEAR_DUPLICATE_ALGORITHM_VERSION,
    NearDuplicateDecision,
    SubjectSafeNearDuplicateComparator,
    _prepare_aligned_pair,
    _validate_alignment,
    coherent_change_metrics,
)
from workers.easy_delete_worker import EasyDeleteWorker


def _textured_rgb(height: int = 768, width: int = 1024) -> np.ndarray:
    rng = np.random.default_rng(42)
    image = rng.normal(110, 24, (height, width, 3))
    return np.clip(image, 0, 255).astype(np.uint8)


def _no_faces() -> SubjectDescriptor:
    return SubjectDescriptor(())


def _face(offset: float = 0.0) -> FaceDescriptor:
    points = tuple(
        (0.40 + (index % 10) * 0.015, 0.38 + (index // 10) * 0.015 + offset)
        for index in range(100)
    )
    return FaceDescriptor((0.40, 0.38 + offset, 0.535, 0.515 + offset), points)


def _high_contrast_scene(pose: int = 0) -> np.ndarray:
    height, width = 768, 1024
    rng = np.random.default_rng(91)
    image = np.clip(rng.normal(105, 5, (height, width, 3)), 0, 255).astype(
        np.uint8
    )
    for x in range(40, width, 70):
        cv2.line(image, (x, 0), (x + 180, height), (35, 45, 55), 3)
    for y in range(60, height, 85):
        cv2.line(image, (0, y), (width, y), (180, 165, 145), 4)
    cv2.rectangle(image, (420, 490), (604, 650), (60, 95, 55), -1)
    cv2.circle(image, (500, 470), 18, (180, 130, 95), -1)
    cv2.line(image, (500, 488), (500, 570), (30, 50, 100), 12)
    arms = (
        (((500, 515), (460, 550)), ((500, 515), (545, 535)))
        if pose == 0
        else (((500, 515), (455, 500)), ((500, 515), (540, 485)))
    )
    for start, end in arms:
        cv2.line(image, start, end, (30, 50, 100), 10)
    return image


def _handheld_affine(image: np.ndarray) -> np.ndarray:
    height, width = image.shape[:2]
    transform = cv2.getRotationMatrix2D((width / 2, height / 2), 0.15, 1.0)
    transform[:, 2] += (1.8, -2.4)
    return cv2.warpAffine(
        image,
        transform,
        (width, height),
        borderMode=cv2.BORDER_REFLECT,
    )


def test_exact_duplicate_short_circuits_images_and_subject_analysis():
    assessment = SubjectSafeNearDuplicateComparator().assess(
        "first.raw",
        "second.raw",
        (10, 20),
        (10, 20),
        None,
        None,
        identical=True,
    )

    assert assessment.decision is NearDuplicateDecision.EXACT_DUPLICATE
    assert assessment.accepted
    assert assessment.algorithm_version == NEAR_DUPLICATE_ALGORITHM_VERSION


def test_static_high_contrast_handheld_scene_is_safe_at_normal_view():
    first = _high_contrast_scene()
    second = _handheld_affine(first)

    assessment = SubjectSafeNearDuplicateComparator().assess(
        "first.raw",
        "second.raw",
        None,
        None,
        first,
        second,
        descriptor_a=_no_faces(),
        descriptor_b=_no_faces(),
    )

    assert assessment.decision is NearDuplicateDecision.SAFE_NEAR_DUPLICATE
    assert assessment.reason_code == "normal_view_equivalent"
    assert assessment.structural_similarity is not None
    assert assessment.structural_similarity >= 0.995
    assert assessment.metrics["alignment_mode"] == "pyramidal_affine_ecc"
    assert assessment.metrics["alignment_correlation"] >= 0.995
    assert assessment.metrics["alignment_overlap_fraction"] >= 0.85
    assert not assessment.change.meaningful_change


def test_changed_small_body_pose_survives_normal_view_reduction():
    first = _high_contrast_scene(pose=0)
    second = _handheld_affine(_high_contrast_scene(pose=1))

    assessment = SubjectSafeNearDuplicateComparator().assess(
        "first.raw",
        "second.raw",
        None,
        None,
        first,
        second,
        descriptor_a=_no_faces(),
        descriptor_b=_no_faces(),
    )

    assert assessment.decision is NearDuplicateDecision.SUBJECT_CHANGED
    assert assessment.reason_code == "visible_subject_change"
    assert assessment.change is not None and assessment.change.meaningful_change


def test_alignment_safety_boundaries_are_strict():
    identity = np.float32([[1, 0, 0], [0, 1, 0]])
    assert _validate_alignment(identity, correlation=1.0, width=1000, height=800)

    rotation = np.deg2rad(MAX_ALIGNMENT_ROTATION_DEGREES + 0.01)
    too_rotated = np.float32(
        [[np.cos(rotation), -np.sin(rotation), 0], [np.sin(rotation), np.cos(rotation), 0]]
    )
    too_scaled = np.float32(
        [[1 + MAX_ALIGNMENT_SCALE_CHANGE + 0.001, 0, 0], [0, 1, 0]]
    )
    too_sheared = np.float32(
        [[1, MAX_ALIGNMENT_SHEAR + 0.001, 0], [0, 1, 0]]
    )

    assert _validate_alignment(
        too_rotated, correlation=1.0, width=1000, height=800
    ) is None
    assert _validate_alignment(
        too_scaled, correlation=1.0, width=1000, height=800
    ) is None
    assert _validate_alignment(
        too_sheared, correlation=1.0, width=1000, height=800
    ) is None


def test_small_changed_subject_is_not_diluted_by_large_static_background():
    first = _textured_rgb()
    second = first.copy()
    second[550:590, 450:470] = (220, 30, 30)

    assessment = SubjectSafeNearDuplicateComparator().assess(
        "first.raw",
        "second.raw",
        None,
        None,
        first,
        second,
        descriptor_a=_no_faces(),
        descriptor_b=_no_faces(),
    )

    assert assessment.structural_similarity is not None
    assert assessment.structural_similarity >= 0.98
    assert assessment.decision is NearDuplicateDecision.SUBJECT_CHANGED
    assert assessment.change is not None
    assert assessment.change.meaningful_change


def test_exposure_and_sensor_noise_without_coherent_change_remain_safe():
    first = _textured_rgb()
    rng = np.random.default_rng(7)
    second = np.clip(
        first.astype(np.float32) * 1.03 + 3.0 + rng.normal(0, 0.8, first.shape),
        0,
        255,
    ).astype(np.uint8)

    assessment = SubjectSafeNearDuplicateComparator().assess(
        "first.raw",
        "second.raw",
        None,
        None,
        first,
        second,
        descriptor_a=_no_faces(),
        descriptor_b=_no_faces(),
    )

    assert assessment.decision is NearDuplicateDecision.SAFE_NEAR_DUPLICATE
    assert assessment.change is not None
    assert not assessment.change.meaningful_change


def test_face_count_or_landmark_change_vetoes_an_otherwise_unchanged_frame():
    image = _textured_rgb(512, 512)
    comparator = SubjectSafeNearDuplicateComparator()

    count_change = comparator.assess(
        "first.raw",
        "second.raw",
        None,
        None,
        image,
        image.copy(),
        descriptor_a=SubjectDescriptor((_face(),)),
        descriptor_b=SubjectDescriptor(()),
    )
    changed_points = list(_face().landmarks)
    changed_points[-20:] = [(x, y + 0.04) for x, y in changed_points[-20:]]
    changed_face = FaceDescriptor(_face().bbox, tuple(changed_points))
    landmark_change = comparator.assess(
        "first.raw",
        "second.raw",
        None,
        None,
        image,
        image.copy(),
        descriptor_a=SubjectDescriptor((_face(),)),
        descriptor_b=SubjectDescriptor((changed_face,)),
    )

    assert count_change.decision is NearDuplicateDecision.SUBJECT_CHANGED
    assert landmark_change.decision is NearDuplicateDecision.SUBJECT_CHANGED


def test_face_overlap_boundary_is_enforced():
    image = _textured_rgb(512, 512)
    first = _face()
    width = first.bbox[2] - first.bbox[0]

    def shifted(fraction):
        dx = width * fraction
        return FaceDescriptor(
            tuple(
                value + (dx if index % 2 == 0 else 0.0)
                for index, value in enumerate(first.bbox)
            ),
            tuple((x + dx, y) for x, y in first.landmarks),
        )

    comparator = SubjectSafeNearDuplicateComparator()
    above = comparator.assess(
        "a",
        "b",
        None,
        None,
        image,
        image.copy(),
        descriptor_a=SubjectDescriptor((first,)),
        descriptor_b=SubjectDescriptor((shifted(0.24),)),
    )
    below = comparator.assess(
        "a",
        "b",
        None,
        None,
        image,
        image.copy(),
        descriptor_a=SubjectDescriptor((first,)),
        descriptor_b=SubjectDescriptor((shifted(0.26),)),
    )

    assert above.face_min_iou is not None and above.face_min_iou > 0.60
    assert above.decision is NearDuplicateDecision.SAFE_NEAR_DUPLICATE
    assert below.face_min_iou is not None and below.face_min_iou < 0.60
    assert below.decision is NearDuplicateDecision.SUBJECT_CHANGED


def test_landmark_and_face_crop_boundaries_are_strict(monkeypatch):
    image = _textured_rgb(512, 512)
    descriptors = SubjectDescriptor((_face(),))
    comparator = SubjectSafeNearDuplicateComparator()
    monkeypatch.setattr(
        near_duplicate,
        "_landmark_displacement",
        lambda *_args: (
            near_duplicate.FACE_LANDMARK_MEDIAN_LIMIT,
            near_duplicate.FACE_LANDMARK_P90_LIMIT,
        ),
    )
    monkeypatch.setattr(
        near_duplicate,
        "_face_crop_similarity",
        lambda *_args: near_duplicate.FACE_CROP_MIN_SSIM,
    )

    boundary = comparator.assess(
        "a",
        "b",
        None,
        None,
        image,
        image.copy(),
        descriptor_a=descriptors,
        descriptor_b=descriptors,
    )
    monkeypatch.setattr(
        near_duplicate,
        "_landmark_displacement",
        lambda *_args: (
            near_duplicate.FACE_LANDMARK_MEDIAN_LIMIT + 1e-6,
            near_duplicate.FACE_LANDMARK_P90_LIMIT,
        ),
    )
    changed_landmark = comparator.assess(
        "a",
        "b",
        None,
        None,
        image,
        image.copy(),
        descriptor_a=descriptors,
        descriptor_b=descriptors,
    )
    monkeypatch.setattr(
        near_duplicate,
        "_landmark_displacement",
        lambda *_args: (0.0, 0.0),
    )
    monkeypatch.setattr(
        near_duplicate,
        "_face_crop_similarity",
        lambda *_args: near_duplicate.FACE_CROP_MIN_SSIM - 1e-6,
    )
    changed_crop = comparator.assess(
        "a",
        "b",
        None,
        None,
        image,
        image.copy(),
        descriptor_a=descriptors,
        descriptor_b=descriptors,
    )

    assert boundary.decision is NearDuplicateDecision.SAFE_NEAR_DUPLICATE
    assert changed_landmark.decision is NearDuplicateDecision.SUBJECT_CHANGED
    assert changed_crop.decision is NearDuplicateDecision.SUBJECT_CHANGED


def test_missing_face_assessment_fails_closed():
    image = _textured_rgb(512, 512)

    assessment = SubjectSafeNearDuplicateComparator().assess(
        "first.raw",
        "second.raw",
        None,
        None,
        image,
        image.copy(),
    )

    assert assessment.decision is NearDuplicateDecision.UNCERTAIN
    assert not assessment.accepted


def test_face_descriptors_are_loaded_only_after_cheap_comparison_passes():
    first = _high_contrast_scene()
    changed = first.copy()
    changed[100:500, 200:800] = 255 - changed[100:500, 200:800]
    rejected_loader_a = Mock(return_value=_no_faces())
    rejected_loader_b = Mock(return_value=_no_faces())

    rejected = SubjectSafeNearDuplicateComparator().assess(
        "first.raw",
        "changed.raw",
        None,
        None,
        first,
        changed,
        descriptor_loader_a=rejected_loader_a,
        descriptor_loader_b=rejected_loader_b,
    )

    assert rejected.decision in {
        NearDuplicateDecision.SUBJECT_CHANGED,
        NearDuplicateDecision.UNCERTAIN,
    }
    rejected_loader_a.assert_not_called()
    rejected_loader_b.assert_not_called()

    accepted_loader_a = Mock(return_value=_no_faces())
    accepted_loader_b = Mock(return_value=_no_faces())
    accepted = SubjectSafeNearDuplicateComparator().assess(
        "first.raw",
        "second.raw",
        None,
        None,
        first,
        _handheld_affine(first),
        descriptor_loader_a=accepted_loader_a,
        descriptor_loader_b=accepted_loader_b,
    )

    assert accepted.decision is NearDuplicateDecision.SAFE_NEAR_DUPLICATE
    accepted_loader_a.assert_called_once_with()
    accepted_loader_b.assert_called_once_with()


def test_cancellation_is_honored_before_expensive_comparison():
    assessment = SubjectSafeNearDuplicateComparator(lambda: True).assess(
        "first.raw",
        "second.raw",
        None,
        None,
        _textured_rgb(128, 128),
        _textured_rgb(128, 128),
        descriptor_a=_no_faces(),
        descriptor_b=_no_faces(),
    )

    assert assessment.decision is NearDuplicateDecision.UNCERTAIN
    assert assessment.detail == "cancelled"


def test_cancellation_is_honored_between_alignment_levels_and_face_loaders():
    image = _high_contrast_scene()
    alignment_checks = 0

    def stop_during_alignment():
        nonlocal alignment_checks
        alignment_checks += 1
        return alignment_checks >= 2

    assert _prepare_aligned_pair(image, image.copy(), stop_during_alignment) is None

    cancelled = False
    second_loader = Mock(return_value=_no_faces())

    def first_loader():
        nonlocal cancelled
        cancelled = True
        return _no_faces()

    assessment = SubjectSafeNearDuplicateComparator(lambda: cancelled).assess(
        "first.raw",
        "second.raw",
        None,
        None,
        image,
        _handheld_affine(image),
        descriptor_loader_a=first_loader,
        descriptor_loader_b=second_loader,
    )

    assert assessment.decision is NearDuplicateDecision.UNCERTAIN
    assert assessment.reason_code == "cancelled"
    second_loader.assert_not_called()


def test_cancellation_is_honored_during_component_analysis():
    first = np.zeros((256, 256), dtype=np.float32)
    second = first.copy()
    for y in range(10, 240, 20):
        for x in range(10, 240, 20):
            second[y : y + 8, x : x + 8] = 0.5

    assert (
        coherent_change_metrics(
            first,
            second,
            should_stop=lambda: True,
        )
        is None
    )


def test_component_area_boundary_is_conservative():
    height = width = 512
    rng = np.random.default_rng(3)
    first = np.clip(rng.normal(0.45, 0.08, (height, width)), 0, 1).astype(
        np.float32
    )
    below = first.copy()
    above = first.copy()
    minimum_area = int(np.ceil(first.size * CHANGE_COMPONENT_MIN_FRACTION))
    below_side = max(3, int(np.sqrt(minimum_area)) - 3)
    above_side = int(np.ceil(np.sqrt(minimum_area))) + 8
    below[100 : 100 + below_side, 100 : 100 + below_side] += 0.2
    above[100 : 100 + above_side, 100 : 100 + above_side] += 0.2

    below_metrics = coherent_change_metrics(first, np.clip(below, 0, 1))
    above_metrics = coherent_change_metrics(first, np.clip(above, 0, 1))

    assert below_metrics is not None and not below_metrics.meaningful_change
    assert above_metrics is not None and above_metrics.meaningful_change


def test_subject_descriptor_cache_avoids_second_model_initialization(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis"))
    image = _textured_rgb(128, 128)
    path = str(tmp_path / "photo.jpg")
    (tmp_path / "photo.jpg").write_bytes(b"fingerprinted")
    fingerprint = (len(b"fingerprinted"), 123)

    class Service:
        def __init__(self, descriptor):
            self.descriptor = descriptor
            self.calls = 0

        def describe(self, _image):
            self.calls += 1
            return self.descriptor

        def close(self):
            pass

    first_service = Service(SubjectDescriptor((_face(),)))
    first_worker = EasyDeleteWorker(
        [path],
        analysis_cache=cache,
        folder_path=str(tmp_path),
        fingerprints={path: fingerprint},
        face_analysis_service=first_service,
    )
    first = first_worker._subject_descriptor(path, image)
    first_worker._flush_subject_descriptors()

    second_service = Service(None)
    second_worker = EasyDeleteWorker(
        [path],
        analysis_cache=cache,
        folder_path=str(tmp_path),
        fingerprints={path: fingerprint},
        face_analysis_service=second_service,
    )
    second = second_worker._subject_descriptor(path, image)

    assert first == second
    assert first_service.calls == 1
    assert second_service.calls == 0
    stored = cache.load_subject_descriptor(
        str(tmp_path),
        path,
        fingerprint=fingerprint,
        signature=face_descriptor_signature(),
    )
    assert stored == first.to_dict()
    cache.close()


def test_worker_batches_subject_descriptor_persistence(tmp_path):
    paths = [str(tmp_path / "first.jpg"), str(tmp_path / "second.jpg")]
    analysis_cache = Mock()
    analysis_cache.load_subject_descriptor.return_value = None

    class Service:
        def describe(self, _image):
            return _no_faces()

        def close(self):
            pass

    worker = EasyDeleteWorker(
        paths,
        analysis_cache=analysis_cache,
        folder_path=str(tmp_path),
        fingerprints={path: (index + 1, 123) for index, path in enumerate(paths)},
        face_analysis_service=Service(),
    )
    image = _textured_rgb(128, 128)
    for path in paths:
        worker._subject_descriptor(path, image)

    analysis_cache.save_subject_descriptors_batch.assert_not_called()
    worker._flush_subject_descriptors()

    analysis_cache.save_subject_descriptors_batch.assert_called_once()
    records = analysis_cache.save_subject_descriptors_batch.call_args.args[1]
    assert set(records) == set(paths)


def test_analysis_cache_rejects_changed_subject_fingerprint(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis"))
    folder = str(tmp_path)
    path = str(tmp_path / "photo.jpg")
    descriptor = SubjectDescriptor((_face(),)).to_dict()
    cache.save_subject_descriptor(
        folder,
        path,
        fingerprint=(10, 20),
        signature=FACE_DESCRIPTOR_VERSION,
        descriptor=descriptor,
    )

    assert (
        cache.load_subject_descriptor(
            folder,
            path,
            fingerprint=(10, 21),
            signature=FACE_DESCRIPTOR_VERSION,
        )
        is None
    )
    cache.close()


def test_subject_descriptors_follow_central_mutation_invalidation(tmp_path):
    cache = AnalysisCache(str(tmp_path / "analysis"))
    source = str(tmp_path / "source")
    destination = str(tmp_path / "destination")
    old_path = str(tmp_path / "source" / "photo.jpg")
    new_path = str(tmp_path / "destination" / "photo.jpg")
    descriptor = SubjectDescriptor((_face(),)).to_dict()
    cache.save_subject_descriptor(
        source,
        old_path,
        fingerprint=(10, 20),
        signature="model-v1",
        descriptor=descriptor,
    )

    cache.migrate_folder_paths(source, destination, {old_path: new_path})
    assert (
        cache.load_subject_descriptor(
            destination,
            new_path,
            fingerprint=(10, 20),
            signature="model-v1",
        )
        == descriptor
    )

    cache.invalidate_similarity(destination)
    assert (
        cache.load_subject_descriptor(
            destination,
            new_path,
            fingerprint=(10, 20),
            signature="model-v1",
        )
        is None
    )
    cache.close()
