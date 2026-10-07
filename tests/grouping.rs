use photosort::core::{dates, metadata, model::*};
use std::cell::Cell;
// Fixtures provide the already-extracted shared artifacts; the production
// regional service owns clustering and date-folder numbering.
struct TestGroups {
    records: Vec<photosort::core::regional::grouping::Record>,
    store: photosort::core::subjects::store::Store,
}
impl Default for TestGroups {
    fn default() -> Self {
        Self {
            records: Vec::new(),
            store: photosort::core::subjects::store::Store::new(Default::default(), false).unwrap(),
        }
    }
}
impl TestGroups {
    fn record(&mut self, media: &Media, metadata: &Metadata, embedding: Option<&[f32]>) {
        if media.kind == MediaKind::Video {
            return;
        }
        let key = embedding.map(|v| {
            let key =
                photosort::core::subjects::store::Store::regional_key(&media.id.0.to_string());
            let mut padded = v.to_vec();
            padded.resize(384, 0.);
            self.store
                .put_regional(
                    &key,
                    &photosort::core::regional::Embedding {
                        vectors: vec![padded; 6],
                    },
                )
                .unwrap();
            key
        });
        self.records
            .push(photosort::core::regional::grouping::Record {
                id: media.id,
                key,
                portrait: false,
                date: dates::label(media, metadata),
            });
    }
    fn finish(
        self,
        cancelled: impl Fn() -> bool,
    ) -> Option<std::collections::HashMap<MediaId, String>> {
        photosort::core::regional::grouping::group(
            &self.records,
            &self.store,
            true,
            0.055,
            cancelled,
        )
        .unwrap()
        .map(|r| r.folders)
    }
}
fn media(id: u64, name: &str) -> Media {
    Media {
        id: MediaId(id),
        path: name.into(),
        kind: MediaKind::Photo,
        fingerprint: Fingerprint {
            bytes: 1,
            modified_ns: 0,
            ..Default::default()
        },
    }
}
fn day(date: &str) -> Metadata {
    Metadata {
        captured: date.into(),
        ..Default::default()
    }
}
#[test]
fn mixed_partitions_before_similarity_and_numbers_groups_per_day() {
    let mut groups = TestGroups::default();
    let a = [1., 0.];
    let b = [0., 1.];
    for (id, date, vector) in [
        (1, "2025:03:15 10:00:00", &a),
        (2, "2025:03:15 10:01:00", &a),
        (3, "2025:03:16 10:00:00", &a),
        (4, "2025:03:15 10:02:00", &b),
        (5, "2025:03:15 10:03:00", &b),
        (6, "2025:03:17 10:00:00", &b),
        (7, "2025:03:17 10:01:00", &b),
    ] {
        groups.record(&media(id, "photo.jpg"), &day(date), Some(vector));
    }
    let labels = groups.finish(|| false).unwrap();
    assert!(matches!(
        labels[&MediaId(1)].as_str(),
        "2025-03-15/group-1" | "2025-03-15/group-2"
    ));
    assert_eq!(labels[&MediaId(1)], labels[&MediaId(2)]);
    assert_eq!(
        labels[&MediaId(3)],
        "2025-03-16",
        "Do not share a cluster across dates"
    );
    assert!(matches!(
        labels[&MediaId(4)].as_str(),
        "2025-03-15/group-1" | "2025-03-15/group-2"
    ));
    assert_ne!(labels[&MediaId(1)], labels[&MediaId(4)]);
    assert_eq!(labels[&MediaId(5)], labels[&MediaId(4)]);
    assert_eq!(labels[&MediaId(6)], "2025-03-17/group-1");
    assert_eq!(labels[&MediaId(7)], labels[&MediaId(6)]);
}
#[test]
fn unscored_and_single_photos_go_to_the_day_and_videos_stay_in_place() {
    let mut groups = TestGroups::default();
    groups.record(&media(1, "single.jpg"), &day("2025-01-02"), Some(&[1., 0.]));
    groups.record(&media(2, "broken.jpg"), &day("2025-01-02"), None);
    let mut undated = media(3, "unknown.jpg");
    undated.fingerprint.modified_ns = u128::MAX;
    groups.record(&undated, &Metadata::default(), Some(&[1., 0.]));
    let mut video = media(4, "clip.mov");
    video.kind = MediaKind::Video;
    groups.record(&video, &day("2025-01-02"), None);
    let labels = groups.finish(|| false).unwrap();
    assert_eq!(labels[&MediaId(1)], "2025-01-02");
    assert_eq!(labels[&MediaId(2)], "2025-01-02");
    assert_eq!(labels[&MediaId(3)], "undated");
    assert!(!labels.contains_key(&MediaId(4)));
}
#[test]
fn cancelled_group_materialization_never_publishes_partial_labels() {
    let mut groups = TestGroups::default();
    for id in 0..100 {
        groups.record(&media(id, "photo.jpg"), &day("2025-01-02"), Some(&[1.]));
    }
    let calls = Cell::new(0);
    assert!(
        groups
            .finish(|| {
                calls.set(calls.get() + 1);
                calls.get() > 10
            })
            .is_none()
    );
}
#[test]
fn date_preference_uses_metadata_filename_then_catalog_time_without_io() {
    let photo = media(1, "/nonexistent/IMG_20250317_001.jpg");
    assert_eq!(
        dates::label(&photo, &day("2024:02:29 12:00:00")),
        Some("2024-02-29".into())
    );
    assert_eq!(
        dates::label(&photo, &day("invalid")),
        Some("2025-03-17".into())
    );
    let fallback = media(2, "/nonexistent/no-date.jpg");
    use chrono::{Local, TimeZone};
    let expected = Local
        .timestamp_opt(0, 0)
        .single()
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();
    assert_eq!(
        dates::label(&fallback, &Metadata::default()),
        Some(expected)
    );
    let preferred = Metadata {
        grouping_date: Some("2023-04-05".into()),
        captured: "2024-01-02".into(),
        ..Default::default()
    };
    assert_eq!(dates::label(&photo, &preferred), Some("2023-04-05".into()));
}
#[test]
fn date_parsing_validates_calendar_and_reference_filename_patterns() {
    for text in [
        "2024:02:29 12:00:00",
        "2024-02-29T12:00:00+08:00",
        "2024/02/29",
        "2024.02.29",
    ] {
        assert_eq!(dates::parse(text), Some("2024-02-29".into()));
    }
    assert_eq!(dates::parse("2025-02-29"), None);
    for text in [
        "IMG_20250317_001.jpg",
        "旅2025-03-17.jpg",
        "2025_03_17.png",
        "2025.03.17.png",
    ] {
        assert_eq!(dates::filename(text, 2026), Some("2025-03-17".into()));
    }
    for text in [
        "IMG_20250230_001.jpg",
        "IMG_18990101_001.jpg",
        "IMG_20400101_001.jpg",
        "IMG_20250317.jpg",
    ] {
        assert_eq!(dates::filename(text, 2026), None);
    }
}
#[test]
fn xmp_dates_resolve_namespaces_and_accept_attributes_and_elements() {
    let values = metadata::read_xmp_dates(r#"<r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:x="http://ns.adobe.com/xap/1.0/" xmlns:p="http://ns.adobe.com/photoshop/1.0/" xmlns:fake="urn:unrelated"><r:Description x:CreateDate="2024-02-29T23:30:00-08:00" fake:CreateDate="2000-01-01"><p:DateCreated>2023:01:02</p:DateCreated></r:Description></r:RDF>"#).unwrap();
    assert_eq!(
        values,
        [Some("2024-02-29".into()), Some("2023-01-02".into())]
    );
}
#[test]
fn sidecar_date_changes_invalidate_shared_metadata_and_proxies() {
    use photosort::core::{catalog::Catalog, pipeline::Pipeline};
    let folder = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let path = folder.path().join("photo.png");
    image::RgbImage::new(4, 4).save(&path).unwrap();
    let catalog = Catalog::scan(folder.path(), || false, |_| {}).unwrap();
    let photo = &catalog.media[0];
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
    let write = |date: &str| {
        std::fs::write(metadata::sidecar(&path), format!(r#"<r:RDF xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:x="http://ns.adobe.com/xap/1.0/"><r:Description x:CreateDate="{date}"/></r:RDF>"#)).unwrap()
    };
    write("2024-02-29");
    let first = pipeline.load(photo).unwrap();
    assert_eq!(
        dates::label(photo, &first.metadata),
        Some("2024-02-29".into())
    );
    for _ in Step::ALL {
        pipeline.load(photo).unwrap();
    }
    assert_eq!(
        pipeline.stats.decodes, 1,
        "All workflows must reuse the same decode"
    );
    write("2025-01-02T12:00:00Z");
    let changed = pipeline.load(photo).unwrap();
    assert_eq!(
        dates::label(photo, &changed.metadata),
        Some("2025-01-02".into())
    );
    assert_eq!(pipeline.stats.decodes, 2);
}

#[test]
fn date_group_labels_reach_the_shared_companion_safe_move_plan() {
    use photosort::core::{
        catalog::Catalog,
        operations::{self, Action},
    };
    let folder = tempfile::tempdir().unwrap();
    let output = tempfile::tempdir().unwrap();
    for name in ["a.jpg", "b.jpg", "c.jpg"] {
        image::RgbImage::new(4, 4)
            .save(folder.path().join(name))
            .unwrap();
    }
    std::fs::write(folder.path().join("a.xmp"), "preserve companion").unwrap();
    std::fs::write(folder.path().join("clip.mov"), "standalone video").unwrap();
    std::fs::write(folder.path().join("notes.txt"), "unmanaged").unwrap();
    let catalog = Catalog::scan(folder.path(), || false, |_| {}).unwrap();
    let mut groups = TestGroups::default();
    for item in &catalog.media {
        groups.record(
            item,
            &day(if item.name() == "c.jpg" {
                "2025-03-16"
            } else {
                "2025-03-15"
            }),
            Some(&[1.]),
        );
    }
    let labels = groups.finish(|| false).unwrap();
    let plan = operations::organize_plan(&catalog, &labels, output.path(), true).unwrap();
    let destinations: Vec<_> = plan
        .actions
        .iter()
        .filter_map(|action| {
            if let Action::Move { files } = action {
                Some(files)
            } else {
                None
            }
        })
        .flatten()
        .map(|(_, destination)| {
            destination
                .strip_prefix(output.path().canonicalize().unwrap())
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
        .collect();
    assert_eq!(destinations.len(), 4);
    for name in [
        "2025-03-15/group-1/a.jpg",
        "2025-03-15/group-1/a.xmp",
        "2025-03-15/group-1/b.jpg",
        "2025-03-16/c.jpg",
    ] {
        assert!(
            destinations
                .iter()
                .any(|path| path.replace('\\', "/") == name),
            "Missing destination {name}"
        );
    }
    assert!(
        folder.path().join("a.jpg").exists(),
        "Preview must not move originals"
    );
    assert!(folder.path().join("clip.mov").exists());
    assert!(folder.path().join("notes.txt").exists());
}

#[test]
fn visual_and_mixed_reanalysis_after_moves_propose_no_second_moves() {
    use photosort::core::{catalog::Catalog, operations, regional::grouping};
    for mixed in [false, true] {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let library = root.join("photos");
        std::fs::create_dir(&library).unwrap();
        for name in ["a.jpg", "b.jpg", "c.jpg", "d.jpg", "e.jpg"] {
            std::fs::write(library.join(name), name).unwrap();
        }
        std::fs::write(library.join("a.xmp"), "companion").unwrap();
        let original = Catalog::scan(&library, || false, |_| {}).unwrap();
        let labels = |catalog: &Catalog, reversed: bool| {
            let mut evidence = TestGroups::default();
            let mut media: Vec<_> = catalog.media.iter().collect();
            if reversed {
                media.reverse();
            }
            for item in media {
                let vector: &[f32] = match item.name().as_str() {
                    "a.jpg" | "b.jpg" => &[1., 0., 0.],
                    "c.jpg" | "d.jpg" => &[0., 1., 0.],
                    _ => &[0., 0., 1.],
                };
                evidence.record(item, &day("2025-03-15"), Some(vector));
            }
            let output =
                grouping::group(&evidence.records, &evidence.store, mixed, 0.055, || false)
                    .unwrap()
                    .unwrap();
            if mixed {
                output.folders
            } else {
                output
                    .groups
                    .into_iter()
                    .map(|(id, n)| (id, format!("Scene {n:03}")))
                    .collect()
            }
        };
        let before = labels(&original, false);
        let plan = operations::organize_plan(&original, &before, &library, true).unwrap();
        assert_eq!(plan.actions.iter().flat_map(|a| a.sources()).count(), 6);
        let report =
            operations::execute(&plan, &root.join("journal"), || false, |_, _| {}).unwrap();
        assert!(report.errors.is_empty(), "{:?}", report.errors);
        let moved = Catalog::scan(&library, || false, |_| {}).unwrap();
        let after = labels(&moved, true);
        for item in &moved.media {
            let old = original
                .media
                .iter()
                .find(|old| old.name() == item.name())
                .unwrap();
            assert_ne!(item.id, old.id, "Moved paths change catalog identities");
            assert_eq!(before[&old.id], after[&item.id]);
            assert_eq!(std::fs::read(&item.path).unwrap(), item.name().as_bytes());
        }
        let second = operations::organize_plan(&moved, &after, &library, true).unwrap();
        assert!(
            second.actions.is_empty(),
            "Reanalysis must not shuffle completed destinations"
        );
        let companion = library
            .join(&after[&moved.media.iter().find(|m| m.name() == "a.jpg").unwrap().id])
            .join("a.xmp");
        assert_eq!(std::fs::read_to_string(companion).unwrap(), "companion");
    }
}
