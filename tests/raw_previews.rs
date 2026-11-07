//! Native RAW preview/detail contracts. Only independent copies receive sidecars or corruptions.
#![cfg(feature = "raw")]
use photosort::core::{
    catalog::Catalog,
    inspection::{Region, Request},
    metadata,
    model::*,
    operations,
    pipeline::Pipeline,
};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

#[test]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR/arw_sample.ARW"]
fn shared_raw_preview_geometry_recipes_reuse_and_fallbacks() -> anyhow::Result<()> {
    let source =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap()).join("arw_sample.ARW");
    let original_hash = operations::content_hash(&source, || false)?;
    let temp = tempfile::tempdir()?;
    let normal = temp.path().join("normal.ARW");
    std::fs::copy(&source, &normal)?;
    let normal = normal.canonicalize()?;
    let bytes = std::fs::read(&normal)?;
    assert_eq!(&bytes[..4], b"II*\0");
    let u16_at = |at: usize| u16::from_le_bytes(bytes[at..at + 2].try_into().unwrap());
    let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
    let mut missing = bytes.clone();
    let mut corrupt = bytes.clone();
    let mut ifd = u32_at(4) as usize;
    let mut previews = 0;
    // TIFF's top-level IFD chain contains both this fixture's embedded JPEGs.
    while ifd != 0 {
        let count = u16_at(ifd) as usize;
        let mut jpeg = None;
        let mut length = None;
        for n in 0..count {
            let at = ifd + 2 + n * 12;
            match u16_at(at) {
                0x201 => {
                    jpeg = Some(u32_at(at + 8) as usize);
                    missing[at + 8..at + 12].fill(0);
                }
                0x202 => {
                    length = Some(u32_at(at + 8) as usize);
                    missing[at + 8..at + 12].fill(0);
                }
                _ => {}
            }
        }
        if let (Some(start), Some(length)) = (jpeg, length) {
            assert!(length > 2 && start + length <= corrupt.len());
            corrupt[start..start + length].fill(0);
            previews += 1;
        }
        ifd = u32_at(ifd + 2 + count * 12) as usize;
    }
    assert_eq!(previews, 2, "Fixture changed; inspect its TIFF structure");
    std::fs::write(temp.path().join("missing.ARW"), missing)?;
    std::fs::write(temp.path().join("corrupt.ARW"), corrupt)?;
    let media = Catalog::scan(temp.path(), || false, |_| {})?.media;
    for photo in &media {
        let fallback = photo.path != normal;
        for orientation in [1, 2, 6, 8] {
            metadata::write_sidecar(&photo.path, Some(4), Some(orientation))?;
            for display_first in [false, true] {
                let cache = tempfile::tempdir()?;
                let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
                if display_first {
                    pipeline.load(photo)?;
                }
                let analysis = pipeline.analysis(photo, || false)?;
                let display = pipeline.load(photo)?;
                assert_eq!(
                    (display.metadata.width, display.metadata.height),
                    (6024, 4024)
                );
                assert_eq!(display.metadata.orientation, orientation);
                assert_eq!(display.metadata.rating, 4);
                assert!(display.pixels.width().max(display.pixels.height()) <= 2048);
                assert!(analysis.pixels.width().max(analysis.pixels.height()) <= 1024);
                assert_eq!(
                    (pipeline.stats.decodes, pipeline.stats.metadata_reads),
                    (1, 1)
                );
                assert_eq!(
                    (pipeline.stats.raw_embedded, pipeline.stats.raw_fallback),
                    if fallback { (0, 1) } else { (1, 0) },
                    "{}",
                    photo.name()
                );
                for _ in Step::ALL {
                    assert!(Arc::ptr_eq(&display.pixels, &pipeline.load(photo)?.pixels));
                    assert!(Arc::ptr_eq(
                        &analysis.pixels,
                        &pipeline.analysis(photo, || false)?.pixels
                    ));
                }
                let mut reopened = Pipeline::new(128 * 1024 * 1024, cache.path().into());
                assert_eq!(reopened.load(photo)?.pixels, display.pixels);
                assert_eq!(reopened.analysis(photo, || false)?.pixels, analysis.pixels);
                assert_eq!((reopened.stats.decodes, reopened.stats.disk_hits), (0, 2));
                let (width, height) = if orientation >= 5 {
                    (4024, 6024)
                } else {
                    (6024, 4024)
                };
                let request = Request {
                    slot: 0,
                    media: photo.clone(),
                    region: Region {
                        x: width - 32,
                        y: height - 32,
                        width: 32,
                        height: 32,
                    },
                    output_edge: 32,
                    allow_large: false,
                };
                let detail = pipeline.inspect(&request)?;
                assert_eq!(detail.dimensions(), (32, 32));
                assert_eq!(
                    pipeline.stats.decodes, 2,
                    "Full RAW rendering is deferred until inspection"
                );
                assert_eq!(pipeline.inspect(&request)?, detail);
                assert_eq!(pipeline.stats.decodes, 2);
                assert_eq!(
                    pipeline.analysis(photo, || false)?.pixels,
                    analysis.pixels,
                    "Inspecting full RAW must not change the analysis recipe"
                );
                assert_eq!(
                    (
                        pipeline.metadata(photo)?.width,
                        pipeline.metadata(photo)?.height
                    ),
                    (6024, 4024)
                );
            }
        }
    }
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let photo = media.iter().find(|m| m.path == normal).unwrap();
    let polls = AtomicUsize::new(0);
    assert!(
        pipeline
            .load_cancellable(photo, || polls.fetch_add(1, Ordering::Relaxed) >= 6)
            .is_err()
    );
    assert_eq!(pipeline.stats.resident_bytes, 0);
    assert_eq!(pipeline.stats.detail_bytes, 0);
    pipeline.load(photo)?;
    let before = pipeline.stats.decodes;
    metadata::write_sidecar(&photo.path, Some(2), Some(3))?;
    let replaced = pipeline.load(photo)?;
    assert_eq!(replaced.metadata.orientation, 3);
    assert_eq!(replaced.metadata.rating, 2);
    assert_eq!(
        pipeline.stats.decodes,
        before + 1,
        "Sidecar change must reject old prepared pixels"
    );
    assert_eq!(operations::content_hash(&source, || false)?, original_hash);
    Ok(())
}

#[test]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR/arw_sample.ARW"]
fn raw_analysis_only_geometry_survives_metadata_only_restart() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").unwrap());
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    let photo = catalog
        .media
        .iter()
        .find(|m| m.name() == "arw_sample.ARW")
        .unwrap();
    let cache = tempfile::tempdir()?;
    let mut first = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let analysis = first.analysis(photo, || false)?;
    assert_eq!(
        (analysis.metadata.width, analysis.metadata.height),
        (6024, 4024)
    );
    drop(first);
    let mut reopened = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let metadata = reopened.metadata(photo)?;
    assert_eq!(
        serde_json::to_value(metadata)?,
        serde_json::to_value(analysis.metadata)?,
        "Metadata-only cache hits must retain decoded RAW geometry"
    );
    assert_eq!(
        (
            reopened.stats.decodes,
            reopened.stats.metadata_reads,
            reopened.stats.metadata_disk_hits
        ),
        (0, 0, 1)
    );
    assert_eq!(
        (reopened.stats.resident_bytes, reopened.stats.detail_bytes),
        (0, 0)
    );
    Ok(())
}

#[test]
fn rotation_input_ignores_the_raw_display_look() -> anyhow::Result<()> {
    use photosort::core::raw_look::RawLook;
    let temp = tempfile::tempdir()?;
    let library = temp.path().join("library");
    std::fs::create_dir(&library)?;
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/samples/arw_sample.ARW"),
        library.join("photo.ARW"),
    )?;
    let catalog = Catalog::scan(&library, || false, |_| {})?;
    let media = catalog.media.first().expect("one RAW photo");
    let mut neutral = Pipeline::new(256 * 1024 * 1024, temp.path().join("neutral-cache"));
    let expected = neutral.load(media)?.pixels;
    let mut enhanced = Pipeline::new(256 * 1024 * 1024, temp.path().join("enhanced-cache"));
    enhanced.set_raw_look(RawLook::Enhanced);
    // Both orders: the neutral entry must never be the enhanced display asset.
    let before = enhanced.load_neutral_cancellable(media, || false)?.pixels;
    let display = enhanced.load(media)?.pixels;
    let after = enhanced.load_neutral_cancellable(media, || false)?.pixels;
    assert_eq!(*before, *expected);
    assert_eq!(*after, *expected);
    assert_ne!(
        *display, *expected,
        "the Enhanced look should change display pixels"
    );
    Ok(())
}

/// LibRaw's processed image header (`libraw_processed_image_t`).
#[repr(C)]
struct RawImage {
    kind: std::ffi::c_int,
    height: u16,
    width: u16,
    colors: u16,
    bits: u16,
    size: u32,
    data: [u8; 1],
}
unsafe extern "C" {
    fn photosort_raw_decode(
        path: *const std::ffi::c_char,
        recipe: std::ffi::c_int,
        allowance: u64,
        needed: *mut u64,
        embedded: *mut std::ffi::c_int,
        source_width: *mut u32,
        source_height: *mut u32,
        progress: unsafe extern "C" fn(
            *mut std::ffi::c_void,
            std::ffi::c_int,
            std::ffi::c_int,
            std::ffi::c_int,
        ) -> std::ffi::c_int,
        context: *mut std::ffi::c_void,
        error: *mut std::ffi::c_int,
    ) -> *mut RawImage;
    fn libraw_dcraw_clear_mem(image: *mut RawImage);
}
unsafe extern "C" fn keep_going(
    _: *mut std::ffi::c_void,
    _: std::ffi::c_int,
    _: std::ffi::c_int,
    _: std::ffi::c_int,
) -> std::ffi::c_int {
    0
}
/// (image kind, embedded, source size, LibRaw error, bytes needed) for one
/// native decode allowed `allowance` bytes.
fn native_raw(
    path: &std::path::Path,
    recipe: i32,
    allowance: u64,
) -> (Option<i32>, i32, (u32, u32), i32, u64) {
    let path = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap();
    let (mut embedded, mut width, mut height, mut error, mut needed) = (0, 0, 0, 0, 0);
    // SAFETY: every out-pointer lives through the call; the image is freed here.
    unsafe {
        let image = photosort_raw_decode(
            path.as_ptr(),
            recipe,
            allowance,
            &mut needed,
            &mut embedded,
            &mut width,
            &mut height,
            keep_going,
            std::ptr::null_mut(),
            &mut error,
        );
        let kind = (!image.is_null()).then(|| (*image).kind);
        if !image.is_null() {
            libraw_dcraw_clear_mem(image);
        }
        (kind, embedded, (width, height), error, needed)
    }
}

/// The public ARW sample with its sensor directory claiming `width` x `height`.
fn patched_arw(width: u16, height: u16) -> (tempfile::TempDir, PathBuf) {
    let mut bytes = std::fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/samples/arw_sample.ARW"),
    )
    .unwrap();
    let u16_at = |b: &[u8], at: usize| u16::from_le_bytes(b[at..at + 2].try_into().unwrap());
    let u32_at = |b: &[u8], at: usize| u32::from_le_bytes(b[at..at + 4].try_into().unwrap());
    let ifd0 = u32_at(&bytes, 4) as usize;
    let sensor = (0..usize::from(u16_at(&bytes, ifd0)))
        .map(|n| ifd0 + 2 + n * 12)
        .find(|&at| u16_at(&bytes, at) == 0x14A)
        .map(|at| u32_at(&bytes, at + 8) as usize)
        .unwrap();
    for n in 0..usize::from(u16_at(&bytes, sensor)) {
        let at = sensor + 2 + n * 12;
        match u16_at(&bytes, at) {
            0x100 => bytes[at + 8..at + 10].copy_from_slice(&width.to_le_bytes()),
            0x101 => bytes[at + 8..at + 10].copy_from_slice(&height.to_le_bytes()),
            _ => {}
        }
    }
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("large.ARW");
    std::fs::write(&path, bytes).unwrap();
    (temp, path)
}
#[test]
fn a_sensor_past_100_megapixels_previews_from_its_jpeg_and_decodes_when_memory_allows() {
    // The sample's sensor directory claims 12000x9000 (108 MP), like a GFX100.
    let (_temp, path) = patched_arw(12000, 9000);

    let (kind, embedded, (width, height), error, _) = native_raw(&path, 1, 0);
    assert_eq!(
        (kind, embedded, error),
        (Some(1), 1, 0),
        "embedded JPEG preview"
    );
    assert!(
        u64::from(width) * u64::from(height) > 100_000_000,
        "{width}x{height}"
    );
    // A full-size decode says what it needs when that is over its allowance:
    // 18 bytes a sensor pixel, about 1.9 GB here.
    let gib = 1 << 30;
    let (kind, _, (width, height), error, needed) = native_raw(&path, 0, gib);
    assert_eq!((kind, error), (None, -200_001));
    assert!(needed >= 108_000_000 * 18, "{needed}");
    assert!(u64::from(width) * u64::from(height) > 100_000_000);
    // Allowed more, the same decode goes past the memory check; only the
    // patched file's short sensor data can stop it now.
    let (_, _, _, error, _) = native_raw(&path, 0, 4 * gib);
    assert_ne!(error, -200_001);
}
#[test]
fn a_sensor_too_large_for_libraw_to_size_is_refused_before_any_decode() {
    // 40000x40000 is 1.6 GP: LibRaw's 32-bit output sizing would overflow.
    let (_temp, path) = patched_arw(40000, 40000);
    let (kind, _, _, error, needed) = native_raw(&path, 0, u64::MAX);
    assert_eq!((kind, error, needed), (None, -200_002, 0));
    // The camera's own preview still shows.
    let (kind, embedded, _, error, _) = native_raw(&path, 1, 0);
    assert_eq!((kind, embedded, error), (Some(1), 1, 0));
}

#[test]
fn raw_photos_in_folders_with_accented_names_decode() -> anyhow::Result<()> {
    // LibRaw's narrow-path open reads the ANSI code page on Windows; the bridge
    // must open these paths through its wide-character API instead.
    let temp = tempfile::tempdir()?;
    let library = temp.path().join("Férias São João 照片");
    std::fs::create_dir(&library)?;
    std::fs::copy(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/samples/arw_sample.ARW"),
        library.join("Praia ção.ARW"),
    )?;
    let catalog = Catalog::scan(&library, || false, |_| {})?;
    let media = catalog.media.first().expect("one RAW photo");
    assert_eq!(media.kind, MediaKind::Raw);
    let mut pipeline = Pipeline::new(256 * 1024 * 1024, temp.path().join("cache"));
    let loaded = pipeline.load(media)?;
    assert!(loaded.pixels.width() > 0 && loaded.pixels.height() > 0);
    Ok(())
}
