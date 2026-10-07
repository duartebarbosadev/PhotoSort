#![cfg(feature = "metadata-native")]
use photosort::core::{metadata, model::*, pipeline::Pipeline};
use std::path::{Path, PathBuf};
fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata")
}
fn check(path: &Path, case: &serde_json::Value) {
    let meta = metadata::read(path);
    assert_eq!(
        meta.metadata_fields,
        case["fields"].as_u64().unwrap() as usize,
        "{}",
        path.display()
    );
    assert_eq!(meta.camera, case["camera"].as_str().unwrap());
    assert_eq!(meta.lens, case["lens"].as_str().unwrap());
    assert_eq!(meta.iso, case["iso"].as_str().unwrap());
    assert_eq!(
        meta.orientation,
        case["orientation"].as_u64().unwrap() as u8
    );
}
#[test]
fn complete_metadata_counts_match_python_including_empty_scalars_and_collections() {
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/metadata/reference.json")).unwrap();
    for case in reference["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["sample"] == false)
    {
        check(&fixtures().join(case["name"].as_str().unwrap()), case);
    }
    let mixed = metadata::read(&fixtures().join("mixed.jpg"));
    assert_eq!(mixed.rating, 4);
    assert_eq!(mixed.captured, "2024:02:03 04:05:06");
    assert_eq!(mixed.grouping_date.as_deref(), Some("2024-02-03"));
    assert!(mixed.exposure.contains("125"));
    assert!(mixed.aperture.contains("2.8"));
    let (lat, lon) = metadata::read(&fixtures().join("gps.jpg")).gps.unwrap();
    assert!((lat + 31.95).abs() < 1e-10);
    assert!((lon - (115. + 52. / 60. + 12. / 3600.)).abs() < 1e-10);
}
#[test]
fn pipeline_reuses_native_metadata_for_browsing_preview_and_sidecar_precedence() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("métadonnées 照片.jpg");
    std::fs::copy(fixtures().join("mixed.jpg"), &path).unwrap();
    let media = Media {
        id: MediaId(1),
        fingerprint: Fingerprint::read(&path).unwrap(),
        path,
        kind: MediaKind::Photo,
    };
    let mut pipeline = Pipeline::new(32 * 1024 * 1024, temp.path().join("cache"));
    let meta = pipeline.metadata(&media).unwrap();
    assert_eq!(meta.metadata_fields, 18);
    assert_eq!(pipeline.load(&media).unwrap().metadata.metadata_fields, 18);
    pipeline.metadata(&media).unwrap();
    pipeline.load(&media).unwrap();
    assert_eq!(
        (pipeline.stats.metadata_reads, pipeline.stats.decodes),
        (1, 1)
    );
    metadata::write_sidecar(&media.path, Some(2), Some(3)).unwrap();
    let updated = pipeline.metadata(&media).unwrap();
    assert_eq!(
        (updated.rating, updated.orientation, updated.metadata_fields),
        (2, 3, 18)
    );
    assert_eq!(pipeline.stats.metadata_reads, 2);
    assert_eq!(
        pipeline.stats.decodes, 1,
        "Metadata reads must not decode pixels"
    );
}
#[test]
fn malformed_or_missing_metadata_has_no_phantom_tags() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("broken.jpg");
    std::fs::write(&path, b"\xff\xd8\xff\xe1\x00\x08Exif\0\0garbage").unwrap();
    let result = metadata::read(&path);
    // Exiv2 recognizes this JPEG container but has no tags: only the five
    // reference file-information entries remain. Unsupported files have none.
    assert_eq!(result.metadata_fields, 5);
    assert_eq!((result.width, result.height), (0, 0));
    std::fs::write(&path, b"not an image").unwrap();
    assert_eq!(metadata::read(&path).metadata_fields, 0);
    assert_eq!(result.orientation, 1);
    assert!(result.camera.is_empty());
    assert_eq!(
        metadata::read(&temp.path().join("missing.jpg")).metadata_fields,
        0
    );
}
#[test]
#[ignore = "requires the seven original samples; read-only metadata parity check"]
fn seven_samples_match_python_metadata_richness_and_preserve_all_bytes() {
    let root =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let reference: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/metadata/reference.json")).unwrap();
    for case in reference["cases"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|c| c["sample"] == true)
    {
        let path = root.join(case["name"].as_str().unwrap());
        let before = std::fs::read(&path).unwrap();
        check(&path, case);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        eprintln!(
            "{}: {} populated metadata fields",
            path.file_name().unwrap().display(),
            case["fields"]
        );
    }
}

#[test]
#[cfg(all(feature = "heif", target_os = "macos"))]
fn heif_container_rotation_and_sidecar_override_survive_native_metadata() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("rotated.heic");
    std::fs::copy(fixtures().join("oriented.heic"), &path).unwrap();
    let metadata = metadata::read(&path);
    assert_eq!(metadata.orientation, 6);
    metadata::write_sidecar(&path, Some(3), Some(8)).unwrap();
    let metadata = metadata::read(&path);
    assert_eq!((metadata.orientation, metadata.rating), (8, 3));
}
