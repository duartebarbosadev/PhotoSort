//! Easy Delete's closed-eyes moments through the real analysis, on a folder.
use photosort::core::{
    analysis::Analysis,
    catalog::Catalog,
    delete_review::{self, Category},
    jobs::{AnalysisProfile, Event, Runtime},
};
use std::{collections::HashMap, path::PathBuf, sync::Arc, time::Duration};

/// Runs Easy Delete's analysis twice (cold, then from the cache) and prints
/// the closed-eyes shots, the other shots of their moment and the cards.
/// Read-only: caches go to a temporary folder.
#[test]
#[ignore = "requires PHOTOSORT_SAMPLE_DIR with closed-eyes photos and installed face models"]
fn sample_closed_eyes_moments_survive_a_warm_cache() -> anyhow::Result<()> {
    let root =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("Set PHOTOSORT_SAMPLE_DIR"));
    let media = Arc::new(Catalog::scan(&root, || false, |_| {})?.media);
    let names: HashMap<_, _> = media
        .iter()
        .map(|m| {
            let name = m.path.strip_prefix(&root).unwrap_or(&m.path);
            (m.id, name.display().to_string())
        })
        .collect();
    let cache = tempfile::tempdir()?;
    let runtime = Runtime::with_cache(|| {}, cache.path().into());
    let mut found = Vec::new();
    for pass in ["cold", "warm"] {
        let run = runtime
            .analyze(media.clone(), AnalysisProfile::Duplicates)
            .map_err(anyhow::Error::msg)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(600);
        let mut result = None;
        loop {
            anyhow::ensure!(std::time::Instant::now() < deadline, "{pass} run timed out");
            match runtime.events.recv_timeout(Duration::from_millis(500)) {
                Ok(envelope) => match envelope.event {
                    Event::DuplicatePairs {
                        run: r,
                        pairs,
                        blinks,
                    } if r == run => result = Some((pairs, blinks)),
                    Event::AnalysisDone { run: r, .. } if r == run => break,
                    Event::RunError { run: r, error } if r == run => anyhow::bail!(error),
                    _ => {}
                },
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(error) => anyhow::bail!(error),
            }
        }
        let (pairs, blinks) = result.expect("pairs before completion");
        println!(
            "{pass}: {} pairs, {} closed-eyes shots",
            pairs.len(),
            blinks.len()
        );
        for blink in &blinks {
            println!("  {}", names[&blink.frame.id]);
            for other in &blink.others {
                let better = photosort::core::eyes::better_shot(&blink.frame.people, &other.people);
                println!(
                    "    {:+.1} s {}{}",
                    (other.at_ms - blink.frame.at_ms) as f64 / 1000.0,
                    names[&other.id],
                    if better.is_some() {
                        " (better shot)"
                    } else {
                        ""
                    }
                );
            }
        }
        let analysis: HashMap<_, _> = media.iter().map(|m| (m.id, Analysis::default())).collect();
        let rows =
            delete_review::from_analysis(&media, &analysis, &HashMap::new(), &pairs, &blinks);
        for category in Category::ALL {
            for e in rows.iter().filter(|e| e.category == category) {
                println!(
                    "  card {:?}: {} -> {}",
                    category,
                    names[&e.subject],
                    e.companion.map_or("(kept)".into(), |c| names[&c].clone())
                );
            }
        }
        found.push(blinks.len());
    }
    assert!(found[0] > 0, "No closed-eyes shots found");
    assert_eq!(
        found[0], found[1],
        "The warm run found other closed-eyes shots"
    );
    Ok(())
}
