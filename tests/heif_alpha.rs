#![cfg(feature = "heif")]

use photosort::core::{
    inspection::{Region, Request},
    model::{Fingerprint, Media, MediaId, MediaKind, Step},
    pipeline::Pipeline,
};
use std::sync::Arc;

#[test]
fn transparent_heic_uses_straight_colors_in_every_workflow_and_after_restart() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("alpha.HEIC");
    let original = include_bytes!("fixtures/heif/alpha.heic");
    std::fs::write(&path, original).unwrap();
    let media = Media {
        id: MediaId::for_path(&path),
        fingerprint: Fingerprint::read(&path).unwrap(),
        path,
        kind: MediaKind::Photo,
    };
    let disk = root.path().join("cache");
    let mut pipeline = Pipeline::new(8 * 1024 * 1024, disk.clone());
    let display = pipeline.load(&media).unwrap();
    assert_eq!(display.pixels.dimensions(), (96, 64));
    for (column, alpha) in [(12, 255), (36, 128), (60, 64), (84, 0)] {
        let pixel = display.pixels.get_pixel(column, 32).0;
        assert_eq!(pixel[3], alpha);
        if alpha == 0 {
            assert_eq!(pixel, [0, 0, 0, 0]);
        } else {
            // HEVC color compression and 8-bit unpremultiplication can round
            // slightly; colors must remain close to the same opaque source.
            for (actual, expected) in pixel[..3].iter().zip([220u8, 100, 40]) {
                assert!(actual.abs_diff(expected) <= 5, "alpha {alpha}: {pixel:?}");
            }
        }
    }
    for _ in Step::ALL {
        assert!(Arc::ptr_eq(
            &display.pixels,
            &pipeline.load(&media).unwrap().pixels
        ));
        let analysis = pipeline.analysis(&media, || false).unwrap();
        assert_eq!(*analysis.pixels, *display.pixels);
        let detail = pipeline
            .inspect(&Request {
                slot: 0,
                media: media.clone(),
                region: Region {
                    x: 0,
                    y: 0,
                    width: 96,
                    height: 64,
                },
                output_edge: 96,
            })
            .unwrap();
        assert_eq!(*detail, *display.pixels);
    }
    assert_eq!(
        (pipeline.stats.decodes, pipeline.stats.metadata_reads),
        (1, 1)
    );
    let mut reopened = Pipeline::new(8 * 1024 * 1024, disk);
    assert_eq!(*reopened.load(&media).unwrap().pixels, *display.pixels);
    assert_eq!(
        *reopened.analysis(&media, || false).unwrap().pixels,
        *display.pixels
    );
    assert_eq!(
        (
            reopened.stats.decodes,
            reopened.stats.metadata_reads,
            reopened.stats.disk_hits
        ),
        (0, 0, 2)
    );
    assert_eq!(std::fs::read(&media.path).unwrap(), original);
}
