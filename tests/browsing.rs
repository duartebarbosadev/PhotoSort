use photosort::core::{
    browsing::{self, Options, RatingFilter, Sort},
    catalog::Catalog,
    dates,
    jobs::{Event, ImageWork, Runtime},
    metadata,
    model::*,
    pipeline::Pipeline,
};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
fn media(id: u64, name: &str) -> Media {
    Media {
        id: MediaId(id),
        path: format!("/unavailable/{name}").into(),
        kind: MediaKind::Photo,
        fingerprint: Fingerprint::default(),
    }
}
#[test]
fn exact_and_minimum_ratings_use_staged_values_and_do_not_treat_unknown_as_zero() {
    for filter in RatingFilter::ALL {
        for n in 0..=5 {
            assert_eq!(
                filter.matches(Some(n)),
                match filter {
                    RatingFilter::All => true,
                    RatingFilter::Exactly(v) => n == v,
                    RatingFilter::AtLeast(v) => n >= v,
                }
            );
        }
        assert_eq!(filter.matches(None), filter == RatingFilter::All);
    }
    let items = vec![
        media(0, "a.jpg"),
        media(1, "b.jpg"),
        media(2, "c.jpg"),
        media(3, "d.jpg"),
    ];
    let meta = HashMap::from([
        (
            MediaId(0),
            Metadata {
                rating: 5,
                ..Default::default()
            },
        ),
        (
            MediaId(1),
            Metadata {
                rating: 3,
                ..Default::default()
            },
        ),
    ]);
    let staged = HashMap::from([
        (
            MediaId(0),
            Decision {
                rating: Some(0),
                ..Default::default()
            },
        ),
        (
            MediaId(2),
            Decision {
                rating: Some(3),
                ..Default::default()
            },
        ),
    ]);
    let groups = HashMap::from([(MediaId(1), 4), (MediaId(2), 7)]);
    let mut rows = vec![0, 1, 2, 3];
    browsing::apply(
        &mut rows,
        &items,
        &meta,
        &staged,
        &groups,
        &Options {
            rating: RatingFilter::Exactly(0),
            ..Default::default()
        },
    );
    assert_eq!(rows, vec![0]);
    rows = vec![0, 1, 2, 3];
    browsing::apply(
        &mut rows,
        &items,
        &meta,
        &staged,
        &groups,
        &Options {
            rating: RatingFilter::AtLeast(3),
            group: Some(7),
            ..Default::default()
        },
    );
    assert_eq!(rows, vec![2]);
    assert_eq!(meta[&MediaId(0)].rating, 5);
}
#[test]
fn chronological_sort_preserves_time_offsets_ties_and_unknown_last_without_file_access() {
    let mut items = vec![
        media(0, "z.jpg"),
        media(1, "B.jpg"),
        media(2, "a.jpg"),
        media(3, "unknown.jpg"),
    ];
    items[0].fingerprint.modified_ns = 1;
    let meta = HashMap::from([
        (
            MediaId(0),
            Metadata {
                captured: "2026-09-27T12:00:00+08:00".into(),
                ..Default::default()
            },
        ),
        (
            MediaId(1),
            Metadata {
                captured: "2026-09-27T03:00:00Z".into(),
                ..Default::default()
            },
        ),
        (
            MediaId(2),
            Metadata {
                captured: "2026-09-27T03:00:00Z".into(),
                ..Default::default()
            },
        ),
    ]);
    let mut rows = vec![0, 1, 2, 3];
    browsing::apply(
        &mut rows,
        &items,
        &meta,
        &HashMap::new(),
        &HashMap::new(),
        &Options::default(),
    );
    assert_eq!(rows, vec![2, 1, 0, 3]);
    browsing::apply(
        &mut rows,
        &items,
        &meta,
        &HashMap::new(),
        &HashMap::new(),
        &Options {
            sort: Sort::Newest,
            ..Default::default()
        },
    );
    assert_eq!(rows, vec![0, 2, 1, 3]);
    let m = media(4, "x.jpg");
    let meta = Metadata {
        created_ms: Some(1000),
        ..Default::default()
    };
    assert_eq!(dates::capture_millis(&m, Some(&meta)), Some(1000));
    assert_eq!(
        dates::timestamp("2026-09-27T12:00:00+08:00"),
        dates::timestamp("2026-09-27T04:00:00Z")
    );
}
#[test]
fn metadata_only_loading_reuses_the_image_pipeline_and_invalidates_changed_sidecars() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("photo.png");
    image::RgbImage::new(32, 24).save(&source).unwrap();
    metadata::write_sidecar(&source, Some(4), None).unwrap();
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let m = &catalog.media[0];
    let cache = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
    assert_eq!(pipeline.metadata(m).unwrap().rating, 4);
    assert_eq!(pipeline.stats.decodes, 0);
    assert_eq!(pipeline.stats.resident_bytes, 0);
    assert_eq!(pipeline.metadata(m).unwrap().rating, 4);
    let asset = pipeline.load(m).unwrap();
    assert_eq!(asset.metadata.rating, 4);
    assert_eq!(pipeline.stats.metadata_reads, 1);
    assert_eq!(pipeline.stats.decodes, 1);
    let before = pipeline.metadata(m).unwrap();
    metadata::write_sidecar(&source, Some(2), Some(6)).unwrap();
    let after = pipeline.metadata(m).unwrap();
    assert_eq!(before.rating, 4);
    assert_eq!(after.rating, 2);
    assert_eq!(after.orientation, 6);
    assert_eq!(pipeline.stats.metadata_reads, 2);
    assert_eq!(pipeline.stats.decodes, 1);
}
#[test]
fn xmp_capture_time_preserves_hours_and_offsets_for_both_attributes_and_elements() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("undecodable.jpg");
    std::fs::write(&path, b"headers only fixture").unwrap();
    for body in [
        r#"<rdf:Description xmp:CreateDate="2026-09-27T12:34:56+08:00"/>"#,
        r#"<rdf:Description><xmp:CreateDate>2026-09-27T12:34:56+08:00</xmp:CreateDate></rdf:Description>"#,
    ] {
        let xml = format!(
            r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:xmp="http://ns.adobe.com/xap/1.0/">{body}</rdf:RDF>"#
        );
        std::fs::write(path.with_extension("xmp"), xml).unwrap();
        let meta = metadata::read(&path);
        assert_eq!(meta.grouping_date.as_deref(), Some("2026-09-27"));
        assert_eq!(meta.captured_ms, dates::timestamp("2026-09-27T04:34:56Z"));
    }
}
#[test]
fn metadata_worker_batches_reuses_results_and_cancels_independently_of_rendering() {
    let temp = tempfile::tempdir().unwrap();
    for n in 0..257 {
        std::fs::write(
            temp.path().join(format!("{n:04}.jpg")),
            b"invalid pixels, valid catalog entry",
        )
        .unwrap();
    }
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let items = Arc::new(catalog.media);
    let cache = tempfile::tempdir().unwrap();
    let runtime = Runtime::with_cache(|| {}, cache.path().into());
    for pass in 0..2 {
        let run = runtime.index_metadata(items.clone()).unwrap();
        let mut counts = vec![];
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            assert!(Instant::now() < deadline);
            let e = runtime.events.recv_timeout(Duration::from_secs(2)).unwrap();
            if let Event::MetadataBatch {
                run: r,
                items: batch,
                done,
                total,
                stats,
            } = e.event
            {
                assert_eq!(r, run);
                assert!(batch.iter().all(|(_, r)| r.is_ok()));
                counts.push(batch.len());
                assert_eq!(stats.decodes, 0);
                if done == total {
                    assert_eq!(stats.metadata_reads, 257, "pass={pass}");
                    break;
                }
            } else {
                panic!("unexpected work during metadata-only indexing");
            }
        }
        assert_eq!(counts, vec![64, 64, 64, 64, 1]);
    }
    // Stop a backpressured large index and supersede it with a new folder epoch.
    let many = Arc::new(items.iter().cycle().take(10000).cloned().collect());
    let old = runtime.index_metadata(many).unwrap();
    loop {
        if let Event::MetadataBatch { run, .. } = runtime
            .events
            .recv_timeout(Duration::from_secs(2))
            .unwrap()
            .event
            && run == old
        {
            break;
        }
    }
    runtime.cancel_metadata();
    let epoch = runtime.next_epoch();
    let new = runtime.index_metadata(Arc::new(vec![])).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline);
        let e = runtime.events.recv_timeout(Duration::from_secs(1)).unwrap();
        if let Event::MetadataBatch {
            run, done, total, ..
        } = e.event
            && run == new
        {
            assert_eq!(e.epoch, epoch);
            assert_eq!((done, total), (0, 0));
            break;
        }
    }
    // A failed image request still gets serviced; cancelled indexing does not own the worker.
    runtime
        .image(ImageWork::Asset(items[0].clone(), true))
        .unwrap();
    loop {
        let e = runtime.events.recv_timeout(Duration::from_secs(2)).unwrap();
        if let Event::AssetError { id, .. } = e.event {
            assert_eq!(id, items[0].id);
            break;
        }
    }
}
