//! One lazy native model registry, shared by every workflow.
#[cfg(feature = "onnx")]
use super::model::Fingerprint;
use super::pipeline::Asset;
use super::processing::{Backend, Control, RunConfig};
use anyhow::{Context, Result};
use image::{DynamicImage, imageops::FilterType};
use std::path::{Path, PathBuf};
#[cfg(feature = "onnx")]
mod execution;

#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RotationSuggestion {
    pub turns: u8,
    /// Minimum softmax score among agreeing views, not a calibrated probability.
    pub confidence: f32,
    pub uncertain: bool,
    pub views: u8,
}
impl RotationSuggestion {
    pub fn valid(&self) -> bool {
        self.turns < 4
            && (0.0..=1.0).contains(&self.confidence)
            && match self.views {
                1 => self.turns == 0 && !self.uncertain,
                2 => self.turns != 0 && !self.uncertain,
                3 => !self.uncertain || self.turns != 0,
                _ => false,
            }
    }
}
pub struct Models {
    cpu_threads: usize,
    compiled_cache: Option<PathBuf>,
    backend: Backend,
    processing: Control,
    cancellation: Option<std::sync::Arc<dyn Fn() -> bool + Send + Sync>>,
    #[cfg(feature = "onnx")]
    sessions: std::collections::HashMap<
        &'static str,
        (PathBuf, Fingerprint, execution::NativeSession, Backend),
    >,
    pub initializations: usize,
    pub inferences: u64,
}
impl Default for Models {
    fn default() -> Self {
        Self {
            cpu_threads: super::processing::Preferences::default()
                .threads(super::processing::available()),
            compiled_cache: None,
            backend: Backend::default(),
            processing: Control::default(),
            cancellation: None,
            #[cfg(feature = "onnx")]
            sessions: Default::default(),
            initializations: 0,
            inferences: 0,
        }
    }
}
impl Models {
    pub fn with_processing(processing: Control) -> Self {
        Self {
            processing,
            ..Self::default()
        }
    }
    /// Prepared models live in `root` regardless of the disk limit. Clearing
    /// releases the sessions that hold them open so the copies can go.
    pub(crate) fn configure_compiled_cache(&mut self, root: PathBuf, clear: bool) {
        let release = clear || self.compiled_cache.as_ref().is_some_and(|old| old != &root);
        if release {
            #[cfg(feature = "onnx")]
            self.sessions.retain(|kind, (_, _, session, _)| {
                if session.cached() {
                    self.processing.clear_model_status(kind);
                    false
                } else {
                    true
                }
            });
        }
        self.compiled_cache = Some(root);
    }
    pub fn begin_run(
        &mut self,
        config: RunConfig,
        cancelled: impl Fn() -> bool + Send + Sync + 'static,
    ) {
        self.configure(config);
        self.cancellation = Some(std::sync::Arc::new(cancelled));
    }
    pub fn configure(&mut self, config: RunConfig) -> bool {
        self.cancellation = None;
        let changed = self.backend != config.backend;
        if changed {
            #[cfg(feature = "onnx")]
            self.sessions.clear();
            self.processing.clear_status();
            self.backend = config.backend;
        }
        self.configure_threads(config.threads) || changed
    }

    pub fn cpu_threads(&self) -> usize {
        self.cpu_threads
    }
    /// Called only by the image owner at the start of a new accepted run. Native
    /// sessions cannot change thread pools in place, so release them lazily here;
    /// decoded pixels, measurements and persisted model evidence stay reusable.
    pub fn configure_threads(&mut self, threads: usize) -> bool {
        let threads = threads.clamp(1, super::processing::available());
        if threads == self.cpu_threads {
            return false;
        }
        #[cfg(feature = "onnx")]
        self.sessions.clear();
        self.processing.clear_status();
        self.cpu_threads = threads;
        true
    }

    pub(crate) fn infer_all(
        &mut self,
        kind: &'static str,
        path: &Path,
        size: usize,
        input: Vec<f32>,
    ) -> Result<Vec<Vec<f32>>> {
        self.infer_batch_all(kind, path, size, 1, input, &|| false)
    }
    #[cfg(feature = "onnx")]
    fn infer_batch_all(
        &mut self,
        kind: &'static str,
        path: &Path,
        size: usize,
        batch: usize,
        input: Vec<f32>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Vec<f32>>> {
        let run_cancellation = self.cancellation.clone();
        let cancelled = || cancelled() || run_cancellation.as_ref().is_some_and(|check| check());
        anyhow::ensure!(!cancelled(), "Inference cancelled");
        anyhow::ensure!(
            (1..=5).contains(&batch),
            "Native image batch must contain 1–5 inputs"
        );
        let stride = size
            .checked_mul(size)
            .and_then(|n| n.checked_mul(3))
            .context("Image tensor dimensions overflow")?;
        anyhow::ensure!(
            stride > 0 && stride.checked_mul(batch) == Some(input.len()),
            "Image tensor length mismatch"
        );
        let fingerprint = self.ensure_session(kind, path, &cancelled)?;
        let dynamic_batch = self.sessions.get(kind).is_some_and(|(_, _, session, _)| {
            matches!(session.inputs()[0].dtype(), ort::value::ValueType::Tensor { shape, .. }
                if shape.len() == 4 && shape[0] == -1)
        });
        if batch > 1 && !dynamic_batch {
            // A fixed-one export remains usable through this same registry. Do
            // not attempt a failing native batch and silently retry model errors.
            let mut combined: Vec<Vec<f32>> = Vec::new();
            for image in input.chunks_exact(stride) {
                anyhow::ensure!(!cancelled(), "Inference cancelled");
                let outputs = self.infer_all(kind, path, size, image.to_vec())?;
                if combined.is_empty() {
                    combined = outputs;
                } else {
                    anyhow::ensure!(
                        combined.len() == outputs.len(),
                        "Model output count changed"
                    );
                    for (all, values) in combined.iter_mut().zip(outputs) {
                        all.extend(values);
                    }
                }
            }
            return Ok(combined);
        }
        anyhow::ensure!(!cancelled(), "Inference cancelled");
        let tensor =
            ort::value::Tensor::from_array(([batch, 3, size, size], input.into_boxed_slice()))?;
        self.run_with_fallback(kind, path, fingerprint, tensor, batch, &cancelled)
    }
    #[cfg(feature = "onnx")]
    /// Points input `[1, count, 2]`, such as Face Mesh landmarks for blendshapes.
    pub(crate) fn infer_points(
        &mut self,
        kind: &'static str,
        path: &Path,
        points: Vec<f32>,
    ) -> Result<Vec<Vec<f32>>> {
        let run_cancellation = self.cancellation.clone();
        let cancelled = || run_cancellation.as_ref().is_some_and(|check| check());
        anyhow::ensure!(!cancelled(), "Inference cancelled");
        anyhow::ensure!(
            !points.is_empty() && points.len().is_multiple_of(2),
            "Point tensor length mismatch"
        );
        let fingerprint = self.ensure_session(kind, path, &cancelled)?;
        let tensor =
            ort::value::Tensor::from_array(([1, points.len() / 2, 2], points.into_boxed_slice()))?;
        self.run_with_fallback(kind, path, fingerprint, tensor, 1, &cancelled)
    }
    #[cfg(feature = "onnx")]
    fn ensure_session(
        &mut self,
        kind: &'static str,
        path: &Path,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Fingerprint> {
        let fingerprint = Fingerprint::read(path)?;
        if self
            .sessions
            .get(kind)
            .is_none_or(|(p, f, _, _)| p != path || *f != fingerprint)
        {
            let runtime = runtime_path()
                .context("Install ONNX Runtime or set ORT_DYLIB_PATH to its native library")?;
            anyhow::ensure!(
                runtime.is_file(),
                "ONNX Runtime library is missing: {}",
                runtime.display()
            );
            ort::init_from(&runtime)?
                .with_name("PhotoSort")
                .with_telemetry(false)
                .commit();
            self.sessions.remove(kind);
            self.processing.preparing(kind, self.backend);
            let processing = self.processing.clone();
            let (session, backend, detail) = match execution::create(
                path,
                kind,
                self.cpu_threads,
                self.backend,
                cancelled,
                self.compiled_cache.as_ref(),
                &|backend, step| processing.step(kind, backend, step),
            ) {
                Ok(created) => created,
                Err(error) => {
                    self.processing.report(
                        kind,
                        self.backend,
                        format!("Initialization stopped: {error}"),
                    );
                    return Err(error);
                }
            };
            self.processing.report(kind, backend, detail);
            self.sessions
                .insert(kind, (path.to_owned(), fingerprint, session, backend));
            self.initializations += 1;
        }
        Ok(fingerprint)
    }
    #[cfg(feature = "onnx")]
    /// One attempt on the configured provider, then a single CPU retry.
    fn run_with_fallback(
        &mut self,
        kind: &'static str,
        path: &Path,
        fingerprint: Fingerprint,
        tensor: ort::value::Tensor<f32>,
        batch: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Vec<f32>>> {
        let first = execution::run(
            &mut self
                .sessions
                .get_mut(kind)
                .context("Model session unavailable")?
                .2,
            &tensor,
            batch,
        );
        self.inferences += 1;
        anyhow::ensure!(!cancelled(), "Inference cancelled");
        if first.is_ok() || self.sessions[kind].3 == Backend::Cpu {
            return first;
        }
        let error = first.unwrap_err();
        self.sessions.get_mut(kind).unwrap().2.invalidate();
        // Drop the failed provider before allocating the CPU session. Keep the
        // prepared tensor and pixels; a failing accelerator is tried only once.
        self.sessions.remove(kind);
        self.processing.report(
            kind,
            Backend::Cpu,
            format!("CPU fallback after inference error: {error}"),
        );
        let (mut session, _, _) = execution::create(
            path,
            kind,
            self.cpu_threads,
            Backend::Cpu,
            cancelled,
            None,
            &|_, _| {},
        )?;
        self.initializations += 1;
        let result = execution::run(&mut session, &tensor, batch);
        self.inferences += 1;
        self.sessions
            .insert(kind, (path.to_owned(), fingerprint, session, Backend::Cpu));
        anyhow::ensure!(!cancelled(), "Inference cancelled");
        result
    }
    #[cfg(not(feature = "onnx"))]
    pub(crate) fn infer_points(
        &mut self,
        _kind: &'static str,
        _path: &Path,
        _points: Vec<f32>,
    ) -> Result<Vec<Vec<f32>>> {
        anyhow::bail!("This build has no ONNX support")
    }
    #[cfg(not(feature = "onnx"))]
    fn infer_batch_all(
        &mut self,
        _kind: &'static str,
        _path: &Path,
        _size: usize,
        _batch: usize,
        _input: Vec<f32>,
        _cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Vec<f32>>> {
        anyhow::bail!("This build has no ONNX support")
    }
    fn infer(
        &mut self,
        kind: &'static str,
        path: &Path,
        size: usize,
        input: Vec<f32>,
    ) -> Result<Vec<f32>> {
        Ok(self.infer_all(kind, path, size, input)?.remove(0))
    }
    pub fn faces(&mut self, asset: &Asset) -> Result<super::faces::Faces> {
        super::faces::analyze(self, asset)
    }
    pub fn rotation(&mut self, path: &Path, asset: &Asset) -> Result<RotationSuggestion> {
        self.rotation_cancellable(path, asset, &|| false)
    }
    pub fn rotation_cancellable(
        &mut self,
        path: &Path,
        asset: &Asset,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<RotationSuggestion> {
        anyhow::ensure!(!cancelled(), "Orientation cancelled");
        // Prepare once. Cross-checks rotate the small normalized tensor, never
        // re-decode, resize or normalize the source, and reuse this registry.
        let input = orientation_input(asset);
        orientation_consensus(cancelled, |turns| {
            let values = self
                .infer_batch_all(
                    "orientation",
                    path,
                    384,
                    1,
                    rotate_orientation_tensor(&input, 384, turns),
                    cancelled,
                )?
                .remove(0);
            orientation_vote(&values)
        })
    }
    fn embedding_tokens(&mut self, asset: &Asset) -> Result<Vec<f32>> {
        let path = model_path("dinov2-small.onnx")
            .context("DINOv2 is not installed in resources/models")?;
        let values = self.infer("dinov2", &path, 224, embedding_input(asset))?;
        anyhow::ensure!(
            values.len() >= 384 && values.len().is_multiple_of(384),
            "DINOv2-small must return 384-dimensional tokens"
        );
        Ok(values)
    }
    pub fn embedding(&mut self, asset: &Asset) -> Result<Vec<f32>> {
        let tokens = self.embedding_tokens(asset)?;
        Ok(super::subjects::normalized(&tokens[..384]))
    }
    /// One shared DINO inference supplies both CLS and pooled spatial evidence.
    /// No additional image decode or second model instance is created here.
    pub fn dense_embedding(&mut self, asset: &Asset) -> Result<super::subjects::DenseEmbedding> {
        let tokens = self.embedding_tokens(asset)?;
        anyhow::ensure!(
            tokens.len() == 257 * 384,
            "DINOv2-small dense output must contain CLS plus a 16x16 patch grid"
        );
        super::subjects::DenseEmbedding::from_tokens(&tokens, 384)
    }
    /// The shared whole-image inference supplies region zero. Five crops use a
    /// bounded native batch through the existing DINO session and canonical pixels.
    /// Fixed-one exports fall back to individual calls without a second session.
    pub fn similarity_regions(
        &mut self,
        asset: &Asset,
        global: &[f32],
        cancelled: impl Fn() -> bool,
    ) -> Result<super::regional::Embedding> {
        anyhow::ensure!(
            global.len() == 384 && global.iter().all(|x| x.is_finite()),
            "Invalid global embedding"
        );
        let boxes = super::regional::crop_boxes(asset.pixels.width(), asset.pixels.height());
        let batch = boxes.len();
        if batch == 0 {
            anyhow::ensure!(!cancelled(), "Regional extraction cancelled");
            return Ok(super::regional::Embedding {
                vectors: vec![global.to_vec()],
            });
        }
        let mut input = Vec::with_capacity(batch * 3 * 224 * 224);
        for [left, top, right, bottom] in boxes {
            anyhow::ensure!(!cancelled(), "Regional extraction cancelled");
            let pixels = std::sync::Arc::new(
                image::imageops::crop_imm(&*asset.pixels, left, top, right - left, bottom - top)
                    .to_image(),
            );
            let crop = Asset {
                decoder_warning: asset.decoder_warning.clone(),
                pixels: pixels.clone(),
                thumbnail: pixels,
                metadata: asset.metadata.clone(),
            };
            input.extend(embedding_input(&crop));
        }
        anyhow::ensure!(!cancelled(), "Regional extraction cancelled");
        let path = model_path("dinov2-small.onnx")
            .context("DINOv2 is not installed in resources/models")?;
        let tokens = self
            .infer_batch_all("dinov2", &path, 224, batch, input, &cancelled)?
            .remove(0);
        anyhow::ensure!(!cancelled(), "Regional extraction cancelled");
        anyhow::ensure!(
            tokens.len().is_multiple_of(batch * 384),
            "Invalid batched scene tokens"
        );
        let stride = tokens.len() / batch;
        anyhow::ensure!(stride >= 384, "Empty batched scene tokens");
        let mut vectors = Vec::with_capacity(batch + 1);
        vectors.push(global.to_vec());
        vectors.extend(
            tokens
                .chunks_exact(stride)
                .map(|tokens| super::subjects::normalized(&tokens[..384])),
        );
        Ok(super::regional::Embedding { vectors })
    }
    pub fn aesthetic(&mut self, asset: &Asset) -> Result<f32> {
        let path = model_path("aesthetic.onnx")
            .context("BeIT aesthetic model is not installed in resources/models")?;
        let image = DynamicImage::ImageRgba8(image::imageops::resize(
            &*asset.pixels,
            384,
            384,
            FilterType::CatmullRom,
        ));
        let values = self.infer("aesthetic", &path, 384, nchw(&image, [0.5; 3], [0.5; 3]))?;
        anyhow::ensure!(
            values.len() == 2,
            "BeIT aesthetic model must return [not_aesthetic, aesthetic] logits"
        );
        let max = values[0].max(values[1]);
        Ok((values[1] - max).exp() / values.iter().map(|v| (*v - max).exp()).sum::<f32>())
    }
}
fn embedding_input(asset: &Asset) -> Vec<f32> {
    let (w, h) = asset.pixels.dimensions();
    let scale = 256.0 / w.min(h) as f64;
    let resized = image::imageops::resize(
        &*asset.pixels,
        (f64::from(w) * scale).round() as u32,
        (f64::from(h) * scale).round() as u32,
        FilterType::CatmullRom,
    );
    let crop = image::imageops::crop_imm(
        &resized,
        (resized.width() - 224) / 2,
        (resized.height() - 224) / 2,
        224,
        224,
    )
    .to_image();
    nchw(
        &DynamicImage::ImageRgba8(crop),
        [0.485, 0.456, 0.406],
        [0.229, 0.224, 0.225],
    )
}
fn nchw(image: &DynamicImage, mean: [f32; 3], std: [f32; 3]) -> Vec<f32> {
    let image = image.to_rgb8();
    let area = (image.width() * image.height()) as usize;
    let mut output = vec![0.0; 3 * area];
    for (i, pixel) in image.pixels().enumerate() {
        for c in 0..3 {
            output[c * area + i] = (f32::from(pixel[c]) / 255.0 - mean[c]) / std[c];
        }
    }
    output
}
fn orientation_vote(values: &[f32]) -> Result<(u8, f32)> {
    anyhow::ensure!(
        values.len() == 4 && values.iter().all(|v| v.is_finite()),
        "Orientation model must return four finite logits"
    );
    let (index, &max) = values
        .iter()
        .enumerate()
        .max_by(|a, b| a.1.total_cmp(b.1))
        .context("Empty model output")?;
    Ok((
        index as u8,
        1.0 / values.iter().map(|v| (*v - max).exp()).sum::<f32>(),
    ))
}

/// Evaluate at most three views. Upright predictions use one inference; proposed
/// corrections need two agreeing votes expressed in the original coordinates.
fn orientation_consensus(
    cancelled: &dyn Fn() -> bool,
    mut infer: impl FnMut(u8) -> Result<(u8, f32)>,
) -> Result<RotationSuggestion> {
    let mut votes = Vec::with_capacity(3);
    for input_turns in [0, 2, 1] {
        anyhow::ensure!(!cancelled(), "Orientation cancelled");
        let (turns, score) = infer(input_turns)?;
        anyhow::ensure!(!cancelled(), "Orientation cancelled");
        let mapped = (turns + input_turns) % 4;
        votes.push((mapped, score));
        if votes.len() == 1 && mapped == 0 {
            return Ok(RotationSuggestion {
                turns: 0,
                confidence: score,
                uncertain: false,
                views: 1,
            });
        }
        if votes.iter().filter(|(vote, _)| *vote == mapped).count() >= 2 {
            let confidence = votes
                .iter()
                .filter(|(vote, _)| *vote == mapped)
                .map(|(_, score)| *score)
                .fold(1.0, f32::min);
            return Ok(RotationSuggestion {
                turns: mapped,
                confidence,
                uncertain: false,
                views: votes.len() as u8,
            });
        }
    }
    // Keep the first candidate available for inspection, but do not select or
    // bulk-confirm it: contradictory evidence requires an individual decision.
    Ok(RotationSuggestion {
        turns: votes[0].0,
        confidence: votes[0].1,
        uncertain: true,
        views: 3,
    })
}

fn rotate_orientation_tensor(input: &[f32], size: usize, turns: u8) -> Vec<f32> {
    debug_assert_eq!(input.len(), 3 * size * size);
    if turns.is_multiple_of(4) {
        return input.to_vec();
    }
    let mut output = vec![0.0; input.len()];
    for c in 0..3 {
        for y in 0..size {
            for x in 0..size {
                let (dx, dy) = match turns % 4 {
                    0 => (x, y),
                    1 => (size - 1 - y, x),
                    2 => (size - 1 - x, size - 1 - y),
                    _ => (y, size - 1 - x),
                };
                output[c * size * size + dy * size + dx] = input[c * size * size + y * size + x];
            }
        }
    }
    output
}

/// Model input: RGB, 416 square, center crop 384, ImageNet NCHW.
pub fn orientation_input(asset: &Asset) -> Vec<f32> {
    let image = DynamicImage::ImageRgba8(image::imageops::resize(
        &*asset.pixels,
        416,
        416,
        FilterType::Triangle,
    ))
    .crop_imm(16, 16, 384, 384);
    nchw(&image, [0.485, 0.456, 0.406], [0.229, 0.224, 0.225])
}
pub fn resource_roots() -> Vec<PathBuf> {
    let mut paths = vec![super::session::data_dir()];
    if let Ok(cwd) = std::env::current_dir() {
        paths.push(cwd.join("resources"));
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        paths.push(parent.join("../Resources"));
        paths.push(parent.join("../../resources"));
    }
    paths
}
pub fn model_path(name: &str) -> Option<PathBuf> {
    resource_roots()
        .into_iter()
        .map(|p| p.join("models").join(name))
        .find(|p| p.is_file())
}
pub fn runtime_path() -> Option<PathBuf> {
    std::env::var_os("ORT_DYLIB_PATH")
        .map(PathBuf::from)
        .or_else(|| {
            let name = if cfg!(target_os = "macos") {
                "libonnxruntime.dylib"
            } else if cfg!(target_os = "windows") {
                "onnxruntime.dll"
            } else {
                "libonnxruntime.so"
            };
            resource_roots()
                .into_iter()
                .map(|p| p.join("runtime").join(name))
                .chain([
                    PathBuf::from("/opt/homebrew/lib/libonnxruntime.dylib"),
                    PathBuf::from("/usr/local/lib/libonnxruntime.so"),
                ])
                .find(|p| p.is_file())
        })
}
pub fn orientation_model() -> Option<PathBuf> {
    std::env::var_os("PHOTOSORT_ROTATION_MODEL")
        .map(PathBuf::from)
        .or_else(|| model_path("orientation.onnx"))
}

/// Analysis descriptors must be invalidated when any installed model changes.
/// Subject records embed face geometry, so the face recipe is part of it too.
pub fn analysis_signature() -> String {
    format!(
        "{}:{}",
        super::faces::RECIPE,
        model_signature(&[
            "dinov2-small.onnx",
            "aesthetic.onnx",
            "face_detector.onnx",
            "face_landmarks_detector.onnx",
        ])
    )
}
pub fn aesthetic_signature() -> String {
    model_signature(&["aesthetic.onnx"])
}
pub fn recognition_signature() -> String {
    format!(
        "{}:{}",
        super::face_groups::RECIPE,
        model_signature(&[super::face_groups::MODEL])
    )
}
pub fn face_signature() -> String {
    format!(
        "{}:{}",
        super::faces::RECIPE,
        model_signature(&[
            "face_detector.onnx",
            "face_landmarks_detector.onnx",
            "face_blendshapes.onnx",
        ])
    )
}
fn model_signature(names: &[&str]) -> String {
    let mut hash = blake3::Hasher::new();
    for name in names {
        if let Some(path) = model_path(name) {
            hash.update(path.as_os_str().as_encoded_bytes());
            hash.update(format!("{:?}", super::model::Fingerprint::read(&path).ok()).as_bytes());
        }
    }
    hash.finalize().to_hex().to_string()
}

#[cfg(all(test, feature = "onnx"))]
mod batch_probe {
    use super::*;
    #[test]
    #[ignore = "runs the installed native DINO model; PHOTOSORT_BATCH_WIDTH chooses 1, 2 or 5"]
    fn scene_batch_timing_and_equivalence() -> Result<()> {
        let batch: usize = std::env::var("PHOTOSORT_BATCH_WIDTH")
            .unwrap_or_else(|_| "1".into())
            .parse()?;
        anyhow::ensure!([1, 2, 5].contains(&batch), "Use batch width 1, 2 or 5");
        let runtime = runtime_path().context("Native runtime required")?;
        let path = model_path("dinov2-small.onnx").context("Native scene model required")?;
        ort::init_from(&runtime)?
            .with_name("PhotoSort batch measurement")
            .with_telemetry(false)
            .commit();
        let threads = Models::default().cpu_threads();
        let mut session = ort::session::Session::builder()?
            .with_intra_threads(threads)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .with_parallel_execution(false)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .with_intra_op_spinning(false)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .with_inter_op_spinning(false)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?
            .commit_from_file(path)?;
        assert_eq!(session.inputs().len(), 1);
        let image_values = 3 * 224 * 224;
        let inputs: Vec<Vec<f32>> = (0..5)
            .map(|sample| {
                (0..image_values)
                    .map(|i| ((i * 31 + sample * 17) % 256) as f32 / 128.0 - 1.0)
                    .collect()
            })
            .collect();
        let mut reference = Vec::new();
        for input in &inputs {
            let tensor = ort::value::Tensor::from_array((
                [1usize, 3, 224, 224],
                input.clone().into_boxed_slice(),
            ))?;
            let outputs = session.run(ort::inputs![tensor])?;
            let (shape, values) = outputs[0].try_extract_tensor::<f32>()?;
            assert_eq!(&shape[..], &[1, 257, 384]);
            reference.push(values.to_vec());
        }
        let mut elapsed_ms = Vec::new();
        let mut maximum_error = 0.0f32;
        for round in 0..5 {
            let start = std::time::Instant::now();
            let mut results = Vec::new();
            for chunk in inputs.chunks(batch) {
                let packed: Vec<_> = chunk.iter().flatten().copied().collect();
                let tensor = ort::value::Tensor::from_array((
                    [chunk.len(), 3, 224, 224],
                    packed.into_boxed_slice(),
                ))?;
                let outputs = session.run(ort::inputs![tensor])?;
                let (shape, values) = outputs[0].try_extract_tensor::<f32>()?;
                assert_eq!(&shape[..], &[chunk.len() as i64, 257, 384]);
                results.extend(
                    values
                        .as_chunks::<{ 257 * 384 }>()
                        .0
                        .iter()
                        .map(|v| v.to_vec()),
                );
            }
            if round > 0 {
                elapsed_ms.push(start.elapsed().as_secs_f64() * 1000.0);
            }
            assert_eq!(results.len(), 5);
            for (actual, expected) in results.iter().zip(&reference) {
                for (a, b) in actual.iter().zip(expected) {
                    assert!(a.is_finite());
                    maximum_error = maximum_error.max((a - b).abs());
                }
            }
        }
        anyhow::ensure!(
            maximum_error <= 0.001,
            "Batch numerical drift: {maximum_error}"
        );
        println!(
            "{}",
            serde_json::json!({
                "batch": batch, "images": 5, "threads": threads,
                "calls_per_pass": 5usize.div_ceil(batch), "elapsed_ms": elapsed_ms,
                "maximum_token_error": maximum_error,
            })
        );
        Ok(())
    }
}

#[cfg(all(test, feature = "onnx"))]
mod regional_batch_tests {
    use super::*;
    #[test]
    fn batch_bounds_and_lengths_fail_before_model_io() {
        let mut models = Models::default();
        for (size, batch, input) in [
            (224, 0, vec![]),
            (224, 6, vec![]),
            (0, 2, vec![]),
            (usize::MAX, 2, vec![]),
            (224, 2, vec![0.0; 3]),
        ] {
            assert!(
                models
                    .infer_batch_all(
                        "test",
                        Path::new("/missing/model.onnx"),
                        size,
                        batch,
                        input,
                        &|| false
                    )
                    .is_err()
            );
        }
        assert_eq!(models.initializations, 0);
        assert_eq!(models.inferences, 0);
    }
    #[test]
    #[ignore = "runs installed face detector export to verify fixed-one fallback and cancellation"]
    fn fixed_one_export_reuses_registry_and_honors_cancellation() -> Result<()> {
        let path = model_path("face_detector.onnx").context("Face detector required")?;
        let mut models = Models::default();
        let image = vec![0.0; 3 * 128 * 128];
        let reference = models.infer_all("face_detector", &path, 128, image.clone())?;
        let shape = models.sessions["face_detector"].2.inputs()[0].dtype();
        anyhow::ensure!(
            matches!(shape, ort::value::ValueType::Tensor { shape, .. } if shape[0]==1),
            "Expected fixed-one fixture"
        );
        let combined =
            models.infer_batch_all("face_detector", &path, 128, 2, image.repeat(2), &|| false)?;
        assert_eq!(combined.len(), reference.len());
        for (all, original) in combined.iter().zip(&reference) {
            assert_eq!(*all, original.repeat(2));
        }
        assert_eq!(models.initializations, 1);
        assert_eq!(models.inferences, 3);
        let checks = std::cell::Cell::new(0);
        let before = models.inferences;
        assert!(
            models
                .infer_batch_all("face_detector", &path, 128, 2, image.repeat(2), &|| {
                    checks.set(checks.get() + 1);
                    checks.get() >= 3
                })
                .is_err()
        );
        assert!(
            models.inferences - before < 2,
            "Cancellation must stop remaining individual calls"
        );
        assert_eq!(models.initializations, 1);
        Ok(())
    }
    #[test]
    #[ignore = "compares single and batched regional embeddings on all seven PHOTOSORT_SAMPLE_DIR photos"]
    fn seven_samples_match_individual_crops_with_one_batched_forward() -> Result<()> {
        use crate::core::{catalog::Catalog, operations, pipeline::Pipeline};
        let root = std::path::PathBuf::from(
            std::env::var_os("PHOTOSORT_SAMPLE_DIR").context("Sample directory required")?,
        );
        let catalog = Catalog::scan(&root, || false, |_| {})?;
        assert_eq!(catalog.media.len(), 7);
        let cache = tempfile::tempdir()?;
        let mut pipeline = Pipeline::new(128 * 1024 * 1024, cache.path().into());
        let mut models = Models::default();
        let mut total_single = 0.0;
        let mut total_batch = 0.0;
        for media in &catalog.media {
            let before = operations::content_hash(&media.path, || false)?;
            let asset = pipeline.analysis(media, || false)?;
            let global = models.embedding(&asset)?;
            let start = std::time::Instant::now();
            let mut individual = vec![global.clone()];
            for [left, top, right, bottom] in
                crate::core::regional::crop_boxes(asset.pixels.width(), asset.pixels.height())
            {
                let pixels = std::sync::Arc::new(
                    image::imageops::crop_imm(
                        &*asset.pixels,
                        left,
                        top,
                        right - left,
                        bottom - top,
                    )
                    .to_image(),
                );
                individual.push(models.embedding(&Asset {
                    decoder_warning: asset.decoder_warning.clone(),
                    pixels: pixels.clone(),
                    thumbnail: pixels,
                    metadata: asset.metadata.clone(),
                })?);
            }
            total_single += start.elapsed().as_secs_f64();
            let calls = models.inferences;
            let decodes = pipeline.stats.decodes;
            let start = std::time::Instant::now();
            let batched = models.similarity_regions(&asset, &global, || false)?;
            total_batch += start.elapsed().as_secs_f64();
            assert_eq!(models.inferences - calls, 1);
            assert_eq!(models.initializations, 1);
            assert_eq!(pipeline.stats.decodes, decodes);
            assert_eq!(
                batched.vectors,
                individual,
                "Batched descriptors changed for {}",
                media.name()
            );
            assert_eq!(operations::content_hash(&media.path, || false)?, before);
        }
        println!(
            "{}",
            serde_json::json!({"samples":7,"single_crop_seconds":total_single,"batched_crop_seconds":total_batch,"model_initializations":models.initializations})
        );
        Ok(())
    }
}

#[cfg(test)]
mod orientation_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn orientation_consensus_fast_path_agreement_and_ambiguous_votes() {
        for initial in 0..4 {
            for second in 0..4 {
                for third in 0..4 {
                    let mapped = [initial, second, third];
                    let mut calls = Vec::new();
                    let suggestion = orientation_consensus(&|| false, |turns| {
                        let i = calls.len();
                        calls.push(turns);
                        Ok(((mapped[i] + 4 - turns) % 4, [0.9, 0.8, 0.7][i]))
                    })
                    .unwrap();
                    if initial == 0 {
                        assert_eq!(calls, [0]);
                        assert_eq!(suggestion.turns, 0);
                    } else if second == initial {
                        assert_eq!(calls, [0, 2]);
                        assert_eq!(suggestion.turns, initial);
                        assert_eq!(suggestion.confidence, 0.8);
                    } else {
                        assert_eq!(calls, [0, 2, 1]);
                        if third == initial || third == second {
                            assert_eq!(suggestion.turns, third);
                            assert_eq!(suggestion.confidence, 0.7);
                        } else {
                            assert_eq!(suggestion.turns, initial);
                            assert!(suggestion.uncertain);
                        }
                    }
                    assert_eq!(suggestion.views as usize, calls.len());
                    assert_eq!(
                        suggestion.uncertain,
                        initial != 0 && initial != second && third != initial && third != second
                    );
                }
            }
        }
    }

    #[test]
    fn orientation_cancellation_never_returns_partial_consensus() {
        for cancel_after in 0..=3 {
            let calls = Cell::new(0);
            let result = orientation_consensus(&|| calls.get() >= cancel_after, |turns| {
                calls.set(calls.get() + 1);
                // Three distinct mapped votes exercise every cancellation boundary.
                Ok(((calls.get() + 4 - turns) % 4, 0.9))
            });
            assert!(result.unwrap_err().to_string().contains("cancelled"));
            assert_eq!(calls.get(), cancel_after);
        }
        let mut models = Models::default();
        let pixels = std::sync::Arc::new(image::RgbaImage::new(2, 2));
        let asset = Asset {
            decoder_warning: None,
            pixels: pixels.clone(),
            thumbnail: pixels,
            metadata: Default::default(),
        };
        assert!(
            models
                .rotation_cancellable(Path::new("missing.onnx"), &asset, &|| true)
                .is_err()
        );
        assert_eq!((models.initializations, models.inferences), (0, 0));
    }

    #[test]
    fn orientation_tensor_rotates_spatial_axes_without_mixing_channels() {
        let input: Vec<_> = (0..27).map(|i| i as f32).collect();
        let once = rotate_orientation_tensor(&input, 3, 1);
        assert_eq!(&once[..9], &[6., 3., 0., 7., 4., 1., 8., 5., 2.]);
        for turns in 0..4 {
            let rotated = rotate_orientation_tensor(&input, 3, turns);
            assert_eq!(rotate_orientation_tensor(&rotated, 3, 4 - turns), input);
            for c in 0..3 {
                assert!(
                    rotated[c * 9..(c + 1) * 9]
                        .iter()
                        .all(|v| *v >= (c * 9) as f32 && *v < ((c + 1) * 9) as f32)
                );
            }
        }
    }

    #[test]
    fn orientation_vote_rejects_invalid_logits_and_handles_large_values() {
        for values in [
            vec![],
            vec![0.; 3],
            vec![f32::NAN; 4],
            vec![f32::INFINITY; 4],
        ] {
            assert!(orientation_vote(&values).is_err());
        }
        let (turns, score) = orientation_vote(&[10000., 10001., 9999., 9998.]).unwrap();
        assert_eq!(turns, 1);
        assert!((0.64..0.65).contains(&score));
    }
}

#[cfg(all(test, feature = "onnx"))]
#[path = "models/orientation_audit.rs"]
mod orientation_audit;
