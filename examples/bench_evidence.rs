//! Synthetic varied-library cache/grouping benchmark, not photographic accuracy.
//! Both paths use the production clustering policy; only evidence loading differs.
use anyhow::{Context, Result, ensure};
use photosort::core::{
    regional::{self, Embedding, Features, Input},
    subjects::store::Store,
};
use std::time::Instant;
fn main() -> Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "binary".into());
    ensure!(
        matches!(mode.as_str(), "binary" | "json"),
        "choose binary or json"
    );
    let count: usize = args.next().unwrap_or_else(|| "512".into()).parse()?;
    ensure!(
        (4..=4096).contains(&count) && count.is_multiple_of(4),
        "use 4..4096 records, in multiples of four"
    );
    let root = tempfile::tempdir()?;
    let store = Store::new(root.path().into(), false)?;
    let mut inputs = Vec::new();
    let mut bytes = 0u64;
    for photo in 0..count {
        let mut state = (photo / 4 + 1) as u64;
        let mut random = || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 32) as u32 as f64 / u32::MAX as f64 * 2.0 - 1.0
        };
        let mut vectors = Vec::new();
        for _ in 0..6 {
            let mut row: Vec<f32> = (0..384)
                .map(|i| (random() + ((photo % 4) as f64 * ((i % 7) as f64 - 3.0) * 0.0001)) as f32)
                .collect();
            let norm = row
                .iter()
                .map(|x| f64::from(*x).powi(2))
                .sum::<f64>()
                .sqrt();
            for x in &mut row {
                *x = (f64::from(*x) / norm) as f32;
            }
            vectors.push(row);
        }
        let value = Embedding { vectors };
        let key = Store::regional_key(&photo.to_string());
        if mode == "binary" {
            // Persist just during setup to measure the actual on-disk envelope size.
            store.persistent(true);
            store.put_regional(&key, &value)?;
            bytes += std::fs::metadata(root.path().join(format!("{key}.bin")))?.len();
            store.persistent(false);
        } else {
            let encoded = serde_json::to_vec(&value)?;
            bytes += encoded.len() as u64;
            std::fs::write(root.path().join(format!("{key}.json")), encoded)?;
        }
        inputs.push(Input {
            key,
            portrait: false,
        });
    }
    let mut runs = Vec::new();
    for round in 0..4 {
        let start = Instant::now();
        let groups = regional::cluster(
            &inputs,
            regional::DEFAULT_EPS,
            |key| {
                let value: Embedding = if mode == "binary" {
                    store.regional(key).context("Binary evidence missing")?
                } else {
                    let path = root.path().join(format!("{key}.json"));
                    let value: Embedding = serde_json::from_slice(&std::fs::read(&path)?)?;
                    ensure!(
                        path.is_file() && value.valid_model_output(),
                        "JSON evidence invalid"
                    );
                    value
                };
                Features::new(value.vectors)
            },
            || false,
        )?
        .context("Unexpected cancellation")?;
        let elapsed = start.elapsed().as_secs_f64();
        ensure!(
            groups
                .labels
                .iter()
                .enumerate()
                .all(|(i, label)| *label == i / 4 + 1),
            "Grouping changed"
        );
        if round > 0 {
            runs.push(serde_json::json!({"seconds":elapsed,"evidence_loads":groups.stats.evidence_loads,"comparisons":groups.stats.comparisons,"unique_records":groups.stats.unique_records,"peak_resident_records":groups.stats.peak_resident_records}));
        }
    }
    println!(
        "{}",
        serde_json::json!({"mode":mode,"records":count,"cache_bytes":bytes,"groups":count/4,"warmup_passes":1,"runs":runs})
    );
    Ok(())
}
