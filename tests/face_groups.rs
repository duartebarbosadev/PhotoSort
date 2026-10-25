use photosort::core::{
    face_groups::{self, Recognition},
    faces::{self, Faces},
    model::MediaId,
    models::Models,
    pipeline::Asset,
    regional::grouping::Record,
    subjects::store::Store,
};
use std::sync::Arc;
fn vector(degrees: f32) -> Vec<f32> {
    let mut v = vec![0.; 128];
    v[0] = degrees.to_radians().cos();
    v[1] = degrees.to_radians().sin();
    v
}
fn put(store: &Store, id: u64, degrees: f32) -> Record {
    let key = Store::recognition_key(&id.to_string(), "model");
    store
        .put_recognition(
            &key,
            &Recognition {
                vectors: vec![vector(degrees)],
            },
        )
        .unwrap();
    Record {
        id: MediaId(id),
        key: Some(key),
        portrait: false,
        date: None,
    }
}
#[test]
fn recognition_groups_cannot_bridge_different_people_and_are_order_independent() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    // A~B and B~C, but A and C fail. A bridge must not merge all three.
    let mut records = vec![
        put(&store, 1, 0.),
        put(&store, 2, 45.),
        put(&store, 3, 90.),
        put(&store, 4, 0.),
        Record {
            id: MediaId(5),
            key: None,
            portrait: false,
            date: None,
        },
    ];
    let result = face_groups::group(&records, &store, || false)
        .unwrap()
        .unwrap()
        .groups;
    assert_eq!(result[&MediaId(1)], result[&MediaId(2)]);
    assert_ne!(result[&MediaId(1)], result[&MediaId(3)]);
    assert_eq!(result[&MediaId(1)], result[&MediaId(4)]);
    assert_eq!(result[&MediaId(5)], 0);
    records.reverse();
    assert_eq!(
        result,
        face_groups::group(&records, &store, || false)
            .unwrap()
            .unwrap()
            .groups
    );
    assert!(
        face_groups::group(&records, &store, || true)
            .unwrap()
            .is_none()
    );
    let count = std::cell::Cell::new(0);
    assert!(
        face_groups::group(&records, &store, || {
            count.set(count.get() + 1);
            count.get() > 5
        })
        .unwrap()
        .is_none()
    );
}
fn put_faces(store: &Store, id: u64, vectors: Vec<Vec<f32>>) -> Record {
    let key = Store::recognition_key(&format!("faces-{id}"), "model");
    store
        .put_recognition(&key, &Recognition { vectors })
        .unwrap();
    Record {
        id: MediaId(id),
        key: Some(key),
        portrait: false,
        date: None,
    }
}
/// A unit vector with `lead` on the first axis, which sets admission order
/// (descriptors are taken in descending order), and the rest at `degrees` in
/// the plane of the next two axes.
fn ordered(lead: f32, degrees: f32) -> Vec<f32> {
    let rest = (1. - lead * lead).sqrt();
    let mut v = vec![0.; 128];
    v[0] = lead;
    v[1] = rest * degrees.to_radians().cos();
    v[2] = rest * degrees.to_radians().sin();
    v
}
#[test]
fn one_poor_face_cannot_split_a_person_but_different_people_stay_apart() {
    // Modelled on a phone library where one person's sunglasses photos matched
    // her other faces 0.72 on average but a tiny inset selfie only 0.47, so
    // requiring a match with every member gave her two folders.
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let records = vec![
        put(&store, 1, 0.),
        put(&store, 2, 10.),
        put(&store, 3, -10.),
        // The inset selfie, taken before the sunglasses photos.
        put(&store, 4, -30.),
        // Sunglasses: 0.42 with the inset selfie, 0.71 on average.
        put(&store, 5, 35.),
        put(&store, 6, 40.),
        // Someone else, alone and next to her.
        put(&store, 7, 90.),
        put(&store, 8, 100.),
        put_faces(&store, 9, vec![vector(0.), vector(95.)]),
        // A third person who matches one of his faces 0.5, but 0.42 on
        // average, is still a third.
        put(&store, 10, 160.),
    ];
    let result = face_groups::group(&records, &store, || false)
        .unwrap()
        .unwrap();
    let her = result.groups[&MediaId(1)];
    for id in 2..=6 {
        assert_eq!(result.groups[&MediaId(id)], her, "photo {id}");
    }
    let him = result.groups[&MediaId(7)];
    assert_ne!(him, her);
    assert_eq!(result.groups[&MediaId(8)], him);
    assert!(result.folders[&MediaId(9)].starts_with("People "));
    assert_ne!(result.groups[&MediaId(10)], him);
    assert_ne!(result.groups[&MediaId(10)], her);
    assert_eq!(
        face_groups::summarize(&result),
        face_groups::Summary {
            photos: 10,
            people: 3,
            repeated: 2
        }
    );
}
#[test]
fn people_split_by_admission_order_merge_unless_seen_together() {
    // `q` comes first and matches `p0` only 0.46, so it starts its own group,
    // although it matches the other faces of that person 0.81 and 0.83.
    let faces = [
        ordered(0.3, 65.),
        ordered(0.2, 0.),
        ordered(0.1, 30.),
        ordered(0., 35.),
    ];
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let apart: Vec<_> = (0..4)
        .map(|i| put_faces(&store, i, vec![faces[i as usize].clone()]))
        .collect();
    let result = face_groups::group(&apart, &store, || false)
        .unwrap()
        .unwrap();
    assert!((1..4).all(|i| result.groups[&MediaId(i)] == result.groups[&MediaId(0)]));
    // In one photo together they are two people, and stay two.
    let mut together = apart[2..].to_vec();
    together.push(put_faces(
        &store,
        10,
        vec![faces[0].clone(), faces[1].clone()],
    ));
    let result = face_groups::group(&together, &store, || false)
        .unwrap()
        .unwrap();
    assert_eq!(result.folders[&MediaId(10)], "People 001 + 002");
}
#[test]
fn recognition_cache_is_model_keyed_persistent_and_rejects_invalid_descriptors() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    let key = Store::recognition_key("faces", "model1");
    assert_ne!(key, Store::recognition_key("faces", "model2"));
    assert_ne!(key, Store::recognition_key("other", "model1"));
    store
        .put_recognition(
            &key,
            &Recognition {
                vectors: vec![vector(30.)],
            },
        )
        .unwrap();
    let reopened = Store::new(root.path().into(), true).unwrap();
    assert_eq!(
        reopened.recognition(&key).unwrap().vectors,
        vec![vector(30.)]
    );
    for invalid in [
        vec![0.; 128],
        vec![f32::NAN; 128],
        vec![1.; 128],
        vec![1.; 592],
    ] {
        assert!(
            store
                .put_recognition(
                    &key,
                    &Recognition {
                        vectors: vec![invalid]
                    }
                )
                .is_err()
        );
    }
    assert!(store.recognition("../escape").is_none());
    let missing = Record {
        id: MediaId(1),
        key: Some(Store::recognition_key("missing", "model1")),
        portrait: false,
        date: None,
    };
    assert!(face_groups::group(&[missing], &store, || false).is_err());
}
#[test]
fn main_people_sets_have_one_destination_independent_of_face_and_photo_order() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    let mut records = vec![
        put(&store, 1, 0.),
        put(&store, 2, 100.),
        put(&store, 3, 200.),
    ];
    for (id, people) in [
        (4, vec![0., 100.]),
        (5, vec![100., 0.]),
        (6, vec![100., 200.]),
        (7, vec![0., 0.]),
        (8, vec![]),
    ] {
        let key = Store::recognition_key(&id.to_string(), "model");
        store
            .put_recognition(
                &key,
                &Recognition {
                    vectors: people.into_iter().map(vector).collect(),
                },
            )
            .unwrap();
        records.push(Record {
            id: MediaId(id),
            key: Some(key),
            portrait: false,
            date: None,
        });
    }
    let result = face_groups::group(&records, &store, || false)
        .unwrap()
        .unwrap();
    assert_eq!(result.groups.len(), 8);
    assert_eq!(result.folders.len(), 8);
    assert_eq!(result.folders[&MediaId(1)], "Person 001");
    assert_eq!(result.folders[&MediaId(4)], "People 001 + 002");
    assert_eq!(result.folders[&MediaId(4)], result.folders[&MediaId(5)]);
    assert_eq!(result.groups[&MediaId(4)], result.groups[&MediaId(5)]);
    assert_ne!(result.groups[&MediaId(4)], result.groups[&MediaId(6)]);
    assert_ne!(result.groups[&MediaId(1)], result.groups[&MediaId(4)]);
    assert_eq!(
        result.groups[&MediaId(7)],
        0,
        "Ambiguous repeated identity is unassigned"
    );
    assert_eq!(result.folders[&MediaId(8)], "Unassigned");
    records.reverse();
    let reopened = Store::new(root.path().into(), true).unwrap();
    let repeated = face_groups::group(&records, &reopened, || false)
        .unwrap()
        .unwrap();
    assert_eq!(result.groups, repeated.groups);
    assert_eq!(result.folders, repeated.folders);
    assert_ne!(
        Store::recognition_key("face", "model"),
        blake3::hash(b"face-recognition-v2-sface112-main60:face:model")
            .to_hex()
            .to_string()
    );
    assert!(
        store
            .put_recognition(
                &Store::recognition_key("too-many", "model"),
                &Recognition {
                    vectors: vec![vector(0.); 11]
                }
            )
            .is_err()
    );
}

#[test]
fn verified_path_changes_cannot_renumber_people_or_change_cluster_admission() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), false).unwrap();
    let mut records = vec![put(&store, 1, 0.), put(&store, 2, 45.), put(&store, 3, 90.)];
    let key = Store::recognition_key("pair", "model");
    store
        .put_recognition(
            &key,
            &Recognition {
                vectors: vec![vector(0.), vector(90.)],
            },
        )
        .unwrap();
    records.push(Record {
        id: MediaId(4),
        key: Some(key),
        portrait: false,
        date: None,
    });
    let before = face_groups::group(&records, &store, || false)
        .unwrap()
        .unwrap();
    let identities: Vec<_> = records.iter().map(|r| r.id).collect();
    for (i, record) in records.iter_mut().enumerate() {
        record.id = MediaId(100 - i as u64);
    }
    let after = face_groups::group(&records, &store, || false)
        .unwrap()
        .unwrap();
    for (old, record) in identities.iter().zip(&records) {
        assert_eq!(before.groups[old], after.groups[&record.id]);
        assert_eq!(before.folders[old], after.folders[&record.id]);
    }
}

#[test]
fn people_combination_labels_use_the_existing_companion_plan_once() {
    use photosort::core::{catalog::Catalog, operations};
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().canonicalize().unwrap();
    for name in ["friends.JPG", "friends.ARW", "friends.xmp"] {
        std::fs::write(root.join(name), name).unwrap();
    }
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let labels = catalog
        .media
        .iter()
        .map(|media| (media.id, "People 001 + 003".into()))
        .collect();
    let plan = operations::organize_plan(&catalog, &labels, &root, true).unwrap();
    let sources: Vec<_> = plan.actions.iter().flat_map(|a| a.sources()).collect();
    assert_eq!(sources.len(), 3);
    assert_eq!(
        sources
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len(),
        3
    );
    let report = operations::execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    for name in ["friends.JPG", "friends.ARW", "friends.xmp"] {
        assert!(!root.join(name).exists());
        assert_eq!(
            std::fs::read_to_string(root.join("People 001 + 003").join(name)).unwrap(),
            name
        );
    }
}

#[test]
fn shared_verified_faces_reuse_previous_work_and_cancel_without_publishing() {
    let root = tempfile::tempdir().unwrap();
    let store = Store::new(root.path().into(), true).unwrap();
    let mut models = Models::default();
    let pixels = Arc::new(image::RgbaImage::new(16, 16));
    let asset = Asset {
        decoder_warning: None,
        pixels: pixels.clone(),
        thumbnail: pixels,
        metadata: Default::default(),
    };
    let legacy = Faces::default();
    faces::cached(&mut models, &asset, &store, "source", Some(&legacy), || {
        false
    })
    .unwrap();
    faces::cached(&mut models, &asset, &store, "source", None, || false).unwrap();
    let reopened = Store::new(root.path().into(), true).unwrap();
    faces::cached(&mut models, &asset, &reopened, "source", None, || false).unwrap();
    assert_eq!((models.initializations, models.inferences), (0, 0));
    assert!(
        faces::cached(&mut models, &asset, &store, "cancel", Some(&legacy), || {
            true
        })
        .is_err()
    );
    assert!(
        store
            .faces(&Store::face_key(
                "cancel",
                &photosort::core::models::face_signature()
            ))
            .is_none()
    );
}
#[cfg(feature = "onnx")]
#[test]
#[ignore = "requires seven samples and native SFace, detector and landmark models"]
fn native_people_grouping_reuses_detection_inference_and_survives_restart() -> anyhow::Result<()> {
    use photosort::core::{
        catalog::Catalog,
        jobs::{AnalysisProfile, Event, Runtime},
        operations,
        pipeline::Pipeline,
    };
    use std::{
        collections::HashMap,
        time::{Duration, Instant},
    };
    let samples = std::path::PathBuf::from(
        std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"),
    );
    let catalog = Catalog::scan(&samples, || false, |_| {})?;
    assert_eq!(catalog.media.len(), 7);
    let hashes: HashMap<_, _> = catalog
        .media
        .iter()
        .map(|m| Ok((m.id, operations::content_hash(&m.path, || false)?)))
        .collect::<anyhow::Result<_>>()?;
    let cache = tempfile::tempdir()?;
    let mut runtime = Runtime::with_cache(|| {}, cache.path().into());
    let mut snapshots = Vec::new();
    let mut expected = None;
    for pass in 0..3 {
        if pass == 2 {
            runtime.shutdown();
            runtime = Runtime::with_cache(|| {}, cache.path().into());
        }
        let start = Instant::now();
        let run = runtime
            .analyze(Arc::new(catalog.media.clone()), AnalysisProfile::Faces)
            .map_err(anyhow::Error::msg)?;
        let mut received = false;
        loop {
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(90),
                "Face analysis timed out"
            );
            match runtime.events.recv_timeout(Duration::from_secs(1)) {
                Ok(e) => match e.event {
                    Event::FaceGroups {
                        run: r,
                        groups: output,
                    } if r == run => {
                        assert_eq!(output.folders.len(), 7);
                        let groups = output.groups;
                        assert_eq!(groups.len(), 7);
                        if let Some(expected) = &expected {
                            assert_eq!(&groups, expected);
                        } else {
                            expected = Some(groups);
                        }
                        received = true;
                    }
                    Event::AnalysisDone { run: r, stats } if r == run => {
                        assert!(received);
                        assert_eq!(stats.technical_measurements, 0);
                        assert_eq!(stats.quality_measurements, 0);
                        println!("People pass {pass}: {:?}, {stats:?}", start.elapsed());
                        snapshots.push(stats);
                        break;
                    }
                    Event::AnalysisError { error, .. } | Event::Error(error) => {
                        anyhow::bail!(error)
                    }
                    Event::Analyzed { .. }
                    | Event::GeneralGroups { .. }
                    | Event::SubjectGroups { .. } => {
                        panic!("Face grouping polluted another workflow's results")
                    }
                    _ => {}
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    assert_eq!(snapshots[0], snapshots[1]);
    assert_eq!(snapshots[2].decodes, 0);
    assert_eq!(snapshots[2].model_inferences, 0);
    assert_eq!(snapshots[2].model_initializations, 0);
    runtime.shutdown();
    // Verify shared detection on all original formats. The portrait fixture test
    // below separately exercises recognition when these samples have no faces.
    let store_root = tempfile::tempdir()?;
    let store = Store::new(store_root.path().into(), true)?;
    let mut pipeline = Pipeline::new(64 * 1024 * 1024, store_root.path().join("images"));
    let mut models = Models::default();
    let mut recognized = 0;
    for media in &catalog.media {
        let asset = pipeline.analysis(media, || false)?;
        let source = Pipeline::key(media)?;
        let faces = faces::cached(&mut models, &asset, &store, &source, None, || false)?;
        let before = models.inferences;
        faces::cached(&mut models, &asset, &store, &source, None, || false)?;
        assert_eq!(before, models.inferences);
        let descriptor = face_groups::describe(&mut models, &asset, &faces, || false)?;
        for v in descriptor.vectors {
            recognized += 1;
            assert_eq!(v.len(), 128);
            assert!(v.iter().all(|x| x.is_finite()));
        }
        println!(
            "{}: {} verified faces, descriptor {}",
            media.name(),
            faces.count,
            recognized
        );
        assert_eq!(
            hashes[&media.id],
            operations::content_hash(&media.path, || false)?
        );
    }
    println!("Recognized {recognized} main faces across seven unmodified samples");
    Ok(())
}

#[cfg(feature = "onnx")]
#[test]
#[ignore = "requires native face models and SFace"]
fn native_portrait_recognition_aligns_and_groups_brightness_variants() -> anyhow::Result<()> {
    use photosort::core::{
        catalog::Catalog,
        jobs::{AnalysisProfile, Event, Runtime},
        pipeline::Pipeline,
    };
    use std::time::{Duration, Instant};
    let demo =
        image::load_from_memory(include_bytes!("fixtures/recognition/sface-demo.jpg"))?.to_rgba8();
    let portrait = image::imageops::crop_imm(&demo, 0, 12, 512, 488).to_image();
    let brighter = image::imageops::brighten(&portrait, 18);
    let group = image::imageops::crop_imm(&demo, 512, 122, 512, 267).to_image();
    // A close-up pair exercises recognition independently of the small faces
    // in the untouched group scene above.
    let other = image::imageops::resize(
        &image::imageops::crop_imm(&demo, 686, 176, 48, 60).to_image(),
        224,
        280,
        image::imageops::FilterType::Triangle,
    );
    let face = image::imageops::crop_imm(&portrait, 208, 32, 224, 280).to_image();
    let mut pair = image::RgbaImage::new(448, 280);
    image::imageops::replace(&mut pair, &face, 0, 0);
    image::imageops::replace(&mut pair, &face, 224, 0);
    let mut together = image::RgbaImage::new(448, 280);
    image::imageops::replace(&mut together, &face, 0, 0);
    image::imageops::replace(&mut together, &other, 224, 0);
    let root = tempfile::tempdir()?;
    other.save(root.path().join("other.png"))?;
    together.save(root.path().join("together.png"))?;
    image::imageops::brighten(&together, 12).save(root.path().join("together-brighter.png"))?;
    pair.save(root.path().join("pair.png"))?;
    portrait.save(root.path().join("portrait.png"))?;
    brighter.save(root.path().join("brighter.png"))?;
    group.save(root.path().join("group.png"))?;
    let catalog = Catalog::scan(root.path(), || false, |_| {})?;
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(32 * 1024 * 1024, cache.path().join("pixels"));
    let mut models = Models::default();
    let mut descriptors = Vec::new();
    for media in &catalog.media {
        let asset = pipeline.analysis(media, || false)?;
        let detected = faces::analyze_cancellable(&mut models, &asset, || false)?;
        println!("{}: {} verified faces", media.name(), detected.count);
        let recognized = face_groups::describe(&mut models, &asset, &detected, || false)?;
        match media.name().as_str() {
            // Five people facing the camera, faces about 30 pixels wide; the
            // sixth, half-hidden face at the edge is not verified.
            "group.png" => {
                assert_eq!(detected.count, 5, "Small group faces must be found");
                assert_eq!(recognized.vectors.len(), 5);
            }
            "pair.png" | "together.png" | "together-brighter.png" => {
                assert!(detected.count > 1);
                assert_eq!(
                    recognized.vectors.len(),
                    2,
                    "Both main faces must produce descriptors: {}",
                    media.name()
                );
            }
            "other.png" => assert_eq!(
                recognized.vectors.len(),
                1,
                "Second portrait must exercise SFace"
            ),
            _ => {
                assert!(detected.count >= 1);
                descriptors.push(
                    recognized
                        .vectors
                        .into_iter()
                        .next()
                        .expect("Clear portrait must exercise SFace"),
                );
            }
        }
    }
    assert_eq!(
        models.initializations, 4,
        "One detector, landmark, blendshape and recognition session"
    );
    assert_eq!(descriptors.len(), 2);
    let cosine: f32 = descriptors[0]
        .iter()
        .zip(&descriptors[1])
        .map(|(a, b)| a * b)
        .sum();
    println!("Brightness variant similarity: {cosine:.4}");
    assert!(cosine >= face_groups::MIN_SIMILARITY);
    let mut runtime = Runtime::with_cache(|| {}, cache.path().join("runtime"));
    let mut snapshots = Vec::new();
    for pass in 0..3 {
        if pass == 2 {
            runtime.shutdown();
            runtime = Runtime::with_cache(|| {}, cache.path().join("runtime"));
        }
        let run = runtime
            .analyze(Arc::new(catalog.media.clone()), AnalysisProfile::Faces)
            .map_err(anyhow::Error::msg)?;
        let start = Instant::now();
        let mut received = false;
        loop {
            anyhow::ensure!(
                start.elapsed() < Duration::from_secs(120),
                "Portrait worker timed out"
            );
            match runtime.events.recv_timeout(Duration::from_secs(1)) {
                Ok(envelope) => match envelope.event {
                    Event::FaceGroups {
                        run: r,
                        groups: output,
                    } if r == run => {
                        let groups = output.groups;
                        let id = |name: &str| {
                            catalog.media.iter().find(|m| m.name() == name).unwrap().id
                        };
                        assert_ne!(groups[&id("portrait.png")], 0);
                        assert_eq!(groups[&id("portrait.png")], groups[&id("brighter.png")]);
                        assert_ne!(groups[&id("group.png")], 0);
                        assert!(output.folders[&id("group.png")].starts_with("People "));
                        assert_ne!(groups[&id("together.png")], 0);
                        assert_eq!(
                            groups[&id("together.png")],
                            groups[&id("together-brighter.png")]
                        );
                        assert!(output.folders[&id("together.png")].starts_with("People "));
                        assert!(output.folders[&id("together.png")].contains(" + "));
                        assert_ne!(groups[&id("portrait.png")], groups[&id("other.png")]);
                        assert_ne!(groups[&id("portrait.png")], groups[&id("together.png")]);
                        assert_eq!(groups[&id("pair.png")], 0);
                        received = true;
                    }
                    Event::AnalysisDone { run: r, stats } if r == run => {
                        assert!(received);
                        println!("Portrait pass {pass}: {:?}, {stats:?}", start.elapsed());
                        snapshots.push(stats);
                        break;
                    }
                    Event::AnalysisError { error, .. } | Event::Error(error) => {
                        anyhow::bail!(error)
                    }
                    _ => {}
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
    assert_eq!(snapshots[0].model_initializations, 4);
    assert_eq!(snapshots[0], snapshots[1]);
    assert_eq!(snapshots[2].decodes, 0);
    assert_eq!(snapshots[2].model_initializations, 0);
    assert_eq!(snapshots[2].model_inferences, 0);
    runtime.shutdown();
    Ok(())
}

#[cfg(feature = "onnx")]
#[test]
#[ignore = "requires native face models and SFace"]
fn native_distant_faces_are_found_and_recognized_from_the_display_image() -> anyhow::Result<()> {
    use photosort::core::{catalog::Catalog, pipeline::Pipeline};
    // A phone-sized photo of someone standing a few metres away: about 110
    // pixels of face in 4032x3024, so about 28 pixels in the 1024-pixel
    // analysis image. Five fixed detector windows never saw such a face, and
    // its eyes are too close together there for recognition.
    let demo =
        image::load_from_memory(include_bytes!("fixtures/recognition/sface-demo.jpg"))?.to_rgba8();
    let portrait = image::imageops::crop_imm(&demo, 0, 12, 512, 488).to_image();
    let small = image::imageops::resize(&portrait, 400, 381, image::imageops::FilterType::Lanczos3);
    let mut photo = image::RgbaImage::from_pixel(4032, 3024, image::Rgba([128, 128, 128, 255]));
    image::imageops::replace(&mut photo, &small, 2600, 1900);
    let root = tempfile::tempdir()?;
    portrait.save(root.path().join("near.png"))?;
    photo.save(root.path().join("far.png"))?;
    let catalog = Catalog::scan(root.path(), || false, |_| {})?;
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().join("pixels"));
    let mut models = Models::default();
    let mut descriptors = std::collections::HashMap::new();
    for media in &catalog.media {
        let asset = pipeline.analysis(media, || false)?;
        let detected = faces::analyze_cancellable(&mut models, &asset, || false)?;
        assert_eq!(detected.count, 1, "{}: one face", media.name());
        let recognized = face_groups::describe_detailed(
            &mut models,
            &asset,
            &detected,
            || pipeline.load_neutral_cancellable(media, || false),
            || false,
        )?;
        assert_eq!(recognized.vectors.len(), 1, "{}: recognized", media.name());
        if media.name() == "far.png" {
            assert!(
                face_groups::describe(&mut models, &asset, &detected, || false)?
                    .vectors
                    .is_empty(),
                "The analysis image alone is too small to recognize this face"
            );
        }
        descriptors.insert(media.name(), recognized.vectors[0].clone());
    }
    let cosine: f32 = descriptors["near.png"]
        .iter()
        .zip(&descriptors["far.png"])
        .map(|(a, b)| a * b)
        .sum();
    println!("Near and far similarity: {cosine:.4}");
    assert!(cosine >= face_groups::MIN_SIMILARITY);
    Ok(())
}
