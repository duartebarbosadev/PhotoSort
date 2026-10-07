use photosort::core::subjects::{
    Artifact, Geometry, Pair, Strictness, candidates, clusters, verify_geometry, verify_pair,
};
use serde::Deserialize;
use std::{cell::Cell, collections::BTreeMap};

#[derive(Deserialize)]
struct PairCase {
    name: String,
    first: Artifact,
    second: Artifact,
    strictness: Strictness,
    geometry: Geometry,
    time_a: Option<i64>,
    time_b: Option<i64>,
    expected: Pair,
    needs_geometry: bool,
}
#[derive(Deserialize)]
struct CandidateCase {
    name: String,
    artifacts: Vec<Artifact>,
    times: Vec<Option<i64>>,
    expected: Vec<(usize, usize)>,
}
#[derive(Deserialize)]
struct Edge {
    a: usize,
    b: usize,
    pair: Pair,
}
#[derive(Deserialize)]
struct ClusterCase {
    count: usize,
    pairs: Vec<Edge>,
    expected: Vec<usize>,
}
#[derive(Deserialize)]
struct GeometryCase {
    name: String,
    first: Artifact,
    second: Artifact,
    expected: Geometry,
    strong: bool,
}
#[derive(Deserialize)]
struct DenseCase {
    name: String,
    tokens: Vec<f32>,
    width: usize,
    expected: Artifact,
}
#[derive(Deserialize)]
struct Reference {
    dense: Vec<DenseCase>,
    pairs: Vec<PairCase>,
    candidates: Vec<CandidateCase>,
    clusters: Vec<ClusterCase>,
    geometry: Vec<GeometryCase>,
}
fn reference() -> Reference {
    serde_json::from_str(include_str!("subject_reference.json")).unwrap()
}
fn close(a: f64, b: f64, context: &str) {
    assert!((a - b).abs() < 2e-6, "{context}: {a} != {b}");
}
#[test]
fn pair_policy_and_assignment_match_python_reference() {
    let cases = reference().pairs;
    assert_eq!(cases.len(), 624);
    for c in cases {
        let actual = verify_pair(
            &c.first,
            &c.second,
            c.geometry,
            c.time_a,
            c.time_b,
            c.strictness,
        );
        let expected = c.expected;
        assert_eq!(
            (actual.accepted, actual.complete, &actual.reason),
            (expected.accepted, expected.complete, &expected.reason),
            "{} {:?}",
            c.name,
            c.strictness
        );
        close(
            actual.subject_similarity,
            expected.subject_similarity,
            &c.name,
        );
        close(actual.scene_similarity, expected.scene_similarity, &c.name);
        close(actual.confidence, expected.confidence, &c.name);
        assert_eq!(
            actual.face_similarity.is_some(),
            expected.face_similarity.is_some()
        );
        if let Some(a) = actual.face_similarity {
            close(a, expected.face_similarity.unwrap(), &c.name);
        }
        assert_eq!(
            actual.time_delta_seconds, expected.time_delta_seconds,
            "{}",
            c.name
        );
        assert_eq!(
            actual.needs_geometry(c.strictness),
            c.needs_geometry,
            "{}",
            c.name
        );
        // Policy changes consume immutable pair evidence, so cannot decode or infer.
        for policy in [
            Strictness::Broad,
            Strictness::Standard,
            Strictness::Conservative,
        ] {
            let reclassified = actual.with_strictness(policy);
            let fresh = verify_pair(&c.first, &c.second, c.geometry, c.time_a, c.time_b, policy);
            assert_eq!(reclassified.accepted, fresh.accepted);
            assert_eq!(reclassified.reason, fresh.reason);
            assert_eq!(reclassified.confidence, actual.confidence);
        }
    }
}
#[test]
fn semantic_and_temporal_candidates_match_python_reference() {
    let cases = reference().candidates;
    assert_eq!(cases.len(), 27);
    for c in cases {
        assert_eq!(
            candidates(&c.artifacts, &c.times, || false).unwrap(),
            c.expected,
            "{}",
            c.name
        );
    }
}
#[test]
fn complete_link_matches_python_without_bridging_missing_or_rejected_pairs() {
    let cases = reference().clusters;
    assert_eq!(cases.len(), 48);
    for c in cases {
        let pairs = c.pairs.into_iter().map(|e| ((e.a, e.b), e.pair)).collect();
        assert_eq!(clusters(c.count, &pairs, || false).unwrap(), c.expected);
    }
    let accepted = reference()
        .pairs
        .into_iter()
        .find(|c| c.expected.accepted)
        .unwrap()
        .expected;
    let pairs = BTreeMap::from([((0, 1), accepted.clone()), ((1, 2), accepted)]);
    assert_eq!(clusters(3, &pairs, || false), Some(vec![1, 1, 2]));
}
#[test]
fn geometric_verification_matches_reference_on_transforms_and_rejects_degeneracy() {
    let cases = reference().geometry;
    assert_eq!(cases.len(), 27);
    for c in cases {
        let actual = verify_geometry(&c.first, &c.second, || false).unwrap();
        assert_eq!(actual.evaluated, c.expected.evaluated);
        assert_eq!(
            actual.strong(),
            c.strong,
            "{}: {:?} vs {:?}",
            c.name,
            actual,
            c.expected
        );
        // Different deterministic RANSAC sampling/refinement can select a different
        // mask on noisy/outlier cases. Exact masks are required on clean transforms.
        if !c.name.starts_with("noise") && !c.name.starts_with("outliers") {
            assert_eq!(actual.inlier_count, c.expected.inlier_count, "{}", c.name);
            close(actual.inlier_ratio, c.expected.inlier_ratio, &c.name);
            close(actual.coverage_a, c.expected.coverage_a, &c.name);
            close(actual.coverage_b, c.expected.coverage_b, &c.name);
        }
    }
}
#[test]
fn subject_work_is_cancellable_before_and_during_expensive_loops() {
    let r = reference();
    let a = &r.candidates.last().unwrap().artifacts;
    assert!(candidates(a, &[], || true).is_none());
    let calls = Cell::new(0);
    assert!(
        candidates(a, &[], || {
            calls.set(calls.get() + 1);
            calls.get() > a.len() + 20
        })
        .is_none()
    );
    assert!(calls.get() > a.len() + 20);
    // Check the temporal pass separately, after normalization and semantic search.
    let calls = Cell::new(0);
    assert!(
        candidates(a, &r.candidates.last().unwrap().times, || {
            calls.set(calls.get() + 1);
            calls.get() > a.len() * 3 + 4
        })
        .is_none()
    );
    let c = r.geometry.iter().find(|c| c.name == "outliers-0").unwrap();
    assert!(verify_geometry(&c.first, &c.second, || true).is_none());
    let calls = Cell::new(0);
    assert!(
        verify_geometry(&c.first, &c.second, || {
            calls.set(calls.get() + 1);
            calls.get() > 18
        })
        .is_none()
    );
    let c = r.clusters.into_iter().last().unwrap();
    let pairs = c.pairs.into_iter().map(|e| ((e.a, e.b), e.pair)).collect();
    assert!(clusters(c.count, &pairs, || true).is_none());
    let calls = Cell::new(0);
    assert!(
        clusters(c.count, &pairs, || {
            calls.set(calls.get() + 1);
            calls.get() > 4
        })
        .is_none()
    );
}

#[test]
fn dense_pooling_matches_python_on_square_irregular_and_zero_patch_grids() {
    let cases = reference().dense;
    assert_eq!(cases.len(), 16);
    for c in cases {
        let dense =
            photosort::core::subjects::DenseEmbedding::from_tokens(&c.tokens, c.width).unwrap();
        let artifact = dense.into_artifact(Vec::new());
        assert_eq!(artifact.global, c.expected.global, "{} CLS", c.name);
        assert_eq!(artifact.subjects.len(), c.expected.subjects.len());
        assert!(artifact.subjects.len() <= 16);
        for (a, b) in artifact.subjects.iter().zip(&c.expected.subjects) {
            assert_eq!(a.descriptor, b.descriptor, "{} descriptor", c.name);
            assert_eq!(a.bbox, b.bbox, "{} bbox", c.name);
            assert_eq!(a.area, b.area, "{} area", c.name);
            assert_eq!(a.kind, b.kind);
        }
    }
    for (tokens, width) in [
        (vec![], 8),
        (vec![0.0; 8], 8),
        (vec![0.0; 17], 8),
        (vec![0.0; 16], 0),
        (vec![f32::NAN; 16], 8),
    ] {
        assert!(photosort::core::subjects::DenseEmbedding::from_tokens(&tokens, width).is_err());
    }
}
