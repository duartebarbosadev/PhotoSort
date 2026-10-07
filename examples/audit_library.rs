//! Read-only application-worker audit on real folders; all evidence uses a temporary cache.
//! Usage: audit_library <folder> [technical|quality|visual]
//! Reports repeatability and reuse, not accuracy without human-reviewed labels.
use anyhow::{Context, Result, ensure};
use photosort::core::{
    catalog::Catalog,
    jobs::{AnalysisProfile, AnalysisStats, Event, Runtime},
    model::{Media, MediaId},
    processing::{Backend, Mode, Preferences},
};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

fn statistics(s: AnalysisStats) -> Value {
    json!({"decodes":s.decodes,"technical_measurements":s.technical_measurements,
        "quality_measurements":s.quality_measurements,"content_hashes":s.content_hashes,
        "model_initializations":s.model_initializations,"model_inferences":s.model_inferences,
        "model_threads":s.model_threads})
}
fn run(
    rt: &Runtime,
    media: Arc<Vec<Media>>,
    profile: AnalysisProfile,
) -> Result<(AnalysisStats, Value)> {
    let accepted = rt
        .analyze(media.clone(), profile)
        .map_err(anyhow::Error::msg)?;
    let deadline = Instant::now() + Duration::from_secs(1800);
    let mut values = BTreeMap::new();
    let mut groups = Value::Null;
    let mut errors = BTreeMap::new();
    let names: HashMap<MediaId, String> = media
        .iter()
        .map(|m| (m.id, m.path.to_string_lossy().into_owned()))
        .collect();
    loop {
        ensure!(
            Instant::now() < deadline,
            "Analysis exceeded 30-minute deadline"
        );
        let envelope = match rt.events.recv_timeout(Duration::from_secs(1)) {
            Ok(e) => e,
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
            Err(e) => return Err(e.into()),
        };
        match envelope.event {
            Event::Analyzed {
                run,
                id,
                analysis,
                metadata,
                ..
            } if run == accepted => {
                ensure!(
                    analysis.score.is_finite() && analysis.sharpness.is_finite(),
                    "Non-finite measurement"
                );
                ensure!(
                    metadata.width > 0 && metadata.height > 0,
                    "Missing source geometry"
                );
                values.insert(
                    names[&id].clone(),
                    json!({"analysis":analysis,"metadata":metadata}),
                );
            }
            Event::AnalysisError { run, id, error } if run == accepted => {
                errors.insert(names[&id].clone(), error);
            }
            Event::GeneralGroups {
                run,
                groups: labels,
                ..
            }
            | Event::SubjectGroups {
                run,
                groups: labels,
            } if run == accepted => {
                groups = serde_json::to_value(
                    labels
                        .iter()
                        .map(|(id, g)| (names[id].clone(), *g))
                        .collect::<BTreeMap<_, _>>(),
                )?;
            }
            Event::MixedGroups { run, labels } if run == accepted => {
                groups = serde_json::to_value(
                    labels
                        .iter()
                        .map(|(id, g)| (names[id].clone(), g.clone()))
                        .collect::<BTreeMap<_, _>>(),
                )?;
            }
            Event::Progress { run, done, total }
                if run == accepted && (done % 20 == 0 || done == total) =>
            {
                eprintln!("{done}/{total}")
            }
            Event::ModelsNeeded { run, .. } if run == accepted => {
                anyhow::bail!("Required models unavailable")
            }
            Event::Error(error) => anyhow::bail!("{error}"),
            Event::AnalysisDone { run, stats } if run == accepted => {
                ensure!(
                    values.len() + errors.len() == media.len(),
                    "Missing analysis results"
                );
                return Ok((
                    stats,
                    json!({"photos":values,"groups":groups,"errors":errors}),
                ));
            }
            _ => {}
        }
    }
}
fn main() -> Result<()> {
    let root =
        PathBuf::from(std::env::args_os().nth(1).context("Folder required")?).canonicalize()?;
    let mode = std::env::args()
        .nth(2)
        .unwrap_or_else(|| "technical".into());
    let profile = match mode.as_str() {
        "technical" => AnalysisProfile::Technical,
        "quality" => AnalysisProfile::Quality,
        "visual" => AnalysisProfile::Visual,
        _ => anyhow::bail!("Unknown profile"),
    };
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    ensure!(
        !catalog.media.is_empty() && catalog.warnings.is_empty(),
        "Empty library or scan warnings"
    );
    let media = Arc::new(catalog.media);
    let cache = tempfile::tempdir()?;
    let mut phases = Vec::new();
    let mut expected = None;
    for phase in ["cold", "reopened"] {
        let mut rt = Runtime::with_cache(|| {}, cache.path().into());
        rt.processing.configure(Preferences {
            mode: Mode::Custom,
            custom_threads: 4,
            backend: Backend::Cpu,
        });
        let start = Instant::now();
        let result = run(&rt, media.clone(), profile);
        rt.shutdown();
        let (stats, values) = result?;
        let elapsed = start.elapsed().as_secs_f64();
        if let Some(expected) = &expected {
            ensure!(&values == expected, "Results changed after reopening cache");
            ensure!(
                (
                    stats.decodes,
                    stats.technical_measurements,
                    stats.quality_measurements,
                    stats.content_hashes,
                    stats.model_initializations,
                    stats.model_inferences
                ) == (0, 0, 0, 0, 0, 0),
                "Reopened analysis repeated expensive work: {stats:?}"
            );
        } else {
            expected = Some(values);
        }
        phases.push(
            json!({"phase":phase,"seconds_including_shutdown":elapsed,"stats":statistics(stats)}),
        );
        eprintln!("{phase}: {elapsed:.3}s {stats:?}");
    }
    let values = expected.unwrap();
    let failures = values["errors"].as_object().unwrap().len();
    println!(
        "{}",
        json!({"root":root,"profile":mode,"count":media.len(),"native_imaging":cfg!(feature="native-imaging"),"phases":phases,"results":values})
    );
    ensure!(
        failures == 0,
        "{failures} photos failed analysis; see JSON report"
    );
    Ok(())
}
