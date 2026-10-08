//! Isolated ONNX session-memory experiment, not another application model registry.
//! Usage: bench_model_memory <models-directory> <policy> [init-only | export <new-directory> | reuse-aesthetic <directory>]
//! Set PHOTOSORT_PROBE_THREADS to override the default four CPU threads.
//! Reload exported models with no-optimization in a separate process.
//! Policies: production, default, no-arena, device-initializers, no-prepacking, basic, no-optimization,
//! no-constant-folding, no-transpose, no-matmul-add.
//! Uses deterministic tensors and keeps both sessions alive to measure combined use.
#[cfg(feature = "onnx")]
fn main() -> anyhow::Result<()> {
    probe::run()
}
#[cfg(not(feature = "onnx"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Build this probe with the onnx feature")
}
#[cfg(feature = "onnx")]
mod probe {
    use anyhow::{Context, Result, ensure};
    use ort::{
        ep,
        session::{Session, builder::GraphOptimizationLevel},
        value::Tensor,
    };
    use serde_json::{Value, json};
    use std::{path::PathBuf, time::Instant};

    fn resident_kib() -> Option<u64> {
        #[cfg(unix)]
        {
            let output = std::process::Command::new("ps")
                .args(["-o", "rss=", "-p", &std::process::id().to_string()])
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
            std::str::from_utf8(&output.stdout)
                .ok()?
                .trim()
                .parse()
                .ok()
        }
        #[cfg(not(unix))]
        {
            None
        }
    }
    pub fn run() -> Result<()> {
        let directory = PathBuf::from(
            std::env::args_os()
                .nth(1)
                .context("Models directory required")?,
        );
        let policy = std::env::args().nth(2).context("Policy required")?;
        ensure!(
            [
                "production",
                "default",
                "no-arena",
                "device-initializers",
                "no-prepacking",
                "basic",
                "no-optimization",
                "no-constant-folding",
                "no-transpose",
                "no-matmul-add"
            ]
            .contains(&policy.as_str()),
            "Unknown policy"
        );
        let mut extra = std::env::args_os().skip(3);
        let (init_only, export, reuse) = match extra.next().as_deref() {
            None => (false, None, None),
            Some(mode) if mode == "init-only" => (true, None, None),
            Some(mode) if mode == "export" => (
                false,
                Some(PathBuf::from(
                    extra.next().context("Export directory required")?,
                )),
                None,
            ),
            Some(mode) if mode == "reuse-aesthetic" => (
                false,
                None,
                Some(PathBuf::from(
                    extra.next().context("Prepared directory required")?,
                )),
            ),
            Some(_) => anyhow::bail!(
                "Expected init-only, export <new-directory> or reuse-aesthetic <directory>"
            ),
        };
        ensure!(extra.next().is_none(), "Unexpected argument");
        let threads = std::env::var("PHOTOSORT_PROBE_THREADS")
            .ok()
            .map(|value| value.parse::<usize>())
            .transpose()
            .context("Invalid PHOTOSORT_PROBE_THREADS")?
            .unwrap_or(4);
        ensure!(
            threads > 0 && threads <= std::thread::available_parallelism()?.get(),
            "Invalid CPU thread count"
        );
        if let Some(path) = &export {
            // Diagnostic output must never replace the supplied model directory.
            std::fs::create_dir(path).context("Export requires a new directory")?;
        }
        let runtime = photosort::core::models::runtime_path().context("Native runtime required")?;
        ort::init_from(&runtime)?
            .with_name("PhotoSort session-memory experiment")
            .with_telemetry(false)
            .commit();
        let mut stages: Vec<Value> =
            vec![json!({"stage":"before_sessions","rss_kib":resident_kib()})];
        let mut sessions = Vec::new();
        for (name, edge, batches) in [
            ("aesthetic.onnx", 384usize, vec![1usize]),
            ("dinov2-small.onnx", 224usize, vec![1usize, 5usize]),
        ] {
            let start = Instant::now();
            // Match the registry's sequential, non-spinning CPU configuration.
            // These isolated alternatives intentionally do not alter application state.
            let mut builder = Session::builder()?
                .with_intra_threads(threads)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .with_parallel_execution(false)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .with_intra_op_spinning(false)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?
                .with_inter_op_spinning(false)
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            if policy == "no-arena" {
                builder = builder
                    .with_execution_providers([ep::CPU::default()
                        .with_arena_allocator(false)
                        .build()
                        .error_on_failure()])
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            } else if policy == "device-initializers" {
                builder = builder
                    .with_device_allocated_initializers()
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            }
            builder = match policy.as_str() {
                "production"
                    if cfg!(all(target_os = "macos", target_arch = "aarch64"))
                        && name == "aesthetic.onnx"
                        && (2..=4).contains(&threads) =>
                {
                    builder.with_disabled_optimizers("MatMulAddFusion")
                }
                "no-prepacking" => builder.with_prepacking(false),
                "no-constant-folding" => builder.with_disabled_optimizers("ConstantFolding"),
                "no-transpose" => builder.with_disabled_optimizers("TransposeOptimizer"),
                "no-matmul-add" => builder.with_disabled_optimizers("MatMulAddFusion"),
                "basic" => builder.with_optimization_level(GraphOptimizationLevel::Level1),
                "no-optimization" => {
                    builder.with_optimization_level(GraphOptimizationLevel::Disable)
                }
                _ => Ok(builder),
            }
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            if let Some(path) = &export {
                builder = builder
                    .with_optimized_model_path(path.join(name))
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            }
            let input = if let Some(path) = reuse.as_ref().filter(|_| name == "aesthetic.onnx") {
                builder = builder
                    .with_optimization_level(GraphOptimizationLevel::Disable)
                    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
                path.join(name)
            } else {
                directory.join(name)
            };
            let mut session = builder.commit_from_file(input)?;
            let init_ms = start.elapsed().as_secs_f64() * 1000.;
            stages.push(
                json!({"stage":"initialized","model":name,"ms":init_ms,"rss_kib":resident_kib()}),
            );
            for batch in batches.into_iter().filter(|_| !init_only) {
                let mut times = Vec::new();
                let mut hashes = Vec::new();
                let mut shape = Vec::new();
                for round in 0..7 {
                    let input: Vec<f32> = (0..batch * 3 * edge * edge)
                        .map(|i| ((i * 31 + round % 3 * 17) % 256) as f32 / 128. - 1.)
                        .collect();
                    let tensor =
                        Tensor::from_array(([batch, 3, edge, edge], input.into_boxed_slice()))?;
                    let start = Instant::now();
                    let outputs = session.run(ort::inputs![tensor])?;
                    times.push(start.elapsed().as_secs_f64() * 1000.);
                    let (dimensions, values) = outputs[0].try_extract_tensor::<f32>()?;
                    shape = dimensions.to_vec();
                    ensure!(
                        values.iter().all(|v| v.is_finite()),
                        "Non-finite model output"
                    );
                    let mut hash = blake3::Hasher::new();
                    for value in values {
                        hash.update(&value.to_le_bytes());
                    }
                    hashes.push(hash.finalize().to_hex().to_string());
                }
                let mut warm = times[1..].to_vec();
                warm.sort_by(f64::total_cmp);
                stages.push(json!({"stage":"inference","model":name,"batch":batch,"times_ms":times,
                    "warm_median_ms":(warm[2]+warm[3])/2.,"output_shape":shape,"output_hashes":hashes,"rss_kib":resident_kib()}));
            }
            sessions.push(session);
        }
        stages.push(json!({"stage":"both_retained","rss_kib":resident_kib()}));
        drop(sessions);
        stages.push(json!({"stage":"after_drop","rss_kib":resident_kib()}));
        println!(
            "{}",
            json!({"policy":policy,"init_only":init_only,"threads":threads,"export":export,"reuse_aesthetic":reuse,"runtime":runtime,"stages":stages,
            "note":"Session-memory diagnostic with deterministic tensors; not an application memory budget or photographic accuracy result."})
        );
        Ok(())
    }
}
