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

/// EXIF as a scanner might write it: the 3x2 size and Orientation 6 in IFD0, and
/// a capture date and the size again in the Exif directory (widths SHORT,
/// heights LONG).
fn exif() -> Vec<u8> {
    let mut exif = b"II*\0\x08\0\0\0".to_vec();
    let entry = |exif: &mut Vec<u8>, tag: u16, kind: u16, count: u32, value: u32| {
        exif.extend(tag.to_le_bytes());
        exif.extend(kind.to_le_bytes());
        exif.extend(count.to_le_bytes());
        exif.extend(value.to_le_bytes());
    };
    exif.extend(4u16.to_le_bytes());
    entry(&mut exif, 0x0100, 3, 1, 3);
    entry(&mut exif, 0x0101, 4, 1, 2);
    entry(&mut exif, 0x0112, 3, 1, 6);
    entry(&mut exif, 0x8769, 4, 1, 62);
    exif.extend(0u32.to_le_bytes());
    exif.extend(3u16.to_le_bytes());
    entry(&mut exif, 0x9003, 2, 20, 104);
    entry(&mut exif, 0xA002, 3, 1, 3);
    entry(&mut exif, 0xA003, 4, 1, 2);
    exif.extend(0u32.to_le_bytes());
    exif.extend(b"2019:07:14 10:11:12\0");
    exif
}

#[test]
fn png_rotation_keeps_the_capture_date_with_upright_exif() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("scan.png");
    let image = pattern();
    let mut bytes = Vec::new();
    let mut encoder = PngEncoder::new(&mut bytes);
    encoder.set_exif_metadata(exif()).unwrap();
    encoder
        .write_image(image.as_raw(), 3, 2, image::ExtendedColorType::Rgb8)
        .unwrap();
    std::fs::write(&path, bytes).unwrap();
    assert_eq!(metadata::read(&path).orientation, 6);
    reencode::rotate(&path, Fingerprint::read(&path).unwrap(), 6).unwrap();

    let meta = metadata::read(&path);
    assert_eq!(meta.grouping_date.as_deref(), Some("2019-07-14"));
    assert_eq!(meta.orientation, 1, "the turn is in the pixels now");
    let rotated = image::open(&path).unwrap().to_rgb8();
    assert_eq!(rotated.as_raw(), image::imageops::rotate90(&image).as_raw());
    let mut decoder = image::codecs::png::PngDecoder::new(std::io::BufReader::new(
        std::fs::File::open(&path).unwrap(),
    ))
    .unwrap();
    let raw = image::ImageDecoder::exif_metadata(&mut decoder)
        .unwrap()
        .unwrap();
    let exif = exif::Reader::new().read_raw(raw).unwrap();
    let size = |tag| {
        let field = exif.get_field(tag, exif::In::PRIMARY).unwrap();
        field.value.get_uint(0).unwrap()
    };
    for (width, height) in [
        (exif::Tag::ImageWidth, exif::Tag::ImageLength),
        (exif::Tag::PixelXDimension, exif::Tag::PixelYDimension),
    ] {
        assert_eq!(
            (size(width), size(height)),
            (2, 3),
            "the EXIF pixel size turns with the pixels"
        );
    }
}

/// The PNG's chunks after the signature, as (type, data).
fn png_chunks(png: &[u8]) -> Vec<([u8; 4], Vec<u8>)> {
    let mut chunks = Vec::new();
    let mut at = 8;
    while at < png.len() {
        let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let kind = png[at + 4..at + 8].try_into().unwrap();
        chunks.push((kind, png[at + 8..at + 8 + length].to_vec()));
        at += 12 + length;
    }
    chunks
}

fn png_from_chunks(chunks: &[([u8; 4], Vec<u8>)]) -> Vec<u8> {
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in chunks {
        png.extend((data.len() as u32).to_be_bytes());
        let mut crc = !0u32;
        for byte in kind.iter().chain(data) {
            crc ^= u32::from(*byte);
            for _ in 0..8 {
                crc = (crc >> 1) ^ (0xEDB8_8320 & (crc & 1).wrapping_neg());
            }
        }
        png.extend(kind);
        png.extend(data);
        png.extend((!crc).to_be_bytes());
    }
    png
}

#[test]
fn png_rotation_rewrites_compressed_xmp_under_any_tiff_prefix() {
    // Both forms, spaced out and under a prefix other than "tiff".
    let xmp = r#"<?xpacket begin="" id="W5M0MpCehiHzreSzNTczkc9d"?>
<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
<rdf:Description rdf:about="" xmlns:t="http://ns.adobe.com/tiff/1.0/" t:Orientation = "6"
 xmlns:exif="http://ns.adobe.com/exif/1.0/" exif:PixelXDimension="3" exif:PixelYDimension="2"/>
<rdf:Description rdf:about="" xmlns:t="http://ns.adobe.com/tiff/1.0/"
 xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:CreatorTool="Scanner X"><t:Orientation >6</t:Orientation >
 <t:ImageWidth>3</t:ImageWidth><t:ImageLength>2</t:ImageLength></rdf:Description>
</rdf:RDF></x:xmpmeta>
<?xpacket end="w"?>"#;
    assert_eq!(metadata::read_xmp(xmp).unwrap().1, Some(6));
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("scan.png");
    write_png(&path, None);
    let mut chunks = png_chunks(&std::fs::read(&path).unwrap());
    let header = b"XML:com.adobe.xmp\0\x01\0\0\0";
    let compressed = miniz_oxide::deflate::compress_to_vec_zlib(xmp.as_bytes(), 6);
    chunks.insert(1, (*b"iTXt", [&header[..], &compressed].concat()));
    std::fs::write(&path, png_from_chunks(&chunks)).unwrap();

    reencode::rotate(&path, Fingerprint::read(&path).unwrap(), 6).unwrap();

    let chunks = png_chunks(&std::fs::read(&path).unwrap());
    let (_, data) = chunks
        .iter()
        .find(|(kind, data)| kind == b"iTXt" && data.starts_with(b"XML:com.adobe.xmp\0"))
        .expect("the XMP is kept");
    assert!(data.starts_with(header), "and stays compressed");
    let inflated = miniz_oxide::inflate::decompress_to_vec_zlib(&data[header.len()..]).unwrap();
    let xmp = String::from_utf8(inflated).unwrap();
    assert_eq!(metadata::read_xmp(&xmp).unwrap().1, Some(1));
    assert!(!xmp.contains("Orientation>6") && !xmp.contains("\"6\""));
    assert!(xmp.contains(r#"xmp:CreatorTool="Scanner X""#));
    // The XMP copies of the pixel size turn too.
    assert!(
        xmp.contains(r#"exif:PixelXDimension="2""#) && xmp.contains(r#"exif:PixelYDimension="3""#)
    );
    assert!(
        xmp.contains("<t:ImageWidth>2</t:ImageWidth>")
            && xmp.contains("<t:ImageLength>3</t:ImageLength>")
    );
}

#[test]
fn tiff_rotation_keeps_exif_dates_camera_and_compression() {
    use tiff::{encoder::colortype::RGB8, tags::Tag};
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("scan.tif");
    {
        let file = std::fs::File::create(&path).unwrap();
        let mut encoder = tiff::encoder::TiffEncoder::new(file)
            .unwrap()
            .with_compression(tiff::encoder::Compression::Lzw);
        let mut exif = encoder.extra_directory().unwrap();
        exif.write_tag(Tag::Unknown(0x9003), "2019:07:14 10:11:12")
            .unwrap();
        let exif = exif.finish_with_offsets().unwrap().offset;
        let mut image = encoder.new_image::<RGB8>(3, 2).unwrap();
        let tags = image.encoder();
        tags.write_tag(Tag::Unknown(272), "Scanner X").unwrap();
        tags.write_tag(Tag::Unknown(274), 6u16).unwrap();
        tags.write_tag(Tag::ExifDirectory, exif).unwrap();
        image.write_data(pattern().as_raw()).unwrap();
    }
    assert_eq!(metadata::read(&path).orientation, 6);
    reencode::rotate(&path, Fingerprint::read(&path).unwrap(), 6).unwrap();

    let meta = metadata::read(&path);
    assert_eq!(meta.grouping_date.as_deref(), Some("2019-07-14"));
    assert_eq!(meta.camera, "Scanner X");
    assert_eq!(meta.orientation, 1, "the turn is in the pixels now");
    let rotated = image::open(&path).unwrap().to_rgb8();
    assert_eq!(
        rotated.as_raw(),
        image::imageops::rotate90(&pattern()).as_raw()
    );
    let mut decoder = tiff::decoder::Decoder::new(std::fs::File::open(&path).unwrap()).unwrap();
    assert_eq!(
        decoder.get_tag_unsigned::<u16>(Tag::Compression).unwrap(),
        5,
        "LZW stays LZW"
    );
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
