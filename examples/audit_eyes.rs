//! Read-only eye audit: for every photo, its capture time and each face's eye
//! spacing, aspect ratios, blink scores and verdict, as Easy Delete's Eyes
//! closed tab judges them. Uses a temporary cache; never writes to the library.
//! Usage: audit_eyes <folder> [--crops <new-directory>] (saves each face's eyes)
use anyhow::Result;
use photosort::core::{catalog::Catalog, eyes, faces, model::MediaKind, pipeline::Pipeline};

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
    let mut models = photosort::core::models::Models::default();
    // Per photo: id, name, camera, capture time, closed count, faces, size.
    let mut seen = Vec::new();
    for media in catalog.media.iter().filter(|m| m.kind != MediaKind::Video) {
        let name = media.path.strip_prefix(&root).unwrap_or(&media.path);
        let Ok(asset) = pipeline.analysis(media, || false) else {
            println!("{}: decode failed", name.display());
            continue;
        };
        let metadata = pipeline.metadata(media).ok();
        let (w, h) = asset.pixels.dimensions();
        let found = faces::analyze(&mut models, &asset)?;
        println!(
            "{} ({w}x{h}) captured {:?}+{:?}ms camera {:?}: {} faces, {} closed",
            name.display(),
            metadata.as_ref().and_then(|m| m.captured_ms),
            metadata.as_ref().and_then(|m| m.captured_subsec_ms),
            metadata.as_ref().map(|m| m.camera.clone()),
            found.count,
            found.closed_eyes
        );
        seen.push((
            media.id,
            name.display().to_string(),
            metadata
                .as_ref()
                .map(|m| m.camera.trim().to_owned())
                .unwrap_or_default(),
            metadata
                .as_ref()
                .and_then(|m| Some(m.captured_ms? + i64::from(m.captured_subsec_ms.unwrap_or(0)))),
            found.closed_eyes,
            found.geometry.clone(),
            (w, h),
        ));
        for (i, face) in found.geometry.iter().enumerate() {
            let state = faces::eye_state(face, w, h);
            let m = faces::eye_measures(face, w, h);
            let [l, t, r, _] = face.bbox;
            println!(
                "  face {i} at {:.0},{:.0} {:.0}px: {state:?} spacing {:.1} ratio {:.3}/{:.3} blink {:.2}/{:.2}",
                l * w as f32,
                t * h as f32,
                (r - l) * w as f32,
                m.map_or(f32::NAN, |m| m.spacing),
                m.map_or(f32::NAN, |m| m.ratios[0]),
                m.map_or(f32::NAN, |m| m.ratios[1]),
                m.map_or(f32::NAN, |m| m.blink[0]),
                m.map_or(f32::NAN, |m| m.blink[1]),
            );
            // Faces too small to judge here, judged again on the display image.
            if m.is_some_and(|m| (10.0..20.0).contains(&m.spacing))
                && let Ok(display) = pipeline.load_neutral_cancellable(media, || false)
                && let Some(again) = faces::rejudge(&mut models, &display, face)?
            {
                let (dw, dh) = display.pixels.dimensions();
                let m = faces::eye_measures(&again, dw, dh);
                println!(
                    "    on {dw}x{dh}: {:?} spacing {:.1} ratio {:.3}/{:.3} blink {:.2}/{:.2}",
                    faces::eye_state(&again, dw, dh),
                    m.map_or(f32::NAN, |m| m.spacing),
                    m.map_or(f32::NAN, |m| m.ratios[0]),
                    m.map_or(f32::NAN, |m| m.ratios[1]),
                    m.map_or(f32::NAN, |m| m.blink[0]),
                    m.map_or(f32::NAN, |m| m.blink[1]),
                );
            }
            if let (Some(dir), Some(rect)) = (&crops, faces::eye_rect(face, w, h)) {
                let [x0, y0, x1, y1] = [
                    rect[0] * w as f32,
                    rect[1] * h as f32,
                    rect[2] * w as f32,
                    rect[3] * h as f32,
                ];
                if x1 - x0 >= 2.0 && y1 - y0 >= 2.0 {
                    let crop = image::imageops::crop_imm(
                        &*asset.pixels,
                        x0 as u32,
                        y0 as u32,
                        (x1 - x0) as u32,
                        (y1 - y0) as u32,
                    )
                    .to_image();
                    let stem = name.display().to_string().replace('/', "_");
                    image::imageops::resize(&crop, 240, 80, image::imageops::FilterType::Triangle)
                        .save(dir.join(format!("{stem}-{i}-{state:?}.png")))?;
                }
            }
        }
    }
    // The moments Easy Delete would look at, as if every photo were one
    // scene (the app also needs the same similarity group).
    let candidates: Vec<_> = seen
        .iter()
        .map(|(id, _, camera, at_ms, _, found, _)| eyes::Candidate {
            id: *id,
            group: 0,
            camera,
            at_ms: *at_ms,
            faces: found.len(),
        })
        .collect();
    let blinks = eyes::moments(
        &candidates,
        // Analysis-size judgements only: the app also looks again at small
        // faces on the display image (printed above as "on WxH").
        |i| Some(eyes::people(&seen[i].5, seen[i].6.0, seen[i].6.1)),
        || false,
    )
    .unwrap_or_default();
    let name = |id| {
        seen.iter()
            .find(|s| s.0 == id)
            .map_or("?", |s| s.1.as_str())
    };
    println!("\nClosed-eyes shots and the other shots of their moment:");
    for blink in &blinks {
        println!("{}", name(blink.frame.id));
        for other in &blink.others {
            println!(
                "  {:+.1} s {}: {}",
                (other.at_ms - blink.frame.at_ms) as f64 / 1000.0,
                name(other.id),
                if eyes::better_shot(&blink.frame.people, &other.people).is_some() {
                    "better shot"
                } else {
                    "not better"
                }
            );
        }
    }
    Ok(())
}
