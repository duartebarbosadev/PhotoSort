//! Metadata-service throughput with independent temporary caches; never edits photos.
use anyhow::{Result, ensure};
use photosort::core::{catalog::Catalog, pipeline::Pipeline};
use sha2::{Digest, Sha256};
use std::{fmt::Write, path::PathBuf, time::Instant};
fn main() -> Result<()> {
    ensure!(
        cfg!(feature = "metadata-native"),
        "Build with metadata-native for this comparison"
    );
    let root = PathBuf::from(
        std::env::args()
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("folder required"))?,
    )
    .canonicalize()?;
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    ensure!(!catalog.media.is_empty(), "No photos in fixture");
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let mut phases = Vec::new();
    let mut expected = None;
    let mut expected_snapshot = None;
    for phase in ["initial", "warm", "reopened", "rewarmed"] {
        if phase == "reopened" {
            pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
        }
        let before = pipeline.stats.metadata_reads;
        let hits_before = pipeline.stats.metadata_disk_hits;
        let start = Instant::now();
        let values = catalog
            .media
            .iter()
            .map(|media| pipeline.metadata(media))
            .collect::<Result<Vec<_>>>()?;
        let seconds = start.elapsed().as_secs_f64();
        ensure!(
            pipeline.stats.decodes == 0,
            "Metadata benchmark decoded pixels"
        );
        // Validate every Rust metadata field outside the timed service pass.
        let snapshot = blake3::hash(&serde_json::to_vec(&values)?);
        if let Some(expected) = expected_snapshot {
            ensure!(
                snapshot == expected,
                "Full metadata snapshot changed between passes"
            );
        } else {
            expected_snapshot = Some(snapshot);
        }
        let rows: Vec<_> = catalog
            .media
            .iter()
            .zip(&values)
            .map(|(media, meta)| {
                serde_json::json!([
                    media.path.strip_prefix(&root).unwrap().to_string_lossy(),
                    meta.width,
                    meta.height,
                    meta.orientation,
                    meta.camera,
                    meta.lens
                ])
            })
            .collect();
        let encoded = serde_json::to_vec(&rows)?;
        let mut checksum = String::with_capacity(64);
        for byte in Sha256::digest(encoded) {
            write!(checksum, "{byte:02x}")?;
        }
        if let Some(expected) = &expected {
            ensure!(&checksum == expected, "Metadata changed between passes");
        } else {
            expected = Some(checksum.clone());
        }
        phases.push(serde_json::json!({"phase":phase,"seconds":seconds,"count":rows.len(),"source_reads":pipeline.stats.metadata_reads-before,"metadata_disk_hits":pipeline.stats.metadata_disk_hits-hits_before,"full_snapshot_blake3":snapshot.to_hex().to_string(),"sha256":checksum,"sample_rows":rows.iter().take(7).collect::<Vec<_>>() }));
    }
    println!(
        "{}",
        serde_json::json!({"implementation":"rust","phases":phases,"reopen_scope":"new metadata owner sharing the same cache directory"})
    );
    Ok(())
}
