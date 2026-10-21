//! Combined shared-pipeline workload, with disposable caches and read-only sources.
//! No GUI, model inference, filesystem edits, or cold OS-cache claims.
use anyhow::{Result, ensure};
use photosort::core::{
    analysis::{self, Thresholds},
    catalog::Catalog,
    inspection::{Region, Request},
    pipeline::{Pipeline, Stats},
};
use serde_json::{Value, json};
use std::{path::PathBuf, time::Instant};
fn stats(s: Stats) -> Value {
    json!({"decodes":s.decodes,"native_renders":s.native_renders,
        "metadata_reads":s.metadata_reads,"disk_hits":s.disk_hits,
        "memory_hits":s.hits,"preview_bytes":s.resident_bytes,"original_bytes":s.detail_bytes})
}
fn main() -> Result<()> {
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("folder required"))?,
    );
    let start = Instant::now();
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    ensure!(!catalog.media.is_empty(), "empty library");
    ensure!(
        catalog.warnings.is_empty(),
        "scan warnings: {:?}",
        catalog.warnings
    );
    let scan_ms = start.elapsed().as_secs_f64() * 1000.;
    let disk = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, disk.path().into());
    let mut stages = Vec::new();
    let start = Instant::now();
    for media in &catalog.media {
        std::hint::black_box(pipeline.load(media)?);
    }
    stages.push(json!({"name":"browse_cold_proxies","ms":start.elapsed().as_secs_f64()*1000.,"stats":stats(pipeline.stats)}));
    let start = Instant::now();
    let mut measurements = Vec::new();
    for media in &catalog.media {
        let asset = pipeline.analysis(media, || false)?;
        measurements.push(json!({"file":media.name(), "dimensions":asset.pixels.dimensions(),
            "analysis":analysis::analyze(&asset, Thresholds::default()), "warning":asset.decoder_warning}));
    }
    stages.push(json!({"name":"technical_analysis","ms":start.elapsed().as_secs_f64()*1000.,"stats":stats(pipeline.stats)}));
    let start = Instant::now();
    let mut inspect_ms = 0.;
    let mut revisit_ms = 0.;
    for media in &catalog.media {
        let metadata = pipeline.metadata(media)?;
        let request = Request {
            slot: 0,
            media: media.clone(),
            region: Region {
                x: 0,
                y: 0,
                width: metadata.width.min(512),
                height: metadata.height.min(512),
            },
            output_edge: 512,
        };
        let one = Instant::now();
        let first = pipeline.inspect(&request)?;
        let inspected_metadata = pipeline.metadata(media)?;
        ensure!(
            (metadata.width, metadata.height)
                == (inspected_metadata.width, inspected_metadata.height),
            "Preview and original geometry disagree for {}",
            media.name()
        );
        inspect_ms += one.elapsed().as_secs_f64() * 1000.;
        let before = pipeline.stats.decodes;
        let one = Instant::now();
        let second = pipeline.inspect(&request)?;
        revisit_ms += one.elapsed().as_secs_f64() * 1000.;
        ensure!(first == second, "revisit changed pixels");
        ensure!(
            before == pipeline.stats.decodes,
            "immediate revisit decoded source again"
        );
    }
    stages.push(json!({"name":"original_and_immediate_revisit","ms":start.elapsed().as_secs_f64()*1000.,"first_ms":inspect_ms,"revisit_ms":revisit_ms,"stats":stats(pipeline.stats)}));
    let start = Instant::now();
    for media in &catalog.media {
        std::hint::black_box(pipeline.load(media)?);
        std::hint::black_box(pipeline.analysis(media, || false)?);
    }
    stages.push(json!({"name":"return_to_browsing","ms":start.elapsed().as_secs_f64()*1000.,"stats":stats(pipeline.stats)}));
    drop(pipeline);
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, disk.path().into());
    let start = Instant::now();
    for media in &catalog.media {
        std::hint::black_box(pipeline.load(media)?);
        std::hint::black_box(pipeline.analysis(media, || false)?);
    }
    ensure!(
        pipeline.stats.decodes == 0,
        "restart did not reuse disk proxies"
    );
    stages.push(json!({"name":"new_owner_disk_proxies","ms":start.elapsed().as_secs_f64()*1000.,"stats":stats(pipeline.stats)}));
    println!(
        "{}",
        json!({"native_imaging":cfg!(feature="native-imaging"),"count":catalog.media.len(),"scan_ms":scan_ms,"stages":stages,"measurements":measurements})
    );
    Ok(())
}
