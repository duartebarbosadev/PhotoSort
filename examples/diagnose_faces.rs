//! Read-only Group by Faces diagnosis: for every photo, the detector candidates
//! with their scores and boxes, which survived each gate, how many faces were
//! recognized, and the person folder each photo ends up in.
//! Uses a temporary cache; never writes to the library.
//! Usage: diagnose_faces <folder> (DIAGNOSE_PROXY=1 detects on the display image)
use anyhow::Result;
use photosort::core::{
    catalog::Catalog, face_groups, faces, model::MediaKind, models::Models, pipeline::Pipeline,
    regional::grouping::Record, subjects::store::Store,
};

fn main() -> Result<()> {
    let root = std::path::PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("Folder required"))?,
    )
    .canonicalize()?;
    // DIAGNOSE_CROPS=<new-directory> saves every kept face, for a visual check.
    let crops = std::env::var_os("DIAGNOSE_CROPS").map(std::path::PathBuf::from);
    if let Some(dir) = &crops {
        std::fs::create_dir(dir)?;
    }
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    let cache = tempfile::tempdir()?;
    let store = Store::new(cache.path().join("store"), false)?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().join("pipeline"));
    let mut models = Models::default();
    let mut records = Vec::new();
    let mut names = std::collections::HashMap::new();
    let mut vectors = Vec::new();
    for media in catalog.media.iter().filter(|m| m.kind != MediaKind::Video) {
        let name = media.path.strip_prefix(&root).unwrap_or(&media.path);
        names.insert(media.id, name.display().to_string());
        let asset = if std::env::var_os("DIAGNOSE_PROXY").is_some() {
            pipeline.load_neutral_cancellable(media, || false)
        } else {
            pipeline.analysis(media, || false)
        };
        let asset = match asset {
            Ok(asset) => asset,
            Err(error) => {
                println!("{}: decode failed: {error:#}", name.display());
                records.push(Record {
                    id: media.id,
                    key: None,
                    portrait: false,
                    date: None,
                });
                continue;
            }
        };
        let (w, h) = (asset.pixels.width() as f32, asset.pixels.height() as f32);
        let (faces, candidates) = faces::analyze_with_candidates(&mut models, &asset)?;
        // The same source choice as Organize's Group by Faces.
        let recognition = face_groups::describe_detailed(
            &mut models,
            &asset,
            &faces,
            || pipeline.load_neutral_cancellable(media, || false),
            || false,
        )?;
        println!(
            "{} ({}x{}): {} candidates, {} faces kept, {} closed eyes, {} recognized",
            name.display(),
            w,
            h,
            candidates.len(),
            faces.count,
            faces.closed_eyes,
            recognition.vectors.len()
        );
        for c in &candidates {
            println!(
                "  score {:.2} presence {:.2} redetect {:.2} box {:.0},{:.0} {:.0}px {}",
                c.detector_score,
                c.presence,
                c.redetect,
                c.bbox[0] * w,
                c.bbox[1] * h,
                (c.bbox[2] - c.bbox[0]) * w,
                if c.accepted { "kept" } else { "rejected" }
            );
        }
        for (i, face) in faces.geometry.iter().enumerate() {
            let [l, t, r, b] = face.bbox;
            if let Some(dir) = &crops {
                let (x0, y0) = ((l * w).max(0.0) as u32, (t * h).max(0.0) as u32);
                let (x1, y1) = ((r * w).min(w) as u32, (b * h).min(h) as u32);
                if x1 > x0 + 1 && y1 > y0 + 1 {
                    let crop = image::imageops::crop_imm(&*asset.pixels, x0, y0, x1 - x0, y1 - y0)
                        .to_image();
                    let stem = name.display().to_string().replace('/', "_");
                    image::imageops::resize(&crop, 96, 96, image::imageops::FilterType::Triangle)
                        .save(dir.join(format!("{stem}-{i}.png")))?;
                }
            }
            let eyes = face_groups::eye_distance(&asset, face);
            if let Some(c) = candidates.iter().filter(|c| c.accepted).nth(i) {
                let iw = (c.bbox[2].min(r) - c.bbox[0].max(l)).max(0.0) * w;
                let ih = (c.bbox[3].min(b) - c.bbox[1].max(t)).max(0.0) * h;
                let area = |x: [f32; 4]| (x[2] - x[0]) * w * (x[3] - x[1]) * h;
                let inter = iw * ih;
                println!(
                    "  mesh/detector IoU {:.2} blink {:?}",
                    inter / (area(c.bbox) + area(face.bbox) - inter),
                    face.blink
                );
            }
            println!(
                "  face {:.0}x{:.0}px eye distance {:.1}px",
                (r - l) * w,
                (b - t) * h,
                eyes.unwrap_or(f64::NAN)
            );
        }
        for (i, vector) in recognition.vectors.iter().enumerate() {
            vectors.push(serde_json::json!({
                "photo": name.display().to_string(),
                "face": i,
                "vector": vector,
            }));
        }
        let key = (!recognition.vectors.is_empty())
            .then(|| Store::recognition_key(&format!("photo-{}", media.id.0), "diagnosis"));
        if let Some(key) = &key {
            store.put_recognition(key, &recognition)?;
        }
        records.push(Record {
            id: media.id,
            key,
            portrait: false,
            date: None,
        });
    }
    // DIAGNOSE_VECTORS=<file> saves every recognition descriptor as JSON.
    if let Some(path) = std::env::var_os("DIAGNOSE_VECTORS") {
        std::fs::write(path, serde_json::to_vec(&vectors)?)?;
    }
    let output = face_groups::group(&records, &store, || false)?.expect("not cancelled");
    let mut rows: Vec<_> = output
        .folders
        .iter()
        .map(|(id, folder)| (folder.clone(), names[id].clone()))
        .collect();
    rows.sort();
    println!("\nAssignments:");
    for (folder, name) in &rows {
        println!("  {folder}: {name}");
    }
    let summary = face_groups::summarize(&output);
    println!(
        "\n{} photos with a recognized person, {} without; {} people",
        summary.photos,
        records.len() - summary.photos,
        summary.people
    );
    Ok(())
}
