//! Opt-in evidence gathering through the production image/model owners.
use super::*;
use crate::core::{catalog::Catalog, operations, pipeline::Pipeline};

fn views(models: &mut Models, model: &Path, asset: &Asset) -> Result<serde_json::Value> {
    let input = orientation_input(asset);
    let mut records = Vec::new();
    let mut average = [0.0_f32; 4];
    for turns in 0..4 {
        let logits = models
            .infer_all(
                "orientation",
                model,
                384,
                rotate_orientation_tensor(&input, 384, turns),
            )?
            .remove(0);
        let (vote, confidence) = orientation_vote(&logits)?;
        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let sum: f32 = logits.iter().map(|v| (v - max).exp()).sum();
        let mut mapped = [0.0; 4];
        for (i, v) in logits.iter().enumerate() {
            let index = (i + usize::from(turns)) % 4;
            mapped[index] = (v - max).exp() / sum;
            average[index] += mapped[index] / 4.0;
        }
        records.push(serde_json::json!({
            "input_turns": turns, "mapped_vote": (vote + turns) % 4,
            "confidence": confidence, "mapped_probabilities": mapped,
        }));
    }
    let production = models.rotation(model, asset)?;
    Ok(serde_json::json!({
        "dimensions": asset.pixels.dimensions(),
        "pixel_blake3": blake3::hash(asset.pixels.as_raw()).to_hex().to_string(),
        "views": records, "average_probabilities": average,
        "production": {"turns":production.turns,"confidence":production.confidence,
            "uncertain":production.uncertain,"views":production.views},
    }))
}

#[test]
#[ignore = "read-only diagnostic requiring PHOTOSORT_SAMPLE_DIR, native model/runtime and PHOTOSORT_ORIENTATION_AUDIT_OUTPUT"]
fn inspect_rotation_view_evidence_without_mutating_samples() -> Result<()> {
    let root =
        PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").context("Set sample folder")?);
    let output = PathBuf::from(
        std::env::var_os("PHOTOSORT_ORIENTATION_AUDIT_OUTPUT").context("Set audit output")?,
    );
    let catalog = Catalog::scan(&root, || false, |_| {})?;
    let model = orientation_model().context("Install orientation model")?;
    let cache = tempfile::tempdir()?;
    let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
    let mut models = Models::default();
    // Match the native UI audit while keeping one shared session for every view.
    models.configure_threads(4);
    let mut records = Vec::new();
    for media in &catalog.media {
        let before = operations::content_hash(&media.path, || false)?;
        let analysis = pipeline.analysis(media, || false)?;
        let analysis_views = views(&mut models, &model, &analysis)?;
        let decodes = pipeline.stats.decodes;
        let display = pipeline.load_neutral_cancellable(media, || false)?;
        let display_views = views(&mut models, &model, &display)?;
        assert_eq!(
            pipeline.stats.decodes, decodes,
            "Audit duplicated source decode"
        );
        assert_eq!(operations::content_hash(&media.path, || false)?, before);
        records.push(serde_json::json!({"name":media.name(),"analysis":analysis_views,"display":display_views}));
    }
    assert_eq!(models.initializations, 1);
    std::fs::write(
        output,
        serde_json::to_vec_pretty(&serde_json::json!({
            "native_imaging":cfg!(feature="native-imaging"),"model":model,
            "source_decodes":pipeline.stats.decodes,"initializations":models.initializations,
            "inferences":models.inferences,"records":records,
            "limits":"Softmax values are not calibrated correctness probabilities. No accuracy claim without reviewed labels."
        }))?,
    )?;
    Ok(())
}
