//! Pre-apply ONNX Runtime's basic (hardware-independent) graph optimizations to a
//! model, then prove the result is bit-identical to the source on fixed inputs.
//! Folding the BeIT aesthetic export offline removes a ~3.5 GiB constant-folding
//! peak from the first analysis on every machine.
//! Usage: fold_model <source.onnx> <new-output.onnx> [input-edge, default 384]
#[cfg(feature = "onnx")]
fn main() -> anyhow::Result<()> {
    fold::run()
}
#[cfg(not(feature = "onnx"))]
fn main() -> anyhow::Result<()> {
    anyhow::bail!("Build this tool with the onnx feature")
}
#[cfg(feature = "onnx")]
mod fold {
    use anyhow::{Context, Result, ensure};
    use ort::{
        session::{Session, builder::GraphOptimizationLevel},
        value::Tensor,
    };
    use sha2::{Digest, Sha256};
    use std::path::{Path, PathBuf};

    pub fn run() -> Result<()> {
        let mut args = std::env::args_os().skip(1);
        let source = PathBuf::from(args.next().context("Source model required")?);
        let output = PathBuf::from(args.next().context("Output path required")?);
        let edge: usize = args
            .next()
            .map(|v| v.to_string_lossy().parse())
            .transpose()?
            .unwrap_or(384);
        ensure!(!output.exists(), "Output must be a new file");
        let runtime = photosort::core::models::runtime_path().context("Native runtime required")?;
        ort::init_from(&runtime)?
            .with_name("PhotoSort model folding")
            .with_telemetry(false)
            .commit();
        Session::builder()?
            .with_intra_threads(1)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .with_optimization_level(GraphOptimizationLevel::Level1)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .with_optimized_model_path(&output)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .commit_from_file(&source)?;
        // Same session settings for both files, so only the graph differs.
        let (mut before, mut after) = (session(&source)?, session(&output)?);
        for round in 0..5 {
            let input: Vec<f32> = (0..3 * edge * edge)
                .map(|i| ((i * 31 + round * 17) % 256) as f32 / 128. - 1.)
                .collect();
            let tensor =
                || Tensor::from_array(([1, 3, edge, edge], input.clone().into_boxed_slice()));
            let a = before.run(ort::inputs![tensor()?])?;
            let b = after.run(ort::inputs![tensor()?])?;
            let (a, b) = (
                a[0].try_extract_tensor::<f32>()?.1,
                b[0].try_extract_tensor::<f32>()?.1,
            );
            ensure!(
                a.iter()
                    .map(|v| v.to_bits())
                    .eq(b.iter().map(|v| v.to_bits())),
                "Folded model output differs on round {round}: {a:?} vs {b:?}"
            );
        }
        println!(
            "{}",
            serde_json::json!({
                "source": {"bytes": std::fs::metadata(&source)?.len(), "sha256": sha256(&source)?},
                "folded": {"bytes": std::fs::metadata(&output)?.len(), "sha256": sha256(&output)?},
                "onnxruntime": ort::info(),
                "identical_outputs": true,
            })
        );
        Ok(())
    }

    fn session(path: &Path) -> Result<Session> {
        Ok(Session::builder()?
            .with_intra_threads(1)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .commit_from_file(path)?)
    }

    fn sha256(path: &Path) -> Result<String> {
        use std::io::Read;
        let (mut hash, mut file, mut buffer) =
            (Sha256::new(), std::fs::File::open(path)?, vec![0; 1 << 20]);
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hash.update(&buffer[..n]);
        }
        Ok(hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }
}
