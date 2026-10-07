use photosort::core::regional::{self, Features, Input};
use serde::Deserialize;
#[derive(Deserialize)]
struct Reference {
    boxes: Vec<BoxCase>,
    cases: Vec<Case>,
}
#[derive(Deserialize)]
struct BoxCase {
    width: u32,
    height: u32,
    boxes: Vec<[u32; 4]>,
}
#[derive(Deserialize)]
struct Case {
    name: String,
    rows: Vec<Vec<Vec<f32>>>,
    portrait: Vec<bool>,
    eps: f32,
    distances: Vec<Vec<f32>>,
    labels: Vec<usize>,
}
fn reference() -> Reference {
    serde_json::from_str(include_str!("regional_reference.json")).unwrap()
}
#[test]
fn region_boxes_match_python_rounding_and_clamping() {
    for case in reference().boxes {
        assert_eq!(
            regional::crop_boxes(case.width, case.height),
            case.boxes,
            "{}x{}",
            case.width,
            case.height
        );
    }
    for (w, h, portrait) in [
        (90, 100, false),
        (89, 100, true),
        (100, 100, false),
        (110, 100, false),
        (0, 100, false),
        (100, 0, false),
    ] {
        assert_eq!(regional::portrait(w, h), portrait);
    }
}
#[test]
fn streaming_groups_and_distances_match_python_dbscan() {
    for case in reference().cases {
        let features: Vec<_> = case
            .rows
            .into_iter()
            .map(|rows| Features::new(rows).unwrap())
            .collect();
        for (i, a) in features.iter().enumerate() {
            for (j, b) in features.iter().enumerate() {
                if i != j {
                    assert!(
                        (a.distance(b).unwrap() - case.distances[i][j]).abs() < 1e-6,
                        "{} {i},{j}: {} != {}",
                        case.name,
                        a.distance(b).unwrap(),
                        case.distances[i][j]
                    );
                }
            }
        }
        let inputs: Vec<_> = case
            .portrait
            .into_iter()
            .enumerate()
            .map(|(i, portrait)| Input {
                key: i.to_string(),
                portrait,
            })
            .collect();
        let groups = regional::cluster(
            &inputs,
            case.eps,
            |key| Ok(features[key.parse::<usize>().unwrap()].clone()),
            || false,
        )
        .unwrap()
        .unwrap();
        // Membership is the functional contract. Rust deliberately assigns
        // content-stable numbers instead of Python's scan-order numbers.
        for i in 0..inputs.len() {
            for j in 0..inputs.len() {
                assert_eq!(
                    groups.labels[i] == groups.labels[j],
                    case.labels[i] == case.labels[j],
                    "{}: {i},{j}",
                    case.name
                );
                if inputs[i].portrait && !inputs[j].portrait {
                    assert!(groups.labels[i] < groups.labels[j]);
                }
                let size = |label| case.labels.iter().filter(|&&v| v == label).count();
                if inputs[i].portrait == inputs[j].portrait
                    && size(case.labels[i]) > 1
                    && size(case.labels[j]) == 1
                {
                    assert!(groups.labels[i] < groups.labels[j]);
                }
            }
        }
    }
}
#[test]
fn descriptor_working_set_is_bounded_without_quadratic_graph_storage() {
    let inputs: Vec<_> = (0..257)
        .map(|i| Input {
            key: i.to_string(),
            portrait: false,
        })
        .collect();
    let groups = regional::cluster(
        &inputs,
        0.055,
        |key| {
            let mut vector = vec![0.; 384];
            vector[key.parse::<usize>().unwrap()] = 1.;
            Features::new(vec![vector; 6])
        },
        || false,
    )
    .unwrap()
    .unwrap();
    let mut numbered = groups.labels.clone();
    numbered.sort_unstable();
    assert_eq!(numbered, (1..=257).collect::<Vec<_>>());
    assert_eq!(groups.stats.comparisons, 257 * 256 / 2);
    assert!(groups.stats.peak_resident_records <= 128);
    assert_eq!(groups.stats.unique_records, 257);
}
#[test]
fn hundred_thousand_identical_records_need_no_pair_comparisons() {
    let inputs: Vec<_> = (0..100_000)
        .map(|i| Input {
            key: i.to_string(),
            portrait: i % 2 == 0,
        })
        .collect();
    let groups = regional::cluster(
        &inputs,
        0.055,
        |_| Features::new(vec![vec![1., 0.]; 6]),
        || false,
    )
    .unwrap()
    .unwrap();
    assert_eq!(groups.stats.unique_records, 2);
    assert_eq!(groups.stats.comparisons, 0);
    assert_eq!(groups.stats.peak_resident_records, 1);
    assert!(
        groups
            .labels
            .iter()
            .enumerate()
            .all(|(i, label)| *label == if i % 2 == 0 { 1 } else { 2 })
    );
}
#[test]
fn cancellation_and_invalid_evidence_never_return_partial_groups() {
    use std::cell::Cell;
    let inputs: Vec<_> = (0..140)
        .map(|i| Input {
            key: i.to_string(),
            portrait: false,
        })
        .collect();
    for limit in [0, 70, 160, 400] {
        let calls = Cell::new(0);
        let groups = regional::cluster(
            &inputs,
            0.055,
            |key| {
                let mut v = vec![0.; 140];
                v[key.parse::<usize>().unwrap()] = 1.;
                Features::new(vec![v])
            },
            || {
                calls.set(calls.get() + 1);
                calls.get() > limit
            },
        )
        .unwrap();
        assert!(groups.is_none());
    }
    assert!(Features::new(vec![]).is_err());
    assert!(Features::new(vec![vec![f32::NAN]]).is_err());
    assert!(Features::new(vec![vec![1.], vec![1., 0.]]).is_err());
    assert!(regional::cluster(&[], f32::NAN, |_| unreachable!(), || false).is_err());
}

#[test]
fn crop_extraction_cancellation_and_tiny_images_do_not_initialize_a_model() {
    use photosort::core::{models::Models, pipeline::Asset};
    use std::sync::Arc;
    let pixels = Arc::new(image::RgbaImage::new(3, 3));
    let asset = Asset {
        decoder_warning: None,
        pixels: pixels.clone(),
        thumbnail: pixels,
        metadata: Default::default(),
    };
    let mut models = Models::default();
    let global = vec![1.; 384];
    assert!(models.similarity_regions(&asset, &global, || true).is_err());
    assert_eq!((models.initializations, models.inferences), (0, 0));
    let pixels = Arc::new(image::RgbaImage::new(1, 23));
    let tiny = Asset {
        pixels: pixels.clone(),
        thumbnail: pixels,
        ..asset
    };
    assert_eq!(
        models
            .similarity_regions(&tiny, &global, || false)
            .unwrap()
            .vectors,
        vec![global]
    );
    assert_eq!((models.initializations, models.inferences), (0, 0));
}

#[test]
fn labels_survive_reordered_inputs_and_changed_cache_keys_without_extra_loads() {
    use std::cell::Cell;
    // Two connected components, a chain, independent noise and both orientations.
    let angles = [0f32, 10., 20., 90., 91., 170., 0., 1., 70.];
    let rows: Vec<_> = angles
        .iter()
        .map(|angle| {
            Features::new(vec![
                vec![angle.to_radians().cos(), angle.to_radians().sin()];
                6
            ])
            .unwrap()
        })
        .collect();
    let run = |order: &[usize], prefix: &str| {
        let calls = Cell::new(0);
        let inputs: Vec<_> = order
            .iter()
            .map(|&i| Input {
                key: format!("{prefix}/{i}"),
                portrait: i >= 6,
            })
            .collect();
        let groups = regional::cluster(
            &inputs,
            0.02,
            |key| {
                calls.set(calls.get() + 1);
                Ok(rows[key.rsplit('/').next().unwrap().parse::<usize>().unwrap()].clone())
            },
            || false,
        )
        .unwrap()
        .unwrap();
        assert_eq!(groups.stats.evidence_loads, calls.get());
        assert_eq!(
            calls.get(),
            2 * rows.len(),
            "Numbering must reuse the digest from the first load"
        );
        let mut labels = vec![0; rows.len()];
        for (&i, label) in order.iter().zip(groups.labels) {
            labels[i] = label;
        }
        labels
    };
    let baseline = run(&(0..rows.len()).collect::<Vec<_>>(), "before");
    assert_eq!(
        baseline[0], baseline[2],
        "Transitive DBSCAN membership is unchanged"
    );
    assert_ne!(baseline[0], baseline[3]);
    assert_ne!(
        baseline[0], baseline[6],
        "Orientation partitions remain separate"
    );
    for order in [
        (0..rows.len()).rev().collect::<Vec<_>>(),
        vec![5, 8, 4, 7, 3, 6, 2, 1, 0],
    ] {
        assert_eq!(baseline, run(&order, "moved/cache-key"));
    }
}
