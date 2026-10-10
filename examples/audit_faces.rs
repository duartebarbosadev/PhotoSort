//! Read-only face-candidate audit for threshold evaluation: every detector hit with
//! its detector score, Face Mesh presence and eye ratio, and whether it was kept.
//! Uses a temporary cache; never writes to the library.
//! Usage: audit_faces <folder> [--crops <new-directory>] > candidates.json
use anyhow::Result;
use photosort::core::{
    catalog::Catalog, faces, model::MediaKind, models::Models, pipeline::Pipeline,
};
use serde_json::json;

fn main() -> Result<()> {
    let root = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Folder required"))?,
    )
    .canonicalize()?;
    let crops = match (std::env::args_os().nth(2), std::env::args_os().nth(3)) {
        (Some(flag), Some(dir)) if flag == "--crops" => {
            std::fs::create_dir(&dir)?;
            Some(std::path::PathBuf::from(dir))
        }
        (None, None) => None,
        _ => anyhow::bail!("Expected --crops <new-directory>"),
    };
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let mut models = Models::default();
    let mut rows = Vec::new();
    for media in catalog.media.iter().filter(|m| m.kind != MediaKind::Video) {
        let asset = pipeline.analysis(media, || false)?;
        let (_, candidates) = faces::analyze_with_candidates(&mut models, &asset)?;
        if let Some(dir) = &crops {
            let (w, h) = (asset.pixels.width() as f32, asset.pixels.height() as f32);
            for (i, c) in candidates.iter().enumerate() {
                let (x0, y0) = ((c.bbox[0] * w).max(0.0), (c.bbox[1] * h).max(0.0));
                let (x1, y1) = ((c.bbox[2] * w).min(w), (c.bbox[3] * h).min(h));
                if x1 - x0 >= 2.0 && y1 - y0 >= 2.0 {
                    let crop = image::imageops::crop_imm(
                        &*asset.pixels,
                        x0 as u32,
                        y0 as u32,
                        (x1 - x0) as u32,
                        (y1 - y0) as u32,
                    )
                    .to_image();
                    let name = format!("{}-{i}.png", rows.len());
                    image::imageops::resize(&crop, 160, 160, image::imageops::FilterType::Triangle)
                        .save(dir.join(name))?;
                }
            }
        }
        if !candidates.is_empty() {
            rows.push(json!({
                "index": rows.len(),
                "path": media.path,
                "orientation": pipeline.metadata(media)?.orientation,
                "candidates": candidates,
            }));
        }
    }
    println!("{}", serde_json::to_string(&rows)?);
    Ok(())
}
