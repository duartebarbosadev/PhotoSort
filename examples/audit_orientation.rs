//! Read-only orientation consistency audit through the shared Rust pipeline/model owner.
//! Usage: cargo run --release --example audit_orientation -- <photo> ...
use anyhow::{Context, Result};
use photosort::core::{
    model::{Fingerprint, Media, MediaId, MediaKind},
    models::{Models, orientation_model},
    pipeline::{Asset, Pipeline},
};
use std::{path::PathBuf, sync::Arc, time::Instant};

fn main() -> Result<()> {
    let paths: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    anyhow::ensure!(!paths.is_empty(), "Pass one or more sample photo paths");
    let model = orientation_model().context("Orientation model is unavailable")?;
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(64 * 1024 * 1024, cache.path().to_owned());
    let mut models = Models::default();
    let mut records = Vec::new();
    for path in paths {
        let media = Media {
            id: MediaId::for_path(&path),
            kind: MediaKind::from_path(&path).context("Unsupported media")?,
            fingerprint: Fingerprint::read(&path)?,
            path,
        };
        anyhow::ensure!(media.kind != MediaKind::Video, "Pass still photographs");
        let start = Instant::now();
        let prepared = pipeline.analysis(&media, || false)?;
        let decodes = pipeline.stats.decodes;
        let mut turns = Vec::new();
        for clockwise in 0..4 {
            let pixels = match clockwise {
                0 => prepared.pixels.clone(),
                1 => Arc::new(image::imageops::rotate90(&*prepared.pixels)),
                2 => Arc::new(image::imageops::rotate180(&*prepared.pixels)),
                _ => Arc::new(image::imageops::rotate270(&*prepared.pixels)),
            };
            let asset = Asset {
                decoder_warning: None,
                thumbnail: pixels.clone(),
                pixels,
                metadata: prepared.metadata.clone(),
            };
            let suggestion = models.rotation(&model, &asset)?;
            let mut filter_comparisons = Vec::new();
            for (name, filter) in [
                ("catmull_rom", image::imageops::FilterType::CatmullRom),
                ("lanczos3", image::imageops::FilterType::Lanczos3),
            ] {
                let start = Instant::now();
                let pixels = Arc::new(image::imageops::resize(&*asset.pixels, 416, 416, filter));
                // The shared model's subsequent 416 resize is an identity. This
                // isolates filtering while keeping crop/normalization/inference.
                let alternative = Asset {
                    decoder_warning: asset.decoder_warning.clone(),
                    thumbnail: pixels.clone(),
                    pixels,
                    metadata: asset.metadata.clone(),
                };
                let alternative = models.rotation(&model, &alternative)?;
                filter_comparisons.push(serde_json::json!({
                    "filter": name,
                    "suggested_clockwise_turns": alternative.turns,
                    "model_score": alternative.confidence,
                    "views": alternative.views,
                    "uncertain": alternative.uncertain,
                    "resulting_turns_from_prepared": (clockwise + alternative.turns) % 4,
                    "elapsed_seconds": start.elapsed().as_secs_f64(),
                }));
            }
            turns.push(serde_json::json!({
                "input_clockwise_turns": clockwise,
                "filter_comparisons": filter_comparisons,
                "suggested_clockwise_turns": suggestion.turns,
                "model_score": suggestion.confidence,
                "views": suggestion.views,
                "uncertain": suggestion.uncertain,
                "resulting_turns_from_prepared": (clockwise + suggestion.turns) % 4,
            }));
        }
        let display = pipeline.load(&media)?;
        let display_suggestion = models.rotation(&model, &display)?;
        if media.kind != MediaKind::Raw {
            anyhow::ensure!(
                pipeline.stats.decodes == decodes,
                "Display comparison decoded the source again"
            );
        }
        anyhow::ensure!(
            Fingerprint::read(&media.path)? == media.fingerprint,
            "Source changed during audit"
        );
        records.push(serde_json::json!({
            "name": media.name(),
            "kind": format!("{:?}", media.kind),
            "exif_orientation": prepared.metadata.orientation,
            "prepared_dimensions": prepared.pixels.dimensions(),
            "display_dimensions": display.pixels.dimensions(),
            "analysis_quarter_turns": turns,
            "display_suggestion_turns": display_suggestion.turns,
            "display_model_score": display_suggestion.confidence,
            "display_additional_decodes": pipeline.stats.decodes - decodes,
            "elapsed_seconds": start.elapsed().as_secs_f64(),
        }));
    }
    anyhow::ensure!(
        models.initializations == 1,
        "Orientation session was initialized more than once"
    );
    println!(
        "{}",
        serde_json::to_string_pretty(&serde_json::json!({
            "policy": "conditional_rotation_consensus_v1",
            "records": records,
            "model_initializations": models.initializations,
            "model_inferences": models.inferences,
            "source_decodes": pipeline.stats.decodes,
            "interpretation": "Quarter-turn consistency is not proof of correct upright orientation. Scores are model softmax outputs, not calibrated correctness probabilities.",
        }))?
    );
    Ok(())
}
