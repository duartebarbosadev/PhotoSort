//! Opt-in lossless pixel rotation for files that cannot store orientation.
use image::{ImageEncoder, Rgb, RgbImage, codecs::png::PngEncoder};
use photosort::core::{
    catalog::Catalog,
    metadata::{self, WritePolicy, WriteTarget},
    model::{Decision, Fingerprint, MediaId},
    operations::{self, Action, Plan},
    pipeline::Pipeline,
    reencode,
};
use std::{collections::HashMap, path::Path};

/// 3×2 image whose pixels are all different, so any wrong transform is visible.
fn pattern() -> RgbImage {
    RgbImage::from_fn(3, 2, |x, y| Rgb([x as u8 * 80, y as u8 * 120, 7]))
}

fn write_png(path: &Path, icc: Option<&[u8]>) {
    let image = pattern();
    let mut bytes = Vec::new();
    let mut encoder = PngEncoder::new(&mut bytes);
    if let Some(icc) = icc {
        encoder.set_icc_profile(icc.to_vec()).unwrap();
    }
    encoder
        .write_image(image.as_raw(), 3, 2, image::ExtendedColorType::Rgb8)
        .unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn plan(root: &Path, decision: Decision, policy: WritePolicy) -> (Catalog, Plan) {
    let catalog = Catalog::scan(root, || false, |_| {}).unwrap();
    let decisions: HashMap<MediaId, Decision> = catalog
        .media
        .iter()
        .map(|m| (m.id, decision.clone()))
        .collect();
    let mut pipeline = Pipeline::new(16 * 1024 * 1024, root.join("cache"));
    let plan = operations::decisions_plan_with_metadata(
        &catalog,
        &decisions,
        false,
        policy,
        |m| pipeline.metadata(m),
        || false,
    )
    .unwrap();
    (catalog, plan)
}

fn turn() -> Decision {
    Decision {
        turns: 1,
        ..Default::default()
    }
}

#[test]
fn png_pixels_rotate_exactly_and_keep_profile_and_permissions() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("photo.png");
    write_png(&path, Some(b"test-profile"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    reencode::rotate(&path, Fingerprint::read(&path).unwrap(), 6).unwrap();

    let rotated = image::open(&path).unwrap().to_rgb8();
    let expected = image::imageops::rotate90(&pattern());
    assert_eq!(rotated.dimensions(), (2, 3));
    assert_eq!(rotated.as_raw(), expected.as_raw());
    let mut decoder = image::ImageReader::open(&path)
        .unwrap()
        .into_decoder()
        .unwrap();
    use image::ImageDecoder;
    assert_eq!(
        decoder.icc_profile().unwrap().as_deref(),
        Some(&b"test-profile"[..])
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
    }
    assert!(!metadata::sidecar(&path).exists());
}

#[test]
fn tiff_and_bmp_rotate_and_a_changed_file_is_left_alone() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["photo.tiff", "photo.bmp"] {
        let path = temp.path().join(name);
        pattern().save(&path).unwrap();
        reencode::rotate(&path, Fingerprint::read(&path).unwrap(), 3).unwrap();
        let rotated = image::open(&path).unwrap().to_rgb8();
        assert_eq!(
            rotated.as_raw(),
            image::imageops::rotate180(&pattern()).as_raw(),
            "{name}"
        );
    }
    let path = temp.path().join("stale.png");
    write_png(&path, None);
    let stale = Fingerprint::read(&path).unwrap();
    std::thread::sleep(std::time::Duration::from_millis(20));
    RgbImage::new(4, 4).save(&path).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert!(reencode::rotate(&path, stale, 6).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

#[test]
fn multi_page_tiff_is_refused_without_changes() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("pages.tiff");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(file).unwrap();
        for _ in 0..2 {
            encoder
                .write_image::<tiff::encoder::colortype::RGB8>(3, 2, &pattern().into_raw())
                .unwrap();
        }
    }
    let before = std::fs::read(&path).unwrap();
    let error = reencode::rotate(&path, Fingerprint::read(&path).unwrap(), 6).unwrap_err();
    assert!(format!("{error:#}").contains("Multi-page"), "{error:#}");
    assert_eq!(std::fs::read(&path).unwrap(), before);
}

fn target(plan: &Plan) -> WriteTarget {
    match &plan.actions[0] {
        Action::Metadata { target, .. } => *target,
        _ => unreachable!(),
    }
}

#[test]
fn bmp_rotates_pixels_only_when_the_setting_asks_for_it() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("scan.bmp");
    pattern().save(&path).unwrap();
    for policy in [WritePolicy::EmbedWhenSupported, WritePolicy::SidecarOnly] {
        assert_eq!(
            target(&plan(temp.path(), turn(), policy).1),
            WriteTarget::Sidecar
        );
    }
    let (catalog, plan) = plan(temp.path(), turn(), WritePolicy::EmbedOrRotatePixels);
    assert_eq!(target(&plan), WriteTarget::Pixels);
    let report =
        operations::execute(&plan, &temp.path().join("journal"), || false, |_, _| {}).unwrap();
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    // The rotation is now in the pixels: no sidecar, upright metadata, and the
    // decision counts as applied so it is not staged again.
    assert!(!metadata::sidecar(&path).exists());
    assert_eq!(metadata::read(&path).orientation, 1);
    let rotated = image::open(&path).unwrap().to_rgb8();
    assert_eq!(
        rotated.as_raw(),
        image::imageops::rotate90(&pattern()).as_raw()
    );
    assert_eq!(report.applied_decisions(&catalog.media).len(), 1);
    assert_eq!(report.relocated.len(), 1);
}

#[test]
fn sidecar_is_kept_with_a_rating_an_existing_sidecar_or_unsupported_format() {
    let pixels = WritePolicy::EmbedOrRotatePixels;
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("scan.bmp");
    pattern().save(&path).unwrap();
    let rated = Decision {
        turns: 1,
        rating: Some(3),
        ..Default::default()
    };
    assert_eq!(
        target(&plan(temp.path(), rated, pixels).1),
        WriteTarget::Sidecar
    );

    metadata::write_sidecar(&path, Some(2), None).unwrap();
    assert_eq!(
        target(&plan(temp.path(), turn(), pixels).1),
        WriteTarget::Sidecar
    );

    let other = tempfile::tempdir().unwrap();
    pattern().save(other.path().join("photo.gif")).unwrap();
    assert_eq!(
        target(&plan(other.path(), turn(), pixels).1),
        WriteTarget::Sidecar
    );
}
