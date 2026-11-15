use crate::test_support;
use chrono::{DateTime, Local};
use photosort::core::{
    browsing::{self, Options, Sort},
    catalog::Catalog,
    dates, metadata,
    pipeline::Pipeline,
};
use std::{cell::Cell, collections::HashMap, fs, path::Path};

fn movie(path: &Path, seconds: u32) {
    let mut bytes = vec![];
    bytes.extend(24_u32.to_be_bytes());
    bytes.extend(b"moov");
    bytes.extend(16_u32.to_be_bytes());
    bytes.extend(b"mvhd");
    bytes.extend([0; 4]);
    bytes.extend((seconds + 2_082_844_800).to_be_bytes());
    fs::write(path, bytes).unwrap();
}
#[test]
fn shared_metadata_drives_video_sort_calendar_and_grouping_without_decoding() {
    let temp = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    // Names intentionally disagree with embedded chronology.
    movie(&temp.path().join("A_20250101_120000.MOV"), 1_700_000_100);
    movie(&temp.path().join("Z_20260101_120000.mp4"), 1_700_000_000);
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    assert_eq!(catalog.media.len(), 2);
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
    let mut snapshots = HashMap::new();
    for item in &catalog.media {
        let expected = if item.name().starts_with('A') {
            1_700_000_100_000
        } else {
            1_700_000_000_000
        };
        let local = DateTime::from_timestamp_millis(expected)
            .unwrap()
            .with_timezone(&Local);
        for _ in 0..3 {
            let meta = pipeline.metadata(item).unwrap();
            assert_eq!(meta.captured_ms, Some(expected));
            assert_eq!(dates::capture_millis(item, Some(&meta)), Some(expected));
            let date = Some(local.format("%Y-%m-%d").to_string());
            assert_eq!(dates::calendar(Some(expected), Some(&meta)), date);
            assert_eq!(dates::label(item, &meta), date);
            assert_eq!(dates::timestamp(&meta.captured), Some(expected));
            snapshots.insert(item.id, meta);
        }
    }
    let mut rows = vec![0, 1];
    browsing::apply(
        &mut rows,
        &catalog.media,
        &snapshots,
        &HashMap::new(),
        &HashMap::new(),
        &Options::default(),
    );
    assert!(catalog.media[rows[0]].name().starts_with('Z'));
    browsing::apply(
        &mut rows,
        &catalog.media,
        &snapshots,
        &HashMap::new(),
        &HashMap::new(),
        &Options {
            sort: Sort::Newest,
            ..Default::default()
        },
    );
    assert!(catalog.media[rows[0]].name().starts_with('A'));
    assert_eq!(pipeline.stats.metadata_reads, 2);
    assert_eq!(pipeline.stats.decodes, 0);
    assert_eq!(pipeline.stats.resident_bytes, 0);
}
#[test]
fn sidecar_dates_override_movie_headers_and_invalidate_one_shared_snapshot() {
    let temp = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let path = temp.path().join("movie.mp4");
    movie(&path, 1_700_000_000);
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let item = &catalog.media[0];
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
    assert_eq!(
        pipeline.metadata(item).unwrap().captured_ms,
        Some(1_700_000_000_000)
    );
    fs::write(metadata::sidecar(&path), r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" xmlns:xmp="http://ns.adobe.com/xap/1.0/"><xmp:CreateDate>2020-01-02T23:30:00-05:00</xmp:CreateDate></x:xmpmeta>"#).unwrap();
    let meta = pipeline.metadata(item).unwrap();
    assert_eq!(meta.captured_ms, dates::timestamp("2020:01:02 23:30:00"));
    assert_eq!(dates::label(item, &meta).as_deref(), Some("2020-01-02"));
    assert_eq!(
        dates::calendar(meta.captured_ms, Some(&meta)).as_deref(),
        Some("2020-01-02")
    );
    assert_eq!(
        pipeline.metadata(item).unwrap().captured_ms,
        meta.captured_ms
    );
    assert_eq!(pipeline.stats.metadata_reads, 2);
    fs::remove_file(metadata::sidecar(&path)).unwrap();
    assert_eq!(
        pipeline.metadata(item).unwrap().captured_ms,
        Some(1_700_000_000_000)
    );
    // The original snapshot is still valid after removing the sidecar.
    assert_eq!(pipeline.stats.metadata_reads, 2);
    movie(&path, 1_700_000_200);
    assert_eq!(
        pipeline.metadata(item).unwrap().captured_ms,
        Some(1_700_000_200_000)
    );
    assert_eq!(pipeline.stats.metadata_reads, 3);
    assert_eq!(pipeline.stats.decodes, 0);
}
#[test]
fn a_video_with_a_written_offset_sits_beside_the_photos_taken_with_it() {
    fn atom(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = (body.len() as u32 + 8).to_be_bytes().to_vec();
        out.extend(kind);
        out.extend(body);
        out
    }
    // The header holds the UTC instant, as cameras write it.
    let header = |written: &str| {
        let utc = DateTime::parse_from_rfc3339(written).unwrap().timestamp() as u32;
        let mut body = vec![0; 4];
        body.extend((utc + 2_082_844_800).to_be_bytes());
        atom(b"mvhd", &body)
    };
    let temp = tempfile::tempdir().unwrap();
    let photo = temp.path().join("photo.jpg");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata/mixed.jpg"),
        &photo,
    )
    .unwrap();
    assert_eq!(metadata::read(&photo).captured, "2024:02:03 04:05:06");
    // An iPhone clip: the creation date item, five minutes after the photo in Tokyo.
    let key = b"com.apple.quicktime.creationdate";
    let mut keys = vec![0; 4];
    keys.extend(1_u32.to_be_bytes());
    keys.extend((key.len() as u32 + 8).to_be_bytes());
    keys.extend(b"mdta");
    keys.extend(key);
    let mut value = vec![0, 0, 0, 1, 0, 0, 0, 0];
    value.extend(b"2024-02-03T04:10:06+0900");
    let mut meta = atom(b"hdlr", &[0; 24]);
    meta.extend(atom(b"keys", &keys));
    meta.extend(atom(
        b"ilst",
        &atom(&1_u32.to_be_bytes(), &atom(b"data", &value)),
    ));
    let mut body = header("2024-02-03T04:10:06+09:00");
    body.extend(atom(b"meta", &meta));
    fs::write(temp.path().join("tokyo.mov"), atom(b"moov", &body)).unwrap();
    // An older camera: the classic date text, ten minutes after, clock on Pacific time.
    let text = b"2024-02-03T04:15:06-08:00";
    let mut day = (text.len() as u16).to_be_bytes().to_vec();
    day.extend([0; 2]);
    day.extend(text);
    let mut body = header("2024-02-03T04:15:06-08:00");
    body.extend(atom(b"udta", &atom(b"\xA9day", &day)));
    fs::write(temp.path().join("pacific.mov"), atom(b"moov", &body)).unwrap();

    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let mut taken = HashMap::new();
    for item in &catalog.media {
        let meta = metadata::read(&item.path);
        let millis = dates::capture_millis(item, Some(&meta));
        // Whatever zone this computer is in, all three share the day they were taken on.
        let day = Some("2024-02-03".to_owned());
        assert_eq!(dates::calendar(millis, Some(&meta)), day, "{}", item.name());
        assert_eq!(dates::label(item, &meta), day, "{}", item.name());
        taken.insert(item.name(), millis.unwrap());
    }
    assert_eq!(taken["tokyo.mov"] - taken["photo.jpg"], 5 * 60 * 1000);
    assert_eq!(taken["pacific.mov"] - taken["photo.jpg"], 10 * 60 * 1000);
}
#[test]
fn cancelled_video_metadata_is_not_cached_and_retry_succeeds() {
    let temp = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let path = temp.path().join("movie.mp4");
    movie(&path, 1_700_000_000);
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let item = &catalog.media[0];
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
    let checks = Cell::new(0);
    assert!(
        pipeline
            .metadata_cancellable(item, &|| {
                checks.set(checks.get() + 1);
                checks.get() >= 5
            })
            .is_err()
    );
    assert_eq!(pipeline.stats.metadata_reads, 1);
    assert_eq!(
        pipeline.metadata(item).unwrap().captured_ms,
        Some(1_700_000_000_000)
    );
    assert_eq!(pipeline.stats.metadata_reads, 2);
    assert!(pipeline.metadata_cancellable(item, &|| true).is_err());
    assert_eq!(pipeline.stats.metadata_reads, 2);
    assert_eq!(pipeline.stats.decodes, 0);
}
#[test]
fn corrupt_movie_keeps_filename_fallback_and_recording_time_is_not_invented() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("movie_2023-12-25.mp4");
    fs::write(&path, b"broken movie").unwrap();
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let meta = metadata::read(&path);
    assert!(meta.captured_ms.is_none());
    assert!(meta.captured.is_empty());
    assert_eq!(
        dates::capture_millis(&catalog.media[0], Some(&meta)),
        dates::timestamp("2023-12-25")
    );
}
#[test]
#[ignore = "Set PHOTOSORT_VIDEO_TIME_SAMPLE to a disposable ffmpeg clip with creation_time 2023-11-14T22:13:20Z"]
fn real_encoded_movie_recording_time() {
    let path = std::env::var_os("PHOTOSORT_VIDEO_TIME_SAMPLE").expect("fixture path");
    let meta = metadata::read(Path::new(&path));
    assert_eq!(meta.captured_ms, Some(1_700_000_000_000));
    assert_eq!(dates::timestamp(&meta.captured), Some(1_700_000_000_000));
}

#[test]
fn video_metadata_index_reuses_one_owner_across_repeated_requests() {
    use photosort::core::jobs::{Event, Runtime};
    use std::{sync::Arc, time::Instant};
    let temp = tempfile::tempdir().unwrap();
    for i in 0..65 {
        movie(&temp.path().join(format!("{i:03}.mp4")), 1_700_000_000 + i);
    }
    let catalog = Catalog::scan(temp.path(), || false, |_| {}).unwrap();
    let items = Arc::new(catalog.media.clone());
    let runtime = Runtime::new(|| {});
    for _ in 0..2 {
        let run = runtime.index_metadata(items.clone()).unwrap();
        let deadline = Instant::now() + test_support::worker_patience();
        let mut found = 0;
        loop {
            assert!(Instant::now() < deadline);
            let envelope = runtime
                .events
                .recv_timeout(test_support::worker_patience())
                .unwrap();
            if let Event::MetadataBatch {
                run: current,
                items: batch,
                done,
                total,
                stats,
            } = envelope.event
            {
                assert_eq!(current, run);
                for (_, result) in batch {
                    assert!(result.unwrap().captured_ms.is_some());
                    found += 1;
                }
                assert_eq!(stats.decodes, 0);
                if done == total {
                    assert_eq!(found, 65);
                    assert_eq!(stats.metadata_reads, 65);
                    break;
                }
            } else {
                panic!("Metadata indexing must not initiate other image work");
            }
        }
    }
}

#[test]
fn filename_date_fallback_retains_supported_reference_patterns_and_validation() {
    for (name, date) in [
        ("IMG_20231225_143045.jpg", Some("2023-12-25")),
        ("2023-12-25_photo.jpg", Some("2023-12-25")),
        ("20231225_143045.jpg", Some("2023-12-25")),
        ("photo_2023.12.25.jpg", Some("2023-12-25")),
        ("random_filename.jpg", None),
        ("IMG_20991301_invalid.jpg", None),
        ("photo_2023-02-29.jpg", None),
        ("写真_2024-02-29.mov", Some("2024-02-29")),
    ] {
        assert_eq!(dates::filename(name, 2026).as_deref(), date, "{name}");
    }
}

#[test]
fn real_video_source_geometry_is_cached_without_frames_or_playback() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/video");
    let catalog = Catalog::scan(&root, || false, |_| {}).unwrap();
    let cache = tempfile::tempdir().unwrap();
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().into());
    for pass in 0..2 {
        for (name, expected) in [
            ("motion.mp4", (320, 180)),
            ("rotated.mp4", (180, 320)),
            ("large.mp4", (3840, 2160)),
        ] {
            let item = catalog.media.iter().find(|m| m.name() == name).unwrap();
            for _ in 0..3 {
                let meta = pipeline.metadata(item).unwrap();
                assert_eq!(
                    (meta.width, meta.height),
                    expected,
                    "{name}, restart {pass}"
                );
            }
        }
        assert_eq!(pipeline.stats.decodes, 0);
        assert_eq!(pipeline.stats.resident_bytes, 0);
        if pass == 0 {
            assert_eq!(pipeline.stats.metadata_reads, 3);
            pipeline = Pipeline::new(1024 * 1024, cache.path().into());
        } else {
            assert_eq!(pipeline.stats.metadata_reads, 0);
            assert_eq!(pipeline.stats.metadata_disk_hits, 3);
        }
    }
}
