use photosort::core::{
    inspection::{Queue, Region, Request, View},
    metadata,
    model::*,
    pipeline::Pipeline,
};
use std::path::Path;

fn photo(root: &Path, name: &str, width: u32, height: u32) -> Media {
    let path = root.join(name);
    image::RgbaImage::from_fn(width, height, |x, y| {
        image::Rgba([(x % 251) as u8, (y % 251) as u8, ((x + y) % 251) as u8, 255])
    })
    .save(&path)
    .unwrap();
    Media {
        id: MediaId::for_path(&path),
        fingerprint: Fingerprint::read(&path).unwrap(),
        path,
        kind: MediaKind::Photo,
    }
}
fn request(media: &Media, region: Region) -> Request {
    Request {
        slot: 0,
        media: media.clone(),
        region,
        output_edge: 512,
    }
}

#[test]
fn actual_size_uses_display_pixels_and_rotated_visible_bounds() {
    let view = View {
        actual_size: true,
        ..View::default()
    };
    assert_eq!(view.scale([6000, 4000], [400.0, 300.0], 0, 2.0), 0.5);
    assert_eq!(
        view.visible_region([6000, 4000], [400.0, 300.0], 0, 2.0),
        Some(Region {
            x: 2600,
            y: 1700,
            width: 800,
            height: 600
        })
    );
    assert_eq!(
        view.visible_region([6000, 4000], [400.0, 300.0], 1, 2.0),
        Some(Region {
            x: 2700,
            y: 1600,
            width: 600,
            height: 800
        })
    );
    let panned = View {
        pan: [100.0, 0.0],
        ..view
    };
    assert_eq!(
        panned
            .visible_region([6000, 4000], [400.0, 300.0], 0, 2.0)
            .unwrap()
            .x,
        2400
    );
    let outside = View {
        pan: [9000.0, 0.0],
        ..view
    };
    assert_eq!(
        outside.visible_region([6000, 4000], [400.0, 300.0], 0, 2.0),
        None
    );
}
#[test]
fn fit_preserves_the_entire_source_for_every_rotation() {
    for turns in 0..4 {
        assert_eq!(
            View::default().visible_region([6000, 4000], [800.0, 500.0], turns, 2.0),
            Some(Region::full(6000, 4000))
        );
    }
}
#[test]
fn original_detail_retains_native_pixels_and_reuses_preview_decode_across_steps() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = photo(root.path(), "large.png", 3072, 256);
    let mut pipeline = Pipeline::new(4 * 1024 * 1024, cache.path().to_owned());
    let proxy = pipeline.load(&media).unwrap();
    assert_eq!(proxy.pixels.width(), 2048);
    let mut r = request(
        &media,
        Region {
            x: 2100,
            y: 100,
            width: 200,
            height: 120,
        },
    );
    for step in Step::ALL {
        r.slot = step.index() as u64;
        let detail = pipeline.inspect(&r).unwrap();
        assert_eq!(detail.dimensions(), (200, 120));
        assert_eq!(detail.get_pixel(0, 0).0, [92, 100, 192, 255]);
        pipeline.load(&media).unwrap();
    }
    // Native browsing prepares a bounded image, then one source-resolution
    // neighborhood on demand. Every subsequent workflow shares that tile.
    assert_eq!(
        pipeline.stats.decodes,
        if cfg!(feature = "native-imaging") {
            2
        } else {
            1
        }
    );
    assert!(pipeline.stats.detail_bytes <= 8 * 1024 * 1024);
}
#[test]
fn source_and_sidecar_changes_invalidate_original_detail() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = photo(root.path(), "a.png", 96, 64);
    let mut pipeline = Pipeline::new(1024 * 1024, cache.path().to_owned());
    let r = request(&media, Region::full(96, 64));
    pipeline.inspect(&r).unwrap();
    metadata::write_sidecar(&media.path, None, Some(6)).unwrap();
    let rotated = pipeline
        .inspect(&request(&media, Region::full(64, 96)))
        .unwrap();
    assert_eq!(rotated.get_pixel(63, 0).0, [0, 0, 0, 255]);
    assert_eq!(pipeline.stats.decodes, 2);
    image::RgbaImage::from_pixel(64, 96, image::Rgba([12, 34, 56, 255]))
        .save(&media.path)
        .unwrap();
    let changed = pipeline.inspect(&r).unwrap();
    assert_eq!(changed.get_pixel(0, 0).0, [12, 34, 56, 255]);
    assert_eq!(pipeline.stats.decodes, 3);
}
#[test]
fn detail_budget_evicts_sources_but_never_downscales_a_native_crop() {
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let media = photo(root.path(), "a.png", 96, 64);
    let mut pipeline = Pipeline::new(1, cache.path().to_owned());
    let detail = pipeline
        .inspect(&request(&media, Region::full(96, 64)))
        .unwrap();
    assert_eq!(detail.dimensions(), (96, 64));
    assert_eq!(pipeline.stats.detail_bytes, 0);
}
#[test]
fn inspection_mailbox_coalesces_views_and_rejects_late_results() {
    let root = tempfile::tempdir().unwrap();
    let a = photo(root.path(), "a.png", 96, 64);
    let b = photo(root.path(), "b.png", 96, 64);
    let first = request(&a, Region::full(96, 64));
    let next = request(&b, Region::full(96, 64));
    let mut q = Queue::default();
    q.update(1, vec![first.clone()]);
    assert_eq!(q.take(), Some((1, first.clone())));
    q.update(1, vec![first.clone()]);
    assert!(
        q.take().is_none(),
        "unchanged view must not crop/decode again"
    );
    q.update(1, vec![next.clone()]);
    assert!(!q.accepts(1, &first));
    assert!(q.accepts(1, &next));
    q.update(2, vec![]);
    assert!(!q.accepts(1, &next));
    assert!(q.take().is_none());
}

#[test]
fn disk_proxy_preserves_decoded_geometry_when_exif_dimensions_differ() {
    use image::ImageEncoder;
    let root = tempfile::tempdir().unwrap();
    let cache = tempfile::tempdir().unwrap();
    let path = root.path().join("active-area.jpg");
    // TIFF IFD0 points to an EXIF IFD whose dimensions disagree with the JPEG.
    // Camera files can describe an embedded preview or a larger sensor area.
    let mut exif = b"II\x2a\x00\x08\x00\x00\x00".to_vec();
    exif.extend_from_slice(&2u16.to_le_bytes());
    for (tag, kind, value) in [(0x0112u16, 3u16, 6u32), (0x8769, 4, 38)] {
        exif.extend_from_slice(&tag.to_le_bytes());
        exif.extend_from_slice(&kind.to_le_bytes());
        exif.extend_from_slice(&1u32.to_le_bytes());
        exif.extend_from_slice(&value.to_le_bytes());
    }
    exif.extend_from_slice(&0u32.to_le_bytes());
    exif.extend_from_slice(&2u16.to_le_bytes());
    for (tag, value) in [(0xa002u16, 97u32), (0xa003, 65)] {
        exif.extend_from_slice(&tag.to_le_bytes());
        exif.extend_from_slice(&4u16.to_le_bytes());
        exif.extend_from_slice(&1u32.to_le_bytes());
        exif.extend_from_slice(&value.to_le_bytes());
    }
    exif.extend_from_slice(&0u32.to_le_bytes());
    let pixels = image::RgbImage::new(96, 64);
    let mut encoder = image::codecs::jpeg::JpegEncoder::new(std::fs::File::create(&path).unwrap());
    encoder.set_exif_metadata(exif).unwrap();
    encoder.encode_image(&pixels).unwrap();
    let media = Media {
        id: MediaId::for_path(&path),
        fingerprint: Fingerprint::read(&path).unwrap(),
        path,
        kind: MediaKind::Photo,
    };
    let embedded = metadata::read(&media.path);
    assert_eq!(
        (embedded.width, embedded.height, embedded.orientation),
        (97, 65, 6)
    );
    let mut cold = Pipeline::new(1024 * 1024, cache.path().to_owned());
    let decoded = cold.load(&media).unwrap();
    assert_eq!((decoded.metadata.width, decoded.metadata.height), (96, 64));
    assert_eq!(decoded.pixels.dimensions(), (64, 96));
    let mut reopened = Pipeline::new(1024 * 1024, cache.path().to_owned());
    let cached = reopened.load(&media).unwrap();
    assert_eq!((cached.metadata.width, cached.metadata.height), (96, 64));
    assert_eq!(reopened.stats.decodes, 0);
    assert_eq!(reopened.stats.disk_hits, 1);
    let detail = reopened
        .inspect(&request(&media, Region::full(64, 96)))
        .unwrap();
    assert_eq!(detail.dimensions(), (64, 96));
}

#[test]
fn original_requests_follow_preview_density_and_explicit_native_view() {
    let fit = View::default();
    for turns in 0..4 {
        assert!(!fit.needs_original([6000, 4000], Some([2048, 1365]), [800., 500.], turns, 2.));
        assert!(fit.needs_original([6000, 4000], Some([320, 213]), [800., 500.], turns, 2.));
    }
    assert!(!fit.needs_original([6000, 4000], None, [800., 500.], 0, 2.));
    assert!(
        View {
            actual_size: true,
            ..fit
        }
        .needs_original([6000, 4000], None, [800., 500.], 0, 2.)
    );
    assert!(View { zoom: 3., ..fit }.needs_original(
        [6000, 4000],
        Some([2048, 1365]),
        [800., 500.],
        0,
        2.
    ));
    assert!(!fit.needs_original([6000, 4000], Some([1200, 800]), [900., 600.], 0, 1.));
    assert!(fit.needs_original([6000, 4000], Some([1200, 800]), [900., 600.], 0, 2.));
    // Density alone still asks for the original when the size is only known
    // from the preview itself, as for files without dimensions in metadata.
    assert!(View { zoom: 3., ..fit }.needs_original([16, 16], Some([16, 16]), [900., 600.], 0, 2.));
}

#[test]
fn a_preview_holding_every_metadata_pixel_is_complete() {
    assert!(View::preview_is_complete([16, 16], [16, 16]));
    // Stored unrotated against a rotated metadata size.
    assert!(View::preview_is_complete([800, 1200], [1200, 800]));
    assert!(!View::preview_is_complete([6000, 4000], [2048, 1365]));
    // A capped preview may hide a photo whose metadata understates its size.
    assert!(!View::preview_is_complete([2048, 1536], [2048, 1536]));
}
