//! Measure the real catalog scanner on a caller-provided, read-only folder.
//! No image decoding, metadata extraction, model work, file writes or UI startup.
use photosort::core::catalog::Catalog;
use std::{path::PathBuf, time::Instant};
fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let root = PathBuf::from(
        args.next()
            .ok_or_else(|| anyhow::anyhow!("folder required"))?,
    );
    let repeats: usize = args.next().unwrap_or_else(|| "7".into()).parse()?;
    anyhow::ensure!(repeats > 0 && repeats <= 100, "use 1..100 repetitions");
    let mut runs = Vec::new();
    for _ in 0..repeats {
        let start = Instant::now();
        let (mut first, mut received, mut largest) = (None, 0, 0);
        let catalog = Catalog::scan(
            &root,
            || false,
            |batch| {
                first.get_or_insert(start.elapsed().as_secs_f64() * 1000.0);
                received += batch.len();
                largest = largest.max(batch.len());
            },
        )?;
        anyhow::ensure!(
            received == catalog.media.len(),
            "batch count disagrees with catalog"
        );
        runs.push(
            serde_json::json!({"count": catalog.media.len(), "first_batch_ms": first,
            "scan_ms": start.elapsed().as_secs_f64()*1000.0, "largest_batch": largest}),
        );
    }
    println!(
        "{}",
        serde_json::json!({"implementation":"rust", "runs": runs})
    );
    Ok(())
}
