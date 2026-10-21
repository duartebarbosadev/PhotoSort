use photosort::core::{
    faces::Face,
    near_duplicates::{Comparator, Decision, Gray, Reason, Side},
};
use serde::Deserialize;
use std::{cell::Cell, path::PathBuf};
#[derive(Deserialize)]
struct Case {
    name: String,
    first: String,
    second: String,
    suite: String,
    decision: Decision,
    reason: Reason,
    structural: Option<f64>,
    correlation: Option<f64>,
    overlap: Option<f64>,
    alignment: Option<[[f64; 3]; 2]>,
    faces_a: Option<Vec<Face>>,
    faces_b: Option<Vec<Face>>,
}
#[derive(Deserialize)]
struct Reference {
    cases: Vec<Case>,
}
fn cases() -> Vec<Case> {
    serde_json::from_str::<Reference>(include_str!("fixtures/near_duplicate/reference.json"))
        .unwrap()
        .cases
}
fn image(name: &str) -> (Vec<f32>, usize, usize) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/near_duplicate")
        .join(name);
    let pixels = image::open(path).unwrap().to_luma8();
    let (w, h) = pixels.dimensions();
    (
        pixels
            .into_raw()
            .into_iter()
            .map(|v| f32::from(v) / 255.0)
            .collect(),
        w as usize,
        h as usize,
    )
}
fn check(suite: &str) {
    let mut comparator = Comparator::default();
    for c in cases().into_iter().filter(|c| c.suite == suite) {
        let (a, w, h) = image(&c.first);
        let (b, _, _) = image(&c.second);
        let calls = Cell::new(0);
        let actual = comparator.assess(
            Some(Gray::new(&a, w, h).unwrap()),
            Some(Gray::new(&b, w, h).unwrap()),
            false,
            |side| {
                calls.set(calls.get() + 1);
                match side {
                    Side::First => c.faces_a.clone(),
                    Side::Second => c.faces_b.clone(),
                }
            },
            || false,
        );
        println!(
            "{}: {:?} / {:?}; ssim {:?}; correlation {:?}",
            c.name,
            actual.decision,
            actual.reason,
            actual.structural_similarity,
            actual.alignment.map(|a| a.correlation)
        );
        assert_eq!(
            (actual.decision, actual.reason),
            (c.decision, c.reason),
            "{}: {actual:?}",
            c.name
        );
        assert_eq!(
            actual.structural_similarity.is_some(),
            c.structural.is_some(),
            "{}",
            c.name
        );
        if let (Some(a), Some(b)) = (actual.structural_similarity, c.structural) {
            assert!((a - b).abs() < 2e-5, "{} SSIM {a} / {b}", c.name);
        }
        assert_eq!(
            actual.alignment.is_some(),
            c.alignment.is_some(),
            "{}",
            c.name
        );
        if let Some(a) = actual.alignment {
            assert!(
                (a.correlation - c.correlation.unwrap()).abs() < 2e-5,
                "{} correlation {} / {:?}",
                c.name,
                a.correlation,
                c.correlation
            );
            assert!(
                (actual.overlap.unwrap() - c.overlap.unwrap()).abs() < 1e-5,
                "{} overlap",
                c.name
            );
            for (row, expected) in a.transform.iter().zip(c.alignment.unwrap()) {
                for (i, (a, b)) in row.iter().zip(expected).enumerate() {
                    assert!(
                        (a - b).abs() < if i == 2 { 0.002 } else { 2e-5 },
                        "{} transform {a}/{b}",
                        c.name
                    );
                }
            }
        }
        if actual.structural_similarity.is_none_or(|v| v < 0.98) {
            assert_eq!(calls.get(), 0, "{} prematurely loaded faces", c.name);
        }
    }
}
#[test]
fn complete_small_pair_decisions_match_python() {
    check("small");
}
#[test]
#[ignore = "full-size reference frames; run optimized for registration checks"]
fn complete_normal_view_decisions_match_python() {
    check("large");
}
#[test]
fn exact_equality_and_cancellation_do_not_load_pixels_or_faces() {
    let mut comparator = Comparator::default();
    let a = comparator.assess(
        None,
        None,
        true,
        |_| panic!("exact copied photo requested a model"),
        || true,
    );
    assert_eq!(a.decision, Decision::ExactDuplicate);
    assert!(a.accepted());
    let a = comparator.assess(
        None,
        None,
        false,
        |_| panic!("cancelled photo requested a model"),
        || true,
    );
    assert_eq!(a.reason, Reason::Cancelled);
    assert!(!a.accepted());
    let (a, w, h) = image(&cases()[0].first);
    let a = Gray::new(&a, w, h).unwrap();
    for limit in [2, 150, 600, 2000] {
        let checks = Cell::new(0);
        let result = comparator.assess(
            Some(a),
            Some(a),
            false,
            |_| panic!("cancelled registration requested faces"),
            || {
                checks.set(checks.get() + 1);
                checks.get() >= limit
            },
        );
        assert_eq!(result.reason, Reason::Cancelled);
        assert!(!result.accepted());
    }
    let stopped = Cell::new(false);
    let loads = Cell::new(0);
    let result = comparator.assess(
        Some(a),
        Some(a),
        false,
        |side| {
            assert!(matches!(side, Side::First));
            loads.set(loads.get() + 1);
            stopped.set(true);
            Some(Vec::new())
        },
        || stopped.get(),
    );
    assert_eq!(loads.get(), 1);
    assert_eq!(result.reason, Reason::Cancelled);
}
