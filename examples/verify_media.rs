//! Read-only real-media smoke test. Caches live in a disposable temporary folder.
use photosort::core::{model::*, models::Models, pipeline::Pipeline};
use std::{path::PathBuf, time::Instant};
fn main() -> anyhow::Result<()> {
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(64 * 1024 * 1024, cache.path().to_owned());
    let mut models = Models::default();
    for path in std::env::args_os().skip(1).map(PathBuf::from) {
        let media = Media {
            id: MediaId::for_path(&path),
            kind: MediaKind::from_path(&path).ok_or_else(|| anyhow::anyhow!("Unsupported file"))?,
            fingerprint: Fingerprint::read(&path)?,
            path,
        };
        let start = Instant::now();
        let asset = pipeline.load(&media)?;
        println!(
            "{}: {}×{}, cold {:?}",
            media.name(),
            asset.pixels.width(),
            asset.pixels.height(),
            start.elapsed()
        );
        if media.kind != MediaKind::Video {
            let meta = &asset.metadata;
            let (w, h) = if (5..=8).contains(&meta.orientation) {
                (meta.height, meta.width)
            } else {
                (meta.width, meta.height)
            };
            let region = photosort::core::inspection::Region {
                x: w / 4,
                y: h / 4,
                width: 256.min(w / 2),
                height: 256.min(h / 2),
            };
            let decodes = pipeline.stats.decodes;
            let request = photosort::core::inspection::Request {
                slot: 0,
                media: media.clone(),
                region,
                output_edge: 512,
            };
            let detail = pipeline.inspect(&request)?;
            assert_eq!(detail.dimensions(), (region.width, region.height));
            // Backend-specific decode counts are covered by the shared pipeline tests.
            // A first detail request may decode once; repeating it must reuse exact pixels.
            assert!((decodes..=decodes + 1).contains(&pipeline.stats.decodes));
            let after_detail = pipeline.stats.decodes;
            assert_eq!(pipeline.inspect(&request)?, detail);
            assert_eq!(pipeline.stats.decodes, after_detail);
            println!(
                "native detail {}×{}, original detail verified",
                detail.width(),
                detail.height()
            );
        }
        let warm = Instant::now();
        pipeline.load(&media)?;
        println!("warm {:?}, {:?}", warm.elapsed(), pipeline.stats);
        if let Some(model) = std::env::var_os("PHOTOSORT_ROTATION_MODEL") {
            let result = models.rotation(&PathBuf::from(model.clone()), &asset)?;
            println!("rotation: {result:?}");
            models.rotation(&PathBuf::from(model), &asset)?;
            assert_eq!(models.initializations, 1);
        }
    }
    Ok(())
}
