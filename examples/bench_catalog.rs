//! Synthetic filename-only scan benchmark, deliberately separate from image decoding.
use photosort::core::catalog::Catalog;
use std::{fs, time::Instant};
fn main() -> anyhow::Result<()> {
    let count: usize = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "50000".into())
        .parse()?;
    let folder = tempfile::tempdir()?;
    for i in 0..count {
        fs::write(folder.path().join(format!("frame-{i:06}.jpg")), [])?;
    }
    let start = Instant::now();
    let mut first = None;
    let mut largest = 0;
    let catalog = Catalog::scan(
        folder.path(),
        || false,
        |batch| {
            first.get_or_insert(start.elapsed());
            largest = largest.max(batch.len());
        },
    )?;
    assert_eq!(catalog.media.len(), count);
    assert!(largest <= 128);
    println!(
        "{count} file records: first batch {:?}, completed {:?}, largest batch {largest}; image/model loads 0",
        first.unwrap(),
        start.elapsed()
    );
    Ok(())
}
