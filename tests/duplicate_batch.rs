use photosort::core::{
    faces::Face,
    model::*,
    near_duplicates::{
        self,
        batch::{self, Evidence, Input},
    },
    pipeline::Asset,
    subjects::store::Store,
};
use std::{cell::Cell, sync::Arc};
fn input(n: u64) -> Input {
    Input {
        media: Media {
            id: MediaId(n),
            path: format!("/{n}.png").into(),
            kind: MediaKind::Photo,
            fingerprint: Fingerprint {
                bytes: 100,
                ..Default::default()
            },
        },
        source: n.to_string(),
        key: Store::key(&n.to_string(), "model"),
        hash: format!("hash{n}"),
        group: 1,
        embedding: vec![2., 0.],
        sharpness: Some(100.),
        metadata_fields: 0,
        shot: None,
    }
}
#[derive(Default)]
struct Source {
    images: usize,
    faces: usize,
    missing: bool,
    changed: bool,
}
impl Evidence for Source {
    fn image(&mut self, i: &Input) -> anyhow::Result<Asset> {
        self.images += 1;
        let name = if self.changed && i.media.id == MediaId(2) {
            "03.png"
        } else {
            "00.png"
        };
        let pixels = Arc::new(
            image::open(
                std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("tests/fixtures/near_duplicate")
                    .join(name),
            )?
            .to_rgba8(),
        );
        Ok(Asset {
            decoder_warning: None,
            pixels: pixels.clone(),
            thumbnail: pixels,
            metadata: Default::default(),
        })
    }
    fn faces(&mut self, _: &Input) -> Option<Vec<Face>> {
        self.faces += 1;
        (!self.missing).then(Vec::new)
    }
}
#[test]
fn exact_cohorts_are_disjoint_without_pixel_or_face_work() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let mut inputs: Vec<_> = (1..=6).map(input).collect();
    for i in &mut inputs {
        i.hash = "same".into();
        i.group = i.media.id.0 as usize;
        i.embedding = vec![f32::NAN]; // Exact equality needs no similarity gate.
    }
    let mut source = Source::default();
    let pairs = batch::review(&inputs, &store, &mut source, || false, |_, _| {})
        .unwrap()
        .unwrap();
    assert_eq!(
        pairs
            .iter()
            .map(|p| (p.keep, p.discard))
            .collect::<Vec<_>>(),
        vec![
            (MediaId(1), MediaId(2)),
            (MediaId(3), MediaId(4)),
            (MediaId(5), MediaId(6))
        ]
    );
    assert_eq!((source.images, source.faces), (0, 0));
}
#[test]
fn keeper_uses_rounded_sharpness_then_metadata_then_size_and_stable_order() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let mut a = input(1);
    let mut b = input(2);
    b.hash = a.hash.clone();
    let check = |a: &Input, b: &Input, keep: u64, reason: &str| {
        let result = batch::review(
            &[a.clone(), b.clone()],
            &store,
            &mut Source::default(),
            || false,
            |_, _| {},
        )
        .unwrap()
        .unwrap();
        assert_eq!(result[0].keep, MediaId(keep));
        assert!(result[0].reason.contains(reason));
    };
    a.sharpness = Some(100.5);
    b.sharpness = Some(100.4);
    b.metadata_fields = 1;
    check(&a, &b, 2, "metadata"); // Python's ties-to-even, not round-away.
    a.sharpness = Some(101.5);
    check(&a, &b, 1, "sharper");
    a.sharpness = None;
    check(&a, &b, 2, "metadata");
    a.metadata_fields = 1;
    a.media.fingerprint.bytes = 1000;
    check(&a, &b, 1, "larger");
    b.media.fingerprint.bytes = 1000;
    check(&a, &b, 1, "library order");
}
#[test]
fn near_pair_cache_reuses_full_assessment_and_survives_store_restart() {
    let root = tempfile::tempdir().unwrap();
    let inputs = [input(1), input(2)];
    let mut source = Source::default();
    for pass in 0..2 {
        let store = Store::new(root.path().into(), true).unwrap();
        let pairs = batch::review(&inputs, &store, &mut source, || false, |_, _| {})
            .unwrap()
            .unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(
            pairs[0].assessment.decision,
            near_duplicates::Decision::SafeNearDuplicate
        );
        assert_eq!(
            (source.images, source.faces),
            (2, 2),
            "Pass {pass} repeated evidence work"
        );
    }
    let mut changed = inputs.clone();
    changed[1].key = Store::key("new-sidecar", "model");
    let store = Store::new(root.path().into(), true).unwrap();
    batch::review(&changed, &store, &mut source, || false, |_, _| {}).unwrap();
    assert_eq!((source.images, source.faces), (4, 4));
}
#[test]
fn missing_faces_are_not_cached_as_absent_faces_and_cancellation_publishes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    let inputs = [input(1), input(2)];
    let mut source = Source {
        missing: true,
        ..Default::default()
    };
    assert!(
        batch::review(&inputs, &store, &mut source, || false, |_, _| {})
            .unwrap()
            .unwrap()
            .is_empty()
    );
    source.missing = false;
    assert_eq!(
        batch::review(&inputs, &store, &mut source, || false, |_, _| {})
            .unwrap()
            .unwrap()
            .len(),
        1
    );
    let calls = Cell::new(0);
    assert!(
        batch::review(
            &inputs,
            &store,
            &mut source,
            || {
                calls.set(calls.get() + 1);
                calls.get() > 2
            },
            |_, _| {}
        )
        .unwrap()
        .is_none()
    );
}
#[test]
fn distinct_groups_and_low_or_invalid_cosine_never_request_images() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let a = input(1);
    let mut b = input(2);
    let mut source = Source::default();
    for vector in [vec![0., 1.], vec![0., 0.], vec![f32::NAN, 1.]] {
        b.embedding = vector;
        assert!(
            batch::review(
                &[a.clone(), b.clone()],
                &store,
                &mut source,
                || false,
                |_, _| {}
            )
            .unwrap()
            .unwrap()
            .is_empty()
        );
    }
    b.embedding = a.embedding.clone();
    b.group = 2;
    assert!(
        batch::review(&[a, b], &store, &mut source, || false, |_, _| {})
            .unwrap()
            .unwrap()
            .is_empty()
    );
    assert_eq!((source.images, source.faces), (0, 0));
}

#[test]
fn scene_corrections_change_candidate_pairs_without_repeating_cached_evidence() {
    use photosort::core::manual_groups::{self, Edit};
    use std::collections::HashMap;
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let mut inputs = vec![input(1), input(2)];
    let media: Vec<_> = inputs.iter().map(|i| i.media.clone()).collect();
    let automatic = Arc::new(HashMap::from([(MediaId(1), 1), (MediaId(2), 2)]));
    let mut evidence = Source::default();
    let mut run = |manual: &manual_groups::ManualGroups| {
        let groups = manual_groups::resolve_scenes(&automatic, manual, || false).unwrap();
        for input in &mut inputs {
            input.group = groups[&input.media.id];
        }
        batch::review(&inputs, &store, &mut evidence, || false, |_, _| {})
            .unwrap()
            .unwrap()
    };
    assert!(run(&Default::default()).is_empty());
    let merged = manual_groups::edit(
        1,
        automatic.clone(),
        Arc::default(),
        &media,
        Edit::Move {
            selected: vec![MediaId(2)],
            target: Some(1),
        },
        || false,
    )
    .unwrap();
    assert_eq!(run(&merged.manual).len(), 1);
    assert_eq!(run(&merged.manual).len(), 1);
    let split = manual_groups::edit(
        2,
        automatic.clone(),
        merged.manual,
        &media,
        Edit::Move {
            selected: vec![MediaId(2)],
            target: None,
        },
        || false,
    )
    .unwrap();
    assert!(run(&split.manual).is_empty());
    assert_eq!(
        (evidence.images, evidence.faces),
        (2, 2),
        "Only the first candidate comparison needs pixels/faces; regrouping reuses it"
    );
}

/// A JPEG or HEIC input with the capture facts the camera wrote.
fn shot(n: u64, name: &str, ms: i64, subsec: Option<u16>) -> Input {
    let mut i = input(n);
    i.media.path = format!("/photos/{name}").into();
    i.hash = format!("hash{n}");
    let metadata = Metadata {
        camera: "Galaxy S24 Ultra".into(),
        captured_ms: Some(ms),
        captured_subsec_ms: subsec,
        width: 4000,
        height: 2252,
        ..Default::default()
    };
    i.shot = near_duplicates::formats::Shot::new(&i.media.path, &metadata);
    i
}
#[test]
fn a_jpeg_and_heic_of_one_shot_pair_as_two_formats_before_near_review() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let inputs = [
        shot(1, "20251025_154337.jpeg", 1_761_360_217_000, Some(631)),
        // A different name: only the capture facts and pixels pair it.
        shot(2, "IMG_0042.heic", 1_761_360_217_000, Some(631)),
    ];
    let mut source = Source::default();
    let pairs = batch::review(&inputs, &store, &mut source, || false, |_, _| {})
        .unwrap()
        .unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].kind, batch::Kind::Formats);
    assert_eq!((pairs[0].discard, pairs[0].keep), (MediaId(1), MediaId(2)));
    // Paired before the near pass: no face work, and never a Near pair.
    assert_eq!(source.faces, 0);
}
#[test]
fn a_shared_name_with_another_capture_time_or_other_pixels_is_not_two_formats() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let kinds = |inputs: &[Input], source: &mut Source| {
        batch::review(inputs, &store, source, || false, |_, _| {})
            .unwrap()
            .unwrap()
            .into_iter()
            .map(|p| p.kind)
            .collect::<Vec<_>>()
    };
    // Same name, a second apart: two shots, reviewed as near frames at most.
    let later = [
        shot(1, "same.jpeg", 1_761_360_217_000, None),
        shot(2, "same.heic", 1_761_360_218_000, None),
    ];
    assert!(!kinds(&later, &mut Source::default()).contains(&batch::Kind::Formats));
    // Same name and second, another sub-second: a burst.
    let burst = [
        shot(1, "same.jpeg", 1_761_360_217_000, Some(15)),
        shot(2, "same.heic", 1_761_360_217_000, Some(631)),
    ];
    assert!(!kinds(&burst, &mut Source::default()).contains(&batch::Kind::Formats));
    // Same name and capture, different pixels: never the same shot.
    let edited = [
        shot(1, "same.jpeg", 1_761_360_217_000, Some(631)),
        shot(2, "same.heic", 1_761_360_217_000, Some(631)),
    ];
    let mut source = Source {
        changed: true,
        ..Default::default()
    };
    assert!(!kinds(&edited, &mut source).contains(&batch::Kind::Formats));
}
/// A HEIC that cannot be decoded (a build without HEIF) skips only its pair.
#[test]
fn an_undecodable_twin_skips_the_pair_without_failing_the_review() {
    struct Failing;
    impl Evidence for Failing {
        fn image(&mut self, i: &Input) -> anyhow::Result<Asset> {
            anyhow::ensure!(i.media.id != MediaId(2), "Cannot decode HEIC");
            Source::default().image(i)
        }
        fn faces(&mut self, _: &Input) -> Option<Vec<Face>> {
            Some(Vec::new())
        }
    }
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let mut inputs = vec![
        shot(1, "a.jpeg", 1_761_360_217_000, Some(631)),
        shot(2, "a.heic", 1_761_360_217_000, Some(631)),
        input(3),
        input(4),
    ];
    inputs[1].group = 2; // Out of the near pass, which would need its pixels.
    inputs[0].group = 3;
    let pairs = batch::review(&inputs, &store, &mut Failing, || false, |_, _| {})
        .unwrap()
        .unwrap();
    assert_eq!(pairs.len(), 1);
    assert_eq!(pairs[0].kind, batch::Kind::Near);
    assert_eq!((pairs[0].discard, pairs[0].keep), (MediaId(4), MediaId(3)));
}
