//! Sequential preview + thumbnail readiness through the production pipeline.
//! Compare service work, not GUI latency or equal photographic rendering recipes.
use anyhow::{Result, ensure};
use photosort::core::{
    catalog::Catalog,
    model::MediaKind,
    pipeline::{Pipeline, Stats},
};
use serde_json::{Value, json};
use std::{collections::BTreeMap, path::PathBuf, time::Instant};

fn counters(stats: Stats) -> Value {
    json!({"decodes":stats.decodes,"native_renders":stats.native_renders,
        "raw_embedded":stats.raw_embedded,"raw_fallback":stats.raw_fallback,
        "memory_hits":stats.hits,"disk_hits":stats.disk_hits,
        "metadata_reads":stats.metadata_reads,"resident_bytes":stats.resident_bytes})
}

fn main() -> Result<()> {
    let setup = Instant::now();
    let mut args = std::env::args_os().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("folder required"))?,
    )
    .canonicalize()?;
    ensure!(args.next().is_none(), "use one read-only photo folder");
    let mut catalog = Catalog::scan(&root, || false, |_| {})?;
    ensure!(
        catalog.warnings.is_empty(),
        "discovery warnings: {:?}",
        catalog.warnings
    );
    catalog.media.retain(|media| media.kind != MediaKind::Video);
    ensure!(!catalog.media.is_empty(), "empty photo library");
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let setup_ms = setup.elapsed().as_secs_f64() * 1000.;
    let mut phases = Vec::new();
    let mut expected = None;
    let mut all_succeeded = true;
    for phase in ["initial", "warm", "reopened"] {
        if phase == "reopened" {
            drop(pipeline);
            pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
        }
        let before = pipeline.stats;
        let start = Instant::now();
        let mut first_ready_ms = None;
        let mut rows = BTreeMap::new();
        let mut errors = Vec::new();
        for media in &catalog.media {
            let relative = media
                .path
                .strip_prefix(&root)?
                .to_string_lossy()
                .into_owned();
            match pipeline.load(media) {
                Ok(asset) => {
                    first_ready_ms.get_or_insert(start.elapsed().as_secs_f64() * 1000.);
                    rows.insert(
                        relative,
                        json!({"preview":asset.pixels.dimensions(),
                        "thumbnail":asset.thumbnail.dimensions(),"warning":asset.decoder_warning}),
                    );
                }
                Err(error) => errors.push(json!({"file":relative,"error":format!("{error:#}")})),
            }
        }
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.;
        // Validation and directory accounting are outside service timing.
        all_succeeded &= errors.is_empty();
        if let Some(expected) = &expected {
            ensure!(
                &rows == expected,
                "preview geometry/warnings changed on {phase}"
            );
        } else {
            expected = Some(rows.clone());
        }
        let disk_bytes: u64 = walkdir::WalkDir::new(cache.path())
            .into_iter()
            .filter_map(Result::ok)
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.metadata().map(|m| m.len()))
            .collect::<Result<Vec<_>, _>>()?
            .into_iter()
            .sum();
        phases.push(
            json!({"phase":phase,"count":rows.len(),"first_ready_ms":first_ready_ms,
            "elapsed_ms":elapsed_ms,"decode_delta":pipeline.stats.decodes-before.decodes,
            "counters":counters(pipeline.stats),"disk_bytes":disk_bytes,"errors":errors}),
        );
    }
    println!(
        "{}",
        json!({"implementation":"rust","native_imaging":cfg!(feature="native-imaging"),
        "setup_ms":setup_ms,"count":catalog.media.len(),"preview_edge":photosort::core::pipeline::PROXY_EDGE,
        "thumbnail_edge":photosort::core::pipeline::THUMB_EDGE,"memory_budget_bytes":128*1024*1024,
        "concurrent_photo_requests":1,"phases":phases,"rows":expected})
    );
    ensure!(
        all_succeeded,
        "one or more photos failed; do not compare incomplete work"
    );
    Ok(())
}
