//! Details panel values come from the shared metadata snapshot and are formatted
//! identically for one photo or a comparison.
use photosort::core::{
    catalog::Catalog,
    details::{self, Section},
    metadata,
    model::{Details, Media, Metadata},
    pipeline::Pipeline,
};
use std::path::Path;

fn fixture(root: &Path, name: &str) -> Media {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/metadata/details.jpg");
    std::fs::copy(source, root.join(name)).unwrap();
    Catalog::scan(root, || false, |_| {})
        .unwrap()
        .media
        .into_iter()
        .find(|m| m.name() == name)
        .unwrap()
}

fn value<'a>(sections: &'a [Section], label: &str) -> &'a [String] {
    &sections
        .iter()
        .flat_map(|s| &s.rows)
        .find(|r| r.label == label)
        .unwrap_or_else(|| panic!("missing row {label}"))
        .values
}

#[test]
fn exif_values_are_read_as_codes_and_formatted_once() {
    let root = tempfile::tempdir().unwrap();
    let media = fixture(root.path(), "photo.jpg");
    let meta = metadata::read(&media.path);
    let d = &meta.details;
    assert_eq!(d.make.as_deref(), Some("Canon"));
    assert_eq!(d.software.as_deref(), Some("PhotoSort Test"));
    assert_eq!(d.focal_length_mm, Some(50.0));
    assert!((d.exposure_bias_ev.unwrap() + 1.0 / 3.0).abs() < 1e-6);
    assert_eq!(
        (d.flash, d.white_balance, d.metering, d.exposure_mode),
        (Some(25), Some(1), Some(5), Some(2))
    );
    assert_eq!(
        (d.scene, d.color_space, d.bit_depth),
        (Some(3), Some(1), Some(8))
    );

    let sections = details::sections(&[(&media, &meta)]);
    for (label, expected) in [
        ("Format", "JPG"),
        ("Source", "Canon EOS R"),
        ("Focal length", "50 mm"),
        ("Flash", "Fired (Auto)"),
        ("White balance", "Manual"),
        ("Metering", "Multi-segment"),
        ("Exposure mode", "Auto bracket"),
        ("Exposure comp.", "-0.3 EV"),
        ("Scene", "Night"),
        ("Color space", "sRGB"),
        ("Bit depth", "8 bit"),
        ("Software", "PhotoSort Test"),
    ] {
        assert_eq!(value(&sections, label), [expected], "{label}");
    }
}

#[test]
fn details_survive_the_persistent_metadata_cache() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = fixture(root.path(), "photo.jpg");
    let first = Pipeline::new(1024 * 1024, cache.path().into())
        .metadata(&media)
        .unwrap();
    let mut reopened = Pipeline::new(1024 * 1024, cache.path().into());
    let second = reopened.metadata(&media).unwrap();
    assert_eq!(reopened.stats.metadata_disk_hits, 1);
    assert_eq!(second.details, first.details);
    assert_ne!(second.details, Details::default());
}

#[test]
fn comparison_tables_align_photos_and_drop_rows_nobody_has() {
    let root = tempfile::tempdir().unwrap();
    let rich = fixture(root.path(), "a.jpg");
    let plain = fixture(root.path(), "screenshot.jpg");
    let rich_meta = metadata::read(&rich.path);
    let plain_meta = Metadata::default();
    let sections = details::sections(&[(&rich, &rich_meta), (&plain, &plain_meta)]);
    assert_eq!(value(&sections, "Flash"), ["Fired (Auto)", ""]);
    assert_eq!(value(&sections, "Source")[1], "Screen capture");
    assert_eq!(value(&sections, "Orientation")[0], "Rotated 90° CW");
    let upright = Metadata {
        orientation: 1,
        ..Default::default()
    };
    assert!(
        details::sections(&[(&plain, &upright), (&plain, &plain_meta)])
            .iter()
            .flat_map(|s| &s.rows)
            .all(|r| r.label != "Orientation"),
        "all-normal orientation is hidden"
    );
    assert!(
        details::sections(&[(&plain, &plain_meta)])
            .iter()
            .flat_map(|s| &s.rows)
            .all(|r| r.label != "Flash")
    );
}

#[test]
fn flash_codes_match_the_reference_table() {
    let render = |code| {
        let meta = Metadata {
            details: Details {
                flash: Some(code),
                ..Default::default()
            },
            ..Default::default()
        };
        let media = Media {
            id: photosort::core::model::MediaId(1),
            path: "x.jpg".into(),
            kind: photosort::core::model::MediaKind::Photo,
            fingerprint: Default::default(),
        };
        value(&details::sections(&[(&media, &meta)]), "Flash")[0].clone()
    };
    for (code, expected) in [
        (0, "Off"),
        (1, "Fired"),
        (9, "Fired (Forced)"),
        (16, "Off"),
        (24, "Off (Auto)"),
        (25, "Fired (Auto)"),
        (32, "No flash function"),
        (65, "Fired, Red-eye"),
        (73, "Fired (Forced), Red-eye"),
        (89, "Fired (Auto), Red-eye"),
    ] {
        assert_eq!(render(code), expected, "flash {code}");
    }
}

#[test]
fn rational_parsing_rejects_zero_denominators_and_garbage() {
    assert_eq!(details::ratio("28/10"), Some(2.8));
    assert_eq!(details::ratio(" 3 "), Some(3.0));
    assert_eq!(
        details::ratio("-1/3").map(|v| (v * 3.0).round()),
        Some(-1.0)
    );
    assert_eq!(details::ratio("1/0"), None);
    assert_eq!(details::ratio("n/a"), None);
    assert_eq!(details::ratio("inf"), None);
}
