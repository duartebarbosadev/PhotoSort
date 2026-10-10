//! Native provider construction belongs to the shared model registry only.
use super::Backend;
use anyhow::{Result, ensure};
use ort::{
    ep::{self, ExecutionProvider},
    session::Session,
    value::Tensor,
};
use std::path::{Path, PathBuf};

pub(super) struct NativeSession {
    // Declaration order matters: native readers close before releasing the lease.
    inner: Session,
    #[cfg(target_os = "macos")]
    cache: Option<crate::core::cache::compiled::Lease>,
    cache_detail: String,
}
impl std::ops::Deref for NativeSession {
    type Target = Session;
    fn deref(&self) -> &Session {
        &self.inner
    }
}
impl std::ops::DerefMut for NativeSession {
    fn deref_mut(&mut self) -> &mut Session {
        &mut self.inner
    }
}
impl NativeSession {
    pub fn cached(&self) -> bool {
        #[cfg(target_os = "macos")]
        {
            self.cache.is_some()
        }
        #[cfg(not(target_os = "macos"))]
        {
            false
        }
    }
    pub fn invalidate(&mut self) {
        #[cfg(target_os = "macos")]
        if let Some(cache) = &mut self.cache {
            cache.invalidate();
        }
    }
}
#[cfg(target_os = "macos")]
/// Core ML compiles each model to a device-specific program; keep it between runs.
/// CPU sessions need no prepared copy now that the large aesthetic export ships
/// with its constants folded (see examples/fold_model.rs).
fn cached_model(
    path: &Path,
    kind: &str,
    config: Option<&(PathBuf, u64)>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Option<crate::core::cache::compiled::Lease>> {
    let Some((root, budget)) = config.filter(|(_, b)| *b > 0) else {
        return Ok(None);
    };
    // Only the verified, self-contained catalog exports are cacheable. An
    // arbitrary ONNX override may reference external weights outside this file.
    let Some(asset) = crate::core::model_assets::ALL
        .iter()
        .find(|a| path.file_name().is_some_and(|n| n == a.name))
    else {
        return Ok(None);
    };
    ensure!(
        crate::core::model_assets::Inspector::default().verify(asset, path, &|| cancelled())?,
        "Custom model is not eligible for prepared-model caching"
    );
    let runtime = super::runtime_path().ok_or_else(|| anyhow::anyhow!("Missing native runtime"))?;
    let system = std::fs::read("/System/Library/CoreServices/SystemVersion.plist")?;
    let recipe = format!(
        "mlprogram-all-fp32-v1:{kind}:{}:{}:{}",
        std::env::consts::ARCH,
        ort::info(),
        blake3::hash(&system)
    );
    let key = crate::core::cache::compiled::key(path, &runtime, &recipe, cancelled)?;
    Ok(Some(crate::core::cache::compiled::Lease::acquire(
        crate::core::cache::compiled::Format::CoreMl,
        root,
        &key,
        *budget,
        cancelled,
    )?))
}
// On Apple Silicon at 2–4 CPU threads, separate MatMul/Add kernels are faster
// for our BeIT aesthetic export. Other budgets/providers retain ORT defaults. See
// docs/diagnostics/cpu-aesthetic-2026-10-08 for the real-photo comparison.
fn disabled_optimizers(kind: &str, backend: Backend, threads: usize) -> Option<&'static str> {
    (cfg!(all(target_os = "macos", target_arch = "aarch64"))
        && backend == Backend::Cpu
        && kind == "aesthetic"
        && (2..=4).contains(&threads))
    .then_some("MatMulAddFusion")
}
fn build(
    path: &Path,
    kind: &str,
    threads: usize,
    backend: Backend,
    cache_config: Option<&(PathBuf, u64)>,
    cancelled: &dyn Fn() -> bool,
) -> Result<NativeSession> {
    #[cfg(target_os = "macos")]
    let mut cache = None;
    let mut cache_detail = String::new();
    let _ = (&cache_config, &cancelled, &mut cache_detail);
    let mut builder = Session::builder()?
        .with_intra_threads(threads)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .with_parallel_execution(false)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .with_intra_op_spinning(false)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?
        .with_inter_op_spinning(false)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    if let Some(optimizers) = disabled_optimizers(kind, backend, threads) {
        builder = builder
            .with_disabled_optimizers(optimizers)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }
    let provider = match backend {
        Backend::Cpu => None,
        #[cfg(target_os = "macos")]
        Backend::CoreMl => {
            let mut provider = ep::CoreML::default()
                .with_model_format(ep::coreml::ModelFormat::MLProgram)
                .with_compute_units(ep::coreml::ComputeUnits::All);
            ensure!(
                provider.is_available()?,
                "Core ML is not included in this runtime"
            );
            match cached_model(path, kind, cache_config, cancelled) {
                Ok(value) => cache = value,
                Err(error) => {
                    cache_detail = format!(" · Prepared-model cache unavailable: {error}")
                }
            }
            ensure!(!cancelled(), "Model preparation cancelled");
            if let Some(entry) = &cache {
                provider = provider.with_model_cache_dir(entry.path.display());
                cache_detail = if entry.reused {
                    " · Prepared model reused"
                } else {
                    " · Prepared model cached"
                }
                .into();
            }
            // DINO's export has four symbolic axes. One fixed image yields a
            // single Core ML partition; crop batches reuse this same session.
            if kind == "dinov2" {
                for (name, size) in [
                    ("batch_size", 1),
                    ("num_channels", 3),
                    ("height", 224),
                    ("width", 224),
                ] {
                    builder = builder
                        .with_dimension_override(name, size)
                        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                }
            }
            Some(provider.build().error_on_failure())
        }
        #[cfg(any(target_os = "windows", target_os = "linux"))]
        Backend::Cuda => {
            let provider = ep::CUDA::default()
                .with_conv_algorithm_search(ep::cuda::ConvAlgorithmSearch::Heuristic)
                .with_conv_max_workspace(false)
                .with_arena_extend_strategy(ep::ArenaExtendStrategy::SameAsRequested)
                .with_tf32(false);
            ensure!(
                provider.is_available()?,
                "CUDA is not included in this runtime"
            );
            Some(provider.build().error_on_failure())
        }
        #[cfg(target_os = "windows")]
        Backend::DirectMl => {
            let provider = ep::DirectML::default();
            ensure!(
                provider.is_available()?,
                "DirectML is not included in this runtime"
            );
            // DirectML requires sequential execution and disabled memory patterns.
            builder = builder
                .with_memory_pattern(false)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            Some(provider.build().error_on_failure())
        }
        _ => anyhow::bail!("{} is not supported on this platform", backend.label()),
    };
    if let Some(provider) = provider {
        builder = builder
            .with_execution_providers([provider])
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }
    ensure!(!cancelled(), "Model preparation cancelled");
    let session = match builder.commit_from_file(path) {
        Ok(session) => session,
        Err(error) => {
            #[cfg(target_os = "macos")]
            if let Some(entry) = &mut cache {
                entry.invalidate();
            }
            return Err(error.into());
        }
    };
    ensure!(
        session.inputs().len() == 1,
        "Model must have one image input"
    );
    #[cfg(target_os = "macos")]
    if let Some(entry) = &mut cache
        && let Err(error) = entry.complete(cancelled)
    {
        if !entry.reused || !cancelled() {
            entry.invalidate();
        }
        cache_detail = format!(" · Prepared-model cache not retained: {error}");
    }
    #[cfg(target_os = "macos")]
    if cache.as_ref().is_some_and(|entry| !entry.retained())
        && !cache_detail.contains("not retained")
    {
        cache_detail = " · Prepared model exceeds disk budget; not retained".into();
    }
    ensure!(!cancelled(), "Model preparation cancelled");
    Ok(NativeSession {
        inner: session,
        #[cfg(target_os = "macos")]
        cache,
        cache_detail,
    })
}
/// Attempts are per model, with one CPU fallback; cancellation never starts
/// another provider. Native compilation itself is not interruptible via ORT.
fn choose<T>(
    requested: Backend,
    cancelled: &dyn Fn() -> bool,
    mut attempt: impl FnMut(Backend) -> Result<T>,
) -> Result<(T, Backend, String)> {
    let mut failures = Vec::new();
    for backend in requested.candidates() {
        ensure!(!cancelled(), "Model initialization cancelled");
        match attempt(backend) {
            Ok(session) => {
                ensure!(!cancelled(), "Model initialization cancelled");
                return Ok((
                    session,
                    backend,
                    "Provider enabled; unsupported operations may use CPU".into(),
                ));
            }
            Err(error) => failures.push(format!("{}: {error}", backend.label())),
        }
    }
    ensure!(!cancelled(), "Model initialization cancelled");
    let session = attempt(Backend::Cpu)?;
    ensure!(!cancelled(), "Model initialization cancelled");
    let detail = if failures.is_empty() {
        "CPU execution".into()
    } else {
        format!("CPU fallback · {}", failures.join("; "))
    };
    Ok((session, Backend::Cpu, detail))
}
pub(super) fn create(
    path: &Path,
    kind: &str,
    threads: usize,
    requested: Backend,
    cancelled: &dyn Fn() -> bool,
    cache: Option<&(PathBuf, u64)>,
) -> Result<(NativeSession, Backend, String)> {
    let (session, backend, mut detail) = choose(requested, cancelled, |backend| {
        build(path, kind, threads, backend, cache, cancelled)
    })?;
    detail.push_str(&session.cache_detail);
    Ok((session, backend, detail))
}
pub(super) fn run(
    session: &mut Session,
    tensor: &Tensor<f32>,
    batch: usize,
) -> Result<Vec<Vec<f32>>> {
    let outputs = session.run(ort::inputs![tensor])?;
    ensure!(outputs.len() > 0, "Model returned no outputs");
    outputs
        .iter()
        .map(|(_, output)| {
            let (shape, values) = output.try_extract_tensor::<f32>()?;
            ensure!(
                batch == 1 || shape.first() == Some(&(batch as i64)),
                "Model output batch mismatch"
            );
            ensure!(
                !values.is_empty() && values.iter().all(|v| v.is_finite()),
                "Model returned invalid data"
            );
            Ok(values.to_vec())
        })
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepted_run_cancellation_reaches_plain_inference_before_model_io() {
        use crate::core::{models::Models, processing::RunConfig};
        let mut models = Models::default();
        models.begin_run(
            RunConfig {
                threads: 1,
                backend: Backend::CoreMl,
            },
            || true,
        );
        let result = models.infer_all(
            "cancelled",
            Path::new("/missing/model.onnx"),
            1,
            vec![0.; 3],
        );
        assert!(result.unwrap_err().to_string().contains("cancelled"));
        assert_eq!((models.initializations, models.inferences), (0, 0));
    }
    #[test]
    fn missing_provider_falls_back_once_and_reports_the_reason() {
        let mut attempts = Vec::new();
        let (_, backend, detail) = choose(Backend::Cuda, &|| false, |backend| {
            attempts.push(backend);
            if backend == Backend::Cuda {
                anyhow::bail!("missing CUDA driver");
            }
            Ok(())
        })
        .unwrap();
        assert_eq!(attempts, vec![Backend::Cuda, Backend::Cpu]);
        assert_eq!(backend, Backend::Cpu);
        assert!(detail.contains("missing CUDA driver"));
    }
    #[test]
    fn cancelled_provider_failure_does_not_initialize_cpu() {
        let cancelled = std::cell::Cell::new(false);
        let mut attempts = 0;
        assert!(
            choose::<()>(Backend::CoreMl, &|| cancelled.get(), |_| {
                attempts += 1;
                cancelled.set(true);
                anyhow::bail!("compile failed")
            })
            .is_err()
        );
        assert_eq!(attempts, 1);
    }
    #[test]
    fn successful_provider_and_cpu_requests_never_initialize_another_session() {
        for expected in [
            Backend::Cpu,
            Backend::CoreMl,
            Backend::Cuda,
            Backend::DirectMl,
        ] {
            let mut attempts = 0;
            let (_, selected, _) = choose(expected, &|| false, |_| {
                attempts += 1;
                Ok(())
            })
            .unwrap();
            assert_eq!(selected, expected);
            assert_eq!(attempts, 1);
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod native_tests {
    use super::*;
    use crate::core::{
        catalog::Catalog,
        models::Models,
        pipeline::Pipeline,
        processing::{Control, RunConfig},
    };
    #[cfg(target_arch = "aarch64")]
    #[test]
    #[ignore = "Runs installed CPU aesthetic model on seven read-only samples"]
    fn cpu_aesthetic_reuses_samples_and_honors_cancellation_and_fallback() -> Result<()> {
        let sample_root =
            PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("PHOTOSORT_SAMPLE_DIR"));
        let catalog = Catalog::scan(&sample_root, || false, |_| {})?;
        ensure!(catalog.media.len() == 7, "Expected seven samples");
        let cache = tempfile::tempdir()?;
        let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
        let assets = catalog
            .media
            .iter()
            .map(|m| pipeline.analysis(m, || false))
            .collect::<Result<Vec<_>>>()?;
        let decodes = pipeline.stats.decodes;
        let control = Control::default();
        let mut models = Models::with_processing(control.clone());
        let config = RunConfig {
            threads: 4,
            backend: Backend::Cpu,
        };
        models.begin_run(config, || false);
        let reference = assets
            .iter()
            .map(|a| models.aesthetic(a))
            .collect::<Result<Vec<_>>>()?;
        ensure!(
            reference
                .iter()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v)),
            "Invalid score"
        );
        assert_eq!((models.initializations, models.inferences), (1, 7));

        models.begin_run(config, || true);
        assert!(
            models
                .aesthetic(&assets[0])
                .unwrap_err()
                .to_string()
                .contains("cancelled")
        );
        assert_eq!((models.initializations, models.inferences), (1, 7));
        models.begin_run(config, || false);
        assert_eq!(
            reference,
            assets
                .iter()
                .map(|a| models.aesthetic(a))
                .collect::<Result<Vec<_>>>()?
        );
        assert_eq!((models.initializations, models.inferences), (1, 14));

        models.begin_run(
            RunConfig {
                backend: Backend::Cuda,
                ..config
            },
            || false,
        );
        assert_eq!(
            reference,
            assets
                .iter()
                .map(|a| models.aesthetic(a))
                .collect::<Result<Vec<_>>>()?
        );
        assert_eq!((models.initializations, models.inferences), (2, 21));
        let status = control.status();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].backend, Backend::Cpu);
        assert!(status[0].detail.contains("not supported on this platform"));
        assert_eq!(pipeline.stats.decodes, decodes);
        println!(
            "{}",
            serde_json::json!({"samples":7,"scores":reference,
            "decodes":decodes,"initializations":models.initializations,
            "inferences":models.inferences,"cancelled_run_inferences":0,
            "repeat_and_fallback_scores_exact":true})
        );
        Ok(())
    }
    #[test]
    #[ignore = "Runs installed Core ML with disposable persistent cache and seven read-only samples"]
    fn prepared_coreml_cache_reuses_samples_and_releases_sessions_for_maintenance() -> Result<()> {
        use crate::core::cache;
        let sample_root =
            PathBuf::from(std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("PHOTOSORT_SAMPLE_DIR"));
        let catalog = Catalog::scan(&sample_root, || false, |_| {})?;
        ensure!(catalog.media.len() == 7, "Expected seven samples");
        let temp = tempfile::tempdir()?;
        let root = temp.path().canonicalize()?;
        let model_cache = root.join("prepared");
        let mut pipeline = Pipeline::new(128 * 1024 * 1024, root.join("photos"));
        let assets = catalog
            .media
            .iter()
            .map(|m| pipeline.analysis(m, || false))
            .collect::<Result<Vec<_>>>()?;
        let decodes = pipeline.stats.decodes;
        let budget = 1024 * cache::MIB;
        let control = Control::default();
        let config = RunConfig {
            threads: control.snapshot(),
            backend: Backend::CoreMl,
        };
        let mut models = Models::with_processing(control.clone());
        models.configure(config);
        models.configure_compiled_cache(model_cache.clone(), budget, false);
        let start = std::time::Instant::now();
        let first = assets
            .iter()
            .map(|a| models.embedding(a))
            .collect::<Result<Vec<_>>>()?;
        let fresh_seven_ms = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(models.initializations, 1);
        assert!(control.status().iter().any(|s| s.backend == Backend::CoreMl && s.detail.contains("Prepared model cached")), "{:?}", control.status());
        let held = cache::maintain(&model_cache, Some(0), || false)?;
        assert!(held.disk_bytes > 0 && held.disk_bytes == held.pinned_disk_bytes);
        models.configure_compiled_cache(model_cache.clone(), budget * 2, false);
        assert_eq!(models.embedding(&assets[0])?, first[0]);
        assert_eq!(
            models.initializations, 1,
            "Increasing budget reinitialized shared model"
        );
        drop(models);
        let released = cache::maintain(&model_cache, None, || false)?;
        assert_eq!(released.pinned_disk_bytes, 0);
        assert!(released.disk_bytes > 0);

        let mut models = Models::with_processing(control.clone());
        models.configure(config);
        models.configure_compiled_cache(model_cache.clone(), budget, false);
        let start = std::time::Instant::now();
        let reused = assets
            .iter()
            .map(|a| models.embedding(a))
            .collect::<Result<Vec<_>>>()?;
        let reused_seven_ms = start.elapsed().as_secs_f64() * 1000.;
        assert_eq!(first, reused);
        assert!(control.status().iter().any(|s| s.backend == Backend::CoreMl && s.detail.contains("Prepared model reused")), "{:?}", control.status());
        assert_eq!(models.initializations, 1);
        models.configure_compiled_cache(model_cache.clone(), budget / 2, false);
        assert!(
            models.sessions.is_empty(),
            "Shrinking budget left native readers open"
        );
        assert!(control.status().is_empty());
        assert_eq!(
            cache::maintain(&model_cache, None, || false)?.pinned_disk_bytes,
            0
        );
        assert_eq!(models.embedding(&assets[0])?, first[0]);
        assert_eq!(models.initializations, 2);
        assert!(control.status()[0].detail.contains("Prepared model reused"));
        models.configure_compiled_cache(model_cache.clone(), budget / 2, true);
        assert!(models.sessions.is_empty());
        assert!(control.status().is_empty());
        assert_eq!(
            cache::maintain(&model_cache, Some(0), || false)?.disk_bytes,
            0
        );

        // Cache maintenance must not destroy a CPU session or initialize a model.
        models.configure(RunConfig {
            backend: Backend::Cpu,
            ..config
        });
        let cpu = models.embedding(&assets[0])?;
        let initialized = models.initializations;
        models.configure_compiled_cache(model_cache.clone(), 0, true);
        assert_eq!(models.embedding(&assets[0])?, cpu);
        assert_eq!(models.initializations, initialized);
        assert_eq!(control.status()[0].backend, Backend::Cpu);
        assert_eq!(cache::maintain(&model_cache, None, || false)?.disk_bytes, 0);
        assert_eq!(pipeline.stats.decodes, decodes);
        println!(
            "{}",
            serde_json::json!({"samples":7,"fresh_seven_ms":fresh_seven_ms,"reused_seven_ms":reused_seven_ms,"cache_bytes":released.disk_bytes,"exact_outputs":first == reused,"decodes":decodes,"clear_and_shrink_release_sessions":true,"cpu_session_preserved":true})
        );
        Ok(())
    }
    #[test]
    #[ignore = "Runs installed Core ML DINO on all seven original samples, read only"]
    fn seven_samples_coreml_matches_cpu_and_reuses_one_session() -> Result<()> {
        let root = std::path::PathBuf::from(
            std::env::var_os("PHOTOSORT_SAMPLE_DIR").expect("PHOTOSORT_SAMPLE_DIR"),
        );
        let catalog = Catalog::scan(&root, || false, |_| {})?;
        ensure!(catalog.media.len() == 7, "Expected seven samples");
        let cache = tempfile::tempdir()?;
        let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
        let assets = catalog
            .media
            .iter()
            .map(|m| pipeline.analysis(m, || false))
            .collect::<Result<Vec<_>>>()?;
        let decodes = pipeline.stats.decodes;
        let control = Control::default();
        let mut models = Models::with_processing(control.clone());
        let config = RunConfig {
            threads: control.snapshot(),
            backend: Backend::Cpu,
        };
        models.configure(config);
        let reference = assets
            .iter()
            .map(|a| models.embedding(a))
            .collect::<Result<Vec<_>>>()?;
        let regional = models.similarity_regions(&assets[0], &reference[0], || false)?;
        let initializations = models.initializations;
        models.configure(RunConfig {
            backend: Backend::CoreMl,
            ..config
        });
        let started = std::time::Instant::now();
        let actual = assets
            .iter()
            .map(|a| models.embedding(a))
            .collect::<Result<Vec<_>>>()?;
        let cold_ms = started.elapsed().as_secs_f64() * 1000.;
        let mut max_error = 0f32;
        for (a, b) in actual.iter().flatten().zip(reference.iter().flatten()) {
            max_error = max_error.max((a - b).abs());
        }
        ensure!(
            max_error < 0.0001,
            "Normalized descriptor error {max_error}"
        );
        let regions = models.similarity_regions(&assets[0], &actual[0], || false)?;
        ensure!(
            regions.vectors.len() == regional.vectors.len(),
            "Crop count changed"
        );
        for (a, b) in regions
            .vectors
            .iter()
            .flatten()
            .zip(regional.vectors.iter().flatten())
        {
            ensure!((a - b).abs() < 0.0001, "Regional descriptor drift");
        }
        ensure!(
            models.initializations == initializations + 1,
            "More than one shared Core ML session initialized"
        );
        ensure!(
            control
                .status()
                .iter()
                .any(|s| s.backend == Backend::CoreMl),
            "Core ML silently fell back"
        );
        let before = models.inferences;
        assert!(
            models
                .similarity_regions(&assets[0], &actual[0], || true)
                .is_err()
        );
        assert_eq!(models.inferences, before);
        assert_eq!(pipeline.stats.decodes, decodes);
        let mut warm_ms = Vec::new();
        for a in &assets {
            let start = std::time::Instant::now();
            models.embedding(a)?;
            warm_ms.push(start.elapsed().as_secs_f64() * 1000.);
        }
        // Force a provider execution error through the fixed input shape. The
        // CPU export accepts 112x112 and must reuse the already-prepared tensor.
        let path = crate::core::models::model_path("dinov2-small.onnx").unwrap();
        let small = vec![0f32; 3 * 112 * 112];
        let fallback = models.infer_all("dinov2", &path, 112, small.clone())?;
        let after_fallback = models.initializations;
        assert_eq!(after_fallback, initializations + 2);
        assert_eq!(models.infer_all("dinov2", &path, 112, small)?, fallback);
        assert_eq!(models.initializations, after_fallback);
        assert_eq!(control.status()[0].backend, Backend::Cpu);
        assert!(control.status()[0].detail.contains("inference error"));
        println!(
            "{}",
            serde_json::json!({"samples":assets.len(),"maximum_normalized_error":max_error,"cold_seven_ms":cold_ms,"warm_image_ms":warm_ms,"decodes":decodes,"initializations":models.initializations})
        );
        Ok(())
    }
    #[test]
    #[ignore = "Runs local native runtime to verify unavailable CUDA fallback stays on one CPU session"]
    fn unavailable_cuda_reuses_cpu_session_without_redecoding() -> Result<()> {
        let control = Control::default();
        let mut models = Models::with_processing(control.clone());
        models.configure(RunConfig {
            threads: 1,
            backend: Backend::Cuda,
        });
        let pixels = std::sync::Arc::new(image::RgbaImage::new(224, 224));
        let asset = crate::core::pipeline::Asset {
            decoder_warning: None,
            thumbnail: pixels.clone(),
            pixels,
            metadata: Default::default(),
        };
        let a = models.embedding(&asset)?;
        let b = models.embedding(&asset)?;
        assert_eq!(a, b);
        assert_eq!(models.initializations, 1);
        assert_eq!(models.inferences, 2);
        let status = control.status();
        assert_eq!(status[0].backend, Backend::Cpu);
        assert!(status[0].detail.contains("not supported on this platform"));
        Ok(())
    }
}
