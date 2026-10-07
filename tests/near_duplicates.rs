use photosort::core::near_duplicates::{
    Alignment, Change, ChangeOptions, Gray, coherent_change, exposure_correct,
    structural_similarity,
};
use serde::Deserialize;
use std::cell::Cell;
#[derive(Deserialize)]
struct Signal {
    name: String,
    width: usize,
    height: usize,
    seed: usize,
    gain: f32,
    offset: f32,
    patches: Vec<(usize, usize, usize, usize, f32)>,
    holes: Vec<(usize, usize, usize, usize)>,
    flat: Option<f32>,
    normal: bool,
    masked: bool,
    change: Option<Change>,
    ssim: Option<f64>,
    exposure: Option<Vec<f32>>,
}
impl Signal {
    fn arrays(&self) -> (Vec<f32>, Vec<f32>, Vec<bool>) {
        let a: Vec<_> = (0..self.width * self.height)
            .map(|i| {
                self.flat.unwrap_or(
                    ((i * 73 + (i / self.width) * 37 + self.seed * 19) % 181 + 32) as f32 / 255.0,
                )
            })
            .collect();
        let mut b: Vec<_> = a
            .iter()
            .map(|v| (*v * self.gain + self.offset).clamp(0.0, 1.0))
            .collect();
        for &(x, y, w, h, d) in &self.patches {
            for yy in y..y + h {
                for xx in x..x + w {
                    let i = yy * self.width + xx;
                    b[i] = (b[i] + d).clamp(0.0, 1.0);
                }
            }
        }
        let mut mask = vec![true; a.len()];
        for &(x, y, w, h) in &self.holes {
            for yy in y..y + h {
                for xx in x..x + w {
                    mask[yy * self.width + xx] = false;
                }
            }
        }
        (a, b, mask)
    }
}
#[derive(Deserialize)]
struct Affine {
    name: String,
    transform: [[f64; 3]; 2],
    correlation: f64,
    width: usize,
    height: usize,
    expected: Option<Alignment>,
}
#[derive(Deserialize)]
struct Reference {
    signals: Vec<Signal>,
    alignment: Vec<Affine>,
}
fn reference() -> Reference {
    serde_json::from_str(include_str!("near_duplicate_reference.json")).unwrap()
}
fn close(a: f64, b: f64, tolerance: f64, context: &str) {
    assert!((a - b).abs() <= tolerance, "{context}: {a} != {b}");
}
#[test]
fn coherent_subject_change_matches_reference_components_and_exposure() {
    let cases = reference().signals;
    assert_eq!(cases.len(), 48);
    for c in cases {
        let (a, b, m) = c.arrays();
        let a = Gray::new(&a, c.width, c.height).unwrap();
        let b = Gray::new(&b, c.width, c.height).unwrap();
        let mask = c.masked.then_some(m.as_slice());
        let options = if c.normal {
            ChangeOptions::normal_view()
        } else {
            ChangeOptions::default()
        };
        let actual = coherent_change(a, b, mask, options, || false);
        assert_eq!(actual.is_some(), c.change.is_some(), "{}", c.name);
        if let (Some(a), Some(b)) = (actual, c.change) {
            assert_eq!(
                a.meaningful_change, b.meaningful_change,
                "{}: {a:?} / {b:?}",
                c.name
            );
            close(a.threshold, b.threshold, 2e-6, &c.name);
            close(
                a.largest_component_fraction,
                b.largest_component_fraction,
                1e-12,
                &c.name,
            );
            close(a.coherent_fraction, b.coherent_fraction, 1e-12, &c.name);
            close(
                a.largest_component_p90,
                b.largest_component_p90,
                2e-6,
                &c.name,
            );
        }
        let actual = exposure_correct(a, b, mask, || false);
        assert_eq!(actual.is_some(), c.exposure.is_some(), "{}", c.name);
        if let (Some(a), Some(b)) = (actual, c.exposure) {
            let mut positions = vec![0, a.len() / 4, a.len() / 2, a.len() - 1];
            positions.sort_unstable();
            positions.dedup();
            for (i, b) in positions.into_iter().zip(b) {
                close(f64::from(a[i]), f64::from(b), 2e-6, &c.name);
            }
        }
    }
}
#[test]
fn structural_similarity_matches_reference_local_tiles_and_mask_support() {
    for c in reference().signals {
        let (a, b, m) = c.arrays();
        let a = Gray::new(&a, c.width, c.height).unwrap();
        let b = Gray::new(&b, c.width, c.height).unwrap();
        let actual = structural_similarity(a, b, c.masked.then_some(m.as_slice()), || false);
        assert_eq!(actual.is_some(), c.ssim.is_some(), "{}", c.name);
        if let (Some(a), Some(b)) = (actual, c.ssim) {
            close(a, b, 3e-6, &c.name);
        }
    }
}
#[test]
fn alignment_bounds_match_reference_including_exact_thresholds() {
    let cases = reference().alignment;
    assert_eq!(cases.len(), 29);
    for c in cases {
        let actual = Alignment::validate(c.transform, c.correlation, c.width, c.height);
        assert_eq!(actual.is_some(), c.expected.is_some(), "{}", c.name);
        if let (Some(a), Some(b)) = (actual, c.expected) {
            close(a.rotation_degrees, b.rotation_degrees, 1e-12, &c.name);
            close(a.scale_x, b.scale_x, 1e-12, &c.name);
            close(a.scale_y, b.scale_y, 1e-12, &c.name);
            close(a.shear, b.shear, 1e-12, &c.name);
        }
    }
    assert!(Alignment::validate([[1., 0., 0.], [0., 1., 0.]], f64::NAN, 100, 100).is_none());
    assert!(Alignment::validate([[1., f64::INFINITY, 0.], [0., 1., 0.]], 1., 100, 100).is_none());
}
#[test]
fn malformed_or_cancelled_evidence_never_returns_a_measurement() {
    for (pixels, w, h) in [
        (&[][..], 0, 0),
        (&[0.5][..], 2, 1),
        (&[f32::NAN][..], 1, 1),
        (&[-0.1][..], 1, 1),
        (&[1.01][..], 1, 1),
    ] {
        assert!(Gray::new(pixels, w, h).is_none());
    }
    assert!(Gray::new(&[], usize::MAX, usize::MAX).is_none());
    let c = reference()
        .signals
        .into_iter()
        .find(|c| c.name == "many-components")
        .unwrap();
    let (a, b, mask) = c.arrays();
    let a = Gray::new(&a, c.width, c.height).unwrap();
    let b = Gray::new(&b, c.width, c.height).unwrap();
    for method in 0..3 {
        let run = |stop: &dyn Fn() -> bool| match method {
            0 => coherent_change(a, b, Some(&mask), ChangeOptions::default(), stop).is_some(),
            1 => structural_similarity(a, b, Some(&mask), stop).is_some(),
            _ => exposure_correct(a, b, Some(&mask), stop).is_some(),
        };
        let checks = Cell::new(0);
        assert!(run(&|| {
            checks.set(checks.get() + 1);
            false
        }));
        let total = checks.get();
        for boundary in [1, total / 4, total / 2, total * 3 / 4, total] {
            checks.set(0);
            assert!(
                !run(&|| {
                    checks.set(checks.get() + 1);
                    checks.get() >= boundary
                }),
                "method {method}, boundary {boundary}/{total}"
            );
        }
    }
    assert!(coherent_change(a, b, Some(&[true]), ChangeOptions::default(), || false).is_none());
    assert!(structural_similarity(a, b, Some(&[true]), || false).is_none());
    assert!(
        coherent_change(
            a,
            b,
            None,
            ChangeOptions {
                component_min_fraction: f64::NAN,
                ..Default::default()
            },
            || false
        )
        .is_none()
    );
}

#[derive(Deserialize)]
struct FaceCase {
    name: String,
    first: Vec<photosort::core::faces::Face>,
    second: Vec<photosort::core::faces::Face>,
    transform: [[f64; 3]; 2],
    decision: String,
    min_iou: Option<f64>,
    max_median: Option<f64>,
    max_p90: Option<f64>,
}
#[test]
fn face_motion_matches_reference_assignment_and_expression_changes() {
    use photosort::core::near_duplicates::face_motion;
    #[derive(Deserialize)]
    struct Faces {
        faces: Vec<FaceCase>,
    }
    let cases: Faces = serde_json::from_str(include_str!("near_duplicate_reference.json")).unwrap();
    assert_eq!(cases.faces.len(), 21);
    for c in cases.faces {
        let alignment = Alignment::validate(c.transform, 0.995, 1000, 800).unwrap();
        let actual = face_motion(&c.first, &c.second, &alignment, 1000, 800, || false);
        assert_eq!(actual.is_none(), c.decision == "uncertain", "{}", c.name);
        if let Some(a) = actual {
            assert_eq!(
                a.changed,
                c.decision == "subject_changed",
                "{} {a:?}",
                c.name
            );
            for (a, b) in [
                (a.min_iou, c.min_iou),
                (a.max_median, c.max_median),
                (a.max_p90, c.max_p90),
            ] {
                assert_eq!(a.is_some(), b.is_some(), "{}", c.name);
                if let (Some(a), Some(b)) = (a, b) {
                    close(a, b, 2e-7, &c.name);
                }
            }
        }
        assert!(face_motion(&c.first, &c.second, &alignment, 1000, 800, || true).is_none());
    }
    let old = photosort::core::faces::Face {
        bbox: [0.2, 0.2, 0.5, 0.5],
        eyes: [[0.3, 0.3], [0.4, 0.3]],
        landmarks: Vec::new(),
    };
    let identity = Alignment::validate([[1., 0., 0.], [0., 1., 0.]], 1., 100, 100).unwrap();
    assert!(
        face_motion(
            std::slice::from_ref(&old),
            std::slice::from_ref(&old),
            &identity,
            100,
            100,
            || false
        )
        .is_none()
    );
    let value = serde_json::json!({"bbox":[0.2,0.2,0.5,0.5],"eyes":[[0.3,0.3],[0.4,0.3]]});
    let legacy: photosort::core::faces::Face = serde_json::from_value(value).unwrap();
    assert!(legacy.landmarks.is_empty());
}

#[test]
fn review_rows_retain_summary_only_and_face_evidence_roundtrips_without_inference() {
    use photosort::core::faces::{Face, Faces};
    let faces = Faces {
        count: 1,
        closed_eyes: 1,
        primary: Some([0.2, 0.2, 0.6, 0.6]),
        geometry: vec![Face {
            bbox: [0.2, 0.2, 0.6, 0.6],
            eyes: [[0.3, 0.3], [0.5, 0.3]],
            landmarks: vec![[0.4, 0.4]; 468],
        }],
    };
    let persisted = serde_json::to_vec(&faces).unwrap();
    let restored: Faces = serde_json::from_slice(&persisted).unwrap();
    assert_eq!(restored.geometry[0].landmarks, faces.geometry[0].landmarks);
    let summary = restored.summary();
    assert_eq!(summary.count, 1);
    assert_eq!(summary.closed_eyes, 1);
    assert_eq!(summary.primary, faces.primary);
    let review = photosort::core::analysis::Analysis {
        faces: Some(summary),
        ..Default::default()
    };
    let bytes = serde_json::to_vec(&review).unwrap();
    assert!(
        !String::from_utf8(bytes.clone())
            .unwrap()
            .contains("landmarks")
    );
    assert!(bytes.len() < 512);
    let legacy = serde_json::json!({"faces":faces,"sharpness":0.0,"aesthetic":null,"brightness":0.0,"score":0.0,"hash":0,"color":[0,0,0],"issues":[]});
    let old: photosort::core::analysis::Analysis = serde_json::from_value(legacy).unwrap();
    assert_eq!(old.faces.unwrap().count, 1);
}
