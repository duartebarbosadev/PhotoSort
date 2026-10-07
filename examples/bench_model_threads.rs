//! Local DINO thread-budget probe. Reads one photo; does not mutate it.
use photosort::core::{catalog::Catalog, models::Models, pipeline::Pipeline, processing};
use std::{path::PathBuf, time::Instant};
fn main() -> anyhow::Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).expect("sample folder"));
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    let media = catalog
        .media
        .iter()
        .find(|m| m.name() == "jpg_sample.jpg")
        .expect("JPEG sample");
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let asset = pipeline.load(media)?;
    let available = processing::available();
    let mut budgets = vec![
        1,
        2.min(available),
        4.min(available),
        8.min(available),
        processing::Preferences::default().threads(available),
        available,
    ];
    budgets.sort_unstable();
    budgets.dedup();
    let mut models = Models::default();
    let mut reference = None::<Vec<f32>>;
    for budget in budgets {
        models.configure_threads(budget);
        let cold = Instant::now();
        let first = models.embedding(&asset)?;
        let cold = cold.elapsed();
        if let Some(reference) = &reference {
            let error = reference
                .iter()
                .zip(&first)
                .map(|(a, b)| (a - b).abs())
                .fold(0f32, f32::max);
            anyhow::ensure!(error < 1e-5, "Thread count changed descriptor by {error}");
        } else {
            reference = Some(first);
        }
        let mut times = Vec::new();
        for _ in 0..5 {
            let start = Instant::now();
            std::hint::black_box(models.embedding(&asset)?);
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
        times.sort_by(f64::total_cmp);
        println!(
            "{budget} threads: cold {:.1} ms, warm median {:.1} ms",
            cold.as_secs_f64() * 1000.,
            times[2]
        );
    }
    assert_eq!(pipeline.stats.decodes, 1);
    Ok(())
}
